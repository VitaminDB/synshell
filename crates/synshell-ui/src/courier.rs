//! Курьер (служба `syncourier`): апплет `courier` на панели и его окно —
//! привязка WhatsApp по QR-коду, состояние телефонного сеанса Claude Code,
//! последние сообщения и поле «написать Claude» прямо с компьютера.
//!
//! Состояние службы опрашивается в фоне (часто — пока открыто окно и ждём
//! сканирования QR), запросы — тоже в фоне.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use syn_courier::{ChannelState, ChannelStatus, Request, Status};
use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::prelude::*;

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, rx, InputArea};

mod gl {
    pub const CHAT: &str = "\u{E0B7}";
    pub const SMART_TOY: &str = "\u{F06C}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const SEND: &str = "\u{E163}";
    pub const PHONE: &str = "\u{E32C}";
}

thread_local! {
    static STATUS: RefCell<Option<RwSignal<Option<std::result::Result<Status, String>>>>> = const { RefCell::new(None) };
}

/// Состояние службы: `None` — ещё не спрашивали, `Err` — служба не запущена.
fn status_sig() -> RwSignal<Option<std::result::Result<Status, String>>> {
    STATUS.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(None)))
}

static STARTED: AtomicBool = AtomicBool::new(false);
/// До какого времени (мс) опрашивать часто: окно открыто, идёт привязка.
static FAST_UNTIL: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn fast_for(ms: u64) {
    FAST_UNTIL.fetch_max(now_ms() + ms, Ordering::SeqCst);
}

pub fn start() {
    let sig = status_sig();
    let _ = sig;
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("shell-courier".into())
        .spawn(|| loop {
            let r = syn_courier::status();
            run_on_main_thread(move || {
                let sig = status_sig();
                if sig.get_untracked().as_ref() != Some(&r) {
                    sig.set(Some(r));
                }
            });
            let fast = now_ms() < FAST_UNTIL.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(if fast { 700 } else { 4000 }));
        })
        .ok();
}

fn send(req: Request) {
    fast_for(20_000);
    std::thread::spawn(move || {
        let r = syn_courier::request(&req);
        if let Err(e) = r {
            log::warn!("Курьер: {e}");
        }
        if let Ok(s) = syn_courier::status() {
            run_on_main_thread(move || status_sig().set(Some(Ok(s))));
        }
    });
}

// ─────────────────────────────────────────────────────────────── QR-код

/// QR-код в RGBA: тёмные модули чёрные, поля 4 модуля, `scale` пикселей на модуль.
pub fn qr_rgba(data: &str, scale: u32) -> Option<(u32, u32, Vec<u8>)> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let w = code.width() as u32;
    let quiet = 4u32;
    let side = (w + quiet * 2) * scale;
    let mut px = vec![255u8; (side * side * 4) as usize];
    for y in 0..w {
        for x in 0..w {
            if code[(x as usize, y as usize)] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px_x, px_y) = ((x + quiet) * scale + dx, (y + quiet) * scale + dy);
                        let o = ((px_y * side + px_x) * 4) as usize;
                        px[o..o + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
    }
    Some((side, side, px))
}

// ─────────────────────────────────────────────────────────────── вид

fn button(label: &str, primary: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label.to_string()).class("link-btn-text")).class(if primary { "link-btn link-btn-primary" } else { "link-btn" }))
        .pointer()
        .on_click(move |b, _, _| {
            if b == MouseButton::Left {
                f();
            }
        })
}

