//! Связь с другими устройствами (демон `synlink`): состояние в оболочке,
//! окно «Устройства» (апплет `link` на панели, карточка в шторке телефона),
//! диалог спаривания, всплывающие сообщения «соединено по USB».
//!
//! Подписка на демон — в фоновом потоке (`synshell_common::link`), команды
//! демону — тоже в фоне; нет демона — раздел просто не показывается.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::prelude::*;
use synshell_common::link::{self, DeviceKind, Event, PairPrompt, PeerInfo, Request, Response, Status, Transport};

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, rx, InputArea};

/// Значки Material Symbols.
pub mod gl {
    pub const PHONE: &str = "\u{E32C}";
    pub const LAPTOP: &str = "\u{E31E}";
    pub const COMPUTER: &str = "\u{E30A}";
    pub const TABLET: &str = "\u{E32F}";
    pub const USB: &str = "\u{E1E0}";
    pub const WIFI: &str = "\u{E63E}";
    pub const LINK: &str = "\u{E326}";
    pub const LINK_OFF: &str = "\u{E327}";
    pub const SCREEN: &str = "\u{E307}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const SHOT: &str = "\u{E3B0}";
    pub const TERMINAL: &str = "\u{EB8E}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const DELETE: &str = "\u{E872}";
    pub const ADD: &str = "\u{E145}";
    pub const BOLT: &str = "\u{EA0B}";
    pub const BATTERY: &str = "\u{E1A4}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const SHIELD: &str = "\u{E9E0}";
}

pub fn kind_glyph(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Phone => gl::PHONE,
        DeviceKind::Tablet => gl::TABLET,
        DeviceKind::Laptop => gl::LAPTOP,
        DeviceKind::Desktop => gl::COMPUTER,
    }
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Подписаться на демон (один раз на процесс оболочки).
pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("shell-link".into())
        .spawn(move || loop {
            match link::Client::connect().and_then(|c| c.subscribe()) {
                Ok(events) => {
                    for e in events {
                        let Ok(e) = e else { break };
                        run_on_main_thread(move || on_event(ShellCtx::get(), e));
                    }
                    run_on_main_thread(|| ShellCtx::get().link.set(None));
                }
                Err(_) => {}
            }
            // Демона нет (или перезапускается) — пробовать снова.
            std::thread::sleep(Duration::from_secs(3));
        })
        .ok();
    let _ = ctx;
}

fn on_event(ctx: ShellCtx, e: Event) {
    match e {
        Event::Status { status } => {
            // Запрос спаривания ушёл (решён там или по тайм-ауту) — закрыть диалог.
            if let Some(p) = ctx.popup.get_untracked() {
                if let PopupKind::LinkPair(id) = &p.kind {
                    if !status.prompts.iter().any(|x| &x.id == id) {
                        ctx.close_popup();
                    }
                }
            }
            ctx.link.set(Some(status));
        }
        Event::Connected { device } => {
            let how = device.transport.map(|t| t.title()).unwrap_or("сети");
            crate::osd::show(ctx, kind_glyph(device.kind), None, format!("{} · соединено по {how}", device.name));
        }
        Event::Disconnected { device } => {
            crate::osd::show(ctx, gl::LINK_OFF, None, format!("{} · соединение разорвано", device.name));
        }
        Event::PairPrompt { prompt } => {
            if !prompt.outgoing || prompt.code.is_some() {
                crate::shade::close();
                ctx.open_popup(PopupKind::LinkPair(prompt.id.clone()), crate::commands::centered());
            }
        }
        Event::Paired { name, ok, message, .. } => {
            let label = if ok {
                format!("{name} · спарено")
            } else {
                format!("{name} · не спарено{}", message.map(|m| format!(": {m}")).unwrap_or_default())
            };
            crate::osd::show(ctx, if ok { gl::LINK } else { gl::LINK_OFF }, None, label);
        }
    }
}

/// Команда демону в фоне; ошибка — всплывающим сообщением.
pub fn send(req: Request) {
    std::thread::spawn(move || {
        let r = link::request(&req);
        let err = match r {
            Ok(Response::Error { message }) => Some(message),
            Err(e) => Some(format!("synlink: {e}")),
            _ => None,
        };
        if let Some(m) = err {
            run_on_main_thread(move || crate::osd::show(ShellCtx::get(), gl::LINK_OFF, None, m));
        }
    });
}

