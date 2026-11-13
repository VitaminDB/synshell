//! Управление машинами из командной строки — для скриптов, проверок и агентов: без окна,
//! с кодами возврата. Те же действия доступны MCP-серверу (`mcp.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use syngui::{n_, t, tn};

use crate::model::{self, HostDev, Vm};
use crate::{devices, qemu, qga, qmp};

/// Справка `--help` на языке пользователя.
pub fn help() -> String {
    // Новые команды — отдельными строками (весь HELP — один ключ перевода).
    let mut h = t!(HELP);
    for l in EXTRA_HELP {
        h.push_str(&syngui::i18n::t(l));
        h.push('\n');
    }
    h
}

const EXTRA_HELP: &[&str] = &[
    n_!("  --optimize МАШИНА [--apply]            Windows: быстрое видео, агент, диск инструментов; советы по диску и сети"),
    n_!("  --present-dump МАШИНА ФАЙЛ.png         экран гостя с кадром игры syn-gpu поверх (проверка слоя без окна)"),
    n_!("  --gpu-tools МАШИНА [install|status]    гостевые инструменты syn-gpu: драйвер 3D-видеокарты в Windows и Linux, диск инструментов DOS"),
];

const HELP: &str = n_!("\
syn-vm — виртуальные машины QEMU/KVM

Окно:
  syn-vm [МАШИНА] [--tab screen|overview|settings|snapshots] [--new-window]
  syn-vm --screenshot СНИМОК.png [--size ШxВ] [--scale N] [--open МАШИНА] [--tab …] [--wizard ШАГ]   снимок окна

Машины (МАШИНА — имя или id):
  --list [--json]                        машины и их состояние
  --status МАШИНА [--json]               состояние, агент, система гостя, адреса, порты
  --start МАШИНА [--wait-boot [СЕК]]     запустить без окна (и ждать гостевого агента)
  --stop МАШИНА [--force] [--timeout СЕК]  выключить (агентом или ACPI); --force — выдернуть шнур по тайм-ауту
  --restart МАШИНА [--timeout СЕК]       перезапустить (штатное выключение и запуск): применить настройки,
                                          которые вступают только после перезапуска (видеокарта и др.)
  --kill МАШИНА                          выключить сразу
  --reset | --pause | --resume МАШИНА
  --wait-boot МАШИНА [--user] [--timeout СЕК]  ждать гостевой агент (300 с); --user — и вход пользователя
  --new ИМЯ [--iso ОБРАЗ] [--os linux|windows11|windows10|bsd|android|other]
        [--disk ФАЙЛ …] [--vars ФАЙЛ] [--tpm КАТАЛОГ] [--cpus N] [--memory МБ] [--disk-gb N]
        [--firmware bios|uefi|uefi-secure] [--disk-bus virtio|sata|nvme] [--nic virtio|e1000e|rtl8139|ne2k_pci|pcnet]
        [--fwd [udp:]ХОСТ:ГОСТЬ …] [--public] [--share КАТАЛОГ …] [--set КЛЮЧ=ЗНАЧЕНИЕ …]
                                          новая машина; с --disk — импорт готовой (диск остаётся на месте,
                                          переменные UEFI и TPM копируются в папку машины;
                                          пробросы слушают 127.0.0.1, --public — все адреса)
  --delete МАШИНА --yes                  удалить машину и диски в её папке
  --get МАШИНА [КЛЮЧ]                    настройки (TOML) или одно значение
  --set МАШИНА КЛЮЧ=ЗНАЧЕНИЕ …           изменить настройки (значение — TOML или строка)

Гость (через гостевой агент qemu-ga):
  --exec МАШИНА [--timeout СЕК] [--env К=З …] [--stdin] [--raw] -- ПРОГРАММА [АРГ…]
                                          (вывод Windows переводится из OEM-кодировки в UTF-8; --raw — как есть)
  --sh МАШИНА [--timeout СЕК] 'КОМАНДА'  через оболочку гостя (sh -c / cmd /c)
       --detach (у --exec и --sh) — не ждать, напечатать pid в госте
  --exec-status МАШИНА PID [--wait СЕК]  итог фоновой команды: вывод и её код; 125 — ещё работает
                                          (вывод агент отдаёт один раз)
  --push МАШИНА ФАЙЛ|КАТАЛОГ ПУТЬ_В_ГОСТЕ   файл или каталог в гостя (размер и SHA-256 сверяются)
  --pull МАШИНА ПУТЬ_В_ГОСТЕ ФАЙЛ|КАТАЛОГ   файл или каталог из гостя
       Свой агент syn-vm-agent (если стоит в госте): --exec с потоковым выводом, --user — в сеансе
       вошедшего пользователя (окна, DXGI), --qga — по-старому через qemu-ga
  --agent-install МАШИНА [ФАЙЛ]          поставить или обновить свой агент в госте (службой)
  --agent-status МАШИНА                  агент на связи: версия, система, пользователь сеанса
  --agent-log МАШИНА [--follow]          журнал гостя: OutputDebugString, журнал событий, journald
  --clipboard МАШИНА set ТЕКСТ|watch     буфер обмена гостя через агента (Windows со своими инструментами)
                                          set-image ФАЙЛ.png — картинку
  --agent МАШИНА КОМАНДА [JSON]          сырая команда агента (guest-info, guest-get-osinfo…)
  --sync-time МАШИНА                     часы гостя по компьютеру

Экран и ввод (без окна, работает и без агента):
  --screendump МАШИНА ФАЙЛ.png           кадр экрана гостя
  --type МАШИНА 'ТЕКСТ'                  набрать текст (раскладка US, \\n — Enter)
  --key МАШИНА СОЧЕТАНИЕ                 ctrl+alt+delete, alt+f4, win+r, enter…
  --click МАШИНА X Y [--right|--middle] [--double] [--move]   щелчок в пикселях кадра
  --scroll МАШИНА ШАГИ                   колесо (> 0 — вниз)

Устройства:
  --devices [--json]                     что можно отдать гостю: PCI, ввод, диски, порты, USB
  --usb МАШИНА list|attach|detach [ПОРТ] USB по физическому порту (3-6, 1-2.4)
  --attach МАШИНА ТИП:ИД                 pci:02:00.0, input:/dev/input/by-id/…, block:/dev/sdb,
                                          block-ro:/dev/sdb1, serial:/dev/ttyUSB0, usb-id:VID:PID, usb:3-6
  --detach МАШИНА ТИП:ИД
  --cdrom МАШИНА ОБРАЗ|eject             образ в приводе
  --tools МАШИНА [on|off|download|ОБРАЗ.iso]  диск «Гостевые инструменты» (virtio-win)
  --tools МАШИНА iso|install             свои «Гостевые инструменты syn-vm» (guest_tools = syn-vm): образ, установка в госте

Запись экрана гостя (родное разрешение, ровная частота, ffmpeg: NVENC/VAAPI/x264):
  --record МАШИНА [ФАЙЛ.mp4] [--fps 30|60] [--audio|--no-audio] [--duration СЕК]
                                          без --duration — в фоне до --record-stop; звук — если он включён у машины
  --record-stop МАШИНА                   остановить, дождаться файла, итог

Прочее:
  --snapshot МАШИНА list|create|restore|delete [ИМЯ]
  --qmp МАШИНА КОМАНДА [JSON]            сырая команда QMP
  --mcp                                   MCP-сервер (stdio): claude mcp add syn-vm -- syn-vm --mcp
");

/// Машина по id или имени (без учёта регистра).
pub fn find(s: &str) -> Result<Vm> {
    let all = model::load_all();
    let l = s.to_lowercase();
    all.iter().find(|v| v.id == s).or_else(|| all.iter().find(|v| v.name.to_lowercase() == l)).cloned().with_context(|| t!("нет машины «{s}» (syn-vm --list)", s = s))
}

fn running(vm: &Vm) -> Result<()> {
    if qemu::pid(vm).is_none() {
        bail!("{}", t!("«{name}» не запущена (syn-vm --start {id})", name = vm.name, id = vm.id));
    }
    Ok(())
}

/// Разбор хвоста: флаги со значениями и без, позиционные; всё после `--` — команда.
struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
    tail: Vec<String>,
}

