//! syn-vm — виртуальные машины synshell на QEMU/KVM: мастер создания по образу с
//! подбором железа под систему (Windows 11 — TPM и Secure Boot), экран гостя прямо в
//! окне (VNC на unix-сокете, разрешение под окно, общий буфер обмена), снимки,
//! проброс портов, общая папка. Машины живут без окна: QEMU уходит в фон, а окно
//! при запуске подхватывает работающие.
//!
//!   syn-vm [ИМЯ|ID] [--tab screen|overview|settings|snapshots]   окно (открыть машину)
//!   syn-vm --start ИМЯ|ID            запустить машину без окна
//!   syn-vm --list                    машины и их состояние
//!   syn-vm --new ИМЯ [--iso ОБРАЗ] [--os linux|windows11|windows10|bsd|android|other]
//!   syn-vm --screenshot out.png [--size 1280x800] [--scale 2] [--open ИМЯ] [--tab …] [--wizard N]
//!
//! Окно одно: повторный запуск передаёт машину открытому окну через сокет
//! `$XDG_RUNTIME_DIR/syn-vm.sock` и выходит.

mod keymap;
mod model;
mod qemu;
mod qmp;
mod state;
mod ui;
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
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut target: Option<String> = None;
    let mut tab: Option<Tab> = None;
    let mut shot: Option<String> = None;
    let mut wizard: Option<usize> = None;
    let mut size = (1280u32, 800u32);
    let mut scale = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-vm [МАШИНА] [--tab screen|overview|settings|snapshots] [--start МАШИНА] [--list] [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--open МАШИНА] [--wizard ШАГ]]"));
                return;
            }
            "--list" => {
                for vm in model::load_all() {
                    let on = qemu::pid(&vm).is_some();
                    println!("{}\t{}\t{}", vm.id, if on { "running" } else { "off" }, vm.name);
                }
                return;
            }
            "--start" => {
                let Some(name) = args.next() else { return };
                match find(&name) {
                    Some(vm) => {
                        if let Err(e) = qemu::start(&vm) {
                            eprintln!("{e}");
                            std::process::exit(1);
                        }
                        qemu::after_boot(&vm);
                    }
                    None => {
                        eprintln!("{}", t!("нет машины «{n}»", n = name));
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--new" => {
                let name = args.next().unwrap_or_default();
                let rest: Vec<String> = args.by_ref().collect();
                let opt = |k: &str| rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned();
                let iso = opt("--iso").unwrap_or_default();
                let os = opt("--os")
                    .and_then(|o| toml::Value::String(o).try_into::<model::OsKind>().ok())
                    .or_else(|| model::OsKind::guess(&iso))
                    .unwrap_or_default();
                let mut vm = os.preset();
                vm.name = name;
                if !iso.is_empty() {
                    vm.cdrom = std::fs::canonicalize(&iso).map(|p| p.to_string_lossy().into_owned()).unwrap_or(iso);
                    vm.boot_cdrom = true;
                }
                match model::create(vm, None) {
                    Ok(vm) => println!("{}", vm.id),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
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
            _ => {}
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
    if let Ok(mut s) = UnixStream::connect(&sock) {
        let line = format!("{}\t{}\t{}", target_id.clone().unwrap_or_default(), tab.map(tab_id).unwrap_or(""), file.clone().unwrap_or_default());
        if writeln!(s, "{line}").is_ok() {
            return;
        }
    }
    let _ = std::fs::remove_file(&sock);
    let listener = std::sync::Mutex::new(UnixListener::bind(&sock).ok());
    App::new()
        .title(t!("Виртуальные машины"))
        .app_id("syn-vm")
        .size(1360, 860)
        .min_size(360, 520)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .run(move |_| {
            let st = St::new();
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
    let _ = std::fs::remove_file(&sock);
}

/// Машина по id или имени (без учёта регистра).
fn find(s: &str) -> Option<model::Vm> {
    let all = model::load_all();
    let l = s.to_lowercase();
    all.iter().find(|v| v.id == s).or_else(|| all.iter().find(|v| v.name.to_lowercase() == l)).cloned()
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
