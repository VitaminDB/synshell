//! synwm — композитор рабочего стола.
//!
//! `synwm` — запустить сеанс (в TTY — на DRM, внутри другого сеанса —
//! во вложенном окне); `synwm msg …` — управление по IPC.

mod anim;
mod appmenu;
mod backend;
mod bindings;
mod config;
mod cursor;
mod deco;
mod encode;
mod focus;
mod grabs;
mod gtk_shell;
mod handlers;
mod input;
mod ipc;
mod libinput_config;
mod remote;
mod render;
mod screencopy;
mod screenshot;
mod spawn;
mod state;
mod stream;
mod touch;
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
use synshell_common::Config;

use crate::{
    backend::Backend,
    state::{Core, State},
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Проба GPU в отдельном процессе (см. backend::probe_gpu): падение драйвера
    // (например, SIGILL программного рендерера) не должно ронять композитор.
    if args.first().map(String::as_str) == Some("--probe-gpu") {
        let ok = args.get(1).is_some_and(|p| backend::probe_gpu_here(std::path::Path::new(p)));
        std::process::exit(if ok { 0 } else { 1 });
    }
    if args.first().map(String::as_str) == Some("msg") {
        std::process::exit(msg(&args[1..]));
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "synwm {} — окружение рабочего стола для Wayland\n\n\
             synwm [--nested [--nested-size ШxВ] | --tty] [--cpu | --gpu] [--no-shell] [--no-autostart]\n\
             synwm msg <version|windows|workspaces|outputs|layouts|events|action ДЕЙСТВИЕ|window ID ОПЕРАЦИЯ|reload|restart-shell|restart>\n",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("synwm {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    init_logging();
    let inherited = InheritedEnv::capture();
    match run(&args) {
        Ok(true) => reexec(&inherited),
        Ok(false) => {}
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("synwm: {e:#}");
            std::process::exit(1);
        }
    }
}

/// Переменные, которые композитор переписывает для детей; при перезапуске
/// новому процессу нужны исходные (иначе вложенный режим смотрел бы сам на себя).
struct InheritedEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl InheritedEnv {
    fn capture() -> Self {
        Self(
            ["WAYLAND_DISPLAY", "DISPLAY", "SYNSHELL_SOCKET", "XDG_CURRENT_DESKTOP"]
                .into_iter()
                .map(|k| (k, std::env::var_os(k)))
                .collect(),
        )
    }
}

/// Заменить процесс свежим `synwm` с теми же аргументами.
fn reexec(inherited: &InheritedEnv) {
    use std::os::unix::process::CommandExt;
    for (k, v) in &inherited.0 {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    // После обновления пакета /proc/self/exe указывает на удалённый файл
    // («… (deleted)») — берём путь без этой пометки.
    let exe = std::env::current_exe()
        .ok()
        .map(|p| std::path::PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)").to_string()))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "synwm".into());
    tracing::info!(exe = %exe.display(), "exec нового композитора");
    let err = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).exec();
    tracing::error!(?err, "перезапуск не удался");
    eprintln!("synwm: перезапуск не удался: {err}");
    std::process::exit(1);
}

fn init_logging() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_env("SYNSHELL_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("synwm=info,smithay=warn"));
    let log_path = spawn::log_dir().join("synwm.log");
    // Предыдущий лог — рядом, для разбора падений.
    let _ = std::fs::rename(&log_path, spawn::log_dir().join("synwm.old.log"));
    let file = std::fs::File::create(&log_path).ok();
    let registry = tracing_subscriber::registry().with(filter).with(fmt::layer().with_writer(std::io::stderr));
    match file {
        Some(f) => registry.with(fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f))).init(),
        None => registry.init(),
    }
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("ПАНИКА: {info}\n{}", std::backtrace::Backtrace::force_capture());
        eprintln!("synwm: паника: {info}");
    }));
}

/// `Ok(true)` — запрошен перезапуск.
fn cpu_backend(event_loop: &EventLoop<'static, State>) -> anyhow::Result<Backend> {
    #[cfg(feature = "pixman")]
    {
        Ok(Backend::KmsCpu(backend::kms_cpu::KmsCpuBackend::new(event_loop)?))
    }
    #[cfg(not(feature = "pixman"))]
    {
        let _ = event_loop;
        anyhow::bail!("сборка без CPU-рендерера (feature pixman)");
    }
}

