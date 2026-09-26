//! Сервер уведомлений `org.freedesktop.Notifications` (D-Bus, zbus):
//! всплывающие карточки в углу экрана и центр уведомлений с историей.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};
use zbus::zvariant::OwnedValue;

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, InputArea};

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    /// Путь к значку (разрешён по теме) или пусто.
    pub icon: Option<String>,
    /// Картинка из `image-data`: ширина, высота, RGBA.
    pub image: Option<(u32, u32, Arc<Vec<u8>>)>,
    pub summary: String,
    pub body: String,
    /// (ключ, подпись); `default` — действие по клику на карточку.
    pub actions: Vec<(String, String)>,
    /// 0 — низкая, 1 — обычная, 2 — критическая.
    pub urgency: u8,
    pub timeout_ms: u32,
    pub time: i64,
    pub transient: bool,
    pub resident: bool,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

struct Server {
    ctx: ShellCtx,
}

/// Грубое снятие разметки (`<b>`, `<a href>`…) и сущностей.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn image_from_hint(v: &OwnedValue) -> Option<(u32, u32, Arc<Vec<u8>>)> {
    let (w, h, stride, alpha, _bps, channels, data): (i32, i32, i32, bool, i32, i32, Vec<u8>) = v.try_clone().ok()?.try_into().ok()?;
    if w <= 0 || h <= 0 || channels < 3 {
        return None;
    }
    let (w, h, stride, ch) = (w as usize, h as usize, stride as usize, channels as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * ch;
            let px = data.get(i..i + ch)?;
            rgba.extend_from_slice(&[px[0], px[1], px[2], if alpha && ch >= 4 { px[3] } else { 255 }]);
        }
    }
    Some((w as u32, h as u32, Arc::new(rgba)))
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["body", "actions", "icon-static", "persistence", "body-hyperlinks"].iter().map(|s| s.to_string()).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 { replaces_id } else { NEXT_ID.fetch_add(1, Ordering::Relaxed) };
        let urgency: u8 = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let image = ["image-data", "image_data", "icon_data"].iter().find_map(|k| hints.get(*k).and_then(image_from_hint));
        let image_path = hints.get("image-path").or_else(|| hints.get("image_path")).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        let desktop = hints.get("desktop-entry").and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        let icon_name = image_path
            .map(|p| p.trim_start_matches("file://").to_string())
            .filter(|p| !p.is_empty())
            .or_else(|| (!app_icon.is_empty()).then(|| app_icon.trim_start_matches("file://").to_string()))
            .or_else(|| desktop.and_then(|d| crate::xdg::app_by_id(&d)).map(|e| e.icon));
        let icon = icon_name.and_then(|n| crate::xdg::lookup_icon(&n)).map(|p| p.to_string_lossy().into_owned());
        let bool_hint = |k: &str| hints.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        let n = Notification {
            id,
            app_name,
            icon,
            image,
            summary: strip_markup(&summary),
            body: strip_markup(&body),
            actions: actions.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect(),
            urgency,
            timeout_ms: expire_timeout.max(-1) as i32 as u32,
            time: crate::clock::unix_now(),
            transient: bool_hint("transient"),
            resident: bool_hint("resident"),
        };
        let n = Notification { timeout_ms: if expire_timeout < 0 { u32::MAX } else { expire_timeout as u32 }, ..n };
        let ctx = self.ctx;
        syngui::async_runtime::run_on_main_thread(move || add(ctx, n));
        id
    }

    fn close_notification(&self, id: u32) {
        let ctx = self.ctx;
        syngui::async_runtime::run_on_main_thread(move || close(ctx, id, 3));
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("syndesktop".into(), "syndesktop".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

/// Поднять сервер (в фоне; если имя занято — без уведомлений).
pub fn start(ctx: ShellCtx) {
    if !ctx.cfg().notifications.enabled {
        return;
    }
    std::thread::Builder::new()
        .name("shell-notify".into())
        .spawn(move || {
            let r = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name(IFACE))
                .and_then(|b| b.serve_at(PATH, Server { ctx }))
                .and_then(|b| b.build());
            match r {
                Ok(conn) => {
                    log::info!("уведомления: сервер {IFACE} запущен");
                    let _ = CONN.set(conn);
                    // Соединение обслуживается внутренним потоком zbus.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => log::warn!("уведомления: не удалось занять {IFACE}: {e}"),
            }
        })
        .ok();
}

fn emit(member: &'static str, id: u32, arg: SignalArg) {
    std::thread::spawn(move || {
        let Some(conn) = CONN.get() else { return };
        let r = match arg {
            SignalArg::Reason(r) => conn.emit_signal(None::<&str>, PATH, IFACE, member, &(id, r)),
            SignalArg::Action(a) => conn.emit_signal(None::<&str>, PATH, IFACE, member, &(id, a.as_str())),
        };
        if let Err(e) = r {
            log::debug!("уведомления: сигнал {member}: {e}");
        }
    });
}

enum SignalArg {
    Reason(u32),
    Action(String),
}

fn add(ctx: ShellCtx, n: Notification) {
    let cfg = ctx.cfg();
    let timeout = match n.timeout_ms {
        u32::MAX | 0 if n.urgency >= 2 => cfg.notifications.critical_timeout,
        u32::MAX => cfg.notifications.timeout,
        t => t,
    };
    let id = n.id;
    if cfg.notifications.history && !n.transient {
        ctx.history.update(|h| {
            h.retain(|x| x.id != id);
            h.insert(0, n.clone());
            h.truncate(cfg.notifications.history_size as usize);
        });
    }
    // «Не беспокоить» пропускает только критические.
    if ctx.dnd.get_untracked() && n.urgency < 2 {
        return;
    }
    ctx.notifications.update(|list| match list.iter_mut().find(|x| x.id == id) {
        Some(x) => *x = n.clone(),
        None => list.push(n.clone()),
    });
    if timeout > 0 {
        syngui_layer::add_timer(Duration::from_millis(timeout as u64), move || {
            // Истёк — карточка уходит, в истории остаётся.
            if ShellCtx::get().notifications.get_untracked().iter().any(|x| x.id == id && x.time == n.time) {
                close(ShellCtx::get(), id, 1);
            }
            None
        });
    }
}

/// Закрыть карточку. Причины: 1 — истекло, 2 — закрыл пользователь,
/// 3 — CloseNotification.
pub fn close(ctx: ShellCtx, id: u32, reason: u32) {
    ctx.notifications.update(|l| l.retain(|x| x.id != id));
    if reason != 1 {
        ctx.history.update(|h| h.retain(|x| x.id != id || x.resident));
    }
    emit("NotificationClosed", id, SignalArg::Reason(reason));
}

fn invoke(ctx: ShellCtx, n: &Notification, key: &str) {
    emit("ActionInvoked", n.id, SignalArg::Action(key.to_string()));
    if !n.resident {
        close(ctx, n.id, 2);
    }
}

// ─── Вид ─────────────────────────────────────────────────────────────────────

thread_local! {
    static SURFACE: std::cell::Cell<Option<SurfaceId>> = const { std::cell::Cell::new(None) };
}

pub fn install(ctx: ShellCtx) {
    create_effect(move || {
        let has = !ctx.notifications.get().is_empty();
        let generation = ctx.generation.get();
        let _ = generation;
        let cur = SURFACE.with(|s| s.get());
        if !has {
            if let Some(id) = SURFACE.with(|s| s.take()) {
                syngui_layer::close_surface(id);
            }
            return;
        }
        if cur.is_some() {
            return;
        }
        let cfg = ctx.cfg();
        let n = &cfg.notifications;
        let anchor = match n.position.as_str() {
            "top-left" => Anchor::TOP | Anchor::LEFT,
            "bottom-right" => Anchor::BOTTOM | Anchor::RIGHT,
            "bottom-left" => Anchor::BOTTOM | Anchor::LEFT,
            "top" => Anchor::TOP,
            "bottom" => Anchor::BOTTOM,
            _ => Anchor::TOP | Anchor::RIGHT,
        };
        let id = syngui_layer::create_surface(
            SurfaceSpec {
                namespace: "syndesktop-notification".into(),
                layer: Layer::Overlay,
                anchor,
                size: (n.width, 0),
                margin: [8, 8, 8, 8],
                exclusive_zone: 0,
                keyboard: KeyboardInteractivity::None,
                output: crate::manager::primary_output(&ctx),
                auto_size: true,
                clear_color: [0.0; 4],
            },
            move || Box::new(popups(ctx)),
        );
        SURFACE.with(|s| s.set(Some(id)));
    });
}

fn popups(ctx: ShellCtx) -> impl Widget {
    Column::new().gap(8.0).child(move || {
        let max = ctx.cfg().notifications.max_visible.max(1) as usize;
        let list = ctx.notifications.get();
        let mut col = Column::new().gap(8.0);
        let skip = list.len().saturating_sub(max);
        for n in list.into_iter().skip(skip).rev() {
            col = col.child(card(ctx, n, false));
        }
        col
    })
}

fn card(ctx: ShellCtx, n: Notification, in_center: bool) -> impl Widget {
    let mut head = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start);
    if let Some((w, h, rgba)) = &n.image {
        head = head.child(
            Image::from_rgba(format!("notif-img-{}-{}", n.id, n.time), *w, *h, rgba.as_ref().clone())
                .fit(ImageFit::Cover)
                .placeholder(false)
                .class("notif-image"),
        );
    } else if let Some(p) = &n.icon {
        head = head.child(Image::new(p.clone()).fit(ImageFit::Contain).placeholder(false).class("notif-icon"));
    } else {
        head = head.child(icon(mi::BELL).class("notif-glyph"));
    }
    let mut text = Column::new().gap(2.0).child(
        Row::new()
            .gap(6.0)
            .child(Text::new(n.app_name.clone()).max_lines(1).class("notif-app grow"))
            .child(Text::new(crate::clock::format(n.time, "%H:%M")).class("notif-time")),
    );
    if !n.summary.is_empty() {
        text = text.child(Text::new(n.summary.clone()).max_lines(2).class("notif-summary"));
    }
    if !n.body.is_empty() {
        text = text.child(Text::new(n.body.clone()).max_lines(if in_center { 3 } else { 5 }).class("notif-body"));
    }
    let id = n.id;
    head = head.child(text.class("grow")).child(
        InputArea::new(DecoratedBox::new().child(icon(mi::CLOSE)).class("notif-close")).pointer().on_click(move |b, _, _| {
            if b == MouseButton::Left {
                let ctx = ShellCtx::get();
                close(ctx, id, 2);
                ctx.history.update(|h| h.retain(|x| x.id != id));
            }
        }),
    );
    let mut col = Column::new().gap(8.0).child(head);
    let buttons: Vec<(String, String)> = n.actions.iter().filter(|(k, _)| k != "default").cloned().collect();
    if !buttons.is_empty() {
        let mut row = Row::new().gap(6.0);
        for (key, label) in buttons {
            let nn = n.clone();
            row = row.child(
                InputArea::new(DecoratedBox::new().child(Text::new(label).class("notif-action-label")).class("notif-action"))
                    .pointer()
                    .on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            invoke(ShellCtx::get(), &nn, &key);
                        }
                    }),
            );
        }
        col = col.child(row);
    }
    let cls = match n.urgency {
        2 => "notif notif-critical",
        0 => "notif notif-low",
        _ => "notif",
    };
    let nn = n.clone();
    let _ = ctx;
    InputArea::new(DecoratedBox::new().child(col).class(if in_center { format!("{cls} notif-in-center") } else { cls.to_string() }))
        .absorb()
        .on_click(move |b, _, _| {
            let ctx = ShellCtx::get();
            match b {
                MouseButton::Left => {
                    if nn.actions.iter().any(|(k, _)| k == "default") {
                        invoke(ctx, &nn, "default");
                    } else if !in_center {
                        close(ctx, nn.id, 2);
                    }
                }
                MouseButton::Middle | MouseButton::Right if !in_center => close(ctx, nn.id, 2),
                _ => {}
            }
        })
}

