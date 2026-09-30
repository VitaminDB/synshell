//! Команды оболочки (`Action::Shell`): приходят от композитора событием
//! `ShellCommand` (сочетания клавиш) или из кнопок самой оболочки.

use crate::ctx::{PopupAnchor, PopupKind, ShellCtx};
use crate::system;
use synshell_common::config::Edge;

/// Всплывающее окно по центру вывода с фокусом (из сочетания клавиш; на
/// телефоне — нижний лист).
pub fn centered() -> PopupAnchor {
    PopupAnchor { output: None, rect: None, edge: Edge::Bottom, attached: false }
}

/// Якорь у кнопки апплета, если она есть на панели (меню запуска по
/// Super открывается там же, где по клику), иначе по центру.
fn at_applet(kind: &str) -> PopupAnchor {
    crate::panel::applet_anchor(kind).unwrap_or_else(centered)
}

thread_local! {
    static EXTRA: std::cell::RefCell<Option<Box<dyn Fn(&str, &str) -> bool>>> = const { std::cell::RefCell::new(None) };
}

/// Свои команды оболочки (телефон: `home`, `shade`, `recents`…): `f(имя,
/// аргументы)` вызывается первым, `true` — команда обработана.
pub fn set_extra(f: impl Fn(&str, &str) -> bool + 'static) {
    EXTRA.with(|e| *e.borrow_mut() = Some(Box::new(f)));
}

/// Свайп от края `edge` с командой оболочке: если у этого края спрятан
/// автоскрытием док или панель — показать их (как навигационная панель
/// Android в полноэкранном режиме), иначе выполнить команду.
pub fn edge_gesture(edge: &str, cmd: &str) {
    let edge = match edge {
        "top" => Some(Edge::Top),
        "bottom" => Some(Edge::Bottom),
        "left" => Some(Edge::Left),
        "right" => Some(Edge::Right),
        _ => None,
    };
    // «Домой» спрятанный док не показывает, а выполняется: окна сворачиваются, и док с умным
    // скрытием появляется сам. Иначе быстрый свайп снизу при открытом окне только выдвигал док.
    let home = cmd.split_whitespace().next() == Some("home");
    if edge.is_some() && !home && crate::panel::reveal_hidden(edge) {
        log::debug!("жест от края {edge:?}: показан спрятанный док");
        return;
    }
    handle(cmd);
}

pub fn handle(cmd: &str) {
    let ctx = ShellCtx::get();
    {
        let mut it = cmd.split_whitespace();
        let name = it.next().unwrap_or("");
        let arg = it.collect::<Vec<_>>().join(" ");
        if EXTRA.with(|e| e.borrow().as_ref().is_some_and(|f| f(name, &arg))) {
            return;
        }
    }
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
        "network" | "wifi" => ctx.open_popup(PopupKind::Network, at_applet("network")),
        // Окно подключения к сети Wi-Fi (пароль, статус): `wifi-connect <SSID>`.
        "wifi-connect" if !arg.trim().is_empty() => ctx.open_popup(PopupKind::WifiConnect { ssid: arg.trim().to_string() }, centered()),
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
        "screenshot-copied" => crate::notifications::local(ctx, "Снимок экрана", "Скопирован в буфер обмена", None),
        "screenshot-path" => crate::notifications::local(
            ctx,
            "Снимок экрана",
            &format!("Путь скопирован в буфер обмена: {}", arg.trim()),
            Some(arg.trim().to_string()),
        ),
        "screenshot-failed" => crate::notifications::local(ctx, "Снимок не сделан", arg.trim(), None),
        "close-popup" => ctx.close_popup(),
        // Показать спрятанные автоскрытием панели и доки (`reveal-panels
        // bottom` — только у края); спрячутся сами через задержку.
        "reveal-panels" | "show-dock" => {
            let edge = match arg.trim() {
                "top" => Some(Edge::Top),
                "bottom" => Some(Edge::Bottom),
                "left" => Some(Edge::Left),
                "right" => Some(Edge::Right),
                _ => None,
            };
            crate::panel::reveal_hidden(edge);
        }
        // Жест «назад» при открытом оверлее: закрыть окно оболочки или
        // выйти из режима редактирования.
        "back" => {
            if crate::recents::is_open() {
                crate::recents::close();
            } else if crate::shade::is_open() {
                crate::shade::close();
            } else if ctx.popup.get_untracked().is_some() {
                ctx.close_popup();
            } else if ctx.editing.get_untracked().is_some() {
                crate::edit::stop_editing();
            }
        }
        "shade" => crate::shade::toggle(),
        "recents" => crate::recents::toggle(),
        "desktop-menu" => crate::edit::open_desktop_menu(&ctx, None, syngui::core::Point::new(40.0, 120.0)),
        "add-panel" => crate::edit::add_panel(false),
        "add-dock" => crate::edit::add_panel(true),
        // Режим редактирования панели/дока N (по умолчанию — первого дока,
        // иначе первой панели); повтор — выйти.
        "edit-panel" | "edit-dock" => {
            let cfg = ctx.cfg();
            let n = arg.trim().parse::<usize>().ok().or_else(|| {
                if name == "edit-dock" {
                    cfg.panels.iter().position(|p| p.is_dock())
                } else {
                    Some(0)
                }
            });
            match (n, ctx.editing.get_untracked()) {
                (_, Some(_)) => crate::edit::stop_editing(),
                (Some(n), None) if n < cfg.panels.len() => crate::edit::start_editing(n),
                _ => {}
            }
        }
        // Окно «Добавить» для панели N.
        "panel-add" => {
            let n = arg.trim().parse::<usize>().unwrap_or(0);
            ctx.editing.set(Some(n));
            ctx.open_popup(PopupKind::AddItem(n), centered());
        }
        // Меню значка лотка по номеру (с 0) — для клавиатуры и проверок.
        "tray-activate" => crate::tray::activate_at(arg.trim().parse().unwrap_or(0)),
        // Отладка: `tray-menu-event N ID` — пункт меню значка N.
        "tray-menu-event" => {
            let mut a = arg.split_whitespace().filter_map(|x| x.parse::<i64>().ok());
            if let (Some(i), Some(id)) = (a.next(), a.next()) {
                crate::tray::menu_event_at(i as usize, id as i32);
            }
        }
        "tray-menu" => crate::tray::open_menu_at(arg.trim().parse().unwrap_or(0), at_applet("tray")),
        "lock" => lock(ctx),
        "lock-preview" => crate::lock::preview(),
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
        anchor: PopupAnchor { output, rect: Some([lx, ly, 0.0, 0.0]), edge: Edge::Top, attached: false },
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
