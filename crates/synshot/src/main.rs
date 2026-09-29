//! synshot — снимок экрана с выбором, как Spectacle в режиме
//! «Прямоугольная область», но без рисования: экран застывает, поверх —
//! затемнение; рамкой выделяется область (ручки, перенос, стрелки), щелчок
//! берёт окно под указателем или весь монитор. Затем — «Копировать»,
//! «Сохранить», «Копировать путь», «Открыть» или отмена.
//!
//! `synshot` — застыть сейчас (попросить композитор снять
//! выводы). `--capture ФАЙЛ` — кадр уже снят композитором по Print.

mod output;
mod overlay;
mod sel;
mod shot;
mod ui;

use std::os::fd::AsRawFd;

use synshell_common::{paths, Config};

use crate::shot::Shot;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .format_timestamp_millis()
        .init();

    let mut capture: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--capture" => capture = it.next(),
            "-h" | "--help" => {
                println!("synshot [--capture ФАЙЛ]\n\nВыделить область или окно и сохранить снимок экрана.");
                return;
            }
            "-V" | "--version" => {
                println!("synshot {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }

    // Один оверлей за раз: повторный Print, пока он открыт, ничего не делает.
    let _lock = match single_instance() {
        Some(l) => l,
        None => {
            if let Some(c) = &capture {
                let _ = Shot::read_capture_file(c).map(Shot::load);
            }
            return;
        }
    };

    let info = match &capture {
        Some(c) => Shot::read_capture_file(c),
        None => Shot::capture_now(),
    };
    let shot = match info.and_then(Shot::load) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("synshot: {e:#}");
            std::process::exit(1);
        }
    };

    let (cfg, err) = Config::load();
    if let Some(e) = err {
        log::warn!("config.toml: {e}");
    }
    let mss = ui::stylesheet(&cfg);
    let font = Some(cfg.appearance.font.trim().to_string()).filter(|f| !f.is_empty());

    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        sel::init(shot);
        ui::open_surfaces();
    });
    if let Err(e) = result {
        eprintln!("synshot: {e:#}");
        std::process::exit(1);
    }

    let Some((choice, rect)) = sel::result() else { return };
    let shot = sel::shot();
    let Some((w, h, rgba)) = shot.crop(&rect) else { return };
    if let Err(e) = output::run(choice, w, h, &rgba, &cfg) {
        eprintln!("synshot: {e:#}");
        output::notify(format!("screenshot-failed {e:#}"));
        std::process::exit(1);
    }
}

/// Замок в `$XDG_RUNTIME_DIR`; `None` — оверлей уже открыт.
fn single_instance() -> Option<std::fs::File> {
    let path = paths::runtime_dir().join("synshot.lock");
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).ok()?;
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f)
}
