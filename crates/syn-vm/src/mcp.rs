//! MCP-сервер (stdio, JSON-RPC 2.0): агенты (Claude Code и др.) управляют машинами
//! напрямую — запуск и выключение, команды и файлы в госте через гостевой агент, экран,
//! клавиатура и мышь через QMP, USB и другие устройства компьютера, снимки, настройки.
//!
//! Подключение: `claude mcp add syn-vm -- syn-vm --mcp`.

use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};

use crate::cli;
use crate::model;
use crate::{devices, qemu, qga, qmp};

const INSTRUCTIONS: &str = "syn-vm управляет виртуальными машинами QEMU/KVM этого компьютера. \
Машина — имя или id из `list`. Команды внутри гостя (`exec`, `read_file`, `write_file`, `push`, `pull`) идут через гостевой агент qemu-ga: \
в Linux-госте нужен пакет qemu-guest-agent, в Windows — «Гостевые инструменты» с диска virtio-win (`tools`). \
После `start` вызовите `wait_boot` — он ждёт, пока агент ответит. Без агента работают `screenshot`, `type_text`, `key`, `click` (через QMP): \
координаты click — пиксели последнего снимка `screenshot` этой машины. Устройства компьютера — `devices`, отдать/забрать — `attach`/`detach`.";

pub fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let reply = match dispatch(method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("{e:#}")}}),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

fn dispatch(method: &str, params: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "syn-vm", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(name, &args) {
                Ok(v) => v,
                Err(e) => json!({"content": [{"type": "text", "text": format!("ошибка: {e:#}")}], "isError": true}),
            })
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        other => bail!("метод {other} не поддерживается"),
    }
}