/// Снимок экрана устройства в «Изображения/Снимки экрана» + уведомление.
fn screenshot(p: &PeerInfo) {
    let id = p.id.clone();
    let name = p.name.clone();
    std::thread::spawn(move || {
        let dir = synshell_common::paths::user_dir_or_default("PICTURES").join("Снимки экрана");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = crate::clock::format(crate::clock::unix_now(), "%Y-%m-%d_%H-%M-%S");
        let path = dir.join(format!("{}_{stamp}.png", name.replace('/', "_")));
        let req = Request::Screenshot { device: id, output: None, path: Some(path.to_string_lossy().into_owned()), max_size: None };
        let r = link::request(&req);
        run_on_main_thread(move || {
            let ctx = ShellCtx::get();
            match r {
                Ok(Response::Screenshot { path, width, height, .. }) => {
                    crate::notifications::local(ctx, &format!("Снимок: {name}"), &format!("{width}×{height}"), Some(path))
                }
                Ok(Response::Error { message }) => crate::osd::show(ctx, gl::LINK_OFF, None, message),
                Err(e) => crate::osd::show(ctx, gl::LINK_OFF, None, format!("synlink: {e}")),
                _ => {}
            }
        });
    });
}

fn open_files(p: &PeerInfo) {
    match p.files_path() {
        Some(path) => crate::actions::spawn(&format!("synfiles {}", shell_quote(&path))),
        None => send(Request::Mount { device: p.id.clone(), mount: true }),
    }
}

fn open_terminal(p: &PeerInfo) {
    let host = p.ssh_host.clone().unwrap_or_else(|| p.id.clone());
    let term = ShellCtx::get().cfg().general.terminal.clone();
    let term = if term.trim().is_empty() { "foot".to_string() } else { term };
    crate::actions::spawn(&format!("{term} -e ssh {}", shell_quote(&host)));
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Подпись состояния: «USB · 3 мс · 95 %».
fn state_line(p: &PeerInfo) -> String {
    let mut parts = Vec::new();
    if p.connected {
        parts.push(format!("Соединено · {}", p.transport.map(|t| t.title()).unwrap_or("?")));
        if let Some(r) = p.rtt_ms {
            parts.push(format!("{} мс", r.round().max(1.0) as u32));
        }
    } else if p.paired {
        parts.push("Не в сети".into());
    } else {
        parts.push(format!("Рядом · {}", p.transport.map(|t| t.title()).unwrap_or("сеть")));
    }
    if let Some(b) = &p.battery {
        parts.push(format!("{}{} %", if b.charging { "⚡" } else { "" }, b.percent));
    }
    parts.join(" · ")
}

fn chip(t: Option<Transport>, connected: bool) -> Box<dyn Widget> {
    let Some(t) = t.filter(|_| connected) else { return Box::new(DecoratedBox::new()) };
    let (glyph, class) = match t {
        Transport::Usb => (gl::USB, "link-chip link-chip-usb"),
        Transport::Wifi => (gl::WIFI, "link-chip link-chip-wifi"),
    };
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class("link-chip-icon"))
                    .child(Text::new(t.title()).class("link-chip-text")),
            )
            .class(class),
    )
}

/// Круглый значок устройства; у соединённого — акцентный с «пульсом».
fn avatar(k: DeviceKind, connected: bool, big: bool) -> impl Widget {
    let class = match (connected, big) {
        (true, true) => "link-avatar link-avatar-big link-avatar-on",
        (false, true) => "link-avatar link-avatar-big",
        (true, false) => "link-avatar link-avatar-on",
        (false, false) => "link-avatar",
    };
    DecoratedBox::new().child(icon(kind_glyph(k)).class(if big { "link-avatar-icon link-avatar-icon-big" } else { "link-avatar-icon" })).class(class)
}

fn action(glyph: &'static str, label: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(3.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class("link-action-icon"))
                    .child(Text::new(label.to_string()).max_lines(1).class("link-action-text")),
            )
            .class("link-action"),
    )
    .class("grow")
    .pointer()
    .on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

fn small_button(label: &str, primary: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
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

fn icon_button(glyph: &'static str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(icon(glyph).class("link-icon-btn-icon")).class("link-icon-btn")).pointer().on_click(
        move |b, _, _| {
            if b == MouseButton::Left {
                f();
            }
        },
    )
}

