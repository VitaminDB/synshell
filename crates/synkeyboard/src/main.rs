//! synkeyboard — экранная клавиатура synshell. Демон рисует клавиатуру на
//! layer-shell поверхности (syngui-layer) и шлёт клавиши композитору через
//! `zwp_virtual_keyboard_v1`; показывается сама, когда приложение открывает
//! поле ввода (`zwp_input_method_v2`), и по команде:
//!
//! ```text
//! synkeyboard              # демон
//! synkeyboard show|hide|toggle
//! synkeyboard show float|dock   # показать плавающей / внизу (без слова — по [osk] placement)
//! synkeyboard show for syngamepad.sock   # клавиши — программе по её сокету (text …/key …), не композитору
//! synkeyboard float on|off|toggle   # плавающая ↔ внизу
//! synkeyboard fn                # показать/спрятать ряды Esc/F1–F12/стрелки
//! synkeyboard lang              # следующая раскладка (EN → RU → …)
//! synkeyboard type "ls -la"    # напечатать текст
//! synkeyboard key ctrl+c        # сочетание: ctrl, shift, alt, super + клавиша
//! synkeyboard auto on|off       # выезжать самой на текстовом поле (режим ввода «Выключен» — off)
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
use syngui::t;

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
    // Подписи клавиш — данные раскладок и не переводятся; переводятся сообщения команд.
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("synkeyboard {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help" | "-h") => {
            println!("{}", t!("synkeyboard [show [float|dock] [for <сокет>]|hide|toggle|fn|lang | float on|off|toggle | type <текст> | key <сочетание> | auto on|off]\n  без аргументов — запустить демон экранной клавиатуры"));
        }
        Some("show") => client(args.join(" ")),
        Some(cmd @ ("hide" | "toggle" | "fn" | "lang")) => client(cmd.to_string()),
        Some(cmd @ ("type" | "key" | "auto" | "float")) => {
            let Some(arg) = args.get(1) else {
                eprintln!("{}", t!("synkeyboard {cmd}: нужен аргумент", cmd = cmd));
                std::process::exit(2);
            };
            client(format!("{cmd} {arg}"));
        }
        Some(other) => {
            eprintln!("{}", t!("synkeyboard: неизвестная команда «{other}»", other = other));
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
    let placement = config.osk.placement;
    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        syngui_layer::set_ui_zoom(scale);
        let kb = ui::Keyboard::new(placement);
        ui::install(kb);
        listen(kb);
        watch_config(kb);
    });
    let _ = std::fs::remove_file(socket_path());
    if let Err(e) = result {
        log::error!("synkeyboard: {e:#}");
        std::process::exit(1);
    }
}

/// `[osk] scale` и `placement` меняются на лету («Параметры → Телефон → Экранная клавиатура»):
/// конфиг проверяется раз в 2 с.
fn watch_config(kb: ui::Keyboard) {
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
                if kb.placement.get_untracked() != cfg.osk.placement {
                    kb.placement.set(cfg.osk.placement);
                    // Настройку сменили — своё прежнее «⧉» забыть, показывать по ней.
                    kb.chosen.set(None);
                }
            }
        }
        Some(std::time::Duration::from_secs(2))
    });
}

/// `show [float|dock] [for <сокет>]` → (плавающая?, кому клавиши).
fn parse_show(arg: &str) -> Result<(Option<bool>, Option<String>), String> {
    let mut float = None;
    let mut capture = None;
    let mut words = arg.split_whitespace();
    while let Some(w) = words.next() {
        match w {
            "float" | "floating" => float = Some(true),
            "dock" | "docked" | "bottom" => float = Some(false),
            "for" => match words.next() {
                Some(s) if !s.is_empty() && !s.contains('/') => capture = Some(s.to_string()),
                _ => return Err(t!("show for: нужно имя сокета в $XDG_RUNTIME_DIR")),
            },
            other => return Err(t!("show: не понято «{other}»", other = other)),
        }
    }
    Ok((float, capture))
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
                        "show" => parse_show(arg).map(|(float, capture)| kb.show(float, capture)),
                        "hide" => Ok(kb.hide()),
                        "toggle" => Ok(if kb.visible.get_untracked() { kb.hide() } else { kb.show(None, None) }),
                        "float" => match arg.trim() {
                            "on" => Ok(kb.set_floating(true)),
                            "off" => Ok(kb.set_floating(false)),
                            "toggle" => Ok(kb.set_floating(!kb.floating.get_untracked())),
                            _ => Err("float on|off|toggle".into()),
                        },
                        "fn" => Ok(kb.act(ui::Action::Fn)),
                        "lang" => Ok(kb.act(ui::Action::Layout)),
                        "auto" => match arg.trim() {
                            "on" => Ok(kb.auto.set(true)),
                            "off" => Ok(kb.auto.set(false)),
                            _ => Err("auto on|off".into()),
                        },
                        "type" => Ok(ui::type_text(kb, arg)),
                        "key" => ui::press_combo(kb, arg),
                        other => Err(t!("команда не поддерживается: {other}", other = other)),
                    };
                    let _ = tx.send(r);
                });
                let reply = match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(Ok(())) => "ok\n".to_string(),
                    Ok(Err(e)) => format!("error {e}\n"),
                    Err(_) => format!("error {}\n", t!("нет ответа от главного потока")),
                };
                let _ = (&stream).write_all(reply.as_bytes());
            }
        })
        .expect("поток управляющего сокета");
}
