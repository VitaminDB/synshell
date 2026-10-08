//! «Связь с устройствами» (демон synlink): эта машина (имя, видимость в
//! Wi-Fi, уведомления, файлы), связанные устройства (экран, файлы,
//! отключить, забыть), устройства рядом (спарить), ssh и отладка.
//!
//! Состояние — от демона раз в 2 с, пока страница открыта.

use std::cell::Cell;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synshell_common::link::{self, DeviceKind, PeerInfo, Request, Response, Status, Transport};

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

type Sig = RwSignal<Option<Option<Status>>>;

thread_local! {
    static SIG: Cell<Option<Sig>> = const { Cell::new(None) };
}

fn refresh(sig: Sig) {
    std::thread::spawn(move || {
        let st = link::status_quick(std::time::Duration::from_millis(800));
        run_on_main_thread(move || sig.set(Some(st)));
    });
}

fn sig() -> Sig {
    SIG.with(|w| match w.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            w.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == "devices" {
                        refresh(s);
                    }
                });
            });
            s
        }
    })
}

/// Команда демону в фоне, итог — всплывающей строкой.
fn send(label: &'static str, req: Request) {
    state::toast(format!("{label}…"));
    let s = sig();
    std::thread::spawn(move || {
        let msg = match link::request(&req) {
            Ok(Response::Error { message }) => format!("{label}: {message}"),
            Err(e) => format!("{label}: synlink не отвечает ({e})"),
            _ => format!("{label}: готово"),
        };
        run_on_main_thread(move || {
            state::toast(msg);
            refresh(s);
        });
    });
}

fn kind_icon(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Phone | DeviceKind::Tablet => "\u{e32c}",
        DeviceKind::Laptop => "\u{e31e}",
        DeviceKind::Desktop => "\u{e30a}",
    }
}

fn state_text(p: &PeerInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if p.connected {
        parts.push(format!("Соединено по {}", p.transport.map(Transport::title).unwrap_or("?")));
        if let Some(r) = p.rtt_ms {
            parts.push(format!("{} мс", r.round().max(1.0) as u32));
        }
    } else if p.paired {
        parts.push("Не в сети".into());
    } else {
        parts.push(format!("Рядом по {}", p.transport.map(Transport::title).unwrap_or("сети")));
    }
    if let Some(b) = &p.battery {
        parts.push(format!("батарея {}%{}", b.percent, if b.charging { ", заряжается" } else { "" }));
    }
    if let Some(a) = &p.address {
        parts.push(a.clone());
    }
    parts.join(" · ")
}

fn device_row(p: &PeerInfo) -> W {
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(kind_icon(p.kind)).class("dev-icon")).class(if p.connected { "dev-badge dev-badge-on" } else { "dev-badge" }))
        .child(
            Column::new()
                .gap(2.0)
                .child(Text::new(p.name.clone()).max_lines(1).class("row-label"))
                .child(Text::new(state_text(p)).max_lines(2).class("row-hint"))
                .class("grow"),
        );
    let mut actions = Row::new().gap(6.0);
    let id = p.id.clone();
    if p.connected {
        let (a, b, c) = (id.clone(), id.clone(), id.clone());
        actions = actions
            .child(button("Экран", move || {
                let _ = std::process::Command::new("synlink-view").arg(&a).spawn();
            }))
            .child(button("Файлы", move || {
                let b = b.clone();
                std::thread::spawn(move || {
                    let path = match link::request(&Request::Mount { device: b, mount: true }) {
                        Ok(Response::Mount { path: Some(p) }) => Some(p),
                        _ => None,
                    };
                    let st = link::status_quick(std::time::Duration::from_millis(800));
                    let files = st.and_then(|s| s.peers.into_iter().find(|p| Some(p.mount.clone()) == Some(path.clone())).and_then(|p| p.files_path()));
                    if let Some(f) = files.or(path) {
                        let _ = std::process::Command::new("synfiles").arg(f).spawn();
                    }
                });
            }))
            .child(button("Отключить", move || send("Отключение", Request::Disconnect { device: c.clone() })));
    }
    if p.paired {
        let d = id.clone();
        actions = actions.child(danger_icon_button("\u{e872}", move || send("Забыть устройство", Request::Unpair { device: d.clone() })));
    } else {
        let d = id.clone();
        actions = actions.child(primary_button("Спарить", move || send("Спаривание", Request::Pair { device: d.clone() })));
    }
    boxed(Column::new().gap(10.0).child(head).child(actions).class("dev-card"))
}

