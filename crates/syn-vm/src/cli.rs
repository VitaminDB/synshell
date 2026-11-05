//! Управление машинами из командной строки — для скриптов, проверок и агентов: без окна,
//! с кодами возврата. Те же действия доступны MCP-серверу (`mcp.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::model::{self, HostDev, Vm};
use crate::{devices, qemu, qga, qmp};

pub const HELP: &str = "\
syn-vm — виртуальные машины QEMU/KVM

Окно:
  syn-vm [МАШИНА] [--tab screen|overview|settings|snapshots] [--new-window]
  syn-vm --screenshot СНИМОК.png [--size ШxВ] [--scale N] [--open МАШИНА] [--tab …] [--wizard ШАГ]   снимок окна

Машины (МАШИНА — имя или id):
  --list [--json]                        машины и их состояние
  --status МАШИНА [--json]               состояние, агент, система гостя, адреса, порты
  --start МАШИНА [--wait-boot [СЕК]]     запустить без окна (и ждать гостевого агента)
  --stop МАШИНА [--force] [--timeout СЕК]  выключить (агентом или ACPI); --force — выдернуть шнур по тайм-ауту
  --kill МАШИНА                          выключить сразу
  --reset | --pause | --resume МАШИНА
  --wait-boot МАШИНА [--timeout СЕК]     ждать, пока ответит гостевой агент (300 с)
  --new ИМЯ [--iso ОБРАЗ] [--os linux|windows11|windows10|bsd|android|other]
        [--disk ФАЙЛ …] [--vars ФАЙЛ] [--tpm КАТАЛОГ] [--cpus N] [--memory МБ] [--disk-gb N]
        [--firmware bios|uefi|uefi-secure] [--disk-bus virtio|sata|nvme] [--nic virtio|e1000e|rtl8139]
        [--fwd [udp:]ХОСТ:ГОСТЬ …] [--public] [--share КАТАЛОГ] [--set КЛЮЧ=ЗНАЧЕНИЕ …]
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
  --push МАШИНА ФАЙЛ ПУТЬ_В_ГОСТЕ        файл в гостя
  --pull МАШИНА ПУТЬ_В_ГОСТЕ ФАЙЛ        файл из гостя
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

Запись экрана гостя (родное разрешение, ровная частота, ffmpeg: NVENC/VAAPI/x264):
  --record МАШИНА [ФАЙЛ.mp4] [--fps 30|60] [--audio|--no-audio] [--duration СЕК]
                                          без --duration — в фоне до --record-stop; звук — если он включён у машины
  --record-stop МАШИНА                   остановить, дождаться файла, итог

Прочее:
  --snapshot МАШИНА list|create|restore|delete [ИМЯ]
  --qmp МАШИНА КОМАНДА [JSON]            сырая команда QMP
  --mcp                                   MCP-сервер (stdio): claude mcp add syn-vm -- syn-vm --mcp
";

/// Машина по id или имени (без учёта регистра).
pub fn find(s: &str) -> Result<Vm> {
    let all = model::load_all();
    let l = s.to_lowercase();
    all.iter().find(|v| v.id == s).or_else(|| all.iter().find(|v| v.name.to_lowercase() == l)).cloned().with_context(|| format!("нет машины «{s}» (syn-vm --list)"))
}

fn running(vm: &Vm) -> Result<()> {
    if qemu::pid(vm).is_none() {
        bail!("«{}» не запущена (syn-vm --start {})", vm.name, vm.id);
    }
    Ok(())
}

/// Разбор хвоста: флаги со значениями и без, позиционные; всё после `--` — команда.
struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
    tail: Vec<String>,
}

const FLAGS: &[&str] = &["--audio", "--no-audio", "--raw", "--public", "--json", "--force", "--yes", "--stdin", "--right", "--middle", "--double", "--move", "--wait-boot"];

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
        self.pos.get(i).cloned().with_context(|| format!("нужен аргумент: {what}"))
    }
    fn secs(&self, k: &str, def: u64) -> Duration {
        Duration::from_secs(self.get(k).and_then(|s| s.parse().ok()).unwrap_or(def))
    }
}