const FLAGS: &[&str] = &["--user", "--qga", "--follow", "--detach", "--audio", "--no-audio", "--raw", "--public", "--json", "--force", "--yes", "--stdin", "--right", "--middle", "--double", "--move", "--wait-boot"];

impl Args {
    fn parse(v: Vec<String>) -> Args {
        let mut a = Args { pos: vec![], opts: vec![], tail: vec![] };
        let mut it = v.into_iter().peekable();
        while let Some(x) = it.next() {
            if x == "--" {
                a.tail = it.collect();
                break;
            }
            if x.starts_with("--") && x.len() > 2 {
                if x == "--wait-boot" {
                    // Необязательное число секунд.
                    let n = it.next_if(|n| n.parse::<u64>().is_ok());
                    a.opts.push((x, n));
                } else if FLAGS.contains(&x.as_str()) {
                    a.opts.push((x, None));
                } else {
                    let val = it.next();
                    a.opts.push((x, val));
                }
            } else {
                a.pos.push(x);
            }
        }
        a
    }
    fn has(&self, k: &str) -> bool {
        self.opts.iter().any(|(o, _)| o == k)
    }
    fn get(&self, k: &str) -> Option<String> {
        self.opts.iter().rev().find(|(o, _)| o == k).and_then(|(_, v)| v.clone())
    }
    fn all(&self, k: &str) -> Vec<String> {
        self.opts.iter().filter(|(o, _)| o == k).filter_map(|(_, v)| v.clone()).collect()
    }
    fn pos(&self, i: usize, what: &str) -> Result<String> {
        self.pos.get(i).cloned().with_context(|| t!("нужен аргумент: {what}", what = syngui::i18n::t(what)))
    }
    fn secs(&self, k: &str, def: u64) -> Duration {
        Duration::from_secs(self.get(k).and_then(|s| s.parse().ok()).unwrap_or(def))
    }
}

