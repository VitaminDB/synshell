//! syngamepad — экранный контроллер synshell: стики, крестовина, кнопки, тачпад поверх игры.
//! Кнопки геймпада и мышь — устройства ядра через `/dev/uinput` (игры SDL, Wine, Steam, Android
//! видят настоящий геймпад), клавиши — через `zwp_virtual_keyboard_v1`. Раскладки —
//! `~/.config/synshell/gamepads/*.toml` (`layout.rs`).
//!
//! ```text
//! syngamepad                 # демон (запускает оболочка)
//! syngamepad show|hide|toggle    # режим «Контроллер» вкл/выкл
//! syngamepad fold            # свернуть до ручки / развернуть
//! syngamepad edit            # правка раскладки (или удержание ручки)
//! syngamepad layout <id>     # раскладка
//! syngamepad layouts         # список раскладок
//! syngamepad status          # shown folded <id>
//! ```
//!
//! Управляющий сокет — `$XDG_RUNTIME_DIR/syngamepad.sock`.

mod editor;
mod keys;
mod layout;
mod output;
mod ui;
mod uinput;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use synshell_common::Config;

const BUILTIN_MSS: &str = include_str!("../styles/gamepad.mss");

/// Кодовые точки Material Icons.
pub mod icons {
    pub const GAMEPAD: &str = "\u{E338}";
    pub const FOLD: &str = "\u{E8F5}";
    pub const UP: &str = "\u{E5C7}";
    pub const DOWN: &str = "\u{E5C5}";
    pub const LEFT: &str = "\u{E5DE}";
    pub const RIGHT: &str = "\u{E5DF}";
}

fn socket_path() -> PathBuf {
    synshell_common::paths::runtime_dir().join("syngamepad.sock")
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,usvg=error"))
        .format_timestamp_millis()
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => println!("syngamepad {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => {
            println!("syngamepad [show|hide|toggle|fold|edit|status|layouts | layout <id>]\n  без аргументов — демон экранного контроллера")
        }
        Some("layouts") => {
            for (id, name) in layout::list() {
                println!("{id}\t{name}");
            }
        }
        Some(cmd @ ("show" | "hide" | "toggle" | "fold" | "edit" | "status")) => client(cmd),
        Some("layout") => match args.get(1) {
            Some(id) => client(&format!("layout {id}")),
            None => {
                eprintln!("syngamepad layout: нужен id раскладки (syngamepad layouts)");
                std::process::exit(2);
            }
        },
        Some(other) => {
            eprintln!("syngamepad: неизвестная команда «{other}»");
            std::process::exit(2);
        }
        None => daemon(),
    }
}

fn client(cmd: &str) {
    match send(cmd) {
        Ok(reply) => {
            if !reply.is_empty() && reply != "ok" {
                println!("{reply}");
            }
        }
        Err(e) => {
            eprintln!("syngamepad: {e}");
            std::process::exit(1);
        }
    }
}

fn send(cmd: &str) -> anyhow::Result<String> {
    let mut s = UnixStream::connect(socket_path())?;
    writeln!(s, "{cmd}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    let reply = reply.trim().to_string();
    if let Some(err) = reply.strip_prefix("error ") {
        anyhow::bail!("{err}");
    }
    Ok(reply)
}

fn daemon() {
    let (config, err) = Config::load();
    if let Some(e) = err {
        log::error!("ошибка в конфиге, взяты значения по умолчанию: {e}");
    }
    let mss = build_mss(&config);
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());
    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        let pad = ui::Pad::new("standard");
        ui::install(pad);
        listen(pad);
    });
    let _ = std::fs::remove_file(socket_path());
    if let Err(e) = result {
        log::error!("syngamepad: {e:#}");
        std::process::exit(1);
    }
}

fn build_mss(config: &Config) -> String {
    let a = &config.appearance;
    let mut out = a.mss_variables();
    out.push_str(&a.theme_mss_variables());
    out.push_str(BUILTIN_MSS);
    if !a.font.trim().is_empty() {
        out.push_str(&format!("Text {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::config_dir().join("gamepad.mss")) {
        out.push_str(&user);
    }
    out
}

/// Управляющий сокет: одна команда на соединение, ответ — строка.
fn listen(pad: ui::Pad) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("сокет {}: {e}", path.display());
            return;
        }
    };
    std::thread::Builder::new()
        .name("syngamepad-ctl".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let cmd = line.trim().to_string();
                let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
                syngui::async_runtime::run_on_main_thread(move || {
                    let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd.as_str(), ""));
                    let ok = || Ok("ok".to_string());
                    let r = match verb {
                        "show" => {
                            pad.folded.set(false);
                            pad.shown.set(true);
                            ok()
                        }
                        "hide" => {
                            pad.shown.set(false);
                            ok()
                        }
                        "toggle" => {
                            pad.folded.set(false);
                            pad.shown.set(!pad.shown.get_untracked());
                            ok()
                        }
                        "edit" => {
                            pad.shown.set(true);
                            editor::open(pad);
                            ok()
                        }
                        // Приложение в фокусе (от оболочки; `-` — нет).
                        "app" => {
                            let a = arg.trim();
                            pad.app.set((!a.is_empty() && a != "-").then(|| a.to_string()));
                            ok()
                        }
                        "fold" => {
                            pad.folded.set(!pad.folded.get_untracked());
                            ok()
                        }
                        "layout" if !arg.trim().is_empty() => {
                            pad.set_layout(arg.trim());
                            ok()
                        }
                        "status" => Ok(format!("{} {} {}", pad.shown.get_untracked(), pad.folded.get_untracked(), pad.layout_id.get_untracked())),
                        other => Err(format!("команда не поддерживается: {other}")),
                    };
                    let _ = tx.send(r);
                });
                let reply = match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(Ok(s)) => format!("{s}\n"),
                    Ok(Err(e)) => format!("error {e}\n"),
                    Err(_) => "error нет ответа от главного потока\n".to_string(),
                };
                let _ = (&stream).write_all(reply.as_bytes());
            }
        })
        .expect("поток управляющего сокета");
}