/// Команда CLI → код возврата; `None` — это не команда CLI (окно).
pub fn run(cmd: &str, rest: Vec<String>) -> Option<i32> {
    let known = [
        "--list", "--status", "--start", "--stop", "--kill", "--reset", "--pause", "--resume", "--wait-boot", "--new", "--delete", "--get", "--set", "--exec", "--sh", "--push",
        "--pull", "--agent", "--sync-time", "--screendump", "--type", "--key", "--click", "--scroll", "--devices", "--usb", "--attach", "--detach", "--cdrom", "--tools", "--snapshot",
        "--qmp", "--mcp", "--vfio-helper", "--vfio-watch", "--record", "--record-stop", "--record-run",
    ];
    if !known.contains(&cmd) {
        return None;
    }
    let a = Args::parse(rest.clone());
    let r = match cmd {
        "--mcp" => crate::mcp::run().map(|_| 0),
        "--vfio-helper" => devices::vfio_helper(&rest).map(|_| 0),
        "--record-run" => (|| -> Result<i32> {
            let vm = find(rest.first().context("МАШИНА")?)?;
            let dur: f64 = rest.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            crate::record::run(
                &vm,
                crate::record::Opts {
                    out: PathBuf::from(rest.get(1).context("ФАЙЛ")?),
                    fps: rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(30),
                    audio: rest.get(3).is_some_and(|s| s == "1"),
                    duration: (dur > 0.0).then(|| Duration::from_secs_f64(dur)),
                },
            )?;
            Ok(0)
        })(),
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
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let s = status(&vm);
            if json_out {
                println!("{s}");
            } else {
                print_status(&s);
            }
            Ok(if s["state"] == "off" { 3 } else { 0 })
        }
        "--start" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            start(&vm)?;
            if a.has("--wait-boot") {
                let t = a.get("--wait-boot").and_then(|s| s.parse().ok()).unwrap_or(300);
                qga::wait(&vm, Duration::from_secs(t))?;
                println!("{}: агент отвечает", vm.name);
            }
            Ok(0)
        }
        "--stop" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let how = stop(&vm, a.secs("--timeout", 90), a.has("--force"))?;
            println!("{}: {how}", vm.name);
            Ok(0)
        }
        "--kill" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            if qmp::quit(&vm).is_err() {
                qemu::kill(&vm);
            }
            Ok(0)
        }
        "--reset" | "--pause" | "--resume" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            match cmd {
                "--reset" => qmp::reset(&vm)?,
                "--pause" => qmp::pause(&vm)?,
                _ => qmp::resume(&vm)?,
            }
            Ok(0)
        }
        "--wait-boot" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let t = a.get("--wait-boot").and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(a.secs("--timeout", 300));
            let t0 = Instant::now();
            qga::wait(&vm, t)?;
            println!("{}: агент отвечает ({:.0} с)", vm.name, t0.elapsed().as_secs_f64());
            Ok(0)
        }
        "--new" => {
            let vm = new_vm(&a.pos(0, "ИМЯ")?, a)?;
            println!("{}", vm.id);
            Ok(0)
        }
        "--delete" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            if !a.has("--yes") {
                bail!("удаление «{}» со всеми дисками в {} — подтвердите флагом --yes", vm.name, vm.dir().display());
            }
            if qemu::pid(&vm).is_some() {
                bail!("«{}» работает — сначала --stop", vm.name);
            }
            model::delete(&vm)?;
            Ok(0)
        }
        "--get" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let t = toml::Value::try_from(&vm)?;
            match a.pos.get(1) {
                Some(k) => println!("{}", get_key(&t, k).with_context(|| format!("нет ключа {k}"))?),
                None => print!("{}", toml::to_string_pretty(&vm)?),
            }
            Ok(0)
        }
        "--set" => {
            let mut vm = find(&a.pos(0, "МАШИНА")?)?;
            let kv: Vec<String> = a.pos[1..].to_vec();
            if kv.is_empty() {
                bail!("нужно КЛЮЧ=ЗНАЧЕНИЕ");
            }
            vm = set_keys(&vm, &kv)?;
            vm.save()?;
            if qemu::pid(&vm).is_none() {
                for w in qemu::port_conflicts(&vm).into_iter().chain(qemu::port_shared_with(&vm)) {
                    eprintln!("внимание: {w}");
                }
            }
            if qemu::pid(&vm).is_some() {
                eprintln!("{}: машина работает — изменения вступят в силу при следующем запуске", vm.name);
            }
            Ok(0)
        }
        "--exec" | "--sh" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
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
                bail!("нет команды: syn-vm --exec МАШИНА -- ПРОГРАММА [АРГ…]");
            }
            let input = if a.has("--stdin") {
                let mut b = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut b)?;
                Some(b)
            } else {
                None
            };
            let mut r = qga::exec(&vm, &argv, input.as_deref(), &a.all("--env"), a.secs("--timeout", 600))?;
            if !a.has("--raw") {
                r.stdout = qga::decode_output(&vm, std::mem::take(&mut r.stdout));
                r.stderr = qga::decode_output(&vm, std::mem::take(&mut r.stderr));
            }
            use std::io::Write;
            std::io::stdout().write_all(&r.stdout)?;
            std::io::stderr().write_all(&r.stderr)?;
            if r.timed_out {
                eprintln!("syn-vm: не дождались (pid {} в госте работает дальше)", r.pid);
            }
            Ok(r.code)
        }
        "--push" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            let n = qga::push(&vm, Path::new(&a.pos(1, "ФАЙЛ")?), &a.pos(2, "ПУТЬ_В_ГОСТЕ")?)?;
            eprintln!("{n} байт");
            Ok(0)
        }
        "--pull" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            let n = qga::pull(&vm, &a.pos(1, "ПУТЬ_В_ГОСТЕ")?, Path::new(&a.pos(2, "ФАЙЛ")?))?;
            eprintln!("{n} байт");
            Ok(0)
        }
        "--agent" | "--qmp" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            let c = a.pos(1, "КОМАНДА")?;
            let args: Value = match a.pos.get(2) {
                Some(s) => serde_json::from_str(s).context("аргументы — JSON")?,
                None => Value::Null,
            };
            let r = if cmd == "--agent" { qga::call(&vm, &c, args)? } else { qmp::call(&vm, &c, args)? };
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(0)
        }
        "--sync-time" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            qga::sync_time(&vm)?;
            Ok(0)
        }
        "--screendump" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            let out = absolute(&a.pos(1, "ФАЙЛ.png")?)?;
            qmp::screendump(&vm, &out)?;
            let (w, h) = image::image_dimensions(&out).unwrap_or((0, 0));
            println!("{}: {w}x{h}", out.display());
            Ok(0)
        }
        "--type" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            let text = a.pos[1..].join(" ").replace("\\n", "\n");
            qmp::type_text(&vm, &text, Duration::from_millis(a.get("--delay").and_then(|s| s.parse().ok()).unwrap_or(15)))?;
            Ok(0)
        }
        "--key" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            for c in &a.pos[1..] {
                qmp::send_combo(&vm, c)?;
                std::thread::sleep(Duration::from_millis(80));
            }
            Ok(0)
        }
        "--click" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
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
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            running(&vm)?;
            qmp::wheel(&vm, a.pos(1, "ШАГИ")?.parse()?)?;
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
        "--usb" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            match a.pos.get(1).map(String::as_str).unwrap_or("list") {
                "list" => {
                    for u in crate::usb::list().into_iter().filter(|u| !u.hub) {
                        let on = vm.usb.iter().any(|p| p.port == u.port);
                        println!("{}\t{}\t{}:{}\t{}", u.port, if on { "guest" } else { "host" }, u.vendor, u.product, u.name);
                    }
                    for p in vm.usb.iter().filter(|p| crate::usb::at(&p.port).is_none()) {
                        println!("{}\tguest\t(пусто)\t{}", p.port, p.name);
                    }
                    Ok(0)
                }
                op @ ("attach" | "detach") => {
                    let port = a.pos(2, "ПОРТ")?;
                    let msg = set_usb(&vm, &port, op == "attach")?;
                    println!("{msg}");
                    Ok(0)
                }
                o => bail!("--usb МАШИНА list|attach|detach ПОРТ (не «{o}»)"),
            }
        }
        "--attach" | "--detach" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let spec = a.pos(1, "ТИП:ИД")?;
            let msg = if let Some(port) = spec.strip_prefix("usb:") {
                set_usb(&vm, port, cmd == "--attach")?
            } else {
                set_device(&vm, devices::parse_spec(&spec)?, cmd == "--attach")?
            };
            println!("{msg}");
            Ok(0)
        }
        "--cdrom" => {
            let mut vm = find(&a.pos(0, "МАШИНА")?)?;
            let iso = a.pos(1, "ОБРАЗ|eject")?;
            let iso = if iso == "eject" || iso.is_empty() { String::new() } else { absolute(&iso)?.to_string_lossy().into_owned() };
            vm.cdrom = iso.clone();
            vm.save()?;
            if qemu::pid(&vm).is_some() {
                qmp::change_cdrom(&vm, &iso)?;
            }
            Ok(0)
        }
        "--tools" => {
            let mut vm = find(&a.pos(0, "МАШИНА")?)?;
            match a.pos.get(1).map(String::as_str).unwrap_or("on") {
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
                        None if vm.tools_cd => eprintln!("virtio-win.iso не найден — syn-vm --tools {} download", vm.id),
                        _ => {}
                    }
                    if qemu::pid(&vm).is_some() {
                        eprintln!("второй привод появится при следующем запуске; сейчас можно вставить в первый: syn-vm --cdrom {} ОБРАЗ", vm.id);
                    }
                }
            }
            Ok(0)
        }
        "--record" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
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
            println!("запись: {} ({}x{}, {} к/с, {}{})", i.path, i.size.0, i.size.1, i.fps, i.encoder, if i.audio { ", со звуком" } else { "" });
            if wait {
                let d = crate::record::wait(&vm)?;
                print_record(&d);
            } else {
                println!("остановить: syn-vm --record-stop {}", vm.id);
            }
            Ok(0)
        }
        "--record-stop" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let d = crate::record::stop(&vm)?;
            print_record(&d);
            Ok(if d.done.as_ref().is_some_and(|s| s.error.is_empty()) { 0 } else { 1 })
        }
        "--snapshot" => {
            let vm = find(&a.pos(0, "МАШИНА")?)?;
            let op = a.pos(1, "list|create|restore|delete")?;
            let on = qemu::pid(&vm).is_some();
            if op == "list" {
                for s in snapshots(&vm, on) {
                    println!("{}\t{}\t{}", s.name, s.date, s.vm_size);
                }
                return Ok(0);
            }
            let name = a.pos(2, "ИМЯ")?;
            snapshot(&vm, on, &op, &name)?;
            Ok(0)
        }
        _ => bail!("неизвестная команда {cmd}"),
    }
}