fn run(args: &[String]) -> anyhow::Result<bool> {
    let nested = if args.iter().any(|a| a == "--tty") {
        false
    } else {
        args.iter().any(|a| a == "--nested") || std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
    };
    if let Some(i) = args.iter().position(|a| a == "--nested-size") {
        if let Some(v) = args.get(i + 1) {
            std::env::set_var("SYNSHELL_NESTED_SIZE", v);
        }
    }
    let no_shell = args.iter().any(|a| a == "--no-shell");
    let no_autostart = args.iter().any(|a| a == "--no-autostart");
    tracing::info!(version = env!("CARGO_PKG_VERSION"), nested, "запуск synwm");

    if let Err(e) = Config::ensure_file_exists() {
        tracing::warn!(?e, "не удалось создать config.toml");
    }
    let (config, config_error) = Config::load();
    synshell_common::haptics::set_config(&config.haptics);
    if let Some(e) = &config_error {
        tracing::warn!(error = e, "ошибка конфига — используются значения по умолчанию");
    }

    let mut config = config;
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;

    // Платформа: рендерер (GPU через GBM/EGL или CPU через pixman) и форм-фактор.
    // Форм-фактор: SYNSHELL_FORM_FACTOR (отладка), [platform], иначе — по мониторам.
    let mut form_factor = match std::env::var("SYNSHELL_FORM_FACTOR").as_deref() {
        Ok("phone") | Ok("mobile") => Some(synshell_common::config::FormFactor::Phone),
        Ok("desktop") => Some(synshell_common::config::FormFactor::Desktop),
        _ => config.form_factor_setting(),
    };
    let backend = if nested {
        Backend::Winit(backend::winit::WinitBackend::new(&event_loop)?)
    } else {
        let renderer = if args.iter().any(|a| a == "--cpu") {
            "cpu".to_string()
        } else if args.iter().any(|a| a == "--gpu") {
            "gpu".to_string()
        } else {
            std::env::var("SYNSHELL_RENDERER").unwrap_or_else(|_| config.platform.renderer.trim().to_string())
        };
        let device = backend::kms_cpu_device_path();
        if form_factor.is_none() {
            form_factor = device.as_deref().and_then(backend::probe_form_factor);
        }
        let cpu = match renderer.as_str() {
            "cpu" | "pixman" => true,
            "gpu" | "gles" => false,
            _ => match device.as_deref() {
                Some(p) => {
                    let gpu = backend::probe_gpu(p);
                    if !gpu {
                        tracing::info!(path = %p.display(), "GPU-рендеринг недоступен — CPU (pixman)");
                    }
                    !gpu
                }
                None => false,
            },
        };
        if cpu {
            cpu_backend(&event_loop)?
        } else {
            match backend::tty::TtyBackend::new(&event_loop) {
                Ok(t) => Backend::Tty(t),
                Err(e) if cfg!(feature = "pixman") => {
                    tracing::error!(?e, "GPU-бэкенд не поднялся — CPU (pixman)");
                    cpu_backend(&event_loop)?
                }
                Err(e) => return Err(e),
            }
        }
    };
    let form_factor = form_factor.unwrap_or_default();
    config.apply_form_factor(form_factor);
    tracing::info!(form_factor = form_factor.as_str(), "форм-фактор");
    let seat_name = backend.seat_name();
    let core = Core::new(display, event_loop.handle(), event_loop.get_signal(), &seat_name, nested, form_factor, config, config_error)?;
    let mut state = State { backend, core };

    match &mut state.backend {
        Backend::Winit(w) => {
            w.init_globals(&mut state.core);
            w.apply_output_config(&mut state.core);
        }
        Backend::Tty(_) => {
            let res = backend::tty::init(&mut state, &event_loop);
            let Backend::Tty(t) = &mut state.backend else { unreachable!() };
            // Мониторы на GPU не включились (EGL отказал, нет общего формата,
            // KMS отверг буфер) — тот же сеанс на CPU.
            let failed = res.is_err() || (t.failed_outputs > 0 && state.core.space.outputs().next().is_none());
            if failed && cfg!(feature = "pixman") {
                tracing::error!(err = ?res.err(), failed = t.failed_outputs, "GPU-бэкенд: мониторы не включились — CPU (pixman)");
                t.teardown();
                if let Some(g) = state.core.dmabuf_global.take() {
                    state.core.dmabuf_state.destroy_global::<State>(&state.core.display_handle, g);
                }
                state.backend = cpu_backend(&event_loop)?;
                #[cfg(feature = "pixman")]
                backend::kms_cpu::init(&mut state, &event_loop)?;
            } else {
                res?;
            }
        }
        #[cfg(feature = "pixman")]
        Backend::KmsCpu(_) => backend::kms_cpu::init(&mut state, &event_loop)?,
    }
    state.outputs_changed();
    state.init_mobile();

    tracing::info!(socket = state.core.socket_name, "WAYLAND_DISPLAY");
    std::env::set_var("WAYLAND_DISPLAY", &state.core.socket_name);
    std::env::set_var("SYNSHELL_SOCKET", state.core.ipc.path());
    std::env::set_var("XDG_CURRENT_DESKTOP", "synshell");
    std::env::set_var("SYNSHELL_FORM_FACTOR", state.core.form_factor.as_str());

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
        state.flush_stale_copies();
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
    let restart = state.core.restart_requested;
    // Освободить DRM, ввод, сокеты до exec.
    drop(state);
    drop(event_loop);
    Ok(restart)
}