/// Центр уведомлений (окно апплета).
pub fn center(ctx: ShellCtx) -> impl Widget {
    Column::new()
        .gap(8.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new("Уведомления").class("popup-title grow"))
                .child(move || {
                    let dnd = ctx.dnd.get();
                    InputArea::new(
                        DecoratedBox::new()
                            .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(if dnd { mi::BELL_OFF } else { mi::BELL })).child(Text::new("Не беспокоить").class("chip-label")))
                            .class(if dnd { "chip chip-on" } else { "chip" }),
                    )
                    .pointer()
                    .on_click(|_, _, _| {
                        let c = ShellCtx::get();
                        c.dnd.set(!c.dnd.get_untracked());
                    })
                })
                .child(
                    InputArea::new(DecoratedBox::new().child(icon(mi::CLEAR_ALL)).class("footer-btn")).pointer().on_click(|_, _, _| {
                        let c = ShellCtx::get();
                        for n in c.history.get_untracked() {
                            emit("NotificationClosed", n.id, SignalArg::Reason(2));
                        }
                        c.history.set(Vec::new());
                        c.notifications.set(Vec::new());
                    }),
                ),
        )
        .child(
            ScrollView::new()
                .vertical()
                .child(move || {
                    let list = ctx.history.get();
                    let mut col = Column::new().gap(6.0);
                    if list.is_empty() {
                        col = col.child(Text::new("Нет уведомлений").class("launcher-empty"));
                    }
                    for n in list {
                        col = col.child(card(ctx, n, true));
                    }
                    col
                })
                .class("notif-center-list"),
        )
}