pub fn record_summary(i: &crate::record::Info) -> String {
    match &i.done {
        Some(s) => format!(
            "{}: {:.1} с, {} кадров ({} к/с, {}x{}, {}), от гостя {} обновлений, повторов из-за задержек {}, звук {:.1} с, {}{}",
            i.path,
            s.seconds,
            s.frames,
            i.fps,
            i.size.0,
            i.size.1,
            i.encoder,
            s.updates,
            s.late,
            s.audio_seconds,
            if s.bytes >= 1_000_000 { format!("{:.1} МБ", s.bytes as f64 / 1e6) } else { format!("{} КБ", s.bytes / 1000) },
            if s.error.is_empty() { String::new() } else { format!("\nошибка: {}", s.error) }
        ),
        None => format!("{}: идёт запись", i.path),
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
    println!("{} ({})\nсостояние: {}\nсистема: {}, {} ядер, {} МБ", g("name"), g("id"), g("state"), g("os"), g("cpus"), g("memory_mb"));
    if let Some(gst) = s.get("guest") {
        println!("гость: {} ({}), адреса: {}", gst["os"].as_str().unwrap_or(""), gst["hostname"].as_str().unwrap_or(""), gst["ips"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default());
    } else if s.get("agent").is_some() {
        println!("агент: {}", g("agent"));
    }
    for k in ["forwards", "usb", "devices"] {
        if s[k].as_array().is_some_and(|a| !a.is_empty()) {
            println!("{k}: {}", s[k]);
        }
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
        return Ok("уже выключена".into());
    }
    let how = if qga::ping(vm) && qga::shutdown(vm, "powerdown").is_ok() { "выключена гостевым агентом" } else {
        qmp::powerdown(vm)?;
        "выключена (ACPI)"
    };
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if qemu::pid(vm).is_none() {
            return Ok(how.into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if force {
        if qmp::quit(vm).is_err() {
            qemu::kill(vm);
        }
        return Ok("не выключилась сама за отведённое время — остановлена принудительно".into());
    }
    bail!("«{}» не выключилась за {} с (--force — выключить принудительно)", vm.name, timeout.as_secs())
}

pub fn click(vm: &Vm, x: f64, y: f64, button: Option<&str>, double: bool) -> Result<()> {
    let tmp = vm.run_dir().join("click.png");
    qmp::screendump(vm, &tmp)?;
    let size = image::image_dimensions(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    qmp::click(vm, x, y, size, button, double)
}

/// USB по порту: в настройки и, если машина работает, сразу.
pub fn set_usb(vm: &Vm, port: &str, on: bool) -> Result<String> {
    if crate::usb::split_port(port).is_none() {
        bail!("неверный порт USB «{port}» (вид 3-6 или 1-2.4; список — syn-vm --usb {} list)", vm.id);
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
        return Ok(format!("порт {port} {} (на ходу)", if on { "отдан гостю" } else { "возвращён компьютеру" }));
    }
    Ok(format!("порт {port} {} — при запуске", if on { "будет у гостя" } else { "останется у компьютера" }))
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
        let Some(old) = vm.devices.iter().find(|x| x.same(&d)).cloned() else { return Ok(format!("{} и так не у гостя", d.spec())) };
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
            (true, true, true) => "подключено (на ходу)",
            (false, true, true) => "отключено (на ходу)",
            (true, true, false) => "будет у гостя при следующем запуске (PCI — только при выключенной машине)",
            (false, true, false) => "вернётся компьютеру после выключения машины",
            (true, false, _) => "будет у гостя при запуске",
            (false, false, _) => "убрано из машины",
        }
    );
    if !warn.is_empty() {
        msg.push_str(&format!("\nвнимание: {warn}"));
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
                    "pci" => "PCI (VFIO, только при выключенной машине)",
                    "input" => "Ввод (evdev)",
                    "block" => "Диски и разделы",
                    "serial" => "Последовательные порты",
                    _ => "USB по VID:PID (по порту — usb:ПОРТ, см. --usb МАШИНА list)",
                }
            );
        }
        let mark = if !d.blocked.is_empty() { "✗" } else if !d.warn.is_empty() { "!" } else { "✓" };
        println!("{mark} {}\t{}", d.dev.spec(), d.title);
        if !d.detail.is_empty() {
            println!("    {}", d.detail);
        }
        if !d.blocked.is_empty() {
            println!("    нельзя: {}", d.blocked);
        }
        if !d.warn.is_empty() {
            println!("    внимание: {}", d.warn);
        }
        if d.needs_access {
            println!("    доступ: спросит пароль при подключении");
        }
    }
    if !devices::iommu_on() {
        println!("\nIOMMU выключен — PCI недоступны (intel_iommu=on iommu=pt)");
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
            o => bail!("неизвестное действие {o}"),
        };
        qmp::hmp(vm, &format!("{c} \"{}\"", name.replace('"', "")))?;
        if op == "restore" {
            let _ = qga::sync_time(vm);
        }
        return Ok(());
    }
    let flag = match op {
        "create" => "-c",
        "restore" => "-a",
        "delete" => "-d",
        o => bail!("неизвестное действие {o}"),
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
pub fn set_keys(vm: &Vm, kv: &[String]) -> Result<Vm> {
    let mut vals = Vec::new();
    for item in kv {
        let (k, v) = item.split_once('=').with_context(|| format!("«{item}»: ожидается КЛЮЧ=ЗНАЧЕНИЕ"))?;
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
            bail!("нет настройки «{k}»; есть: {}", known.join(", "));
        }
        // Строковые поля принимают и «4» как строку.
        let v = match (tbl.get(&k), v) {
            (Some(toml::Value::String(_)), p) if !p.is_str() => toml::Value::String(raw),
            (_, p) => p,
        };
        tbl.insert(k, v);
    }
    let mut out: Vm = t.try_into().context("недопустимое значение")?;
    out.id = vm.id.clone();
    Ok(out)
}

