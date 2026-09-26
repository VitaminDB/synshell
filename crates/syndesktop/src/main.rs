//! syndesktop — композитор рабочего стола.
//!
//! `syndesktop` — запустить сеанс (в TTY — на DRM, внутри другого сеанса —
//! во вложенном окне); `syndesktop msg …` — управление по IPC.

mod anim;
mod backend;
mod bindings;
mod config;
mod cursor;
mod deco;
mod focus;
mod grabs;
mod handlers;
mod input;
mod ipc;
mod libinput_config;
mod render;
mod screenshot;
mod spawn;
mod state;
mod wm;
mod xwayland;

use std::time::Duration;

use smithay::reexports::{
    calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
    wayland_server::Display,
};
use syndesktop_common::Config;

use crate::{
    backend::Backend,
    state::{Core, State},
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("msg") {
        std::process::exit(msg(&args[1..]));
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "syndesktop {} — окружение рабочего стола для Wayland\n\n\
             syndesktop [--nested | --tty] [--no-shell] [--no-autostart]\n\
             syndesktop msg <version|windows|workspaces|outputs|layouts|events|action ДЕЙСТВИЕ|window ID ОПЕРАЦИЯ|reload>\n",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("syndesktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    init_logging();
    if let Err(e) = run(&args) {
        tracing::error!("{e:#}");
        eprintln!("syndesktop: {e:#}");
        std::process::exit(1);
    }
}

fn init_logging() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_env("SYNDESKTOP_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("syndesktop=info,smithay=warn"));
    let log_path = spawn::log_dir().join("syndesktop.log");
    // Предыдущий лог — рядом, для разбора падений.
    let _ = std::fs::rename(&log_path, spawn::log_dir().join("syndesktop.old.log"));
    let file = std::fs::File::create(&log_path).ok();
    let registry = tracing_subscriber::registry().with(filter).with(fmt::layer().with_writer(std::io::stderr));
    match file {
        Some(f) => registry.with(fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f))).init(),
        None => registry.init(),
    }
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("ПАНИКА: {info}\n{}", std::backtrace::Backtrace::force_capture());
        eprintln!("syndesktop: паника: {info}");
    }));
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let nested = if args.iter().any(|a| a == "--tty") {
        false
    } else {
        args.iter().any(|a| a == "--nested") || std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
    };
    let no_shell = args.iter().any(|a| a == "--no-shell");
    let no_autostart = args.iter().any(|a| a == "--no-autostart");
    tracing::info!(version = env!("CARGO_PKG_VERSION"), nested, "запуск syndesktop");

    if let Err(e) = Config::ensure_file_exists() {
        tracing::warn!(?e, "не удалось создать config.toml");
    }
    let (config, config_error) = Config::load();
    if let Some(e) = &config_error {
        tracing::warn!(error = e, "ошибка конфига — используются значения по умолчанию");
    }

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;

    let backend = if nested {
        Backend::Winit(backend::winit::WinitBackend::new(&event_loop)?)
    } else {
        Backend::Tty(backend::tty::TtyBackend::new(&event_loop)?)
    };
    let seat_name = backend.seat_name();
    let core = Core::new(display, event_loop.handle(), event_loop.get_signal(), &seat_name, nested, config, config_error)?;
    let mut state = State { backend, core };

    match &mut state.backend {
        Backend::Winit(w) => {
            w.init_globals(&mut state.core);
            w.apply_output_config(&mut state.core);
        }
        Backend::Tty(_) => backend::tty::init(&mut state, &event_loop)?,
    }
    state.outputs_changed();

    tracing::info!(socket = state.core.socket_name, "WAYLAND_DISPLAY");
    std::env::set_var("WAYLAND_DISPLAY", &state.core.socket_name);
    std::env::set_var("SYNDESKTOP_SOCKET", state.core.ipc.path());
    std::env::set_var("XDG_CURRENT_DESKTOP", "syndesktop");

    state.start_xwayland();
    if !nested {
        update_activation_environment(&state);
    }
    if !no_shell {
        state.start_shell();
    }
    if !no_autostart && !nested {
        state.run_autostart();
    }
    state.core.queue_redraw_all();

    // Раз в секунду: конфиг, простой, оболочка.
    event_loop.handle().insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
        state.poll_config();
        state.check_idle();
        state.watch_shell();
        TimeoutAction::ToDuration(Duration::from_secs(1))
    }).map_err(|e| anyhow::anyhow!("{}", e.error))?;

    let signal = event_loop.get_signal();
    let _ = signal;
    event_loop.run(Some(Duration::from_millis(250)), &mut state, |state| {
        state.core.space.refresh();
        state.core.popups.cleanup();
        state.flush_ipc();
        state.redraw_queued();
        if let Err(e) = state.core.display_handle.flush_clients() {
            tracing::warn!(?e, "flush_clients");
        }
    })?;

    tracing::info!("завершение");
    state.core.shell.stop();
    Ok(())
}