fn update_activation_environment(state: &State) {
    let vars = "WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE DISPLAY SYNSHELL_SOCKET";
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
            self.do_action(synshell_common::Action::Suspend);
        }
        // Спрятать курсор после простоя мыши.
        let hide = self.core.config.windows.hide_cursor_after;
        if hide > 0 && !self.core.cursor_hidden && self.core.last_pointer_motion.elapsed() >= Duration::from_millis(hide as u64) {
            self.core.cursor_hidden = true;
            self.core.queue_redraw_all();
        }
    }
}

/// `synwm msg …`
fn msg(args: &[String]) -> i32 {
    use synshell_common::ipc::{Client, Request, Response, WindowOp};
    let Some(cmd) = args.first() else {
        eprintln!("использование: synwm msg <version|windows|workspaces|outputs|layouts|events|action …|window ID ОПЕРАЦИЯ|reload|restart-shell|restart>");
        return 2;
    };
    let req = match cmd.as_str() {
        "version" => Request::Version,
        "windows" => Request::Windows,
        "workspaces" => Request::Workspaces,
        "outputs" => Request::Outputs,
        "layouts" => Request::KeyboardLayouts,
        // Имя монитора под указателем — для выбора экрана в портале
        // (xdg-desktop-portal-wlr: chooser_cmd).
        "focused-output" => {
            let outputs = Client::connect().and_then(|mut c| c.request(&Request::Outputs));
            return match outputs {
                Ok(Response::Outputs { outputs }) => {
                    match outputs.iter().find(|o| o.focused).or(outputs.first()) {
                        Some(o) => {
                            println!("{}", o.name);
                            0
                        }
                        None => 1,
                    }
                }
                _ => 1,
            };
        }
        "reload" => Request::Action { action: synshell_common::Action::ReloadConfig },
        "restart-shell" => Request::Action { action: synshell_common::Action::RestartShell },
        "restart" => Request::Action { action: synshell_common::Action::Restart },
        "action" => match args[1..].join(" ").parse() {
            Ok(action) => Request::Action { action },
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        },
        "window" => {
            let (Some(id), Some(op)) = (args.get(1).and_then(|s| s.parse().ok()), args.get(2)) else {
                eprintln!("использование: synwm msg window ID activate|minimize|close|kill|maximize|fullscreen|floating|sticky|above|workspace N");
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
            eprintln!("нет связи с композитором ({}): {e}", synshell_common::paths::socket_path().display());
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
