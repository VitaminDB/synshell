//! syndroid — управление Android в synshell (CLI; окно управления — следующий этап).

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use syndroid::api::{self, Request, Response, Session};
use syndroid::container;

const HELP: &str = "syndroid — Android-приложения в synshell

  syndroid                            окно управления
  syndroid status                     состояние Android
  syndroid start | stop | restart     запуск (для текущего сеанса), остановка, перезапуск
  syndroid freeze | unfreeze          заморозить / разморозить
  syndroid show [--instance ЭКЗ]      весь Android одним окном (запустит/переключит Android, если нужно)
  syndroid session                    из автозапуска сеанса: запустить Android, если autostart = true
  syndroid app list                   приложения
  syndroid app launch [--instance ЭКЗ] ПАКЕТ | stop ПАКЕТ
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

/// Нужный экземпляр Android запущен и загружен: другой — переключить (свежий набор этого экземпляра,
/// перезапуск), остановленный — запустить для текущего сеанса; дождаться загрузки.
fn ensure_running(instance: Option<&str>) -> Result<()> {
    use syndroid::api::State;
    let mut s = status()?;
    if let Some(inst) = instance {
        if s.instance.as_deref() != Some(inst) {
            let set = syndroid::images::latest_of(inst).with_context(|| format!("нет образов Android «{inst}»"))?;
            eprintln!("Переключение на {}…", syndroid::images::instance_title(inst));
            api::call(&Request::UseImages { name: set.name })?;
            if s.state != State::Stopped {
                api::call(&Request::Restart)?;
                eprintln!("Android перезапускается…");
            }
            s = status()?;
        }
    }
    match s.state {
        State::Running | State::Frozen => return Ok(()),
        State::Stopped => {
            api::call(&Request::Start { session: Session::from_env()? })?;
            eprintln!("Android запускается…");
        }
        State::Starting | State::Stopping => {}
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let s = status()?;
        match s.state {
            State::Running | State::Frozen => return Ok(()),
            State::Stopped => bail!("Android не запустился: {}", s.error.unwrap_or_default()),
            _ if std::time::Instant::now() > deadline => bail!("Android не загрузился за 3 минуты"),
            _ => {}
        }
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
        [] => syndroid::gui::run(),
        ["help"] | ["-h"] | ["--help"] => println!("{HELP}"),
        ["status"] => {
            let s = status()?;
            println!("Android: {:?}", s.state);
            if let Some(t) = &s.instance_title {
                println!("экземпляр: {t} ({})", s.instance.as_deref().unwrap_or(""));
            }
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
        ["show"] => {
            ensure_running(None)?;
            ok(api::call(&Request::ShowFullUi)?)
        }
        ["show", "--instance", inst] => {
            ensure_running(Some(inst))?;
            ok(api::call(&Request::ShowFullUi)?)
        }
        // Из автозапуска сеанса: запустить Android, если так настроено
        ["session"] => {
            if let Response::Config { config } = api::call(&Request::GetConfig)? {
                if config.autostart && status()?.state == syndroid::api::State::Stopped {
                    api::call(&Request::Start { session: Session::from_env()? })?;
                }
            }
        }
        ["app"] | ["app", "list"] => {
            if let Response::Apps { apps } = api::call(&Request::Apps)? {
                for a in apps {
                    println!("{}", a.package);
                }
            }
        }
        ["app", "launch", pkg] => {
            ensure_running(None)?;
            ok(api::call(&Request::LaunchApp { package: pkg.to_string() })?)
        }
        ["app", "launch", "--instance", inst, pkg] => {
            ensure_running(Some(inst))?;
            ok(api::call(&Request::LaunchApp { package: pkg.to_string() })?)
        }
        ["app", "stop", pkg] => ok(api::call(&Request::StopApp { package: pkg.to_string() })?),
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
            if let Response::Job { id } = api::call(&Request::FetchImages { system_type: None })? {
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