pub fn devices() -> W {
    let c = store::config();
    let l = c.link.clone();
    let sig = sig();
    refresh(sig);
    let mut body: Vec<W> = Vec::new();
    let live = Reactive::new(move || -> Vec<W> {
        let Some(st) = sig.get() else { return vec![note("Чтение состояния…")] };
        let Some(st) = st else {
            return vec![note(
                "Служба связи устройств (synlink) не запущена. Она стартует вместе с сеансом; вручную — synlink daemon.",
            )];
        };
        let mut out: Vec<W> = Vec::new();
        let usb = if st.peers.iter().any(|p| p.connected && p.transport == Some(Transport::Usb)) {
            "Кабель подключён · оболочки соединены"
        } else if st.usb.cable {
            "Кабель подключён · на том конце нет synshell"
        } else {
            "Кабель не подключён"
        };
        out.push(group(
            "Эта машина",
            vec![
                row_inline("Имя для других устройств", &format!("{} · {}", st.me.kind.title(), st.me.id), Text::new(st.me.name.clone()).class("row-value")),
                row_inline("USB", st.usb.interface.as_deref().unwrap_or(""), Text::new(usb).class("row-value")),
            ],
        ));
        let (paired, nearby): (Vec<&PeerInfo>, Vec<&PeerInfo>) = st.peers.iter().partition(|p| p.paired);
        if paired.is_empty() {
            out.push(group("Связанные устройства", vec![note("Пока нет. Подключите телефон кабелем USB — спаривание начнётся само (подтвердите на телефоне), или спарьте устройство из списка «Рядом».")]));
        } else {
            out.push(group("Связанные устройства", paired.into_iter().map(device_row).collect()));
        }
        if !nearby.is_empty() {
            out.push(group("Рядом", nearby.into_iter().map(device_row).collect()));
        }
        let hosts: Vec<W> = st
            .peers
            .iter()
            .filter(|p| p.paired)
            .map(|p| row_inline(&format!("ssh {}", p.ssh_host.clone().unwrap_or_default()), &p.name, Text::new("").class("row-value")))
            .collect();
        if !hosts.is_empty() {
            out.push(group("ssh", hosts));
        }
        out.push(note(
            "ssh идёт внутри соединения synlink (USB или Wi-Fi — какое есть), адрес знать не нужно. Ключи прописываются при спаривании и удаляются, если устройство забыть.",
        ));
        out.push(note("Claude Code и другие агенты: claude mcp add synlink -- synlink mcp — снимки экрана, касания, текст, команды и журнал устройств."));
        let mut col = Column::new().gap(18.0);
        for w in out {
            col = col.child(w);
        }
        vec![boxed(col)]
    });
    body.push(boxed(live));
    body.push(group(
        "Настройки",
        vec![
            switch_row("Связь с устройствами", "Служба synlink в сеансе (после перезахода)", op!["link", "enabled"], l.enabled),
            text_row("Имя этой машины", "Пусто — модель устройства", op!["link", "name"], &l.name, "по модели"),
            switch_row("Видна в Wi-Fi", "Можно спарить по сети; по кабелю — всегда", op!["link", "discoverable"], l.discoverable),
            switch_row("Уведомления", "Пересылать на связанные устройства и показывать их уведомления", op!["link", "notifications"], l.notifications),
            switch_row("Общий буфер обмена", "Скопированный текст сразу вставляется на связанных устройствах", op!["link", "clipboard"], l.clipboard),
            switch_row("Файлы устройств", "Монтировать в Проводник → Устройства", op!["link", "auto_mount"], l.auto_mount),
        ],
    ));
    body.push(group(
        "Трансляция экрана",
        vec![
            choice_row(
                "Кодирование",
                "Видео — аппаратным кодером того устройства, чей экран смотрим; в покое картинка досылается без потерь",
                op!["link", "screen_codec"],
                &l.screen_codec,
                &[
                    ("auto", "Авто: видео при движении"),
                    ("lossless", "Без потерь"),
                    ("hevc", "Видео HEVC"),
                    ("h264", "Видео H.264"),
                ],
            ),
            choice_row(
                "Кодер этого устройства",
                "Чем кодируется экран этой машины, когда его смотрят с другого устройства",
                op!["link", "screen_encoder"],
                &l.screen_encoder,
                &[
                    ("auto", "Авто: телефон — свой кодер, компьютер — Intel, потом NVIDIA"),
                    ("vaapi", "Intel / AMD (VAAPI)"),
                    ("nvenc", "NVIDIA (NVENC)"),
                ],
            ),
            int_row(
                "Битрейт видео, Мбит/с",
                "0 — сам: по кабелю 80, по Wi-Fi 30",
                op!["link", "screen_bitrate"],
                l.screen_bitrate as i64,
                0,
                400,
                5,
            ),
        ],
    ));
    page(
        "Связь с устройствами",
        "Телефон и компьютер с synshell: экран и управление, файлы, уведомления, ssh — по USB и Wi-Fi.",
        body,
    )
}
