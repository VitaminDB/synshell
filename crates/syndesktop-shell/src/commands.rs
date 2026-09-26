//! Команды оболочки (`Action::Shell`): приходят от композитора событием
//! `ShellCommand` (сочетания клавиш) или из кнопок самой оболочки.

use crate::ctx::{PopupAnchor, PopupKind, ShellCtx};
use crate::system;
use syndesktop_common::config::Edge;

/// Всплывающее окно по центру вывода с фокусом (из сочетания клавиш).
fn centered() -> PopupAnchor {
    PopupAnchor { output: None, rect: None, edge: Edge::Bottom }
}

/// Якорь у кнопки апплета, если она есть на панели (меню запуска по
/// Super открывается там же, где по клику), иначе по центру.
fn at_applet(kind: &str) -> PopupAnchor {
    crate::panel::applet_anchor(kind).unwrap_or_else(centered)
}

pub fn handle(cmd: &str) {
    let ctx = ShellCtx::get();
    let mut it = cmd.split_whitespace();
    let name = it.next().unwrap_or("");
    let arg = it.collect::<Vec<_>>().join(" ");
    let num = |d: i32| -> i32 { arg.trim().trim_start_matches('+').parse().unwrap_or(d) };
    log::debug!("команда оболочки: {cmd}");
    match name {
        "launcher" | "menu" => ctx.open_popup(PopupKind::Launcher, at_applet("launcher")),
        "run" => ctx.open_popup(PopupKind::Run, centered()),
        "power-menu" | "power" => ctx.open_popup(PopupKind::Power, centered()),
        "notifications" => ctx.open_popup(PopupKind::Notifications, at_applet("notifications")),
        "calendar" => ctx.open_popup(PopupKind::Calendar, at_applet("clock")),
        "volume" if arg.is_empty() => ctx.open_popup(PopupKind::Volume, at_applet("volume")),
        "volume" => {
            let d = if arg.starts_with('-') { arg.parse().unwrap_or(-5) } else { num(5) };
            system::change_volume(ctx, d, None);
        }
        "mute" => system::toggle_mute(ctx, false),
        "mic-mute" => system::toggle_mute(ctx, true),
        "brightness" => {
            let d = if arg.starts_with('-') { arg.parse().unwrap_or(-5) } else { num(5) };
            system::change_brightness(ctx, d);
        }
        "media" => system::media(arg.trim()),
        "window-switcher" => crate::switcher::command(ctx, arg.trim()),
        "close-popup" => ctx.close_popup(),
        "lock" => lock(ctx),
        "dnd" => ctx.dnd.set(!ctx.dnd.get_untracked()),
        "clipboard" => {
            // История буфера обмена требует wlr-data-control-клиента —
            // пока открываем меню запуска с подсказкой.
            log::info!("история буфера обмена пока не реализована");
        }
        "reload" => crate::reload_config(),
        "quit-shell" => syngui_layer::quit(),
        other => log::warn!("неизвестная команда оболочки «{other}»"),
    }
}

/// Блокировка экрана: внешняя программа из `[lock] command`, иначе
/// `loginctl lock-session` (её перехватывает менеджер блокировки сеанса).
fn lock(ctx: ShellCtx) {
    let cfg = ctx.cfg();
    ctx.close_popup();
    if !cfg.lock.command.trim().is_empty() {
        crate::actions::spawn(&cfg.lock.command);
    } else if crate::actions::which("swaylock") {
        crate::actions::spawn("swaylock -f");
    } else {
        // TODO: встроенный экран блокировки на ext-session-lock + PAM.
        log::warn!("экран блокировки: задайте [lock] command (например, «swaylock -f»)");
        crate::actions::spawn("loginctl lock-session");
    }
}