/// Команда CLI → код возврата; `None` — это не команда CLI (окно).
pub fn run(cmd: &str, rest: Vec<String>) -> Option<i32> {
    let known = [
        "--list", "--status", "--start", "--stop", "--restart", "--kill", "--reset", "--pause", "--resume", "--wait-boot", "--new", "--delete", "--get", "--set", "--exec", "--sh", "--push",
        "--pull", "--exec-status", "--agent", "--sync-time", "--screendump", "--type", "--key", "--click", "--scroll", "--devices", "--usb", "--attach", "--detach", "--cdrom", "--tools", "--snapshot",
        "--qmp", "--mcp", "--optimize", "--present-dump", "--gpu-tools", "--vfio-helper", "--vfio-watch", "--usb-watch", "--record", "--record-stop", "--record-run",
        "--agent-broker", "--agent-install", "--agent-status", "--agent-log", "--clipboard",
    ];
    if !known.contains(&cmd) {
        return None;
    }
    let a = Args::parse(rest.clone());
    if a.has("--qga") {
        crate::agent::FORCE_QGA.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let r = match cmd {
        "--mcp" => crate::mcp::run().map(|_| 0),
        "--vfio-helper" => devices::vfio_helper(&rest).map(|_| 0),
        "--record-run" => (|| -> Result<i32> {
            let vm = find(rest.first().with_context(|| t!("МАШИНА"))?)?;
            let dur: f64 = rest.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            crate::record::run(
                &vm,
                crate::record::Opts {
                    out: PathBuf::from(rest.get(1).with_context(|| t!("ФАЙЛ"))?),
                    fps: rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(30),
                    audio: rest.get(3).is_some_and(|s| s == "1"),
                    duration: (dur > 0.0).then(|| Duration::from_secs_f64(dur)),
                },
            )?;
            Ok(0)
        })(),
        "--usb-watch" => {
            devices::usb_watch(rest.first().map(String::as_str).unwrap_or(""));
            Ok(0)
        }
        "--vfio-watch" => find(rest.first().map(String::as_str).unwrap_or("")).map(|vm| {
            devices::vfio_watch(&vm);
            0
        }),
        _ => dispatch(cmd, &a),
    };
    Some(match r {
        Ok(code) => code,
        Err(e) => {
            eprintln!("syn-vm: {e:#}");
            1
        }
    })
}

fn dispatch(cmd: &str, a: &Args) -> Result<i32> {
    let json_out = a.has("--json");
    match cmd {
        "--list" => {
            let list: Vec<Value> = model::load_all().iter().map(|vm| json!({"id": vm.id, "name": vm.name, "state": power(vm), "os": vm.os})).collect();
            if json_out {
                println!("{}", Value::Array(list));
            } else {
                for v in list {
                    println!("{}\t{}\t{}", v["id"].as_str().unwrap_or(""), v["state"].as_str().unwrap_or(""), v["name"].as_str().unwrap_or(""));
                }
            }
            Ok(0)
        }
        "--status" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let s = status(&vm);
            if json_out {
                println!("{s}");
            } else {
                print_status(&s);
            }
            Ok(if s["state"] == "off" { 3 } else { 0 })
        }
        "--start" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            start(&vm)?;
            if a.has("--wait-boot") {
                let t = a.get("--wait-boot").and_then(|s| s.parse().ok()).unwrap_or(300);
                crate::agent::wait_boot(&vm, a.has("--user"), Duration::from_secs(t))?;
                println!("{}", t!("{name}: агент отвечает", name = vm.name));
            }
            Ok(0)
        }
        "--stop" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let how = stop(&qemu::launched(&vm).unwrap_or(vm.clone()), a.secs("--timeout", 90), a.has("--force"))?;
            println!("{}: {how}", vm.name);
            Ok(0)
        }
        "--restart" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let changes = qemu::launched(&vm).map(|l| vm.restart_changes(&l)).unwrap_or_default();
            let how = stop(&qemu::launched(&vm).unwrap_or(vm.clone()), a.secs("--timeout", 180), a.has("--force"))
                .with_context(|| t!("«{name}» не перезапущена — работает прежний запуск", name = vm.name))?;
            println!("{}: {how}", vm.name);
            start(&vm)?;
            if changes.is_empty() {
                println!("{}", t!("{name}: запущена", name = vm.name));
            } else {
                println!("{}", t!("{name}: запущена с новыми настройками: {list}", name = vm.name, list = changes.join("; ")));
            }
            Ok(0)
        }
        "--kill" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            if qmp::quit(&vm).is_err() {
                qemu::kill(&vm);
            }
            Ok(0)
        }
        "--reset" | "--pause" | "--resume" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            match cmd {
                "--reset" => qmp::reset(&vm)?,
                "--pause" => qmp::pause(&vm)?,
                _ => qmp::resume(&vm)?,
            }
            Ok(0)
        }
        "--wait-boot" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let t = a.get("--wait-boot").and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(a.secs("--timeout", 300));
            let t0 = Instant::now();
            crate::agent::wait_boot(&vm, a.has("--user"), t)?;
            println!("{}", t!("{name}: агент отвечает ({s} с)", name = vm.name, s = format!("{:.0}", t0.elapsed().as_secs_f64())));
            Ok(0)
        }
        "--new" => {
            let vm = new_vm(&a.pos(0, n_!("ИМЯ"))?, a)?;
            println!("{}", vm.id);
            Ok(0)
        }
        "--delete" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            if !a.has("--yes") {
                bail!("{}", t!("удаление «{name}» со всеми дисками в {dir} — подтвердите флагом --yes", name = vm.name, dir = vm.dir().display()));
            }
            if qemu::pid(&vm).is_some() {
                bail!("{}", t!("«{name}» работает — сначала --stop", name = vm.name));
            }
            model::delete(&vm)?;
            Ok(0)
        }
        "--get" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let t = toml::Value::try_from(&vm)?;
            match a.pos.get(1) {
                Some(k) => println!("{}", get_key(&t, k).with_context(|| t!("нет ключа {k}", k = k))?),
                None => print!("{}", toml::to_string_pretty(&vm)?),
            }
            Ok(0)
        }
        "--set" => {
            let mut vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let kv: Vec<String> = a.pos[1..].to_vec();
            if kv.is_empty() {
                bail!("{}", t!("нужно КЛЮЧ=ЗНАЧЕНИЕ"));
            }
            vm = set_keys(&vm, &kv)?;
            vm.save()?;
            if qemu::pid(&vm).is_none() {
                for w in qemu::port_conflicts(&vm).into_iter().chain(qemu::port_shared_with(&vm)) {
                    eprintln!("{}", t!("внимание: {w}", w = w));
                }
            }
            if qemu::pid(&vm).is_some() {
                eprintln!("{}", t!("{name}: машина работает — изменения вступят в силу при следующем запуске", name = vm.name));
            }
            Ok(0)
        }
        "--agent-broker" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            crate::agent::run_broker(&vm)?;
            Ok(0)
        }
        "--clipboard" => {
            // Буфер обмена гостя через агента syn-vm: set ТЕКСТ — положить, watch — печатать изменения.
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let mut c = crate::agent::Client::connect(&vm)?;
            match a.pos.get(1).map(String::as_str) {
                Some("set-image") => {
                    let f = a.pos(2, n_!("ФАЙЛ.png"))?;
                    c.send_clip_data("image/png", std::fs::read(&f).with_context(|| f.clone())?)?;
                    std::thread::sleep(Duration::from_millis(500));
                }
                Some("set") => {
                    let text = if a.pos.len() > 2 { a.pos[2..].join(" ") } else { a.tail.join(" ") };
                    c.send_clip(&text)?;
                    // Дать посреднику отправить кадр до закрытия соединения.
                    std::thread::sleep(Duration::from_millis(300));
                }
                _ => c.follow_clip(&mut |mime, d| {
                    if mime == "text/plain" {
                        println!("{}", String::from_utf8_lossy(&d));
                    } else {
                        println!("[{mime}, {} байт]", d.len());
                    }
                })?,
            }
            Ok(0)
        }
        "--agent-status" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            match crate::agent::Client::connect(&vm)?.info()? {
                Some(i) => {
                    println!("syn-vm-agent {} — {} ({})", i.agent, i.os, i.host);
                    println!("{}", t!("пользователь сеанса: {u}", u = i.user.as_deref().unwrap_or("—")));
                    Ok(0)
                }
                None => {
                    eprintln!("{}", t!("агент гостя не на связи (не установлен: syn-vm --agent-install {id})", id = vm.id));
                    Ok(1)
                }
            }
        }
        "--agent-log" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let path = crate::agent::log_path(&vm);
            if let Ok(s) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = s.lines().collect();
                let n = a.get("--lines").and_then(|n| n.parse().ok()).unwrap_or(200usize);
                for l in &lines[lines.len().saturating_sub(n)..] {
                    println!("{l}");
                }
            }
            if a.has("--follow") {
                running(&vm)?;
                crate::agent::Client::connect(&vm)?.follow_logs(&mut |src, text| {
                    for l in text.lines() {
                        println!("[{src}] {l}");
                    }
                })?;
            }
            Ok(0)
        }
        "--agent-install" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            agent_install(&vm, a.pos.get(1).map(PathBuf::from))?;
            Ok(0)
        }
        "--exec" | "--sh" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let argv = if cmd == "--sh" {
                let line = if a.tail.is_empty() { a.pos[1..].join(" ") } else { a.tail.join(" ") };
                qga::shell_argv(&vm, &line)
            } else if a.tail.is_empty() {
                a.pos[1..].to_vec()
            } else {
                a.tail.clone()
            };
            if argv.is_empty() {
                bail!("{}", t!("нет команды: syn-vm --exec МАШИНА -- ПРОГРАММА [АРГ…]"));
            }
            let input = if a.has("--stdin") {
                let mut b = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut b)?;
                Some(b)
            } else {
                None
            };
            // Свой агент: потоковый вывод, без предела длины, --user — в сеансе пользователя.
            if use_agent(&vm, a)? {
                let raw = a.has("--raw");
                let mut c = crate::agent::Client::connect(&vm)?;
                // Кодировку вывода Windows угадывать по целым строкам: кусок потока может
                // разрезать строку (и символ UTF-8) посередине.
                let mut pending: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                let emit = |fd: u8, d: &[u8]| {
                    use std::io::Write;
                    if fd == 2 {
                        let _ = std::io::stderr().write_all(d);
                    } else {
                        let mut o = std::io::stdout();
                        let _ = o.write_all(d);
                        let _ = o.flush();
                    }
                };
                let r = c.exec(&argv, a.has("--user"), a.get("--cwd"), &a.all("--env"), input.as_deref(), a.secs("--timeout", 600), &mut |fd, data| {
                    if raw {
                        return emit(fd, data);
                    }
                    let buf = &mut pending[(fd == 2) as usize];
                    buf.extend_from_slice(data);
                    let cut = match buf.iter().rposition(|b| *b == b'\n') {
                        Some(i) => i + 1,
                        // Строка без конца (ход выполнения): целый UTF-8 — сразу, иначе ждать.
                        None if std::str::from_utf8(buf).is_ok() || buf.len() > 16384 => buf.len(),
                        None => 0,
                    };
                    if cut > 0 {
                        let line: Vec<u8> = buf.drain(..cut).collect();
                        emit(fd, &qga::decode_output(&vm, line));
                    }
                })?;
                for (i, rest) in pending.into_iter().enumerate() {
                    if !rest.is_empty() {
                        emit(if i == 1 { 2 } else { 1 }, &qga::decode_output(&vm, rest));
                    }
                }
                if r.timed_out {
                    eprintln!("syn-vm: {}", t!("тайм-аут: команда в госте завершена"));
                }
                return Ok(r.code);
            }
            if a.has("--detach") {
                let pid = qga::exec_detached(&vm, &argv, input.as_deref(), &a.all("--env"))?;
                println!("{pid}");
                return Ok(0);
            }
            let mut r = qga::exec(&vm, &argv, input.as_deref(), &a.all("--env"), a.secs("--timeout", 600))?;
            if !a.has("--raw") {
                r.stdout = qga::decode_output(&vm, std::mem::take(&mut r.stdout));
                r.stderr = qga::decode_output(&vm, std::mem::take(&mut r.stderr));
            }
            use std::io::Write;
            std::io::stdout().write_all(&r.stdout)?;
            std::io::stderr().write_all(&r.stderr)?;
            if r.timed_out {
                eprintln!("syn-vm: {}", t!("не дождались (pid {pid} в госте работает дальше)", pid = r.pid));
            }
            Ok(r.code)
        }
        "--exec-status" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let pid: i64 = a.pos(1, "PID")?.parse().with_context(|| t!("PID — число"))?;
            let r = match a.get("--wait") {
                Some(t) => {
                    let r = qga::wait_exec(&vm, pid, Duration::from_secs(t.parse().unwrap_or(600)))?;
                    if r.timed_out {
                        eprintln!("syn-vm: {}", t!("ещё работает"));
                        return Ok(125);
                    }
                    r
                }
                None => match qga::exec_status(&vm, pid)? {
                    Some(r) => r,
                    None => {
                        eprintln!("syn-vm: {}", t!("pid {pid} ещё работает", pid = pid));
                        return Ok(125);
                    }
                },
            };
            use std::io::Write;
            std::io::stdout().write_all(&qga::decode_output(&vm, r.stdout))?;
            std::io::stderr().write_all(&qga::decode_output(&vm, r.stderr))?;
            Ok(r.code)
        }
        "--push" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let (local, guest) = (a.pos(1, n_!("ФАЙЛ"))?, a.pos(2, n_!("ПУТЬ_В_ГОСТЕ"))?);
            let n = if use_agent(&vm, a)? { crate::agent::Client::connect(&vm)?.push(Path::new(&local), &guest)? } else { qga::push(&vm, Path::new(&local), &guest)? };
            eprintln!("{}", tn!(n, "{n} байт", "{n} байта", "{n} байт"));
            Ok(0)
        }
        "--pull" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let (guest, local) = (a.pos(1, n_!("ПУТЬ_В_ГОСТЕ"))?, a.pos(2, n_!("ФАЙЛ"))?);
            let n = if use_agent(&vm, a)? { crate::agent::Client::connect(&vm)?.pull(&guest, Path::new(&local))? } else { qga::pull(&vm, &guest, Path::new(&local))? };
            eprintln!("{}", tn!(n, "{n} байт", "{n} байта", "{n} байт"));
            Ok(0)
        }
        "--agent" | "--qmp" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let c = a.pos(1, n_!("КОМАНДА"))?;
            let args: Value = match a.pos.get(2) {
                Some(s) => serde_json::from_str(s).with_context(|| t!("аргументы — JSON"))?,
                None => Value::Null,
            };
            let r = if cmd == "--agent" { qga::call(&vm, &c, args)? } else { qmp::call(&vm, &c, args)? };
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(0)
        }
        "--sync-time" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            qga::sync_time(&vm)?;
            Ok(0)
        }
        "--gpu-tools" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            // DOS: диск инструментов подключается при запуске — машина может быть выключена.
            if crate::dostools::is_dos(&vm) {
                match a.pos.get(1).map(String::as_str).unwrap_or("status") {
                    "install" => println!("{}", crate::dostools::install(&vm)?),
                    _ => println!("{}", crate::dostools::status(&vm)),
                }
                return Ok(0);
            }
            running(&vm)?;
            match a.pos.get(1).map(String::as_str).unwrap_or("status") {
                "install" => {
                    let r = crate::gputools::install(&vm, &|s| eprintln!("{s}"))?;
                    println!("{r}");
                }
                _ => {
                    let s = crate::gputools::status(&vm)?;
                    println!("{}", s.summary());
                    for g in &s.games {
                        println!("  {}\t{} {}-bit\tDXVK {}{}", g.path, g.api, g.bits, g.dxvk, if g.disabled { t!(" (отключено)") } else { String::new() });
                    }
                }
            }
            Ok(0)
        }
        "--present-dump" => {
            // Слой кадров игр syn-gpu без окна: экран гостя + кадр игры в месте её окна.
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let out = absolute(&a.pos(1, n_!("ФАЙЛ.png"))?)?;
            // syn-gpu — основная видеокарта: экрана QEMU нет, снимок — рабочий стол с карты.
            if qemu::launched(&vm).map(|l| l.display).unwrap_or(vm.display) == model::DisplayModel::SynGpu {
                let d = crate::present::grab(&vm.id, Duration::from_secs(10), true)?;
                let (fw, fh, px, _) = d.frame.latest();
                image::RgbaImage::from_raw(fw, fh, px.to_vec()).context("кадр")?.save(&out)?;
                println!("{}: {fw}x{fh}, рабочий стол syn-gpu", out.display());
                return Ok(0);
            }
            let layer = crate::present::grab(&vm.id, Duration::from_secs(10), false)?;
            qmp::screendump(&vm, &out)?;
            let mut screen = image::open(&out)?.to_rgba8();
            let (fw, fh, px, _) = layer.frame.latest();
            let game = image::RgbaImage::from_raw(fw, fh, px.to_vec()).context("кадр")?;
            let (x, y, w, h) = layer.rect;
            let game = if (w, h) != (fw, fh) { image::imageops::resize(&game, w, h, image::imageops::FilterType::Triangle) } else { game };
            image::imageops::overlay(&mut screen, &game, x as i64, y as i64);
            screen.save(&out)?;
            println!("{}: {}x{}, игра {fw}x{fh} в {x},{y} {w}x{h}{}", out.display(), screen.width(), screen.height(), if layer.fullscreen { ", во весь экран" } else { "" });
            Ok(0)
        }
        "--screendump" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let out = absolute(&a.pos(1, n_!("ФАЙЛ.png"))?)?;
            screendump(&vm, &out)?;
            let (w, h) = image::image_dimensions(&out).unwrap_or((0, 0));
            println!("{}: {w}x{h}", out.display());
            Ok(0)
        }
        "--type" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let text = a.pos[1..].join(" ").replace("\\n", "\n");
            qmp::type_text(&vm, &text, Duration::from_millis(a.get("--delay").and_then(|s| s.parse().ok()).unwrap_or(15)))?;
            Ok(0)
        }
        "--key" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            for c in &a.pos[1..] {
                qmp::send_combo(&vm, c)?;
                std::thread::sleep(Duration::from_millis(80));
            }
            Ok(0)
        }
        "--click" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            let x: f64 = a.pos(1, "X")?.parse()?;
            let y: f64 = a.pos(2, "Y")?.parse()?;
            let button = if a.has("--move") {
                None
            } else if a.has("--right") {
                Some("right")
            } else if a.has("--middle") {
                Some("middle")
            } else {
                Some("left")
            };
            click(&vm, x, y, button, a.has("--double"))?;
            Ok(0)
        }
        "--scroll" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            running(&vm)?;
            qmp::wheel(&vm, a.pos(1, n_!("ШАГИ"))?.parse()?)?;
            Ok(0)
        }
        "--devices" => {
            let list = devices::available();
            if json_out {
                let v: Vec<Value> = list
                    .iter()
                    .map(|d| json!({"spec": d.dev.spec(), "kind": d.kind, "title": d.title, "detail": d.detail, "blocked": d.blocked, "warn": d.warn, "needs_access": d.needs_access}))
                    .collect();
                let usb: Vec<Value> = crate::usb::list().iter().filter(|u| !u.hub).map(|u| json!({"spec": format!("usb:{}", u.port), "title": u.name, "vendor": u.vendor, "product": u.product})).collect();
                println!("{}", json!({"devices": v, "usb_ports": usb, "iommu": devices::iommu_on()}));
            } else {
                print_devices(&list);
            }
            Ok(0)
        }
        "--optimize" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let (new, changes, advice) = vm.optimized();
            if changes.is_empty() && advice.is_empty() {
                println!("{}", t!("Настройки уже оптимальны"));
                return Ok(0);
            }
            for c in &changes {
                println!("• {c}");
            }
            for c in &advice {
                println!("○ {c}");
            }
            if !changes.is_empty() {
                if a.has("--apply") {
                    new.save()?;
                    println!("{}", t!("Применено — при следующем запуске машины"));
                } else {
                    println!("{}", t!("Применить: syn-vm --optimize {id} --apply", id = vm.id));
                }
            }
            Ok(0)
        }
        "--usb" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            match a.pos.get(1).map(String::as_str).unwrap_or("list") {
                "list" => {
                    for u in crate::usb::list().into_iter().filter(|u| !u.hub) {
                        let on = vm.usb.iter().any(|p| p.port == u.port);
                        println!("{}\t{}\t{}:{}\t{}", u.port, if on { "guest" } else { "host" }, u.vendor, u.product, u.name);
                    }
                    for p in vm.usb.iter().filter(|p| crate::usb::at(&p.port).is_none()) {
                        println!("{}\tguest\t{}\t{}", p.port, t!("(пусто)"), p.name);
                    }
                    Ok(0)
                }
                op @ ("attach" | "detach") => {
                    let port = a.pos(2, n_!("ПОРТ"))?;
                    let msg = set_usb(&vm, &port, op == "attach")?;
                    println!("{msg}");
                    Ok(0)
                }
                o => bail!("{}", t!("--usb МАШИНА list|attach|detach ПОРТ (не «{o}»)", o = o)),
            }
        }
        "--attach" | "--detach" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let spec = a.pos(1, n_!("ТИП:ИД"))?;
            let msg = if let Some(port) = spec.strip_prefix("usb:") {
                set_usb(&vm, port, cmd == "--attach")?
            } else {
                set_device(&vm, devices::parse_spec(&spec)?, cmd == "--attach")?
            };
            println!("{msg}");
            Ok(0)
        }
        "--cdrom" => {
            let mut vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let iso = a.pos(1, n_!("ОБРАЗ|eject"))?;
            let iso = if iso == "eject" || iso.is_empty() { String::new() } else { absolute(&iso)?.to_string_lossy().into_owned() };
            vm.cdrom = iso.clone();
            vm.save()?;
            if qemu::pid(&vm).is_some() {
                qmp::change_cdrom(&vm, &iso)?;
            }
            Ok(0)
        }
        "--tools" => {
            let mut vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            match a.pos.get(1).map(String::as_str).unwrap_or("on") {
                // Свои «Гостевые инструменты syn-vm»: образ (собрать) и установка в госте.
                "iso" => println!("{}", crate::guesttools::iso()?.display()),
                "install" => {
                    if !vm.syn_tools() {
                        bail!("{}", t!("у «{name}» инструменты virtio-win — сначала syn-vm --set {id} guest_tools=syn-vm", name = vm.name, id = vm.id));
                    }
                    println!("{}", crate::state::install_syn_tools(&vm, qemu::pid(&vm).is_some())?);
                }
                "download" => {
                    let p = qemu::download_tools()?;
                    println!("{}", p.display());
                }
                path if path.ends_with(".iso") => {
                    let p = std::fs::canonicalize(path)?;
                    std::fs::write(model::root().join("tools-iso.path"), p.to_string_lossy().as_bytes())?;
                    vm.tools_cd = true;
                    vm.save()?;
                    println!("{}", p.display());
                }
                o => {
                    vm.tools_cd = o != "off";
                    vm.save()?;
                    match qemu::tools_iso() {
                        Some(p) if vm.tools_cd => println!("{}", p.display()),
                        None if vm.tools_cd => eprintln!("{}", t!("virtio-win.iso не найден — syn-vm --tools {id} download", id = vm.id)),
                        _ => {}
                    }
                    if qemu::pid(&vm).is_some() {
                        eprintln!("{}", t!("второй привод появится при следующем запуске; сейчас можно вставить в первый: syn-vm --cdrom {id} ОБРАЗ", id = vm.id));
                    }
                }
            }
            Ok(0)
        }
        "--record" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let out = match a.pos.get(1) {
                Some(p) => absolute(p)?,
                None => crate::record::default_path(&vm),
            };
            let o = crate::record::Opts {
                out,
                fps: a.get("--fps").and_then(|s| s.parse().ok()).unwrap_or(30),
                audio: a.has("--audio") || (vm.sound && !a.has("--no-audio")),
                duration: a.get("--duration").and_then(|s| s.parse::<f64>().ok()).map(Duration::from_secs_f64),
            };
            let wait = o.duration.is_some();
            let i = crate::record::start(&vm, &o)?;
            let what = if i.audio {
                t!("запись: {path} ({w}x{h}, {fps} к/с, {enc}, со звуком)", path = i.path, w = i.size.0, h = i.size.1, fps = i.fps, enc = i.encoder)
            } else {
                t!("запись: {path} ({w}x{h}, {fps} к/с, {enc})", path = i.path, w = i.size.0, h = i.size.1, fps = i.fps, enc = i.encoder)
            };
            println!("{what}");
            if wait {
                let d = crate::record::wait(&vm)?;
                print_record(&d);
            } else {
                println!("{}", t!("остановить: syn-vm --record-stop {id}", id = vm.id));
            }
            Ok(0)
        }
        "--record-stop" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let d = crate::record::stop(&vm)?;
            print_record(&d);
            Ok(if d.done.as_ref().is_some_and(|s| s.error.is_empty()) { 0 } else { 1 })
        }
        "--snapshot" => {
            let vm = find(&a.pos(0, n_!("МАШИНА"))?)?;
            let op = a.pos(1, "list|create|restore|delete")?;
            let on = qemu::pid(&vm).is_some();
            if op == "list" {
                for s in snapshots(&vm, on) {
                    println!("{}\t{}\t{}", s.name, s.date, s.vm_size);
                }
                return Ok(0);
            }
            let name = a.pos(2, n_!("ИМЯ"))?;
            snapshot(&vm, on, &op, &name)?;
            Ok(0)
        }
        _ => bail!("{}", t!("неизвестная команда {cmd}", cmd = cmd)),
    }
}

