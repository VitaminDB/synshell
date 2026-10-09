//! Сервер уведомлений `org.freedesktop.Notifications` (D-Bus, zbus):
//! всплывающие карточки в углу экрана и центр уведомлений с историей.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui::containers::Keyed;
use syngui::widgets::{EventHook, GestureDetector, SwipeDirection};
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
    /// Своё уведомление оболочки: по клику открыть этот файл.
    pub open_path: Option<String>,
    /// Своё уведомление оболочки: по клику выполнить команду (открыть приложение на нужном месте).
    pub open_command: Option<String>,
    /// Полоса хода, как в Android (подсказка `value`): 0–100 %, меньше 0 — неопределённый ход.
    pub progress: Option<i32>,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

struct Server {
    ctx: ShellCtx,
}

/// Снять разметку (для других модулей).
pub fn strip_markup_pub(s: &str) -> String {
    strip_markup(s)
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

/// Интерфейс сигнала о новом уведомлении (`Added`) — для synlink: у протокола
/// уведомлений своего сигнала нет, а мониторинг шины — режим отладки.
pub const MIRROR_IFACE: &str = "org.synshell.Notifications";

/// Аргументы `org.synshell.Notifications.Added` (тип `(susssybbb)`).
#[derive(serde::Serialize, zbus::zvariant::Type)]
struct Added {
    app: String,
    replaces: u32,
    icon: String,
    summary: String,
    body: String,
    urgency: u8,
    /// Мгновенное служебное (громкость, яркость).
    transient: bool,
    /// С полосой хода (загрузка) — обновляется, пересылать незачем.
    progress: bool,
    /// Само пришло с другого устройства (подсказка `x-synlink-device`).
    forwarded: bool,
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
        // `value` — int32 по спецификации, но некоторые шлют uint32/int64
        let progress = hints.get("value").and_then(|v| {
            i32::try_from(v)
                .ok()
                .or_else(|| u32::try_from(v).ok().map(|x| x.min(100) as i32))
                .or_else(|| i64::try_from(v).ok().map(|x| x.clamp(-1, 100) as i32))
        });
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
            open_path: None,
            open_command: None,
            progress: progress.map(|p| p.min(100)),
        };
        let n = Notification { timeout_ms: if expire_timeout < 0 { u32::MAX } else { expire_timeout as u32 }, ..n };
        // Для synlink (пересылка на спаренные устройства) — без мониторинга шины.
        let added = Added {
            app: n.app_name.clone(),
            replaces: replaces_id,
            icon: app_icon,
            summary: n.summary.clone(),
            body: n.body.clone(),
            urgency,
            transient: n.transient || hints.contains_key("x-canonical-private-synchronous"),
            progress: progress.is_some(),
            forwarded: hints.contains_key("x-synlink-device"),
        };
        std::thread::spawn(move || {
            if let Some(conn) = CONN.get() {
                if let Err(e) = conn.emit_signal(None::<&str>, PATH, MIRROR_IFACE, "Added", &added) {
                    log::debug!("уведомления: сигнал Added: {e}");
                }
            }
        });
        let ctx = self.ctx;
        syngui::async_runtime::run_on_main_thread(move || add(ctx, n, replaces_id != 0));
        id
    }

    fn close_notification(&self, id: u32) {
        let ctx = self.ctx;
        syngui::async_runtime::run_on_main_thread(move || close(ctx, id, 3));
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("synshell".into(), "synshell".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
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

/// `replace` — обновление уже показанного (`replaces_id`), например хода загрузки: как в Android, карточка
/// меняется на месте, не всплывает заново и не продлевает показ; убранное с экрана обновляется только в
/// истории, а закрытое пользователем больше не появляется.
fn add(ctx: ShellCtx, n: Notification, replace: bool) {
    log::debug!("уведомление #{} от «{}»: {} (срочность {})", n.id, n.app_name, n.summary, n.urgency);
    let cfg = ctx.cfg();
    if replace {
        let id = n.id;
        let on_screen = ctx.notifications.get_untracked().iter().find(|x| x.id == id).map(|x| x.time);
        let in_history = ctx.history.get_untracked().iter().find(|x| x.id == id).map(|x| x.time);
        if let Some(time) = on_screen.or(in_history) {
            // Время — первого показа: по нему же истекает показ на экране
            let n = Notification { time, ..n };
            if cfg.notifications.history && !n.transient {
                ctx.history.update(|h| {
                    if let Some(x) = h.iter_mut().find(|x| x.id == id) {
                        *x = n.clone();
                    }
                });
            }
            if on_screen.is_some() {
                ctx.notifications.update(|list| {
                    if let Some(x) = list.iter_mut().find(|x| x.id == id) {
                        *x = n;
                    }
                });
            }
            return;
        }
        if cfg.notifications.history && id < NEXT_ID.load(Ordering::Relaxed) {
            // Было, но закрыто пользователем
            return;
        }
    }
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
    let fresh = !ctx.notifications.get_untracked().iter().any(|x| x.id == id);
    ctx.notifications.update(|list| match list.iter_mut().find(|x| x.id == id) {
        Some(x) => *x = n.clone(),
        None => list.push(n.clone()),
    });
    // Новое (не обновлённое) уведомление — вибрацией; тихие (transient) — нет.
    if fresh && !n.transient {
        synshell_common::haptics::play(synshell_common::haptics::Feedback::Notification);
    }
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
/// 3 — CloseNotification (приложение убрало его само — и из истории, даже `resident`).
pub fn close(ctx: ShellCtx, id: u32, reason: u32) {
    ctx.notifications.update(|l| l.retain(|x| x.id != id));
    if reason != 1 {
        HANDLERS.with(|h| h.borrow_mut().remove(&id));
    }
    match reason {
        1 => {}
        3 => ctx.history.update(|h| h.retain(|x| x.id != id)),
        _ => ctx.history.update(|h| h.retain(|x| x.id != id || x.resident)),
    }
    emit("NotificationClosed", id, SignalArg::Reason(reason));
}

fn invoke(ctx: ShellCtx, n: &Notification, key: &str) {
    if let Some(h) = HANDLERS.with(|h| h.borrow().get(&n.id).cloned()) {
        close(ctx, n.id, 2);
        h(key);
        return;
    }
    if let Some(c) = &n.open_command {
        crate::actions::spawn(c);
        close(ctx, n.id, 2);
        return;
    }
    if let Some(p) = &n.open_path {
        crate::launchers::open_path(std::path::Path::new(p));
        close(ctx, n.id, 2);
        return;
    }
    emit("ActionInvoked", n.id, SignalArg::Action(key.to_string()));
    if !n.resident {
        close(ctx, n.id, 2);
    }
}

/// Уведомление от самой оболочки (снимок экрана и т.п.). `open` — файл,
/// который откроется по клику; если это картинка — она же и значок.
pub fn local(ctx: ShellCtx, summary: &str, body: &str, open: Option<String>) {
    let icon = open
        .as_ref()
        .filter(|p| p.ends_with(".png") || p.ends_with(".jpg"))
        .cloned()
        .or_else(|| crate::xdg::lookup_icon("preferences-desktop-notification").map(|p| p.to_string_lossy().into_owned()));
    let n = Notification {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        app_name: "synshell".into(),
        icon,
        image: None,
        summary: summary.into(),
        body: body.into(),
        actions: if open.is_some() { vec![("default".into(), t!("Открыть").into())] } else { Vec::new() },
        urgency: 1,
        timeout_ms: u32::MAX,
        time: crate::clock::unix_now(),
        transient: false,
        resident: false,
        open_path: open,
        open_command: None,
        progress: None,
    };
    add(ctx, n, false);
}

thread_local! {
    /// Обработчики кнопок своих уведомлений оболочки (`local_actions`) по номеру уведомления.
    static HANDLERS: std::cell::RefCell<HashMap<u32, std::rc::Rc<dyn Fn(&str)>>> = std::cell::RefCell::new(HashMap::new());
}

/// Убрать свои уведомления с ключом `key` (с экрана и из истории).
pub fn close_local(ctx: ShellCtx, key: &str) {
    let app = format!("synshell:{key}");
    let mut ids: Vec<u32> = ctx.notifications.get_untracked().iter().filter(|n| n.app_name == app).map(|n| n.id).collect();
    ids.extend(ctx.history.get_untracked().iter().filter(|n| n.app_name == app).map(|n| n.id));
    for id in ids {
        close(ctx, id, 3);
    }
}

/// Уведомление оболочки с кнопками: `actions` — (ключ, подпись), `default` — щелчок по карточке;
/// нажатое уходит в `handler` (уведомление закрывается). `key` — как у `local_command`.
pub fn local_actions(
    ctx: ShellCtx,
    key: &str,
    summary: &str,
    body: &str,
    icon: &str,
    actions: &[(&str, &str)],
    handler: impl Fn(&str) + 'static,
) {
    let n = local_note(ctx, key, summary, body, icon, actions, None);
    HANDLERS.with(|h| h.borrow_mut().insert(n.id, std::rc::Rc::new(handler)));
    add(ctx, n, false);
}

/// Уведомление оболочки «идёт работа» (полоса без конца) — до замены другим с тем же `key`.
pub fn local_busy(ctx: ShellCtx, key: &str, summary: &str, body: &str, icon: &str) {
    let n = local_note(ctx, key, summary, body, icon, &[], Some(-1));
    add(ctx, n, false);
}

fn local_note(ctx: ShellCtx, key: &str, summary: &str, body: &str, icon: &str, actions: &[(&str, &str)], progress: Option<i32>) -> Notification {
    close_local(ctx, key);
    Notification {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        app_name: format!("synshell:{key}"),
        icon: crate::xdg::lookup_icon(icon).map(|p| p.to_string_lossy().into_owned()),
        image: None,
        summary: summary.into(),
        body: body.into(),
        actions: actions.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect(),
        urgency: 1,
        timeout_ms: if progress.is_some() { 0 } else { u32::MAX },
        time: crate::clock::unix_now(),
        transient: false,
        resident: false,
        open_path: None,
        open_command: None,
        progress,
    }
}

/// Уведомление оболочки с командой по клику: `icon` — имя значка темы, `key` — заменить прежнее с тем же
/// ключом (новое SMS той же переписки), а не копить карточки.
pub fn local_command(ctx: ShellCtx, key: &str, summary: &str, body: &str, icon: &str, command: String) {
    let app = format!("synshell:{key}");
    let old: Vec<u32> = ctx.notifications.get_untracked().iter().filter(|n| n.app_name == app).map(|n| n.id).collect();
    for id in old {
        close(ctx, id, 3);
    }
    let n = Notification {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        app_name: app,
        icon: crate::xdg::lookup_icon(icon).map(|p| p.to_string_lossy().into_owned()),
        image: None,
        summary: summary.into(),
        body: body.into(),
        actions: vec![("default".into(), t!("Открыть").into())],
        urgency: 1,
        timeout_ms: u32::MAX,
        time: crate::clock::unix_now(),
        transient: false,
        resident: false,
        open_path: None,
        open_command: Some(command),
        progress: None,
    };
    add(ctx, n, false);
}

// ─── Вид ─────────────────────────────────────────────────────────────────────

thread_local! {
    static SURFACE: std::cell::Cell<Option<SurfaceId>> = const { std::cell::Cell::new(None) };
    /// Место и ширина, с которыми создана поверхность: сменили в настройках — пересоздать.
    static PLACED: std::cell::Cell<Option<(Place, u32)>> = const { std::cell::Cell::new(None) };
    /// Карточки на экране: активные и уходящие (`false`), пока не доиграла
    /// анимация ухода. Новые встают в начало (сверху).
    static SHOWN: std::cell::Cell<Option<RwSignal<Vec<(Notification, bool)>>>> = const { std::cell::Cell::new(None) };
}

/// Где всплывают уведомления: край экрана и положение вдоль него.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    pub bottom: bool,
    /// -1 — слева, 0 — по центру, 1 — справа.
    pub side: i8,
}

/// `[notifications] position`: `auto` — на телефоне сверху по центру, на компьютере сверху справа.
pub fn place(ctx: &ShellCtx) -> Place {
    let cfg = ctx.cfg();
    let p = cfg.notifications.position.trim();
    let p = if p.is_empty() || p == "auto" { if ctx.is_phone() { "top" } else { "top-right" } } else { p };
    Place {
        bottom: p.starts_with("bottom"),
        side: if p.ends_with("left") {
            -1
        } else if p.ends_with("right") {
            1
        } else {
            0
        },
    }
}

fn shown() -> RwSignal<Vec<(Notification, bool)>> {
    SHOWN.with(|s| match s.get() {
        Some(sig) => sig,
        None => {
            let sig = use_signal(Vec::new());
            s.set(Some(sig));
            sig
        }
    })
}

pub fn install(ctx: ShellCtx) {
    let shown = shown();
    // Список активных → карточки на экране: пропавшие остаются уходящими.
    create_effect(move || {
        let list = ctx.notifications.get();
        let animate = crate::anim::on(&ctx);
        shown.update(|v| {
            for e in v.iter_mut() {
                match list.iter().find(|n| n.id == e.0.id) {
                    Some(n) => {
                        e.0 = n.clone();
                        e.1 = true;
                    }
                    None => e.1 = false,
                }
            }
            if !animate {
                v.retain(|e| e.1);
            }
            for n in list.iter().rev() {
                if !v.iter().any(|e| e.0.id == n.id) {
                    v.insert(0, (n.clone(), true));
                }
            }
        });
    });
    create_effect(move || {
        let has = !shown.get().is_empty();
        let generation = ctx.generation.get();
        let _ = generation;
        let pl = place(&ctx);
        let width = card_width(&ctx);
        if SURFACE.with(|s| s.get()).is_some() && PLACED.with(|p| p.get()) != Some((pl, width)) {
            // Место или ширину сменили в настройках — поверхность заново
            if let Some(id) = SURFACE.with(|s| s.take()) {
                syngui_layer::close_surface(id);
            }
        }
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
        let edge = if pl.bottom { Anchor::BOTTOM } else { Anchor::TOP };
        let anchor = match pl.side {
            -1 => edge | Anchor::LEFT,
            1 => edge | Anchor::RIGHT,
            _ => edge,
        };
        // Поля внутри поверхности — место под тень карточек; отрицательный отступ
        // от прижатых краёв возвращает карточку на NOTIF_GAP от края экрана.
        let m = NOTIF_GAP - NOTIF_PAD as i32;
        let (mv, mh) = (m, if pl.side == 0 { 0 } else { m });
        let margin = [
            if pl.bottom { 0 } else { mv },
            if pl.side == 1 { mh } else { 0 },
            if pl.bottom { mv } else { 0 },
            if pl.side == -1 { mh } else { 0 },
        ];
        let bounds = Arc::new(syngui::core::sync::Mutex::new(Rect::zero()));
        let slot = bounds.clone();
        let id = syngui_layer::create_surface(
            SurfaceSpec {
                namespace: "syndesktop-notification".into(),
                layer: Layer::Overlay,
                anchor,
                size: (width + 2 * NOTIF_PAD, 0),
                margin,
                exclusive_zone: 0,
                keyboard: KeyboardInteractivity::None,
                output: crate::manager::primary_output(&ctx),
                auto_size: true,
                clear_color: [0.0; 4],
            },
            move || Box::new(popups(ctx, slot)),
        );
        // Ввод — только по карточкам: поля под тень не должны перехватывать клики.
        let mut last: Option<Vec<[i32; 4]>> = None;
        syngui_layer::add_timer(Duration::ZERO, move || {
            if SURFACE.with(|s| s.get()) != Some(id) {
                return None;
            }
            let r = *bounds.lock().unwrap_or_else(|e| e.into_inner());
            let region = vec![[r.origin.x, r.origin.y, r.size.width, r.size.height].map(|v| v.round() as i32)];
            if last.as_ref() != Some(&region) {
                syngui_layer::set_input_region(id, Some(region.clone()));
                last = Some(region);
            }
            Some(Duration::from_millis(100))
        });
        SURFACE.with(|s| s.set(Some(id)));
        PLACED.with(|p| p.set(Some((pl, width))));
    });
}

/// Ширина карточки: на компьютере — `[notifications] width`, на телефоне — почти во всю ширину экрана.
fn card_width(ctx: &ShellCtx) -> u32 {
    let w = ctx.cfg().notifications.width;
    if ctx.is_phone() {
        let (ow, _) = crate::manager::output_size(None);
        ((ow as u32).saturating_sub(2 * NOTIF_GAP as u32 + 8)).max(240)
    } else {
        w
    }
}

/// Поля поверхности всплывающих уведомлений — место под тень карточек (в темах
/// до `0 20px 50px`), чтобы она не обрезалась краем поверхности; = padding `.notif-popups`.
const NOTIF_PAD: u32 = 56;
/// Расстояние от карточки до края экрана.
const NOTIF_GAP: i32 = 12;

fn popups(ctx: ShellCtx, bounds: Arc<syngui::core::sync::Mutex<Rect>>) -> impl Widget {
    let shown = shown();
    let list = Column::new().gap(8.0).child(move || {
        let max = ctx.cfg().notifications.max_visible.max(1) as usize;
        let list = shown.get();
        let dur = crate::anim::ms(&ctx, 380);
        let pl = place(&ctx);
        // Въезд и уход — от своего края: сбоку — вбок, по центру — сверху/снизу
        let (dx, dy, ox, oy) = match (pl.side, pl.bottom) {
            (-1, _) => (-48.0, 0.0, 0.0, 0.5),
            (1, _) => (48.0, 0.0, 1.0, 0.5),
            (_, false) => (0.0, -36.0, 0.5, 0.0),
            (_, true) => (0.0, 36.0, 0.5, 1.0),
        };
        let mut col = Column::new().gap(10.0);
        let mut alive_seen = 0usize;
        // Новые — у края экрана: сверху — первыми, снизу — последними
        let mut items: Vec<(Notification, bool)> = list
            .into_iter()
            .map(|(n, alive)| {
                let visible = alive && {
                    alive_seen += 1;
                    alive_seen <= max
                };
                (n, visible)
            })
            .collect();
        if pl.bottom {
            items.reverse();
        }
        for (n, visible) in items {
            let id = n.id;
            // Версия — содержимое и видимость: смена любого пересобирает
            // карточку на месте, ключ переживает сдвиги соседей.
            let version = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (n.time, &n.summary, &n.body, n.progress, &n.actions, visible).hash(&mut h);
                h.finish()
            };
            col = col.child(Keyed::new(id as u64, version, move || {
                // Смахнуть вбок — закрыть (на телефоне)
                let swipe = GestureDetector::new()
                    .on_swipe(move |dir, _| {
                        if matches!(dir, SwipeDirection::Left | SwipeDirection::Right) {
                            close(ShellCtx::get(), id, 2);
                        }
                    })
                    .child(card(ctx, n.clone(), false));
                let presence = Presence::new(visible, swipe)
                    .enter(Motion::fade().slide(dx, dy).scale(0.94))
                    .exit(Motion::fade().slide(dx, dy).scale(0.96))
                    .collapse(AnimationAxis::Height)
                    .duration_ms(dur)
                    .exit_duration_ms(dur * 3 / 4)
                    .origin(TransformOrigin::Custom(ox, oy))
                    .initial(dur > 0)
                    .on_exit_complete(move || shown.update(|v| v.retain(|e| e.0.id != id)));
                Box::new(AnimatedPosition::new(presence))
            }));
        }
        col
    });
    Column::new().class("notif-popups").child(EventHook::new().report_bounds(bounds).child(list))
}

/// Подпись отправителя: у своих уведомлений оболочки (`synshell:КЛЮЧ`) — без ключа.
fn app_title(app: &str) -> String {
    match app.strip_prefix("synshell:") {
        Some(k) if k.starts_with("device:") => t!("Устройства").into(),
        Some(_) => "synshell".into(),
        None => app.to_string(),
    }
}

fn card(ctx: ShellCtx, n: Notification, in_center: bool) -> impl Widget {
    let mut head = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start);
    if let Some((w, h, rgba)) = &n.image {
        head = head.child(
            Image::from_rgba(format!("notif-img-{}-{}", n.id, n.time), *w, *h, rgba.as_ref().clone())
                .fit(ImageFit::Cover)
                .placeholder(false)
                .class("notif-image"),
        );
    } else if let Some(p) = &n.icon {
        head = head.child(
            DecoratedBox::new().child(Image::new(p.clone()).fit(ImageFit::Contain).placeholder(false).class("notif-icon")).class("notif-plate"),
        );
    } else {
        head = head.child(DecoratedBox::new().child(icon(mi::BELL).class("notif-glyph")).class("notif-plate"));
    }
    let mut text = Column::new().gap(2.0).child(
        Row::new()
            .gap(6.0)
            .child(Text::new(app_title(&n.app_name)).max_lines(1).class("notif-app grow"))
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
    match n.progress {
        Some(p) if p >= 0 => col = col.child(ProgressBar::with_value(p as f32 / 100.0).class("notif-progress")),
        Some(_) => col = col.child(ProgressBar::new().indeterminate().class("notif-progress")),
        None => {}
    }
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
                .child(Text::new(t!("Уведомления")).class("popup-title grow"))
                .child(move || {
                    let dnd = ctx.dnd.get();
                    InputArea::new(
                        DecoratedBox::new()
                            .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(if dnd { mi::BELL_OFF } else { mi::BELL })).child(Text::new(t!("Не беспокоить")).class("chip-label")))
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
                        col = col.child(Text::new(t!("Нет уведомлений")).class("launcher-empty"));
                    }
                    for n in list {
                        col = col.child(card(ctx, n, true));
                    }
                    col
                })
                .class("notif-center-list"),
        )
}
