//! syn-vm — виртуальные машины synshell на QEMU/KVM: мастер создания по образу с
//! подбором железа под систему (Windows 11 — TPM и Secure Boot), экран гостя прямо в
//! окне (VNC на unix-сокете, разрешение под окно, общий буфер обмена), снимки,
//! проброс портов и устройств компьютера, общая папка, гостевой агент. Машины живут без
//! окна: QEMU уходит в фон, а окно при запуске подхватывает работающие. Всё управление
//! есть и в командной строке (`cli.rs`, `syn-vm --help`) и в MCP-сервере (`syn-vm --mcp`).
//!
//! Окно одно: повторный запуск передаёт машину открытому окну через сокет
//! `$XDG_RUNTIME_DIR/syn-vm.sock` и выходит.

mod cli;
mod devices;
mod mcp;
mod qga;
mod record;
mod keymap;
mod model;
mod present;
mod qemu;
mod qmp;
mod state;
mod ui;
mod usb;
mod vnc;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use syngui::prelude::*;
use synshell_common::Config;

use state::{St, Tab};

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_vm=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut target: Option<String> = None;
    let mut tab: Option<Tab> = None;
    let mut shot: Option<String> = None;
    let mut wizard: Option<usize> = None;
    let mut size = (1280u32, 800u32);
    let mut scale = 2.0f64;
    let all: Vec<String> = std::env::args().skip(1).collect();
    if let Some(first) = all.first() {
        if let Some(code) = cli::run(first, all[1..].to_vec()) {
            std::process::exit(code);
        }
    }
    let mut separate = false;
    let mut args = all.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{}", cli::help());
                return;
            }
            "-V" | "--version" => {
                println!("syn-vm {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--new-window" => separate = true,
            "--tab" => tab = args.next().as_deref().and_then(Tab::parse),
            "--open" => target = args.next(),
            "--wizard" => wizard = args.next().and_then(|s| s.parse().ok()),
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            other if !other.starts_with('-') => target = Some(other.to_string()),
            // Неизвестный ключ — ошибка, а не окно.
            other => {
                eprintln!("syn-vm: {}", syngui::t!("неизвестный ключ {k} (syn-vm --help)", k = other));
                std::process::exit(2);
            }
        }
    }
    // Файл (ISO, диск) — мастер новой машины с ним.
    let file = target.as_deref().map(std::path::Path::new).filter(|p| p.is_file()).and_then(|p| std::fs::canonicalize(p).ok()).map(|p| p.to_string_lossy().into_owned());
    let target_id = if file.is_some() { None } else { target.as_deref().and_then(find).map(|v| v.id) };
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            move || {
                let st = St::new();
                state::reload(st);
                if let Some(id) = &target_id {
                    st.sel.set(Some(id.clone()));
                    st.tab.set(tab.unwrap_or(Tab::Overview));
                    if tab == Some(Tab::Settings) {
                        st.host_devs.set(Some(devices::available()));
                    }
                    if let Some(vm) = st.vm(id) {
                        if qemu::pid(&vm).is_some() {
                            st.guest.set(qga::info(&vm).ok());
                            let mut live = st.live.get_untracked();
                            live.entry(vm.id.clone()).or_default().power = state::Power::Running;
                            st.live.set(live);
                        }
                    }
                }
                if let Some(step) = wizard {
                    ui::new_vm(st);
                    st.wiz.set(Some(step));
                }
                Box::new(ui::root(st))
            },
            &mss,
            size,
            scale,
            &out,
        );
        match r {
            Ok((w, h)) => println!("{out}: {w}x{h}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // Окно уже открыто — передать ему машину.
    let sock = socket_path();
    if separate {
        // Отдельное окно (проверки, второй экран): без передачи открытому и без своего сокета.
    } else if let Ok(mut s) = UnixStream::connect(&sock) {
        let line = format!("{}\t{}\t{}", target_id.clone().unwrap_or_default(), tab.map(tab_id).unwrap_or(""), file.clone().unwrap_or_default());
        if writeln!(s, "{line}").is_ok() {
            return;
        }
    }
    if !separate {
        let _ = std::fs::remove_file(&sock);
    }
    let listener = std::sync::Mutex::new(if separate { None } else { UnixListener::bind(&sock).ok() });
    let win = use_signal(syngui::window::WindowState::default());
    App::new()
        .title(t!("Виртуальные машины"))
        .app_id("syn-vm")
        .size(1360, 860)
        .min_size(360, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .run(move |_| {
            let mut st = St::new();
            st.window = win;
            state::reload(st);
            state::start_poller(st);
            if let Some(id) = &target_id {
                ui::open(st, Some(id.clone()), tab);
            }
            if let Some(f) = file.clone() {
                ui::open_file(st, f);
            }
            if let Some(l) = listener.lock().ok().and_then(|mut l| l.take()) {
                listen(l, st);
            }
            Box::new(ui::root(st))
        });
    if !separate {
        let _ = std::fs::remove_file(&sock);
    }
}

/// Машина по id или имени (без учёта регистра).
fn find(s: &str) -> Option<model::Vm> {
    cli::find(s).ok()
}

fn tab_id(t: Tab) -> &'static str {
    match t {
        Tab::Screen => "screen",
        Tab::Overview => "overview",
        Tab::Settings => "settings",
        Tab::Snapshots => "snapshots",
    }
}

fn socket_path() -> PathBuf {
    synshell_common::paths::runtime_dir().join("syn-vm.sock")
}

/// Машины от повторных запусков.
fn listen(l: UnixListener, st: St) {
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() {
                let mut parts = line.trim_end_matches(['\n', '\r']).split('\t');
                let id = parts.next().filter(|s| !s.is_empty()).map(String::from);
                let tab = parts.next().and_then(Tab::parse);
                let file = parts.next().filter(|s| !s.is_empty()).map(String::from);
                run_on_main_thread(move || {
                    state::reload(st);
                    if id.is_some() {
                        ui::open(st, id, tab);
                    }
                    if let Some(f) = file {
                        ui::open_file(st, f);
                    }
                });
            }
        }
    });
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-vm").font_selector("Text, Button").base(include_str!("../styles/syn-vm.mss")).build()
}