fn schema(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

fn tools() -> Value {
    let vm = json!({"type": "string", "description": "машина: имя или id"});
    let s = |d: &str| json!({"type": "string", "description": d});
    let n = |d: &str| json!({"type": "number", "description": d});
    json!([
        {"name": "list", "description": "Машины: id, имя, система, состояние (off/running/paused/busy).", "inputSchema": schema(json!({}), &[])},
        {"name": "status", "description": "Подробно о машине: состояние, гостевой агент, система гостя, адреса, проброшенные порты и устройства.",
         "inputSchema": schema(json!({"vm": vm}), &["vm"])},
        {"name": "start", "description": "Запустить машину без окна. wait_boot=true — дождаться гостевого агента.",
         "inputSchema": schema(json!({"vm": vm, "wait_boot": {"type": "boolean"}, "timeout_s": n("ожидание агента, с (300)")}), &["vm"])},
        {"name": "stop", "description": "Выключить: гостевым агентом или ACPI; force — принудительно, если не выключилась за timeout_s.",
         "inputSchema": schema(json!({"vm": vm, "force": {"type": "boolean"}, "timeout_s": n("ожидание, с (90)")}), &["vm"])},
        {"name": "power", "description": "Сброс, пауза, продолжение, мгновенное выключение.",
         "inputSchema": schema(json!({"vm": vm, "action": {"type": "string", "enum": ["reset", "pause", "resume", "kill"]}}), &["vm", "action"])},
        {"name": "wait_boot", "description": "Ждать, пока ответит гостевой агент (система загрузилась).",
         "inputSchema": schema(json!({"vm": vm, "timeout_s": n("тайм-аут, с (300)")}), &["vm"])},
        {"name": "exec", "description": "Выполнить команду в госте через гостевой агент: command — строка оболочки гостя (sh -c в Linux, cmd /c в Windows) или argv — массив без оболочки. Возвращает код, stdout, stderr.",
         "inputSchema": schema(json!({"vm": vm, "command": s("строка оболочки"), "argv": {"type": "array", "items": {"type": "string"}},
             "stdin": s("ввод программы"), "timeout_s": n("тайм-аут, с (600)")}), &["vm"])},
        {"name": "read_file", "description": "Прочитать текстовый файл гостя (до 1 МБ).", "inputSchema": schema(json!({"vm": vm, "path": s("путь в госте")}), &["vm", "path"])},
        {"name": "write_file", "description": "Записать текстовый файл в госте.", "inputSchema": schema(json!({"vm": vm, "path": s("путь в госте"), "content": {"type": "string"}}), &["vm", "path", "content"])},
        {"name": "push", "description": "Скопировать файл с этого компьютера в гостя.", "inputSchema": schema(json!({"vm": vm, "local": s("путь здесь"), "path": s("путь в госте")}), &["vm", "local", "path"])},
        {"name": "pull", "description": "Скопировать файл из гостя на этот компьютер.", "inputSchema": schema(json!({"vm": vm, "path": s("путь в госте"), "local": s("путь здесь")}), &["vm", "path", "local"])},
        {"name": "screenshot", "description": "Снимок экрана гостя (PNG). Координаты для click — пиксели этого снимка (в исходном разрешении гостя, даже если снимок уменьшен).",
         "inputSchema": schema(json!({"vm": vm, "max_size": n("длинная сторона, px (1280; 0 — без уменьшения)"), "save": s("ещё и сохранить PNG сюда")}), &["vm"])},
        {"name": "type_text", "description": "Набрать текст клавишами (раскладка US; \\n — Enter). Кириллицу и длинный текст лучше передавать через exec/write_file.",
         "inputSchema": schema(json!({"vm": vm, "text": {"type": "string"}}), &["vm", "text"])},
        {"name": "key", "description": "Нажать сочетание: ctrl+alt+delete, alt+f4, win+r, enter, esc, tab, f12…",
         "inputSchema": schema(json!({"vm": vm, "combo": {"type": "string"}}), &["vm", "combo"])},
        {"name": "click", "description": "Щелчок мышью в точке снимка screenshot (button: left/right/middle/none — только переместить).",
         "inputSchema": schema(json!({"vm": vm, "x": n("X"), "y": n("Y"), "button": {"type": "string", "enum": ["left", "right", "middle", "none"]}, "double": {"type": "boolean"}}), &["vm", "x", "y"])},
        {"name": "scroll", "description": "Колесо мыши: steps > 0 — вниз.", "inputSchema": schema(json!({"vm": vm, "steps": n("шаги")}), &["vm", "steps"])},
        {"name": "devices", "description": "Устройства компьютера, которые можно отдать гостю (PCI/VFIO, клавиатуры и мыши, диски, последовательные порты, USB по VID:PID и по порту) — с причинами, если нельзя.",
         "inputSchema": schema(json!({}), &[])},
        {"name": "attach", "description": "Отдать устройство гостю (на ходу, где QEMU умеет; PCI — при следующем запуске). spec: usb:3-6, usb-id:VID:PID, pci:02:00.0, input:/dev/input/by-id/…, block:/dev/sdb, block-ro:/dev/sdb1, serial:/dev/ttyUSB0.",
         "inputSchema": schema(json!({"vm": vm, "spec": {"type": "string"}}), &["vm", "spec"])},
        {"name": "detach", "description": "Вернуть устройство компьютеру (spec — как в attach).", "inputSchema": schema(json!({"vm": vm, "spec": {"type": "string"}}), &["vm", "spec"])},
        {"name": "settings", "description": "Прочитать настройки машины (TOML) или изменить: set — объект {ключ: значение} (cpus, memory_mb, shared_dir, forwards, guest_agent, tools_cd…). Вступают в силу при следующем запуске.",
         "inputSchema": schema(json!({"vm": vm, "set": {"type": "object"}}), &["vm"])},
        {"name": "create", "description": "Новая машина: из образа установки (iso) или импорт готового диска (disk, при UEFI — vars, при TPM — tpm). Возвращает id.",
         "inputSchema": schema(json!({"name": s("имя"), "os": {"type": "string", "enum": ["linux", "windows11", "windows10", "bsd", "android", "other"]},
             "iso": s("образ в привод"), "disk": s("готовый диск (остаётся на месте)"), "vars": s("переменные UEFI (raw/qcow2)"), "tpm": s("каталог состояния swtpm"),
             "cpus": n("ядра"), "memory_mb": n("память, МБ"), "disk_gb": n("размер нового диска, ГБ"),
             "forwards": {"type": "array", "items": {"type": "string"}, "description": "проброс портов: [udp:]ХОСТ:ГОСТЬ"},
             "set": {"type": "object", "description": "прочие настройки"}}), &["name"])},
        {"name": "cdrom", "description": "Вставить образ в привод (iso) или извлечь (пусто).", "inputSchema": schema(json!({"vm": vm, "iso": s("путь или пусто")}), &["vm"])},
        {"name": "tools", "description": "Диск «Гостевые инструменты» virtio-win (драйверы, qemu-ga, spice-vdagent для Windows): on/off — второй привод при запуске, download — скачать ISO.",
         "inputSchema": schema(json!({"vm": vm, "action": {"type": "string", "enum": ["on", "off", "download"]}}), &["vm"])},
        {"name": "snapshot", "description": "Снимки: list, create, restore, delete.",
         "inputSchema": schema(json!({"vm": vm, "action": {"type": "string", "enum": ["list", "create", "restore", "delete"]}, "name": s("имя снимка")}), &["vm", "action"])},
        {"name": "qmp", "description": "Сырая команда QMP (для того, чего нет в других инструментах).",
         "inputSchema": schema(json!({"vm": vm, "command": {"type": "string"}, "arguments": {"type": "object"}}), &["vm", "command"])},
    ])
}

fn text(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}]})
}