pub fn record_summary(i: &crate::record::Info) -> String {
    match &i.done {
        Some(s) => {
            let size = if s.bytes >= 1_000_000 { t!("{v} МБ", v = format!("{:.1}", s.bytes as f64 / 1e6)) } else { t!("{v} КБ", v = s.bytes / 1000) };
            let mut out = t!(
                "{path}: {secs} с, {frames} кадров ({fps} к/с, {w}x{h}, {enc}), от гостя {updates} обновлений, повторов из-за задержек {late}, звук {audio} с, {size}",
                path = i.path,
                secs = format!("{:.1}", s.seconds),
                frames = s.frames,
                fps = i.fps,
                w = i.size.0,
                h = i.size.1,
                enc = i.encoder,
                updates = s.updates,
                late = s.late,
                audio = format!("{:.1}", s.audio_seconds),
                size = size
            );
            if !s.error.is_empty() {
                out.push('\n');
                out.push_str(&t!("ошибка: {e}", e = s.error));
            }
            out
        }
        None => t!("{path}: идёт запись", path = i.path),
    }
}

fn print_record(i: &crate::record::Info) {
    println!("{}", record_summary(i));
}

// ─── Действия (общие с MCP) ─────────────────────────────────────────────

pub fn power(vm: &Vm) -> &'static str {
    if qemu::pid(vm).is_none() {
        return "off";
    }
    match qmp::status(vm).as_deref() {
        Some("running") => "running",
        Some("paused" | "suspended" | "prelaunch" | "inmigrate" | "postmigrate" | "save-vm" | "restore-vm") => "paused",
        Some("shutdown") => "shutdown",
        Some(_) => "running",
        None => "busy",
    }
}

