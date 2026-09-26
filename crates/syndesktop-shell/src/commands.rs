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
        "window-switcher-end" => crate::switcher::end(ctx),
        "window-menu" => window_menu(ctx, &arg),
        "osd-layout" => {
            let (glyph, label) = match arg.trim() {
                "floating" => (crate::ui::mi::FLOAT, "Плавающие окна"),
                "tile" => (crate::ui::mi::TILE, "Мозаика"),
                "columns" => (crate::ui::mi::TILE, "Колонки"),
                "grid" => (crate::ui::mi::GRID, "Сетка"),
                "monocle" => (crate::ui::mi::FULLSCREEN, "Одно окно"),
                other => (crate::ui::mi::TILE, other),
            };
            crate::osd::show(ctx, glyph, None, label.to_string());
        }
        "screenshot-taken" => crate::notifications::local(
            ctx,
            "Снимок экрана",
            &format!("Сохранён в {}", arg.trim()),
            Some(arg.trim().to_string()),
        ),
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

/// Меню окна по ПКМ на заголовке: `<id> <x> <y>` в глобальных координатах.
fn window_menu(ctx: ShellCtx, arg: &str) {
    let v: Vec<&str> = arg.split_whitespace().collect();
    let (Some(id), Some(x), Some(y)) = (
        v.first().and_then(|s| s.parse::<u64>().ok()),
        v.get(1).and_then(|s| s.parse::<f32>().ok()),
        v.get(2).and_then(|s| s.parse::<f32>().ok()),
    ) else {
        return;
    };
    // Вывод, в который попадает точка, и координаты относительно него.
    let outs = ctx.comp_outputs.get_untracked();
    let hit = outs.iter().find(|o| {
        let [ox, oy, w, h] = o.geometry;
        x >= ox as f32 && y >= oy as f32 && x < (ox + w) as f32 && y < (oy + h) as f32
    });
    let (output, lx, ly) = match hit {
        Some(o) => (Some(o.name.clone()), x - o.geometry[0] as f32, y - o.geometry[1] as f32),
        None => (None, x, y),
    };
    ctx.popup.set(Some(crate::ctx::Popup {
        kind: PopupKind::WindowMenu(id),
        anchor: PopupAnchor { output, rect: Some([lx, ly, 0.0, 0.0]), edge: Edge::Top },
    }));
}

/// Блокировка экрана: внешняя программа из `[lock] command`, иначе
/// встроенный экран (ext-session-lock + PAM).
fn lock(ctx: ShellCtx) {
    let cfg = ctx.cfg();
    ctx.close_popup();
    if !cfg.lock.command.trim().is_empty() {
        crate::actions::spawn(&cfg.lock.command);
    } else {
        crate::lock::lock_now(ctx);
    }
}