fn icon_button(glyph: &'static str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(icon(glyph).class("link-icon-btn-icon")).class("link-icon-btn")).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

fn state_text(c: &ChannelStatus) -> String {
    match c.state {
        ChannelState::Online => match &c.account {
            Some(a) => t!("подключён · {a}", a = a),
            None => t!("подключён"),
        },
        ChannelState::Linking => t!("ждёт сканирования QR-кода"),
        ChannelState::Connecting => t!("подключается…"),
        ChannelState::Unlinked => t!("не подключён"),
        ChannelState::Off => t!("выключен"),
        ChannelState::Error => c.error.clone().unwrap_or_else(|| t!("ошибка")),
    }
}

/// Карточка WhatsApp: состояние и действие; при привязке — QR-код.
fn whatsapp_card(c: &ChannelStatus) -> Box<dyn Widget> {
    let on = c.state == ChannelState::Online;
    let mut head = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(icon(gl::PHONE).class("link-avatar-icon")).class(if on { "link-avatar link-avatar-on" } else { "link-avatar" }))
        .child(
            Column::new()
                .gap(2.0)
                .class("grow")
                .child(Text::new(c.title.clone()).class("link-name"))
                .child(Text::new(state_text(c)).max_lines(2).class(if on { "link-state link-state-on" } else { "link-state" })),
        );
    head = match c.state {
        ChannelState::Online => head.child(button(&t!("Отключить"), false, || send(Request::Unlink { channel: "whatsapp".into() }))),
        ChannelState::Linking | ChannelState::Connecting => head,
        _ => head.child(button(&t!("Подключить"), true, || send(Request::Link { channel: "whatsapp".into() }))),
    };
    let mut col = Column::new().gap(10.0).child(head);
    if c.state == ChannelState::Linking {
        match c.qr.as_deref().and_then(|q| qr_rgba(q, 5).map(|r| (q.to_string(), r))) {
            Some((q, (w, h, px))) => {
                let key = format!("courier-qr-{:x}", fxhash(&q));
                col = col
                    .child(
                        Row::new()
                            .main_axis_alignment(MainAxisAlignment::Center)
                            .child(DecoratedBox::new().child(Image::from_rgba(key, w, h, px).fit(ImageFit::Contain).class("courier-qr-image")).class("courier-qr")),
                    )
                    .child(
                        Text::new(t!("На телефоне: WhatsApp → Настройки → Связанные устройства → Привязка устройства, и наведите камеру на код."))
                            .max_lines(4)
                            .class("link-note link-center"),
                    )
                    .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(button(&t!("Отмена"), false, || send(Request::Unlink { channel: "whatsapp".into() }))));
            }
            None => col = col.child(Text::new(t!("Получаю QR-код…")).class("link-note")),
        }
    }
    if c.state == ChannelState::Online {
        col = col.child(Text::new(t!("Пишите в чат «Сообщения себе» — ответит Claude; /help — команды.")).max_lines(3).class("link-note"));
    }
    Box::new(DecoratedBox::new().child(col).class(if on { "link-card link-card-on" } else { "link-card" }))
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn claude_row(st: &Status) -> Box<dyn Widget> {
    let c = &st.claude;
    let state = if !c.enabled {
        t!("выключен")
    } else if c.busy {
        t!("думает…")
    } else if c.running {
        t!("ждёт сообщения")
    } else {
        t!("запустится с первым сообщением")
    };
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().child(icon(gl::SMART_TOY).class("link-avatar-icon")).class(if c.busy { "link-avatar link-avatar-on" } else { "link-avatar" }))
                    .child(
                        Column::new()
                            .gap(2.0)
                            .class("grow")
                            .child(Text::new(t!("Claude «{name}»", name = c.name)).class("link-name"))
                            .child(Text::new(state).max_lines(1).class("link-state")),
                    ),
            )
            .class("link-card link-card-slim"),
    )
}

fn recent(st: &Status) -> Box<dyn Widget> {
    let mut col = Column::new().gap(6.0).child(Text::new(t!("Последние сообщения")).class("link-section"));
    for r in st.recent.iter().rev().take(5).collect::<Vec<_>>().into_iter().rev() {
        let mine = r.dir == "in";
        let bubble = DecoratedBox::new().child(Text::new(r.text.clone()).max_lines(4).class("courier-msg-text")).class(if mine { "courier-msg courier-msg-in" } else { "courier-msg courier-msg-out" });
        col = col.child(Row::new().main_axis_alignment(if mine { MainAxisAlignment::End } else { MainAxisAlignment::Start }).child(bubble));
    }
    Box::new(col)
}

/// Поле «написать Claude» с компьютера (как сообщение из мессенджера).
fn ask_field() -> Box<dyn Widget> {
    let text = use_signal(String::new());
    let field_rev = use_signal(0u64);
    let submit = move || {
        let t = text.get_untracked();
        if t.trim().is_empty() {
            return;
        }
        text.set(String::new());
        field_rev.update(|r| *r += 1);
        send(Request::Ask { text: t });
    };
    Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(rx(move || {
                let _ = field_rev.get();
                Box::new(
                    TextField::new()
                        .placeholder(t!("Написать Claude…"))
                        .on_change(move |t: &str| text.set(t.to_string()))
                        .on_submit(move |_| submit())
                        .class("grow courier-ask"),
                ) as Box<dyn Widget>
            }))
            .child(icon_button(gl::SEND, submit)),
    )
}