pub fn status(vm: &Vm) -> Value {
    let state = power(vm);
    let mut v = json!({
        "id": vm.id, "name": vm.name, "os": vm.os, "state": state, "dir": vm.dir(),
        "cpus": vm.cpus, "memory_mb": vm.memory_mb,
        "forwards": vm.forwards.iter().map(|f| format!("{}{}->{}", if f.udp { "udp:" } else { "" }, f.host, f.guest)).collect::<Vec<_>>(),
        "usb": vm.usb.iter().map(|u| json!({"port": u.port, "name": u.name, "present": crate::usb::at(&u.port).is_some()})).collect::<Vec<_>>(),
        "devices": vm.devices.iter().map(|d| d.spec()).collect::<Vec<_>>(),
        "shared_dir": vm.shared_dir, "cdrom": vm.cdrom,
        "guest_agent": vm.guest_agent,
    });
    if state != "off" {
        v["pid"] = json!(qemu::pid(vm));
        // С чем работает QEMU и что из vm.toml вступит только после перезапуска.
        if let Some(l) = qemu::launched(vm) {
            v["display"] = json!(l.display.short());
            v["pending_restart"] = json!(vm.restart_changes(&l));
        }
        v["qmp"] = json!(vm.qmp_path());
        v["vnc"] = json!(vm.vnc_path());
        match qga::info(vm) {
            Ok(g) => {
                v["agent"] = json!("ok");
                v["guest"] = json!({"os": g.os, "hostname": g.hostname, "ips": g.ips, "agent_version": g.agent_version});
            }
            Err(e) => v["agent"] = json!(format!("{e:#}")),
        }
    }
    v
}