// ─── Новая машина и импорт ──────────────────────────────────────────────

fn new_vm(name: &str, a: &Args) -> Result<Vm> {
    let iso = a.get("--iso").map(|p| absolute(&p)).transpose()?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let disks: Vec<PathBuf> = a.all("--disk").iter().map(|d| std::fs::canonicalize(d).with_context(|| format!("нет диска {d}"))).collect::<Result<_>>()?;
    let os = match a.get("--os") {
        Some(o) => toml::Value::String(o.clone()).try_into::<model::OsKind>().map_err(|_| anyhow::anyhow!("--os: linux, windows11, windows10, bsd, android, other"))?,
        None => model::OsKind::guess(&iso).or_else(|| disks.first().and_then(|d| model::OsKind::guess(&d.to_string_lossy()))).unwrap_or_default(),
    };
    let mut vm = os.preset();
    vm.name = name.to_string();
    if !iso.is_empty() {
        if !Path::new(&iso).exists() {
            bail!("нет образа {iso}");
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
    for f in a.all("--fwd") {
        let (udp, rest) = match f.strip_prefix("udp:") {
            Some(r) => (true, r),
            None => (false, f.strip_prefix("tcp:").unwrap_or(&f)),
        };
        let (h, g) = rest.split_once(':').with_context(|| format!("--fwd {f}: ХОСТ:ГОСТЬ"))?;
        vm.forwards.push(model::PortForward { udp, host: h.parse()?, guest: g.parse()?, public: a.has("--public") });
    }
    if let Some(s) = a.get("--share") {
        vm.shared_dir = std::fs::canonicalize(&s).with_context(|| format!("нет папки {s}"))?.to_string_lossy().into_owned();
    }
    let vars = a.get("--vars").map(|v| std::fs::canonicalize(&v).with_context(|| format!("нет файла {v}"))).transpose()?;
    let tpm = a.get("--tpm").map(|v| std::fs::canonicalize(&v).with_context(|| format!("нет каталога {v}"))).transpose()?;
    if vars.is_some() && vm.firmware == model::Firmware::Bios {
        bail!("--vars — переменные UEFI, а у машины BIOS (--firmware uefi или uefi-secure)");
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
                bail!("не удалось скопировать {}", t.display());
            }
        }
        vm.save()
    })();
    if let Err(e) = r {
        let _ = model::delete(&vm);
        return Err(e);
    }
    for w in qemu::port_conflicts(&vm).into_iter().chain(qemu::port_shared_with(&vm)) {
        eprintln!("внимание: {w}");
    }
    if !disks.is_empty() {
        eprintln!("диск остаётся на месте: {}", disks.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", "));
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
