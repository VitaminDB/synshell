//! Местоположение для программ: оболочка — агент GeoClue
//! ([`synsystem::geoclue_agent`]). Решение — по `[location]` конфига
//! (выключатель, разрешённые и запрещённые навсегда); незнакомую программу
//! спрашивает диалогом по центру экрана: «Запретить», «Только сейчас» (до
//! перезапуска оболочки), «Разрешить».

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::prelude::*;
use synshell_common::config::Config;
use synsystem::geoclue_agent::{self, Decision, Handle, LocationRequest, Policy, Prompter};

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, InputArea};

pub const GLYPH: &str = "\u{E0C8}";

/// Вопросы в ожидании ответа (первый — на экране).
static PENDING: Mutex<Vec<LocationRequest>> = Mutex::new(Vec::new());
/// «Только сейчас»: разрешено до перезапуска оболочки.
static SESSION: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static HANDLE: OnceLock<Arc<Handle>> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

struct ConfigPolicy;

impl Policy for ConfigPolicy {
    fn decide(&self, desktop_id: &str) -> Decision {
        let loc = Config::load().0.location;
        let in_list = |l: &[String]| l.iter().any(|x| x == desktop_id);
        if in_list(&loc.denied) {
            Decision::Deny
        } else if in_list(&loc.allowed) || SESSION.lock().unwrap().as_ref().is_some_and(|s| s.contains(desktop_id)) {
            Decision::Allow
        } else {
            Decision::Ask
        }
    }

    fn enabled(&self) -> bool {
        Config::load().0.location.enabled
    }
}

struct UiPrompter;

impl Prompter for UiPrompter {
    fn ask(&self, req: LocationRequest) {
        let first = {
            let mut p = PENDING.lock().unwrap();
            p.push(req);
            p.len() == 1
        };
        if first {
            run_on_main_thread(show_next);
        }
    }

    fn cancel(&self, id: u64) {
        PENDING.lock().unwrap().retain(|r| r.id != id);
        run_on_main_thread(move || {
            let ctx = ShellCtx::get();
            if ctx.popup.get_untracked().is_some_and(|p| p.kind == PopupKind::LocationAsk(id)) {
                ctx.close_popup();
            }
            show_next();
        });
    }
}

/// Показать первый вопрос очереди, если диалога ещё нет.
fn show_next() {
    let ctx = ShellCtx::get();
    let Some(id) = PENDING.lock().unwrap().first().map(|r| r.id) else { return };
    if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::LocationAsk(_))) {
        return;
    }
    crate::shade::close();
    ctx.open_popup(PopupKind::LocationAsk(id), crate::commands::centered());
}

/// Зарегистрировать агента (в фоне: GeoClue запускается по D-Bus).
pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::on_reload(|ctx, _| {
        if let Some(h) = HANDLE.get() {
            h.set_enabled(ctx.cfg().location.enabled);
        }
    });
    std::thread::Builder::new()
        .name("shell-geoclue".into())
        .spawn(|| match geoclue_agent::start(ConfigPolicy, UiPrompter) {
            Ok(h) => {
                let _ = HANDLE.set(h);
            }
            // GeoClue не установлен — местоположения для программ нет, оболочке это не мешает
            Err(e) => log::info!("агент GeoClue: {e}"),
        })
        .ok();
    let _ = ctx;
}

fn remember(desktop_id: &str, allow: bool) {
    let loc = Config::load().0.location;
    let (key, mut list, other_key, mut other) =
        if allow { ("allowed", loc.allowed, "denied", loc.denied) } else { ("denied", loc.denied, "allowed", loc.allowed) };
    if !list.iter().any(|x| x == desktop_id) {
        list.push(desktop_id.to_string());
    }
    other.retain(|x| x != desktop_id);
    let arr = |l: Vec<String>| toml_edit::Value::Array(l.into_iter().collect());
    for (k, l) in [(key, list), (other_key, other)] {
        if let Err(e) = synshell_common::config_edit::set_value(&["location", k], arr(l)) {
            log::warn!("[location] {k}: {e:#}");
        }
    }
}

/// Ответ на вопрос `id`: `allow`, `forever` — запомнить в конфиге.
fn answer(id: u64, allow: bool, forever: bool) {
    let req = {
        let mut p = PENDING.lock().unwrap();
        p.iter().position(|r| r.id == id).map(|i| p.remove(i))
    };
    if let Some(r) = req {
        if forever {
            remember(&r.desktop_id, allow);
        } else if allow {
            SESSION.lock().unwrap().get_or_insert_with(HashSet::new).insert(r.desktop_id.clone());
        }
        r.respond(allow);
    }
    ShellCtx::get().close_popup();
    show_next();
}

fn button(label: &str, primary: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label.to_string()).class("link-btn-text")).class(if primary {
        "link-btn link-btn-primary"
    } else {
        "link-btn"
    }))
    .pointer()
    .on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

/// Диалог «Программа запрашивает местоположение».
pub fn ask_view(_ctx: ShellCtx, id: u64) -> impl Widget {
    crate::ui::rx(move || view(id))
}

fn view(id: u64) -> Box<dyn Widget> {
    let desktop_id = PENDING.lock().unwrap().iter().find(|r| r.id == id).map(|r| r.desktop_id.clone());
    let Some(desktop_id) = desktop_id else {
        return Box::new(Text::new(t!("Запрос местоположения закрыт")).class("popup-text"));
    };
    let entry = crate::xdg::app_by_id(&desktop_id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| desktop_id.clone());
    let pic: Box<dyn Widget> = match entry.as_ref().and_then(|e| crate::xdg::lookup_icon(&e.icon)) {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("location-app-icon")),
        None => Box::new(DecoratedBox::new().child(icon(GLYPH).class("link-avatar-icon link-avatar-icon-big")).class("link-avatar link-avatar-big")),
    };
    Box::new(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(pic)
            .child(Text::new(t!("Местоположение")).class("link-pair-title"))
            .child(Text::new(t!("«{name}» запрашивает ваше точное местоположение.", name = name)).max_lines(4).class("link-pair-text"))
            .child(
                Row::new()
                    .gap(8.0)
                    .child(button(&t!("Запретить"), false, move || answer(id, false, true)))
                    .child(button(&t!("Только сейчас"), false, move || answer(id, true, false)))
                    .child(button(&t!("Разрешить"), true, move || answer(id, true, true))),
            ),
    )
}
