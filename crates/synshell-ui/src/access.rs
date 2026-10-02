//! Доступ программ к устройствам через порталы: оболочка — бэкенд
//! `org.freedesktop.impl.portal.Access` ([`synsystem::portal_access`]).
//! Портал Camera (и другие, кто спрашивает `AccessDialog`) зовёт его для
//! программы без сохранённого решения; решение запоминает сам портал
//! (хранилище разрешений), оболочка только спрашивает диалогом по центру
//! экрана: «Запретить» / «Разрешить» (подписи — от портала, если он их дал).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::prelude::*;
use synsystem::portal_access::{self, AccessRequest, Prompter};

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, mi, InputArea};

/// Вопросы в ожидании ответа (первый — на экране).
static PENDING: Mutex<Vec<AccessRequest>> = Mutex::new(Vec::new());
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

struct UiPrompter;

impl Prompter for UiPrompter {
    fn ask(&self, req: AccessRequest) {
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
        let had = {
            let mut p = PENDING.lock().unwrap();
            let n = p.len();
            p.retain(|r| r.id != id);
            n != p.len()
        };
        if !had {
            return;
        }
        run_on_main_thread(move || {
            let ctx = ShellCtx::get();
            if ctx.popup.get_untracked().is_some_and(|p| p.kind == PopupKind::AccessAsk(id)) {
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
    if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::AccessAsk(_))) {
        return;
    }
    crate::shade::close();
    ctx.open_popup(PopupKind::AccessAsk(id), crate::commands::centered());
}

/// Занять имя бэкенда портала (в фоне: шина сеанса).
pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = ctx;
    std::thread::Builder::new()
        .name("shell-portal-access".into())
        .spawn(|| match portal_access::start(UiPrompter) {
            Ok(c) => {
                let _ = CONN.set(c);
            }
            // имя занято (вторая оболочка) или нет шины — доступ спросит другой бэкенд
            Err(e) => log::info!("бэкенд портала Access: {e}"),
        })
        .ok();
}

/// Ответ на вопрос `id`.
fn answer(id: u64, allow: bool) {
    let req = {
        let mut p = PENDING.lock().unwrap();
        p.iter().position(|r| r.id == id).map(|i| p.remove(i))
    };
    if let Some(r) = req {
        r.respond(allow);
    }
    ShellCtx::get().close_popup();
    show_next();
}

fn button(label: String, primary: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label).class("link-btn-text")).class(if primary {
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

/// Диалог «Разрешить программе …?».
pub fn ask_view(_ctx: ShellCtx, id: u64) -> impl Widget {
    crate::ui::rx(move || view(id))
}

fn view(id: u64) -> Box<dyn Widget> {
    let req = PENDING.lock().unwrap().iter().find(|r| r.id == id).map(|r| {
        (r.app_id.clone(), r.title.clone(), r.subtitle.clone(), r.body.clone(), r.grant_label.clone(), r.deny_label.clone(), r.icon.clone())
    });
    let Some((app_id, title, subtitle, body, grant, deny, portal_icon)) = req else {
        return Box::new(Text::new("Запрос закрыт").class("popup-text"));
    };
    let entry = (!app_id.is_empty()).then(|| crate::xdg::app_by_id(&app_id)).flatten();
    // значок программы, иначе предложенный порталом (камера и т. п.), иначе глиф
    let camera = portal_icon.as_deref().is_some_and(|i| i.contains("camera")) || title.to_lowercase().contains("камер");
    let path = entry.as_ref().and_then(|e| crate::xdg::lookup_icon(&e.icon)).or_else(|| portal_icon.as_deref().and_then(crate::xdg::lookup_icon));
    let pic: Box<dyn Widget> = match path {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("location-app-icon")),
        None => Box::new(
            DecoratedBox::new()
                .child(icon(if camera { mi::CAMERA } else { mi::LOCK }).class("link-avatar-icon link-avatar-icon-big"))
                .class("link-avatar link-avatar-big"),
        ),
    };
    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(pic);
    if let Some(e) = &entry {
        col = col.child(Text::new(e.name.clone()).class("link-pair-text"));
    }
    col = col.child(Text::new(title).max_lines(3).class("link-pair-title"));
    for t in [subtitle, body] {
        if !t.is_empty() {
            col = col.child(Text::new(t).max_lines(5).class("link-pair-text"));
        }
    }
    Box::new(
        col.child(
            Row::new()
                .gap(8.0)
                .child(button(deny.unwrap_or_else(|| "Запретить".into()), false, move || answer(id, false)))
                .child(button(grant.unwrap_or_else(|| "Разрешить".into()), true, move || answer(id, true))),
        ),
    )
}
