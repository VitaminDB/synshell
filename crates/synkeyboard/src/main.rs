//! synkeyboard — экранная клавиатура synshell. Демон рисует клавиатуру на
//! layer-shell поверхности (syngui-layer) и шлёт клавиши композитору через
//! `zwp_virtual_keyboard_v1`; показывается сама, когда приложение открывает
//! поле ввода (`zwp_input_method_v2`), и по команде:
//!
//! ```text
//! synkeyboard              # демон
//! synkeyboard show|hide|toggle
//! synkeyboard fn                # показать/спрятать ряды Esc/F1–F12/стрелки
//! synkeyboard lang              # следующая раскладка (EN → RU → …)
//! synkeyboard type "ls -la"    # напечатать текст
//! synkeyboard key ctrl+c        # сочетание: ctrl, shift, alt, super + клавиша
//! ```
//!
//! Управляющий сокет — `$XDG_RUNTIME_DIR/synkeyboard.sock` (или
//! `$SYNKEYBOARD_SOCKET`).

mod layout;
mod ui;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use synshell_common::Config;

const BUILTIN_MSS: &str = include_str!("../styles/keyboard.mss");

fn socket_path() -> PathBuf {
    match std::env::var_os("SYNKEYBOARD_SOCKET") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => synshell_common::paths::runtime_dir().join("synkeyboard.sock"),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,usvg=error"))
        .format_timestamp_millis()
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("synkeyboard {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help" | "-h") => {
            println!(
                "synkeyboard [show|hide|toggle|fn|lang | type <текст> | key <сочетание>]\n  без аргументов — запустить демон экранной клавиатуры"
            );
        }
        Some(cmd @ ("show" | "hide" | "toggle" | "fn" | "lang")) => client(cmd.to_string()),
        Some(cmd @ ("type" | "key")) => {
            let Some(arg) = args.get(1) else {
                eprintln!("synkeyboard {cmd}: нужен аргумент");
                std::process::exit(2);
            };
            client(format!("{cmd} {arg}"));
        }
        Some(other) => {
            eprintln!("synkeyboard: неизвестная команда «{other}»");
            std::process::exit(2);
        }
        None => daemon(),
    }
}

fn client(cmd: String) {
    if let Err(e) = send(&cmd) {
        eprintln!("synkeyboard: {e}");
        std::process::exit(1);
    }
}

fn send(cmd: &str) -> anyhow::Result<()> {
    let mut s = UnixStream::connect(socket_path())?;
    writeln!(s, "{cmd}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    if let Some(err) = reply.trim().strip_prefix("error ") {
        anyhow::bail!("{err}");
    }
    Ok(())
}

fn daemon() {
    let (config, err) = Config::load();
    if let Some(e) = err {
        log::error!("ошибка в конфиге, взяты значения по умолчанию: {e}");
    }
    let mss = build_mss(&config);
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());

    let scale = config.osk.scale;
    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        syngui_layer::set_ui_zoom(scale);
        let kb = ui::Keyboard::new();
        ui::install(kb);
        listen(kb);
        watch_scale();
    });
    let _ = std::fs::remove_file(socket_path());
    if let Err(e) = result {
        log::error!("synkeyboard: {e:#}");
        std::process::exit(1);
    }
}

/// `[osk] scale` меняется на лету («Параметры → Клавиатура»): конфиг
/// проверяется раз в 2 с.
fn watch_scale() {
    let path = synshell_common::paths::config_file();
    let mtime = move || std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut last = mtime();
    syngui_layer::add_timer(std::time::Duration::from_secs(2), move || {
        let now = mtime();
        if now != last {
            last = now;
            let (cfg, err) = Config::load();
            if err.is_none() {
                syngui_layer::set_ui_zoom(cfg.osk.scale);
            }
        }
        Some(std::time::Duration::from_secs(2))
    });
}

fn build_mss(config: &Config) -> String {
    let a = &config.appearance;
    let mut out = a.mss_variables();
    out.push_str(&a.theme_mss_variables());
    out.push_str(BUILTIN_MSS);
    if !a.font.trim().is_empty() {
        out.push_str(&format!("Text {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::config_dir().join("keyboard.mss")) {
        out.push_str(&user);
    }
    out
}

/// Управляющий сокет: одна команда на соединение, ответ `ok`.
fn listen(kb: ui::Keyboard) {
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
        .name("synkeyboard-ctl".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let cmd = line.trim().to_string();
                let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
                syngui::async_runtime::run_on_main_thread(move || {
                    let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd.as_str(), ""));
                    let r = match verb {
                        "show" => Ok(kb.visible.set(true)),
                        "hide" => Ok(kb.visible.set(false)),
                        "toggle" => Ok(kb.visible.set(!kb.visible.get_untracked())),
                        "fn" => Ok(kb.act(ui::Action::Fn)),
                        "lang" => Ok(kb.act(ui::Action::Layout)),
                        "type" => Ok(ui::type_text(kb, arg)),
                        "key" => ui::press_combo(kb, arg),
                        other => Err(format!("команда не поддерживается: {other}")),
                    };
                    let _ = tx.send(r);
                });
                let reply = match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(Ok(())) => "ok\n".to_string(),
                    Ok(Err(e)) => format!("error {e}\n"),
                    Err(_) => "error нет ответа от главного потока\n".to_string(),
                };
                let _ = (&stream).write_all(reply.as_bytes());
            }
        })
        .expect("поток управляющего сокета");
}