fn print_status(s: &Value) {
    let g = |k: &str| s.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    println!("{} ({})", g("name"), g("id"));
    println!("{}", t!("состояние: {s}", s = g("state")));
    println!("{}", t!("система: {os}, ядер: {cpus}, {mem} МБ", os = g("os"), cpus = g("cpus"), mem = g("memory_mb")));
    if let Some(gst) = s.get("guest") {
        let ips = gst["ips"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        println!("{}", t!("гость: {os} ({host}), адреса: {ips}", os = gst["os"].as_str().unwrap_or(""), host = gst["hostname"].as_str().unwrap_or(""), ips = ips));
    } else if s.get("agent").is_some() {
        println!("{}", t!("агент: {a}", a = g("agent")));
    }
    for k in ["forwards", "usb", "devices"] {
        if s[k].as_array().is_some_and(|a| !a.is_empty()) {
            println!("{k}: {}", s[k]);
        }
    }
    if s.get("display").is_some() {
        println!("{}", t!("видеокарта: {d}", d = g("display")));
    }
    if let Some(p) = s["pending_restart"].as_array().filter(|a| !a.is_empty()) {
        let list = p.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ");
        println!("{}", t!("после перезапуска (syn-vm --restart): {list}", list = list));
    }
}

pub fn start(vm: &Vm) -> Result<()> {
    let mut v = vm.clone();
    v.last_run = model::now();
    let _ = v.save();
    qemu::start(&v)?;
    qemu::after_boot(&v);
    Ok(())
}

/// Выключить: агентом (надёжнее, без вопросов гостя), иначе ACPI. Ждать до `timeout`.
pub fn stop(vm: &Vm, timeout: Duration, force: bool) -> Result<String> {
    if qemu::pid(vm).is_none() {
        return Ok(t!("уже выключена"));
    }
    let by_agent = qga::ping(vm) && qga::shutdown(vm, "powerdown").is_ok();
    let mut how = if by_agent { t!("выключена гостевым агентом") } else {
        qmp::powerdown(vm)?;
        t!("выключена (ACPI)")
    };
    let t0 = Instant::now();
    // Вторая попытка с половины срока: Windows ждёт программы, которые не дают завершить работу
    // (зависший процесс, «несохранённые данные»), — `shutdown /f` закрывает их; без агента — ACPI ещё раз.
    let mut retried = false;
    while t0.elapsed() < timeout {
        if qemu::pid(vm).is_none() {
            return Ok(how);
        }
        if !retried && t0.elapsed() >= timeout / 2 {
            retried = true;
            if by_agent && vm.os.is_windows() && crate::agent::preferred(vm) {
                let argv: Vec<String> = ["shutdown", "/s", "/f", "/t", "0"].iter().map(|s| s.to_string()).collect();
                if let Ok(mut c) = crate::agent::Client::connect(vm) {
                    let _ = c.exec(&argv, false, None, &[], None, Duration::from_secs(10), &mut |_, _| {});
                }
                how = t!("выключена гостевым агентом (программы, мешавшие выключению, закрыты)");
            } else {
                let _ = qmp::powerdown(vm);
            }
            eprintln!("{}", t!("«{name}» не выключилась за {s} с — повторная попытка", name = vm.name, s = t0.elapsed().as_secs()));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if force {
        if qmp::quit(vm).is_err() {
            qemu::kill(vm);
        }
        return Ok(t!("не выключилась сама за отведённое время — остановлена принудительно"));
    }
    bail!("{}", t!("«{name}» не выключилась за {s} с (--force — выключить принудительно)", name = vm.name, s = timeout.as_secs()))
}

/// Кадр экрана гостя в PNG. syn-gpu — основная видеокарта: у QEMU экрана нет, кадр — рабочий
/// стол с карты (его размер запоминается для `--click`).
pub fn screendump(vm: &Vm, out: &Path) -> Result<()> {
    if qmp::direct_input(vm) {
        let (fw, fh, px) = crate::present::grab_screen(&vm.id, Duration::from_secs(10))?;
        image::RgbaImage::from_raw(fw, fh, px).context("кадр")?.save(out)?;
        return Ok(());
    }
    qmp::screendump(vm, out)
}

pub fn click(vm: &Vm, x: f64, y: f64, button: Option<&str>, double: bool) -> Result<()> {
    // syn-gpu — основная видеокарта: экрана QEMU нет, координаты — в пикселях рабочего стола
    // syn-gpu (размер помнит окно или прошлый снимок; иначе — один кадр с карты).
    if qmp::direct_input(vm) {
        let size = match crate::present::desktop_size(&vm.id) {
            Some(s) => s,
            None => {
                let d = crate::present::grab(&vm.id, Duration::from_secs(10), true)?;
                (d.rect.2, d.rect.3)
            }
        };
        return qmp::click(vm, x, y, size, button, double);
    }
    let tmp = vm.run_dir().join("click.png");
    qmp::screendump(vm, &tmp)?;
    let size = image::image_dimensions(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    qmp::click(vm, x, y, size, button, double)
}

/// USB по порту: в настройки и, если машина работает, сразу.
pub fn set_usb(vm: &Vm, port: &str, on: bool) -> Result<String> {
    if crate::usb::split_port(port).is_none() {
        bail!("{}", t!("неверный порт USB «{port}» (вид 3-6 или 1-2.4; список — syn-vm --usb {id} list)", port = port, id = vm.id));
    }
    let mut vm = vm.clone();
    let had = vm.usb.iter().any(|u| u.port == port);
    if on && !had {
        let name = crate::usb::at(port).map(|d| d.name).unwrap_or_default();
        vm.usb.push(model::UsbPass { port: port.into(), name });
    } else if !on {
        vm.usb.retain(|u| u.port != port);
    }
    vm.save()?;
    if on && crate::usb::at(port).is_some_and(|d| !d.writable) && !crate::usb::allowed_ports().contains(&port.to_string()) {
        crate::usb::allow(&[port.to_string()])?;
    }
    if qemu::pid(&vm).is_some() {
        let r = if on { qmp::usb_attach(&vm, port) } else { qmp::usb_detach(&vm, port) };
        match r {
            Ok(()) => {}
            // Уже подключён (или уже отключён) — не ошибка.
            Err(e) if e.to_string().contains("Duplicate") || e.to_string().contains("not found") => {}
            Err(e) => return Err(e),
        }
        if on {
            qemu::spawn_watcher(&vm, "--usb-watch");
        }
        return Ok(if on { t!("порт {port} отдан гостю (на ходу)", port = port) } else { t!("порт {port} возвращён компьютеру (на ходу)", port = port) });
    }
    Ok(if on { t!("порт {port} будет у гостя — при запуске", port = port) } else { t!("порт {port} останется у компьютера — при запуске", port = port) })
}

/// Устройство компьютера: в настройки и, если можно и машина работает, сразу.
pub fn set_device(vm: &Vm, d: HostDev, on: bool) -> Result<String> {
    let mut vm = vm.clone();
    let warn = if on { devices::check(&d)? } else { String::new() };
    let had = vm.devices.iter().any(|x| x.same(&d));
    let live = qemu::pid(&vm).is_some();
    if on {
        if had {
            vm.devices.retain(|x| !x.same(&d));
        }
        if live && d.hotplug() && !had {
            devices::grant(&devices::missing_access(std::slice::from_ref(&d)))?;
            devices::hot_attach(&vm, &d)?;
        }
        vm.devices.push(d.clone());
    } else {
        let Some(old) = vm.devices.iter().find(|x| x.same(&d)).cloned() else { return Ok(t!("{spec} и так не у гостя", spec = d.spec())) };
        if live && old.hotplug() {
            devices::hot_detach(&vm, &old)?;
        }
        vm.devices.retain(|x| !x.same(&d));
    }
    vm.save()?;
    let mut msg = format!(
        "{} {}",
        d.spec(),
        match (on, live, d.hotplug()) {
            (true, true, true) => t!("подключено (на ходу)"),
            (false, true, true) => t!("отключено (на ходу)"),
            (true, true, false) => t!("будет у гостя при следующем запуске (PCI — только при выключенной машине)"),
            (false, true, false) => t!("вернётся компьютеру после выключения машины"),
            (true, false, _) => t!("будет у гостя при запуске"),
            (false, false, _) => t!("убрано из машины"),
        }
    );
    if !warn.is_empty() {
        msg.push('\n');
        msg.push_str(&t!("внимание: {w}", w = warn));
    }
    Ok(msg)
}

fn print_devices(list: &[devices::Avail]) {
    let mut kind = "";
    for d in list {
        if d.kind != kind {
            kind = d.kind;
            println!(
                "\n## {}",
                match kind {
                    "pci" => t!("PCI (VFIO, только при выключенной машине)"),
                    "input" => t!("Ввод (evdev)"),
                    "block" => t!("Диски и разделы"),
                    "serial" => t!("Последовательные порты"),
                    _ => t!("USB по VID:PID (по порту — usb:ПОРТ, см. --usb МАШИНА list)"),
                }
            );
        }
        let mark = if !d.blocked.is_empty() { "✗" } else if !d.warn.is_empty() { "!" } else { "✓" };
        println!("{mark} {}\t{}", d.dev.spec(), d.title);
        if !d.detail.is_empty() {
            println!("    {}", d.detail);
        }
        if !d.blocked.is_empty() {
            println!("    {}", t!("нельзя: {why}", why = d.blocked));
        }
        if !d.warn.is_empty() {
            println!("    {}", t!("внимание: {w}", w = d.warn));
        }
        if d.needs_access {
            println!("    {}", t!("доступ: спросит пароль при подключении"));
        }
    }
    if !devices::iommu_on() {
        println!("\n{}", t!("IOMMU выключен — PCI недоступны (intel_iommu=on iommu=pt)"));
    }
}

pub fn snapshots(vm: &Vm, on: bool) -> Vec<qmp::Snapshot> {
    if on {
        return qmp::hmp(vm, "info snapshots").map(|s| qmp::parse_snapshots(&s)).unwrap_or_default();
    }
    let Some(d) = vm.disks.first() else { return Vec::new() };
    let out = std::process::Command::new("qemu-img").args(["snapshot", "-l", "-U"]).arg(vm.disk_path(d)).output();
    out.map(|o| qmp::parse_snapshots(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

pub fn snapshot(vm: &Vm, on: bool, op: &str, name: &str) -> Result<()> {
    if on {
        let c = match op {
            "create" => "savevm",
            "restore" => "loadvm",
            "delete" => "delvm",
            o => bail!("{}", t!("неизвестное действие {o}", o = o)),
        };
        qmp::hmp(vm, &format!("{c} \"{}\"", name.replace('"', "")))?;
        if op == "restore" {
            let re = devices::replug_usb(vm);
            if !re.is_empty() {
                eprintln!("{}", t!("USB переподключено после снимка: {list}", list = re.join(", ")));
            }
            let _ = qga::sync_time(vm);
        }
        return Ok(());
    }
    let flag = match op {
        "create" => "-c",
        "restore" => "-a",
        "delete" => "-d",
        o => bail!("{}", t!("неизвестное действие {o}", o = o)),
    };
    let mut files: Vec<_> = vm.disks.iter().map(|d| vm.disk_path(d)).filter(|p| p.extension().is_some_and(|e| e == "qcow2")).collect();
    if vm.vars_path().exists() {
        files.push(vm.vars_path());
    }
    for f in files {
        let o = std::process::Command::new("qemu-img").args(["snapshot", flag, name]).arg(&f).output()?;
        if !o.status.success() && f != vm.vars_path() {
            bail!("{}", String::from_utf8_lossy(&o.stderr).trim());
        }
    }
    Ok(())
}

fn absolute(p: &str) -> Result<PathBuf> {
    let p = Path::new(p);
    Ok(if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir()?.join(p) })
}

// ─── Настройки ключами ──────────────────────────────────────────────────

fn get_key(t: &toml::Value, k: &str) -> Option<String> {
    let v = k.split('.').try_fold(t, |v, part| match part.parse::<usize>() {
        Ok(i) => v.get(i),
        Err(_) => v.get(part),
    })?;
    Some(match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// `ключ=значение`: значение разбирается как TOML (`4`, `true`, `[…]`), иначе — строка.
/// Идти через свой агент: не просили qemu-ga (--qga, --detach) и агент на связи; --user —
/// только через него.
fn use_agent(vm: &Vm, a: &Args) -> Result<bool> {
    if a.has("--qga") || a.has("--detach") {
        if a.has("--user") {
            bail!("{}", t!("--user — только через свой агент (без --qga и --detach)"));
        }
        return Ok(false);
    }
    if vm.syn_tools() && crate::agent::installed(vm) {
        return Ok(true);
    }
    if a.has("--user") || crate::agent::installed(vm) {
        if crate::agent::online(vm) {
            return Ok(true);
        }
        if a.has("--user") {
            bail!("{}", t!("«{name}»: свой агент гостя не на связи — syn-vm --agent-install {id}", name = vm.name, id = vm.id));
        }
    }
    Ok(false)
}

/// Поставить или обновить свой агент: на связи — обновление через него самого, иначе —
/// установка через qemu-ga (exe во временный каталог гостя и `syn-vm-agent install`).
fn agent_install(vm: &Vm, file: Option<PathBuf>) -> Result<()> {
    // Агенты старых систем на связи (COM2): Windows 9x/XP обновляется сам, DOS — только заново с диска.
    if crate::agent::installed(vm) && crate::agent::online(vm) {
        let info = crate::agent::Client::connect(vm)?.info()?;
        if let Some(i) = info.filter(|i| i.agent.contains("-9x") || i.agent.contains("-dos")) {
            if i.agent.contains("-dos") {
                bail!("{}", t!("агент DOS обновляется с диска «Гостевые инструменты syn-vm»: DOS\\AGENT\\SYNAGENT.COM INSTALL"));
            }
            let exe = match file {
                Some(f) => f,
                None => crate::agent::agent_9x().context(t!("нет сборки агента SYNAGENT.EXE (/usr/lib/syn-vm/agent)"))?,
            };
            crate::agent::Client::connect(vm)?.update(&exe, r"C:\SYNVM")?;
            let i = crate::agent::Client::connect(vm)?.wait_ready(false, Duration::from_secs(60))?;
            println!("syn-vm-agent {} — {} ({})", i.agent, i.os, i.host);
            return Ok(());
        }
    }
    let windows = qga::guest_is_windows(vm) || vm.os.is_windows();
    let exe = match file {
        Some(f) => f,
        None => crate::agent::agent_binary(windows).with_context(|| t!("нет сборки агента для гостя ({f}) — SYN_VM_AGENT_EXE / SYN_VM_AGENT или /usr/lib/syn-vm/agent", f = if windows { "syn-vm-agent.exe" } else { "syn-vm-agent" }))?,
    };
    if crate::agent::installed(vm) && crate::agent::online(vm) {
        let dir = if windows { r"C:\Program Files\syn-vm" } else { "/usr/local/bin" };
        crate::agent::Client::connect(vm)?.update(&exe, dir)?;
        eprintln!("{}", t!("агент обновлён: {p} — ждём перезапуска…", p = exe.display()));
    } else {
        let tmp = if windows { r"C:\Windows\Temp\syn-vm-agent.exe" } else { "/tmp/syn-vm-agent" };
        qga::push(vm, &exe, tmp)?;
        let argv: Vec<String> = if windows { vec![tmp.into(), "install".into()] } else { vec!["sh".into(), "-c".into(), format!("chmod +x {tmp} && {tmp} install")] };
        let r = qga::exec(vm, &argv, None, &[], Duration::from_secs(120))?;
        let out = String::from_utf8_lossy(&qga::decode_output(vm, r.stdout)).into_owned() + &String::from_utf8_lossy(&qga::decode_output(vm, r.stderr));
        if r.code != 0 {
            bail!("{}", t!("установка агента: код {c}: {o}", c = r.code, o = out.trim()));
        }
        eprintln!("{}", out.trim());
    }
    // Агент вышел на связь (посредник ждёт его порт).
    let mut c = crate::agent::Client::connect(vm)?;
    let i = c.wait_ready(false, Duration::from_secs(60))?;
    println!("syn-vm-agent {} — {} ({}), {}", i.agent, i.os, i.host, t!("пользователь: {u}", u = i.user.as_deref().unwrap_or("—")));
    Ok(())
}

pub fn set_keys(vm: &Vm, kv: &[String]) -> Result<Vm> {
    let mut vals = Vec::new();
    for item in kv {
        let (k, v) = item.split_once('=').with_context(|| t!("«{item}»: ожидается КЛЮЧ=ЗНАЧЕНИЕ", item = item))?;
        let parsed = toml::from_str::<toml::Table>(&format!("v = {v}")).ok().and_then(|mut t| t.remove("v")).unwrap_or_else(|| toml::Value::String(v.to_string()));
        vals.push((k.trim().to_string(), parsed, v.to_string()));
    }
    set_values(vm, vals)
}

/// Подставить значения (ключ, значение, исходный текст — для строковых полей).
pub fn set_values(vm: &Vm, vals: Vec<(String, toml::Value, String)>) -> Result<Vm> {
    let mut t = toml::Value::try_from(vm)?;
    let tbl = t.as_table_mut().context("toml")?;
    // Пустые списки TOML не пишет — ключи берутся из умолчаний.
    let known: Vec<String> = toml::Value::try_from(Vm { disks: vec![Default::default()], forwards: vec![Default::default()], usb: vec![Default::default()], ..Vm::default() })?
        .as_table()
        .map(|t| t.keys().cloned().chain(["devices".to_string()]).collect())
        .unwrap_or_default();
    for (k, v, raw) in vals {
        if !known.contains(&k) || k == "id" {
            bail!("{}", t!("нет настройки «{k}»; есть: {list}", k = k, list = known.join(", ")));
        }
        // Строковые поля принимают и «4» как строку.
        let v = match (tbl.get(&k), v) {
            (Some(toml::Value::String(_)), p) if !p.is_str() => toml::Value::String(raw),
            (_, p) => p,
        };
        tbl.insert(k, v);
    }
    let mut out: Vm = t.try_into().with_context(|| t!("недопустимое значение"))?;
    out.id = vm.id.clone();
    Ok(out)
}

// ─── Новая машина и импорт ──────────────────────────────────────────────

fn new_vm(name: &str, a: &Args) -> Result<Vm> {
    let iso = a.get("--iso").map(|p| absolute(&p)).transpose()?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let disks: Vec<PathBuf> = a.all("--disk").iter().map(|d| std::fs::canonicalize(d).with_context(|| t!("нет диска {d}", d = d))).collect::<Result<_>>()?;
    let os = match a.get("--os") {
        Some(o) => toml::Value::String(o.clone()).try_into::<model::OsKind>().map_err(|_| anyhow::anyhow!("--os: linux, windows11, windows10, bsd, android, other"))?,
        None => model::OsKind::guess(&iso).or_else(|| disks.first().and_then(|d| model::OsKind::guess(&d.to_string_lossy()))).unwrap_or_default(),
    };
    let mut vm = os.preset();
    vm.name = name.to_string();
    if !iso.is_empty() {
        if !Path::new(&iso).exists() {
            bail!("{}", t!("нет образа {iso}", iso = iso));
        }
        vm.cdrom = iso;
        // Готовый диск — грузиться с него; образ (cloud-init seed и т. п.) — просто в приводе.
        vm.boot_cdrom = disks.is_empty();
    }
    let mut kv = Vec::new();
    for (flag, key) in [("--cpus", "cpus"), ("--memory", "memory_mb"), ("--disk-gb", "disk_gb"), ("--firmware", "firmware"), ("--disk-bus", "disk_bus"), ("--nic", "nic"), ("--display", "display")] {
        if let Some(v) = a.get(flag) {
            let v = if matches!(key, "firmware" | "disk_bus" | "nic" | "display") { format!("\"{v}\"") } else { v };
            kv.push(format!("{key}={v}"));
        }
    }
    kv.extend(a.all("--set"));
    if !kv.is_empty() {
        let id = std::mem::take(&mut vm.id);
        vm = set_keys(&vm, &kv)?;
        vm.id = id;
    }
    // «Другая» система с BIOS — скорее DOS или Windows 9x: им нужен i440fx (IDE), если чипсет не задан.
    if vm.os == model::OsKind::Other && vm.firmware == model::Firmware::Bios && !kv.iter().any(|k| k.starts_with("chipset")) {
        vm.chipset = model::Chipset::I440fx;
    }
    for f in a.all("--fwd") {
        let (udp, rest) = match f.strip_prefix("udp:") {
            Some(r) => (true, r),
            None => (false, f.strip_prefix("tcp:").unwrap_or(&f)),
        };
        let (h, g) = rest.split_once(':').with_context(|| t!("--fwd {f}: ХОСТ:ГОСТЬ", f = f))?;
        vm.forwards.push(model::PortForward { udp, host: h.parse()?, guest: g.parse()?, public: a.has("--public") });
    }
    // --share можно повторять: первая — shared_dir, остальные — shares.
    for (i, s) in a.all("--share").into_iter().enumerate() {
        let p = std::fs::canonicalize(&s).with_context(|| t!("нет папки {s}", s = s))?.to_string_lossy().into_owned();
        if i == 0 {
            vm.shared_dir = p;
        } else {
            vm.shares.push(p);
        }
    }
    let vars = a.get("--vars").map(|v| std::fs::canonicalize(&v).with_context(|| t!("нет файла {v}", v = v))).transpose()?;
    let tpm = a.get("--tpm").map(|v| std::fs::canonicalize(&v).with_context(|| t!("нет каталога {v}", v = v))).transpose()?;
    if vars.is_some() && vm.firmware == model::Firmware::Bios {
        bail!("{}", t!("--vars — переменные UEFI, а у машины BIOS (--firmware uefi или uefi-secure)"));
    }
    if tpm.is_some() {
        vm.tpm = true;
    }
    if !disks.is_empty() {
        vm.disk_gb = 0;
    }
    let mut vm = model::create(vm, disks.first().cloned())?;
    let r = (|| -> Result<()> {
        for d in disks.iter().skip(1) {
            vm.disks.push(model::Disk { file: d.to_string_lossy().into_owned() });
        }
        if let Some(v) = &vars {
            let _ = std::fs::remove_file(vm.vars_path());
            import_vars(v, &vm.vars_path())?;
        }
        if let Some(t) = &tpm {
            let dst = vm.dir().join("tpm");
            let _ = std::fs::remove_dir_all(&dst);
            let st = std::process::Command::new("cp").arg("-a").arg(t).arg(&dst).status()?;
            if !st.success() {
                bail!("{}", t!("не удалось скопировать {p}", p = t.display()));
            }
        }
        vm.save()
    })();
    if let Err(e) = r {
        let _ = model::delete(&vm);
        return Err(e);
    }
    for w in qemu::port_conflicts(&vm).into_iter().chain(qemu::port_shared_with(&vm)) {
        eprintln!("{}", t!("внимание: {w}", w = w));
    }
    if !disks.is_empty() {
        eprintln!("{}", t!("диск остаётся на месте: {list}", list = disks.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")));
    }
    Ok(vm)
}

/// Переменные UEFI: raw или qcow2 → qcow2 в папке машины (снимки сохраняют и их).
fn import_vars(src: &Path, dst: &Path) -> Result<()> {
    let info = std::process::Command::new("qemu-img").args(["info", "--output=json", "-U"]).arg(src).output()?;
    let fmt = serde_json::from_slice::<Value>(&info.stdout).ok().and_then(|v| v.get("format").and_then(Value::as_str).map(String::from)).unwrap_or_else(|| "raw".into());
    let o = std::process::Command::new("qemu-img").args(["convert", "-q", "-f", &fmt, "-O", "qcow2"]).arg(src).arg(dst).output()?;
    if !o.status.success() {
        bail!("{}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_keys_types() {
        let mut vm = model::OsKind::Linux.preset();
        vm.id = "x".into();
        let v = set_keys(&vm, &["cpus=6".into(), "guest_agent=false".into(), "disk_bus=\"sata\"".into(), "name=Тест 1".into(), "extra_args=-s".into()]).unwrap();
        assert_eq!(v.cpus, 6);
        assert!(!v.guest_agent);
        assert_eq!(v.disk_bus, model::DiskBus::Sata);
        assert_eq!(v.name, "Тест 1");
        assert_eq!(v.extra_args, "-s");
        assert_eq!(v.id, "x");
        assert!(set_keys(&vm, &["nope=1".into()]).is_err());
        assert!(set_keys(&vm, &["cpus=abc".into()]).is_err());
        let v = set_keys(&vm, &["forwards=[{host=2222, guest=22}]".into()]).unwrap();
        assert_eq!(v.forwards[0].host, 2222);
        let v = set_keys(&vm, &["display=syn-gpu".into()]).unwrap();
        assert_eq!(v.display, model::DisplayModel::SynGpu);
        assert!(v.gpu_on());
    }

    #[test]
    fn args_parse() {
        let a = Args::parse(["vm", "--timeout", "5", "--json", "--", "ls", "-la"].iter().map(|s| s.to_string()).collect());
        assert_eq!(a.pos, vec!["vm"]);
        assert_eq!(a.get("--timeout").as_deref(), Some("5"));
        assert!(a.has("--json"));
        assert_eq!(a.tail, vec!["ls", "-la"]);
    }
}
