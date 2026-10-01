//! syndroid — управление Android в synshell (CLI; окно управления — следующий этап).

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use syndroid::api::{self, Request, Response, Session};
use syndroid::container;

const HELP: &str = "syndroid — Android-приложения в synshell

  syndroid status                     состояние Android
  syndroid start | stop | restart     запуск (для текущего сеанса), остановка, перезапуск
  syndroid freeze | unfreeze          заморозить / разморозить
  syndroid image list                 наборы образов
  syndroid image check                свежие сборки в OTA-каналах
  syndroid image fetch                скачать последний набор (system + vendor)
  syndroid image import SYSTEM VENDOR [ИМЯ]   набор из zip/img
  syndroid image use ИМЯ | remove ИМЯ
  syndroid shell [КОМАНДА…]           оболочка Android (root)
  syndroid logcat [АРГУМЕНТЫ…]
  syndroid config                     настройки (TOML)";

fn ok(r: Response) {
    if let Response::Error { message } = r {
        eprintln!("{message}");
    }
}

fn status() -> Result<syndroid::api::Status> {
    match api::call(&Request::Status)? {
        Response::Status(s) => Ok(s),
        _ => bail!("неожиданный ответ"),
    }
}

fn human(b: u64) -> String {
    let mb = b as f64 / 1048576.0;
    if mb >= 1024.0 {
        format!("{:.2} ГБ", mb / 1024.0)
    } else {
        format!("{mb:.0} МБ")
    }
}

/// Показывать ход задания, пока не кончится.
fn follow(id: u64) -> Result<()> {
    let mut last = String::new();
    loop {
        let s = status()?;
        let Some(j) = s.jobs.iter().find(|j| j.id == id) else { bail!("задание пропало") };
        let line = if j.total > 0 {
            format!("{}: {} из {} ({}%)", j.step, human(j.done), human(j.total), j.done * 100 / j.total)
        } else if j.done > 0 {
            format!("{}: {}", j.step, human(j.done))
        } else {
            j.step.clone()
        };
        if line != last {
            print!("\r\x1b[K{line}");
            std::io::stdout().flush()?;
            last = line;
        }
        if j.finished {
            println!();
            if let Some(e) = &j.error {
                bail!("{e}");
            }
            println!("готово");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn in_container(argv: Vec<String>) -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        bail!("нужен root: sudo syndroid {}", std::env::args().skip(1).collect::<Vec<_>>().join(" "));
    }
    let pid = status()?.init_pid.context("Android не запущен")?;
    let err = container::command_in(pid, &argv)?.exec();
    Err(err.into())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("__exec") {
        container::exec_main(&args[1..]);
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        [] | ["help"] | ["-h"] | ["--help"] => println!("{HELP}"),
        ["status"] => {
            let s = status()?;
            println!("Android: {:?}", s.state);
            if let Some(i) = &s.image {
                println!("образы: {i}");
            }
            if let Some(v) = &s.android_version {
                println!("версия Android: {v}");
            }
            if let Some(u) = s.uptime {
                println!("работает: {} мин {} с", u / 60, u % 60);
            }
            if let Some(p) = s.init_pid {
                println!("init: PID {p}");
            }
            if let Some(e) = &s.error {
                println!("ошибка: {e}");
            }
            for j in s.jobs.iter().filter(|j| !j.finished) {
                println!("задание {}: {} — {}", j.id, j.title, j.step);
            }
        }
        ["start"] => {
            ok(api::call(&Request::Start { session: Session::from_env()? })?);
            println!("Android запускается (syndroid status)");
        }
        ["stop"] => ok(api::call(&Request::Stop)?),
        ["restart"] => ok(api::call(&Request::Restart)?),
        ["freeze"] => ok(api::call(&Request::Freeze)?),
        ["unfreeze"] => ok(api::call(&Request::Unfreeze)?),
        ["image"] | ["image", "list"] => {
            if let Response::Images { sets, active } = api::call(&Request::Images)? {
                if sets.is_empty() {
                    println!("образов нет: syndroid image fetch");
                }
                for s in sets {
                    let mark = if active.as_deref() == Some(&s.name) { "*" } else { " " };
                    println!("{mark} {}  ({})", s.name, human(s.size));
                }
            }
        }
        ["image", "check"] => {
            if let Response::Updates { system, vendor, installed } = api::call(&Request::CheckUpdates)? {
                println!("system: {} ({})", system.filename, human(system.size));
                println!("vendor: {} ({})", vendor.filename, human(vendor.size));
                println!("{}", if installed { "уже установлен" } else { "новый набор: syndroid image fetch" });
            }
        }
        ["image", "fetch"] => {
            if let Response::Job { id } = api::call(&Request::FetchImages)? {
                follow(id)?;
            }
        }
        ["image", "import", sys, ven, rest @ ..] => {
            let abs = |p: &str| std::fs::canonicalize(p).map(|p| p.display().to_string()).with_context(|| p.to_string());
            let req = Request::ImportImages { system: abs(sys)?, vendor: abs(ven)?, name: rest.first().map(|s| s.to_string()) };
            if let Response::Job { id } = api::call(&req)? {
                follow(id)?;
            }
        }
        ["image", "use", name] => ok(api::call(&Request::UseImages { name: name.to_string() })?),
        ["image", "remove", name] => ok(api::call(&Request::RemoveImages { name: name.to_string() })?),
        ["shell"] => in_container(vec!["/system/bin/sh".into()])?,
        ["shell", cmd @ ..] => in_container(cmd.iter().map(|s| s.to_string()).collect())?,
        ["logcat", rest @ ..] => {
            let mut v = vec!["/system/bin/logcat".to_string()];
            v.extend(rest.iter().map(|s| s.to_string()));
            in_container(v)?
        }
        ["config"] => {
            if let Response::Config { config } = api::call(&Request::GetConfig)? {
                print!("{}", toml::to_string_pretty(&config)?);
            }
        }
        _ => {
            eprintln!("{HELP}");
            std::process::exit(2);
        }
    }
    Ok(())
}
