//! Запуск программ и выполнение действий: что умеет сама оболочка —
//! делает сама (запуск, команды оболочки), остальное шлёт композитору.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use synshell_common::ipc::{Client, Request, Response, WindowOp};
use synshell_common::Action;

use crate::ctx::ShellCtx;

/// Запустить команду через `sh -c`, отвязав от оболочки (свой сеанс,
/// stdio в /dev/null): программа переживёт перезапуск оболочки.
pub fn spawn(cmd: &str) {
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return;
    }
    log::info!("запуск: {cmd}");
    if crate::android_boot::is_android_command(&cmd) {
        crate::android_boot::watch();
    }
    let mut c = Command::new("sh");
    c.arg("-c").arg(&cmd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid() безопасен между fork и exec.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match c.spawn() {
        // Дочерний sh ждём в отдельном потоке, чтобы не копить зомби.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("не удалось запустить «{cmd}»: {e}"),
    }
}

/// Запустить команду и прочитать stdout (для апплетов). Блокирует —
/// вызывать из фоновых потоков.
pub fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Отправить запрос композитору в фоне.
pub fn send(req: Request) {
    std::thread::spawn(move || match Client::connect().and_then(|mut c| c.request(&req)) {
        Ok(Response::Error { message }) => log::warn!("композитор: {message}"),
        Ok(_) => {}
        Err(e) => log::debug!("IPC недоступен ({req:?}): {e}"),
    });
}

/// Монитор ресурсов на странице `page` (`cpu`, `memory`, `processes`…): открытое окно — поднять
/// (запуск с аргументом передаст ему страницу и сразу выйдет), иначе — запустить.
pub fn open_resources(page: &str) {
    let ctx = crate::ShellCtx::get();
    if let Some(w) = ctx.windows.get_untracked().iter().find(|w| w.app_id == "syn-systeminfo") {
        window_op(w.id, WindowOp::Activate);
    }
    spawn(&format!("syn-systeminfo {page}"));
}

pub fn window_op(id: u64, op: WindowOp) {
    if matches!(op, WindowOp::Close | WindowOp::Kill) {
        note_closing(id);
    }
    send(Request::WindowAction { id, op });
}

thread_local! {
    /// Окна, которым оболочка недавно послала «закрыть»: (id, когда).
    static CLOSING: std::cell::RefCell<Vec<(u64, std::time::Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Сколько окно после просьбы закрыться считается закрывающимся: программа может
/// не сразу убрать окно, а может и не закрыться (спросить «сохранить?»).
const CLOSING_FOR: std::time::Duration = std::time::Duration::from_secs(3);

fn note_closing(id: u64) {
    CLOSING.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|(i, t)| *i != id && t.elapsed() < CLOSING_FOR);
        c.push((id, std::time::Instant::now()));
    });
}

/// Окну недавно послали «закрыть» (из оболочки) — оно, скорее всего, вот-вот исчезнет.
pub fn is_closing(id: u64) -> bool {
    CLOSING.with(|c| c.borrow().iter().any(|(i, t)| *i == id && t.elapsed() < CLOSING_FOR))
}

/// Выполнить действие (кнопка панели, меню питания).
pub fn run(action: Action) {
    let ctx = ShellCtx::get();
    let cfg = ctx.cfg();
    match action {
        Action::Spawn(cmd) => spawn(
            &cmd.replace("$TERMINAL", &cfg.general.terminal)
                .replace("$FILE_MANAGER", &cfg.general.file_manager)
                .replace("$BROWSER", &cfg.general.browser),
        ),
        Action::Shell(cmd) => crate::commands::handle(&cmd),
        Action::None => {}
        // Без композитора synshell — системные действия напрямую.
        Action::Suspend if !ctx.connected.get_untracked() => spawn("systemctl suspend"),
        Action::Reboot if !ctx.connected.get_untracked() => spawn("systemctl reboot"),
        Action::PowerOff if !ctx.connected.get_untracked() => spawn("systemctl poweroff"),
        Action::Lock if !ctx.connected.get_untracked() => crate::commands::handle("lock"),
        Action::Quit if !ctx.connected.get_untracked() => spawn("loginctl terminate-session \"$XDG_SESSION_ID\""),
        other => send(Request::Action { action: other }),
    }
}

/// Разобрать строку действия и выполнить.
pub fn run_str(s: &str) {
    match s.parse::<Action>() {
        Ok(a) => run(a),
        Err(e) => log::warn!("действие «{s}»: {e}"),
    }
}