/// Карточка устройства: значок, имя, состояние, действия.
fn device_card(p: PeerInfo, compact: bool) -> Box<dyn Widget> {
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(avatar(p.kind, p.connected, false))
        .child(
            Column::new()
                .gap(1.0)
                .child(Text::new(p.name.clone()).max_lines(1).class("link-name"))
                .child(Text::new(state_line(&p)).max_lines(1).class(if p.connected { "link-state link-state-on" } else { "link-state" }))
                .class("grow"),
        )
        .child(chip(p.transport, p.connected));
    let mut col = Column::new().gap(10.0).child(head);
    if p.connected {
        let (a, b, c, d) = (p.clone(), p.clone(), p.clone(), p.clone());
        let mut actions = Row::new()
            .gap(6.0)
            .child(action(gl::SCREEN, "Экран", move || {
                ShellCtx::get().close_popup();
                crate::shade::close();
                crate::actions::spawn(&format!("synlink-view {}", a.id));
            }))
            .child(action(gl::FOLDER, "Файлы", move || {
                ShellCtx::get().close_popup();
                crate::shade::close();
                open_files(&b);
            }))
            .child(action(gl::SHOT, "Снимок", move || screenshot(&c)));
        if !compact {
            actions = actions.child(action(gl::TERMINAL, "Терминал", move || {
                ShellCtx::get().close_popup();
                open_terminal(&d);
            }));
        }
        col = col.child(actions);
    } else if !p.paired {
        let id = p.id.clone();
        col = col.child(
            Row::new()
                .gap(8.0)
                .child(Text::new("Не спарено — на устройстве появится код для сверки").max_lines(2).class("link-note grow"))
                .child(small_button("Спарить", true, move || send(Request::Pair { device: id.clone() }))),
        );
    }
    Box::new(DecoratedBox::new().child(col).class(if p.connected { "link-card link-card-on" } else { "link-card" }))
}

/// Строка «кабель USB»: подключён ли, есть ли на том конце оболочка.
fn usb_row(st: &Status) -> Box<dyn Widget> {
    let over_usb = st.peers.iter().any(|p| p.connected && p.transport == Some(Transport::Usb));
    let (text, on) = if over_usb {
        ("Кабель USB подключён · оболочки соединены".to_string(), true)
    } else if st.usb.cable {
        (
            if st.me.kind.is_touch() {
                "Кабель USB подключён · ждём оболочку компьютера".into()
            } else {
                "Кабель USB подключён · ждём оболочку телефона".into()
            },
            true,
        )
    } else {
        ("Кабель USB не подключён".to_string(), false)
    };
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class(if over_usb {
                "link-dot link-dot-on"
            } else if on {
                "link-dot link-dot-wait"
            } else {
                "link-dot"
            }))
            .child(icon(gl::USB).class(if on { "link-usb-icon link-usb-icon-on" } else { "link-usb-icon" }))
            .child(Text::new(text).max_lines(1).class("link-usb-text grow")),
    )
}

/// Окно «Устройства» (апплет `link`, карточка шторки).
pub fn view(ctx: ShellCtx) -> impl Widget {
    start(ctx);
    Column::new()
        .gap(10.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new("Устройства").class("popup-title grow"))
                .child(icon_button(gl::SETTINGS, || {
                    ShellCtx::get().close_popup();
                    crate::actions::spawn("synsettings devices");
                })),
        )
        .child(rx(move || {
            let Some(st) = ctx.link.get() else {
                return Box::new(
                    Column::new()
                        .gap(6.0)
                        .child(Text::new("Служба связи устройств не запущена").class("popup-text"))
                        .child(Text::new("Она запускается вместе с сеансом (synlink daemon); включается в [link] config.toml.").max_lines(3).class("link-note")),
                ) as Box<dyn Widget>;
            };
            let mut col = Column::new().gap(8.0).child(usb_row(&st));
            let (paired, nearby): (Vec<PeerInfo>, Vec<PeerInfo>) = st.peers.iter().cloned().partition(|p| p.paired);
            if paired.is_empty() && nearby.is_empty() {
                col = col.child(
                    DecoratedBox::new()
                        .child(
                            Column::new()
                                .gap(4.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(icon(gl::LINK).class("link-empty-icon"))
                                .child(Text::new("Устройств пока нет").class("popup-text"))
                                .child(
                                    Text::new("Подключите телефон кабелем USB или откройте synshell на другом устройстве в той же сети Wi-Fi")
                                        .max_lines(3)
                                        .class("link-note link-center"),
                                ),
                        )
                        .class("link-empty"),
                );
            }
            for p in paired {
                col = col.child(device_card(p, false));
            }
            if !nearby.is_empty() {
                col = col.child(Text::new("Рядом").class("link-section"));
                for p in nearby {
                    col = col.child(device_card(p, false));
                }
            }
            col = col.child(
                Text::new(format!("Эта машина: «{}»{}", st.me.name, if st.me.discoverable { "" } else { " · скрыта в Wi-Fi" }))
                    .max_lines(1)
                    .class("link-note"),
            );
            Box::new(col)
        }))
}

