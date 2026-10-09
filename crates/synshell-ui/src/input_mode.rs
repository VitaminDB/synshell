//! Экранный ввод телефона (`[osk]`): у каждого приложения свой режим — клавиатура, контроллер
//! (syngamepad) или ничего; клавиатура выезжает сама только в режиме «Клавиатура». Оболочка следит за окном в фокусе и командует обоим демонам через их
//! сокеты; выбор режима — плитка «Ввод» в шторке ([`set_for_focused`]).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use synshell_common::config::InputMode;

use syngui::prelude::*;

use crate::ctx::ShellCtx;
use crate::ui::mi;

/// Значок контроллера (Material Icons `sports_esports`).
pub const GAMEPAD: &str = "\u{EA28}";
const CHECK: &str = "\u{E5CA}";
const BLOCK: &str = "\u{E14B}";

pub fn glyph(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Keyboard => mi::KEYBOARD,
        InputMode::Controller => GAMEPAD,
        InputMode::Off => BLOCK,
    }
}

/// Имя приложения в фокусе (из .desktop) — для подписи «для Steam».
pub fn focused_app_name(ctx: &ShellCtx) -> Option<String> {
    let app = focused_app(ctx)?;
    Some(crate::xdg::app_for_window(&app).map(|e| e.name).filter(|n| !n.is_empty()).unwrap_or(app))
}

/// Окно плитки «Ввод»: три режима для приложения в фокусе, показать клавиатуру, свернуть контроллер.
pub fn view(ctx: ShellCtx) -> impl Widget {
    let now = current(&ctx);
    let whom = match focused_app_name(&ctx) {
        Some(n) => format!("Для «{n}»"),
        None => "По умолчанию (приложения без своего режима)".into(),
    };
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new("Экранный ввод").class("popup-title"))
        .child(Text::new(whom).class("popup-subtitle"));
    for mode in [InputMode::Keyboard, InputMode::Controller, InputMode::Off] {
        let label = if mode == now { format!("{}  {CHECK}", mode.title()) } else { mode.title().to_string() };
        col = col.child(crate::popup::menu_item(glyph(mode), label, move || set_for_focused(&ShellCtx::get(), mode)));
    }
    col = col.child(crate::popup::menu_item(mi::KEYBOARD, "Показать клавиатуру", || crate::actions::spawn("synkeyboard show")));
    if now == InputMode::Controller {
        col = col.child(crate::popup::menu_item(GAMEPAD, "Свернуть / развернуть контроллер", || crate::actions::spawn("syngamepad fold")));
    }
    col
}

/// Перезапуск демона контроллера: применённое состояние у него пропало — применить заново.
static GEN: AtomicU64 = AtomicU64::new(0);

/// app_id окна в фокусе (`None` — домашний экран, окон нет).
pub fn focused_app(ctx: &ShellCtx) -> Option<String> {
    let id = ctx.focused.get_untracked()?;
    ctx.windows.get_untracked().iter().find(|w| w.id == id && !w.minimized).map(|w| w.app_id.clone()).filter(|a| !a.is_empty())
}

/// Режим ввода сейчас (для окна в фокусе).
pub fn current(ctx: &ShellCtx) -> InputMode {
    ctx.cfg().osk.mode_for(focused_app(ctx).as_deref())
}

/// Выбрать режим для приложения в фокусе (без окна — режим по умолчанию).
pub fn set_for_focused(ctx: &ShellCtx, mode: InputMode) {
    let value = toml_edit::Value::from(mode.as_str());
    let r = match focused_app(ctx) {
        Some(app) => synshell_common::config_edit::set_value(&["osk", "apps", &app], value),
        None => synshell_common::config_edit::set_value(&["osk", "mode"], value),
    };
    if let Err(e) = r {
        log::warn!("[osk] режим ввода: {e:#}");
    }
}

/// Демон контроллера перезапущен — состояние применить заново.
pub fn daemon_restarted() {
    GEN.fetch_add(1, Ordering::Relaxed);
    // Пересчитать: эффект следит за `config` — тот же конфиг, но новое значение сигнала.
    syngui::async_runtime::run_on_main_thread(|| {
        let ctx = ShellCtx::get();
        ctx.config.set(ctx.config.get_untracked());
    });
}

/// Следить за окном в фокусе и `[osk]`: показывать контроллер, выключать автопоказ клавиатуры.
pub fn start(ctx: ShellCtx) {
    let last = std::cell::RefCell::new(None::<(InputMode, String, u64)>);
    syngui::prelude::create_effect(move || {
        let _ = ctx.focused.get();
        let _ = ctx.windows.get();
        let cfg = ctx.config.get();
        let app = focused_app(&ctx);
        let mode = cfg.osk.mode_for(app.as_deref());
        let layout = cfg.osk.layout_for(app.as_deref()).to_string();
        let state = (mode, layout.clone(), GEN.load(Ordering::Relaxed));
        if last.borrow().as_ref() == Some(&state) {
            return;
        }
        log::info!("экранный ввод: {} — {} ({layout})", app.as_deref().unwrap_or("-"), mode.as_str());
        *last.borrow_mut() = Some(state);
        std::thread::spawn(move || {
            let pad: &[String] = &match mode {
                InputMode::Controller => vec![format!("layout {layout}"), "show".into()],
                _ => vec!["hide".into()],
            };
            for cmd in pad {
                send("syngamepad.sock", cmd);
            }
            // Клавиатура выезжает сама только в режиме «Клавиатура»: у терминала или игры поле ввода
            // в фокусе всегда, и она закрывала бы контроллер (в шторке — «Показать клавиатуру»).
            send("synkeyboard.sock", if mode == InputMode::Keyboard { "auto on" } else { "auto off" });
        });
    });
}

/// Команда демону: несколько попыток — при старте оболочки он может ещё подниматься.
fn send(sock: &str, cmd: &str) {
    let path = synshell_common::paths::runtime_dir().join(sock);
    for _ in 0..10 {
        let r = (|| -> std::io::Result<String> {
            let mut s = UnixStream::connect(&path)?;
            s.set_read_timeout(Some(Duration::from_secs(5)))?;
            writeln!(s, "{cmd}")?;
            let mut reply = String::new();
            BufReader::new(s).read_line(&mut reply)?;
            Ok(reply)
        })();
        match r {
            Ok(reply) => {
                if reply.starts_with("error") {
                    log::warn!("{sock} {cmd}: {}", reply.trim());
                }
                return;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    log::warn!("{sock}: нет ответа на «{cmd}»");
}