fn arg<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a.get(k).and_then(Value::as_str).with_context(|| format!("нужен параметр {k}"))
}

fn num(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(Value::as_f64)
}

fn clip(b: &[u8], max: usize) -> String {
    let s = String::from_utf8_lossy(b);
    if s.len() <= max {
        return s.into_owned();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("[… обрезано {cut} байт …]\n{}", &s[cut..])
}

fn running(vm: &model::Vm) -> Result<()> {
    if qemu::pid(vm).is_none() {
        bail!("«{}» не запущена — сначала start", vm.name);
    }
    Ok(())
}

fn call(name: &str, a: &Value) -> Result<Value> {
    let vm = || cli::find(arg(a, "vm")?);
    match name {
        "list" => {
            let v: Vec<Value> = model::load_all().iter().map(|m| json!({"id": m.id, "name": m.name, "os": m.os, "state": cli::power(m)})).collect();
            Ok(text(serde_json::to_string_pretty(&v)?))
        }
        "status" => Ok(text(serde_json::to_string_pretty(&cli::status(&vm()?))?)),
        "start" => {
            let m = vm()?;
            cli::start(&m)?;
            if a.get("wait_boot").and_then(Value::as_bool).unwrap_or(false) {
                qga::wait(&m, Duration::from_secs(num(a, "timeout_s").unwrap_or(300.0) as u64))?;
                return Ok(text(format!("«{}» запущена, агент отвечает", m.name)));
            }
            Ok(text(format!("«{}» запущена", m.name)))
        }
        "stop" => {
            let m = vm()?;
            Ok(text(cli::stop(&m, Duration::from_secs(num(a, "timeout_s").unwrap_or(90.0) as u64), a.get("force").and_then(Value::as_bool).unwrap_or(false))?))
        }
        "power" => {
            let m = vm()?;
            running(&m)?;
            match arg(a, "action")? {
                "reset" => qmp::reset(&m)?,
                "pause" => qmp::pause(&m)?,
                "resume" => qmp::resume(&m)?,
                "kill" => {
                    if qmp::quit(&m).is_err() {
                        qemu::kill(&m);
                    }
                }
                o => bail!("неизвестное действие {o}"),
            }
            Ok(text("готово"))
        }
        "wait_boot" => {
            let m = vm()?;
            let t0 = std::time::Instant::now();
            qga::wait(&m, Duration::from_secs(num(a, "timeout_s").unwrap_or(300.0) as u64))?;
            Ok(text(format!("агент отвечает ({:.0} с)", t0.elapsed().as_secs_f64())))
        }
        "exec" => {
            let m = vm()?;
            running(&m)?;
            let argv: Vec<String> = match (a.get("argv").and_then(Value::as_array), a.get("command").and_then(Value::as_str)) {
                (Some(v), _) => v.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                (None, Some(c)) => qga::shell_argv(&m, c),
                _ => bail!("нужен command или argv"),
            };
            let input = a.get("stdin").and_then(Value::as_str).map(|s| s.as_bytes().to_vec());
            let r = qga::exec(&m, &argv, input.as_deref(), &[], Duration::from_secs(num(a, "timeout_s").unwrap_or(600.0) as u64))?;
            let mut s = format!("код: {}{}\n", r.code, if r.timed_out { " (не дождались, процесс работает дальше)" } else { "" });
            if !r.stdout.is_empty() {
                s.push_str(&format!("--- stdout ---\n{}\n", clip(&r.stdout, 60_000)));
            }
            if !r.stderr.is_empty() {
                s.push_str(&format!("--- stderr ---\n{}\n", clip(&r.stderr, 20_000)));
            }
            Ok(json!({"content": [{"type": "text", "text": s}], "isError": r.code != 0}))
        }
        "read_file" => {
            let m = vm()?;
            running(&m)?;
            let b = qga::read_file(&m, arg(a, "path")?, 1 << 20)?;
            Ok(text(String::from_utf8_lossy(&b).into_owned()))
        }
        "write_file" => {
            let m = vm()?;
            running(&m)?;
            let c = arg(a, "content")?;
            qga::write_file(&m, arg(a, "path")?, c.as_bytes())?;
            Ok(text(format!("записано {} байт", c.len())))
        }
        "push" => {
            let m = vm()?;
            running(&m)?;
            let n = qga::push(&m, Path::new(arg(a, "local")?), arg(a, "path")?)?;
            Ok(text(format!("{n} байт")))
        }
        "pull" => {
            let m = vm()?;
            running(&m)?;
            let n = qga::pull(&m, arg(a, "path")?, Path::new(arg(a, "local")?))?;
            Ok(text(format!("{n} байт")))
        }
        "screenshot" => {
            let m = vm()?;
            running(&m)?;
            let tmp = m.run_dir().join("mcp-shot.png");
            qmp::screendump(&m, &tmp)?;
            if let Some(p) = a.get("save").and_then(Value::as_str) {
                std::fs::copy(&tmp, p)?;
            }
            let img = image::open(&tmp)?;
            let _ = std::fs::remove_file(&tmp);
            let (w, h) = (img.width(), img.height());
            let max = num(a, "max_size").unwrap_or(1280.0) as u32;
            let img = if max > 0 && w.max(h) > max { img.resize(max, max, image::imageops::FilterType::Triangle) } else { img };
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
            let note = format!("экран гостя {w}x{h}{}", if img.width() != w { format!(", снимок уменьшен до {}x{} — координаты для click указывайте в пикселях снимка", img.width(), img.height()) } else { String::new() });
            // Запомнить масштаб для click.
            let _ = std::fs::write(m.run_dir().join("mcp-shot.scale"), format!("{} {}", w as f64 / img.width() as f64, h as f64 / img.height() as f64));
            Ok(json!({"content": [
                {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"},
                {"type": "text", "text": note}]}))
        }
        "type_text" => {
            let m = vm()?;
            running(&m)?;
            qmp::type_text(&m, arg(a, "text")?, Duration::from_millis(15))?;
            Ok(text("набрано"))
        }
        "key" => {
            let m = vm()?;
            running(&m)?;
            qmp::send_combo(&m, arg(a, "combo")?)?;
            Ok(text("нажато"))
        }
        "click" => {
            let m = vm()?;
            running(&m)?;
            let (sx, sy) = std::fs::read_to_string(m.run_dir().join("mcp-shot.scale"))
                .ok()
                .and_then(|s| {
                    let mut it = s.split_whitespace().map(|x| x.parse::<f64>().ok());
                    Some((it.next()??, it.next()??))
                })
                .unwrap_or((1.0, 1.0));
            let x = num(a, "x").context("x")? * sx;
            let y = num(a, "y").context("y")? * sy;
            let b = a.get("button").and_then(Value::as_str).unwrap_or("left");
            cli::click(&m, x, y, (b != "none").then_some(b), a.get("double").and_then(Value::as_bool).unwrap_or(false))?;
            Ok(text(format!("щелчок в {x:.0},{y:.0} (пиксели гостя)")))
        }
        "scroll" => {
            let m = vm()?;
            running(&m)?;
            qmp::wheel(&m, num(a, "steps").context("steps")? as i32)?;
            Ok(text("готово"))
        }
        "devices" => {
            let list: Vec<Value> = devices::available()
                .iter()
                .map(|d| json!({"spec": d.dev.spec(), "kind": d.kind, "title": d.title, "detail": d.detail, "blocked": d.blocked, "warn": d.warn}))
                .collect();
            let usb: Vec<Value> = crate::usb::list().iter().filter(|u| !u.hub).map(|u| json!({"spec": format!("usb:{}", u.port), "title": u.name, "id": format!("{}:{}", u.vendor, u.product)})).collect();
            Ok(text(serde_json::to_string_pretty(&json!({"usb_ports": usb, "devices": list, "iommu": devices::iommu_on()}))?))
        }
        "attach" | "detach" => {
            let m = vm()?;
            let spec = arg(a, "spec")?;
            let on = name == "attach";
            let msg = match spec.strip_prefix("usb:") {
                Some(port) => cli::set_usb(&m, port, on)?,
                None => cli::set_device(&m, devices::parse_spec(spec)?, on)?,
            };
            Ok(text(msg))
        }
        "settings" => {
            let m = vm()?;
            match a.get("set").and_then(Value::as_object) {
                Some(obj) if !obj.is_empty() => {
                    let vals = obj
                        .iter()
                        .map(|(k, v)| {
                            let t: toml::Value = serde_json::from_value(v.clone()).with_context(|| format!("{k}: значение"))?;
                            Ok((k.clone(), t, v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let n = cli::set_values(&m, vals)?;
                    n.save()?;
                    Ok(text(format!("сохранено{}", if qemu::pid(&n).is_some() { " — вступит в силу при следующем запуске" } else { "" })))
                }
                _ => Ok(text(toml::to_string_pretty(&m)?)),
            }
        }
        "create" => {
            let mut args = vec![arg(a, "name")?.to_string()];
            for (k, flag) in [("os", "--os"), ("iso", "--iso"), ("disk", "--disk"), ("vars", "--vars"), ("tpm", "--tpm")] {
                if let Some(v) = a.get(k).and_then(Value::as_str) {
                    args.extend([flag.to_string(), v.to_string()]);
                }
            }
            for (k, flag) in [("cpus", "--cpus"), ("memory_mb", "--memory"), ("disk_gb", "--disk-gb")] {
                if let Some(v) = num(a, k) {
                    args.extend([flag.to_string(), (v as u64).to_string()]);
                }
            }
            for f in a.get("forwards").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                args.extend(["--fwd".to_string(), f.to_string()]);
            }
            let out = std::process::Command::new(std::env::current_exe()?).arg("--new").args(&args).output()?;
            if !out.status.success() {
                bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
            }
            if let Some(obj) = a.get("set").and_then(Value::as_object).filter(|o| !o.is_empty()) {
                let m = cli::find(String::from_utf8_lossy(&out.stdout).trim())?;
                let vals = obj
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), serde_json::from_value::<toml::Value>(v.clone())?, v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))))
                    .collect::<Result<Vec<_>>>()?;
                cli::set_values(&m, vals)?.save()?;
            }
            Ok(text(format!("создана: {}{}", String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim().lines().map(|l| format!("\n{l}")).collect::<String>())))
        }
        "cdrom" => {
            let mut m = vm()?;
            let iso = a.get("iso").and_then(Value::as_str).unwrap_or("").to_string();
            m.cdrom = iso.clone();
            m.save()?;
            if qemu::pid(&m).is_some() {
                qmp::change_cdrom(&m, &iso)?;
            }
            Ok(text(if iso.is_empty() { "привод пуст".into() } else { format!("в приводе {iso}") }))
        }
        "tools" => {
            let mut m = vm()?;
            match a.get("action").and_then(Value::as_str).unwrap_or("on") {
                "download" => Ok(text(format!("скачано: {}", qemu::download_tools()?.display()))),
                o => {
                    m.tools_cd = o != "off";
                    m.save()?;
                    Ok(text(match (m.tools_cd, qemu::tools_iso()) {
                        (true, Some(p)) => format!("при запуске второй привод — {}", p.display()),
                        (true, None) => "virtio-win.iso не найден — tools action=download".into(),
                        _ => "второй привод убран".into(),
                    }))
                }
            }
        }
        "snapshot" => {
            let m = vm()?;
            let on = qemu::pid(&m).is_some();
            match arg(a, "action")? {
                "list" => Ok(text(cli::snapshots(&m, on).iter().map(|s| format!("{}\t{}\t{}", s.name, s.date, s.vm_size)).collect::<Vec<_>>().join("\n"))),
                op => {
                    cli::snapshot(&m, on, op, arg(a, "name")?)?;
                    Ok(text("готово"))
                }
            }
        }
        "qmp" => {
            let m = vm()?;
            running(&m)?;
            let r = qmp::call(&m, arg(a, "command")?, a.get("arguments").cloned().unwrap_or(Value::Null))?;
            Ok(text(serde_json::to_string_pretty(&r)?))
        }
        other => bail!("нет инструмента {other}"),
    }
}