/// Диалог спаривания (по центру экрана).
pub fn pair_view(ctx: ShellCtx, id: String) -> impl Widget {
    rx(move || {
        let Some(p): Option<PairPrompt> = ctx.link.get().and_then(|s| s.prompts.into_iter().find(|x| x.id == id)) else {
            return Box::new(Text::new("Запрос спаривания закрыт").class("popup-text")) as Box<dyn Widget>;
        };
        let what = format!("{} «{}»", p.kind.title(), p.name);
        let (title, text) = match (&p.code, p.outgoing) {
            (Some(_), true) => ("Спаривание".to_string(), format!("Сверьте код с экраном устройства {what} и подтвердите там.")),
            (Some(_), false) => (
                "Запрос на соединение".to_string(),
                format!("{what} хочет соединиться по Wi-Fi. Совпадает ли код на обоих экранах?"),
            ),
            (None, _) => (
                "Подключено по USB".to_string(),
                format!("{what}. Разрешить ему экран, ввод, файлы, уведомления и ssh этого устройства?"),
            ),
        };
        let mut col = Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(avatar(p.kind, true, true))
            .child(Text::new(title).class("link-pair-title"))
            .child(Text::new(text).max_lines(4).class("link-pair-text"));
        if let Some(code) = &p.code {
            col = col.child(DecoratedBox::new().child(Text::new(code.clone()).class("link-code")).class("link-code-box"));
        }
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(if p.transport == Transport::Usb { "Кабель USB" } else { "Wi-Fi" }).class("link-note"))
                .child(icon(gl::SHIELD).class("link-shield")),
        );
        let (a, b) = (p.id.clone(), p.id.clone());
        let buttons = if p.outgoing {
            Row::new().gap(8.0).child(small_button("Отмена", false, move || {
                send(Request::Disconnect { device: a.clone() });
                ShellCtx::get().close_popup();
            }))
        } else {
            Row::new()
                .gap(8.0)
                .child(small_button("Отклонить", false, move || {
                    send(Request::PairReply { device: a.clone(), accept: false });
                    ShellCtx::get().close_popup();
                }))
                .child(small_button("Разрешить", true, move || {
                    send(Request::PairReply { device: b.clone(), accept: true });
                    ShellCtx::get().close_popup();
                }))
        };
        Box::new(col.child(buttons))
    })
}

/// Карточка шторки телефона: соединённое устройство или кабель.
pub fn shade_card(ctx: ShellCtx) -> impl Widget {
    start(ctx);
    rx(move || {
        let Some(st) = ctx.link.get() else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        let connected: Vec<PeerInfo> = st.peers.iter().filter(|p| p.connected).cloned().collect();
        if connected.is_empty() {
            // Нет соединения: строка кабеля (если он есть) — тап открывает окно устройств.
            if !st.usb.cable && !st.peers.iter().any(|p| !p.paired) {
                return Box::new(DecoratedBox::new());
            }
            return Box::new(
                InputArea::new(DecoratedBox::new().child(usb_row(&st)).class("link-card link-card-slim"))
                    .pointer()
                    .on_click(|b, _, _| {
                        if b == MouseButton::Left {
                            crate::shade::close();
                            ShellCtx::get().open_popup(PopupKind::Link, crate::commands::centered());
                        }
                    }),
            );
        }
        let mut col = Column::new().gap(8.0);
        for p in connected {
            col = col.child(device_card(p, true));
        }
        Box::new(col)
    })
}

/// Апплет панели `link`: значок связи; соединено — акцентный, с чипом USB.
pub fn applet(pc: &crate::panel::PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    start(ctx);
    let pc = pc.clone();
    Box::new(
        crate::applets::applet_button(
            "link",
            Row::new().child(move || {
                let st = ctx.link.get();
                let conn: Vec<PeerInfo> = st.as_ref().map(|s| s.connected().cloned().collect()).unwrap_or_default();
                let usb = conn.iter().any(|p| p.transport == Some(Transport::Usb));
                let mut row = Row::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Center);
                let glyph = conn.first().map(|p| kind_glyph(p.kind)).unwrap_or(gl::LINK);
                row = row.child(icon(glyph).class(if conn.is_empty() { "muted" } else { "link-applet-on" }));
                if usb {
                    row = row.child(icon(gl::USB).class("link-applet-usb"));
                }
                if let Some(b) = conn.first().and_then(|p| p.battery.clone()) {
                    row = row.child(Text::new(format!("{}%", b.percent)).class("applet-label link-applet-battery"));
                }
                row
            }),
        )
        .on_click(move |b, _, r| {
            if b == MouseButton::Left {
                ShellCtx::get().open_popup(PopupKind::Link, pc.anchor(r));
            }
        }),
    )
}

/// Значок связи в лотке (см. `tray::applet`): только без апплета `link` на
/// панелях и когда есть соединение или запрос спаривания.
pub fn tray_item(pc: &crate::panel::PanelCtx) -> Option<Box<dyn Widget>> {
    let ctx = ShellCtx::get();
    start(ctx);
    let has_applet = ctx.cfg().panels.iter().any(|p| p.applets.iter().any(|a| a.kind == "link"));
    if has_applet {
        return None;
    }
    let st = ctx.link.get()?;
    if !st.peers.iter().any(|p| p.connected) && st.prompts.is_empty() {
        return None;
    }
    Some(applet(pc))
}