fn update_activation_environment(state: &State) {
    let vars = "WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE DISPLAY SYNDESKTOP_SOCKET";
    if spawn::which("dbus-update-activation-environment") {
        spawn::spawn_shell(&state.core, &format!("dbus-update-activation-environment --systemd {vars}"));
    } else if spawn::which("systemctl") {
        spawn::spawn_shell(&state.core, &format!("systemctl --user import-environment {vars}"));
    }
}

impl State {
    /// Простой: погасить мониторы, заблокировать, уснуть.
    fn check_idle(&mut self) {
        self.update_idle_inhibit();
        if !self.core.idle_inhibitors.is_empty() {
            return;
        }
        let idle = self.core.last_activity.elapsed().as_secs() as u32;
        let cfg = self.core.config.idle.clone();
        if cfg.lock_after > 0 && idle >= cfg.lock_after && !self.core.is_locked() && idle < cfg.lock_after + 2 {
            self.lock_screen();
        }
        if cfg.dpms_after > 0 && idle >= cfg.dpms_after && !self.core.monitors_off {
            self.set_monitors_power(false);
        }
        if cfg.suspend_after > 0 && idle >= cfg.suspend_after && idle < cfg.suspend_after + 2 {
            self.do_action(syndesktop_common::Action::Suspend);
        }
        // Спрятать курсор после простоя мыши.
        let hide = self.core.config.windows.hide_cursor_after;
        if hide > 0 && !self.core.cursor_hidden && self.core.last_pointer_motion.elapsed() >= Duration::from_millis(hide as u64) {
            self.core.cursor_hidden = true;
            self.core.queue_redraw_all();
        }
    }
}

/// `syndesktop msg …`
fn msg(args: &[String]) -> i32 {
    use syndesktop_common::ipc::{Client, Request, Response, WindowOp};
    let Some(cmd) = args.first() else {
        eprintln!("использование: syndesktop msg <version|windows|workspaces|outputs|layouts|events|action …|window ID ОПЕРАЦИЯ|reload>");
        return 2;
    };
    let req = match cmd.as_str() {
        "version" => Request::Version,
        "windows" => Request::Windows,
        "workspaces" => Request::Workspaces,
        "outputs" => Request::Outputs,
        "layouts" => Request::KeyboardLayouts,
        "reload" => Request::Action { action: syndesktop_common::Action::ReloadConfig },
        "action" => match args[1..].join(" ").parse() {
            Ok(action) => Request::Action { action },
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        },
        "window" => {
            let (Some(id), Some(op)) = (args.get(1).and_then(|s| s.parse().ok()), args.get(2)) else {
                eprintln!("использование: syndesktop msg window ID activate|minimize|close|kill|maximize|fullscreen|floating|sticky|above|workspace N");
                return 2;
            };
            let op = match op.as_str() {
                "activate" => WindowOp::Activate,
                "toggle-minimize" => WindowOp::ToggleMinimize,
                "minimize" => WindowOp::Minimize,
                "close" => WindowOp::Close,
                "kill" => WindowOp::Kill,
                "maximize" => WindowOp::ToggleMaximize,
                "fullscreen" => WindowOp::ToggleFullscreen,
                "floating" => WindowOp::ToggleFloating,
                "sticky" => WindowOp::ToggleSticky,
                "above" => WindowOp::ToggleAlwaysOnTop,
                "workspace" => WindowOp::MoveToWorkspace(args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1).saturating_sub(1)),
                other => {
                    eprintln!("неизвестная операция «{other}»");
                    return 2;
                }
            };
            Request::WindowAction { id, op }
        }
        "events" => {
            let client = match Client::connect() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("нет связи с композитором: {e}");
                    return 1;
                }
            };
            match client.event_stream() {
                Ok(stream) => {
                    for ev in stream {
                        match ev {
                            Ok(e) => println!("{}", serde_json::to_string(&e).unwrap_or_default()),
                            Err(e) => {
                                eprintln!("{e}");
                                return 1;
                            }
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            }
        }
        other => {
            eprintln!("неизвестная команда «{other}»");
            return 2;
        }
    };
    let mut client = match Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("нет связи с композитором ({}): {e}", syndesktop_common::paths::socket_path().display());
            return 1;
        }
    };
    match client.request(&req) {
        Ok(Response::Error { message }) => {
            eprintln!("{message}");
            1
        }
        Ok(Response::Ok) => 0,
        Ok(r) => {
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