pub fn view(ctx: ShellCtx) -> impl Widget {
    let _ = ctx;
    start();
    fast_for(60_000);
    let sig = status_sig();
    Column::new()
        .gap(10.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Курьер")).class("popup-title grow"))
                .child(icon_button(gl::SETTINGS, || {
                    ShellCtx::get().close_popup();
                    crate::actions::spawn("synsettings courier");
                })),
        )
        .child(rx(move || {
            let st = match sig.get() {
                None => return Box::new(Text::new(t!("Связываюсь со службой…")).class("popup-text")) as Box<dyn Widget>,
                Some(Err(_)) => {
                    return Box::new(
                        Column::new()
                            .gap(6.0)
                            .child(Text::new(t!("Служба Курьера не запущена")).class("popup-text"))
                            .child(Text::new(t!("Включите её: systemctl --user enable --now syncourier")).max_lines(3).class("link-note"))
                            .child(button(&t!("Запустить"), true, || {
                                crate::actions::spawn("systemctl --user enable --now syncourier");
                                fast_for(15_000);
                            })),
                    )
                }
                Some(Ok(s)) => s,
            };
            if st.channels.iter().any(|c| matches!(c.state, ChannelState::Linking | ChannelState::Connecting)) {
                fast_for(5_000);
            }
            let mut col = Column::new().gap(10.0);
            for c in &st.channels {
                col = col.child(whatsapp_card(c));
            }
            col = col.child(claude_row(&st));
            if !st.recent.is_empty() {
                col = col.child(recent(&st));
            }
            Box::new(col)
        }))
        .child(ask_field())
}

/// Апплет `courier`: значок разговора; подключён — подсвечен, Claude думает — точка.
pub fn applet(pc: &crate::panel::PanelCtx) -> Box<dyn Widget> {
    start();
    let pc = pc.clone();
    let sig = status_sig();
    Box::new(
        crate::applets::applet_button(
            "courier",
            Row::new().child(move || {
                let st = sig.get().and_then(|r| r.ok());
                let online = st.as_ref().is_some_and(|s| s.channels.iter().any(|c| c.state == ChannelState::Online));
                let busy = st.as_ref().is_some_and(|s| s.claude.busy);
                let mut row = Row::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Center);
                row = row.child(icon(gl::CHAT).class(if online { "courier-applet-on" } else { "muted" }));
                if busy {
                    row = row.child(DecoratedBox::new().class("courier-busy-dot"));
                }
                row
            }),
        )
        .on_click(move |b, _, r| {
            if b == MouseButton::Left {
                ShellCtx::get().open_popup(PopupKind::Courier, pc.anchor(r));
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    /// Снимок окна: `SHOT_DIR=… cargo test -p synshell-ui shot_courier -- --ignored`.
    #[test]
    #[ignore]
    fn shot_courier() {
        use super::*;
        use syn_courier::{ClaudeStatus, Recent};
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let cfg = synshell_common::config::Config::default();
        let mss = crate::theme::build(&cfg);
        let cases = [("linking", ChannelState::Linking), ("online", ChannelState::Online), ("unlinked", ChannelState::Unlinked)];
        for (name, state) in cases {
            let out = format!("{dir}/courier-{name}.png");
            syngui::shot::render_png(
                move || {
                    STARTED.store(true, Ordering::SeqCst);
                    let ctx = crate::ShellCtx::new(synshell_common::config::Config::default(), synshell_common::config::FormFactor::Desktop);
                    provide_context(ctx);
                    let st = Status {
                        channels: vec![ChannelStatus {
                            id: "whatsapp".into(),
                            title: "WhatsApp".into(),
                            state,
                            account: (state == ChannelState::Online).then(|| "+77011234567".into()),
                            qr: (state == ChannelState::Linking).then(|| "2@Yt0g0ZsY8nq3l5i2kEJxUqTn6V0BvDzA9Lr6sF,abcDEF0123456789abcDEF0123456789abcd=,QmFzZTY0S2V5MDEyMzQ1Njc4OWFiY2RlZg==,UmVmMTIzNDU2Nzg5MA==".into()),
                            error: None,
                        }],
                        claude: ClaudeStatus { enabled: true, name: "courier".into(), cwd: "~".into(), running: true, busy: state == ChannelState::Online, ..Default::default() },
                        recent: if state == ChannelState::Online {
                            vec![
                                Recent { dir: "in".into(), text: "/to hexspire-b0 как дела с декомпиляцией?".into(), at: 0 },
                                Recent { dir: "out".into(), text: "Сеанс hexspire-b0 ответил: декомпилировано 14 функций, результат в out/".into(), at: 0 },
                            ]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    };
                    status_sig().set(Some(Ok(st)));
                    Box::new(DecoratedBox::new().class("popup-card").child(view(ctx)))
                },
                &mss,
                (420, 760),
                1.0,
                &out,
            )
            .unwrap();
        }
    }

    #[test]
    fn qr_image() {
        let (w, h, px) = super::qr_rgba("2@abc,def,ghi", 4).unwrap();
        assert_eq!(w, h);
        assert_eq!(px.len(), (w * h * 4) as usize);
        // угол — светлое поле, внутри есть тёмные модули
        assert_eq!(&px[0..3], &[255, 255, 255]);
        assert!(px.chunks(4).any(|p| p[0] == 0));
    }
}
