//! Звук: выходы и входы PipeWire (выбор по умолчанию, громкость, без звука), громкость программ и функции
//! телефона через palaudiod (arch-mobile-port, docs/14-audio.md): динамик/наушники и Dolby Atmos с профилями.
//! Данные — `synsystem::sound`; перечитываются раз в 2 с, пока страница открыта.

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synsystem::sound::{self, DolbyProfile, Node, NodeKind, PhoneAudio, Route};

use crate::{op, state, store};
use crate::ui::*;

const ICON_SPEAKER: &str = "\u{e32d}";
const ICON_HEADPHONES: &str = "\u{e310}";
const ICON_MIC: &str = "\u{e029}";
const ICON_APP: &str = "\u{e5c3}";
const ICON_MUTED: &str = "\u{e04f}";
const ICON_VOLUME: &str = "\u{e050}";
const ICON_MIC_OFF: &str = "\u{e02b}";

#[derive(Clone, PartialEq, Default)]
struct Snap {
    /// `None` — PipeWire не отвечает.
    nodes: Option<Vec<Node>>,
    phone: Option<PhoneAudio>,
    profiles: Vec<DolbyProfile>,
    /// Предел громкости из `[sound]` (переключатель на странице меняет ползунки).
    max: u32,
}

thread_local! {
    static SIG: std::cell::Cell<Option<RwSignal<Option<Snap>>>> = const { std::cell::Cell::new(None) };
}

fn collect() -> Snap {
    let phone = sound::phone_audio();
    let profiles = if phone.as_ref().is_some_and(|p| p.dolby.is_some()) { sound::dolby_profiles() } else { Vec::new() };
    Snap { nodes: sound::nodes(), phone, profiles, max: 100 }
}

fn refresh(sig: RwSignal<Option<Snap>>) {
    std::thread::spawn(move || {
        let mut s = collect();
        run_on_main_thread(move || {
            s.max = store::config().sound.max_volume();
            if sig.get_untracked().as_ref() != Some(&s) {
                sig.set(Some(s));
            }
        });
    });
}

fn sig() -> RwSignal<Option<Snap>> {
    SIG.with(|w| match w.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            w.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == "audio" {
                        refresh(s);
                    }
                });
            });
            s
        }
    })
}

/// Действие в фоне, затем перечитать состояние.
fn act(f: impl FnOnce() + Send + 'static) {
    let s = sig();
    std::thread::spawn(move || {
        f();
        let mut snap = collect();
        run_on_main_thread(move || {
            snap.max = store::config().sound.max_volume();
            s.set(Some(snap));
        });
    });
}

fn node_icon(n: &Node) -> &'static str {
    match n.kind {
        NodeKind::Input if n.muted => ICON_MIC_OFF,
        NodeKind::Input => ICON_MIC,
        NodeKind::AppStream => ICON_APP,
        NodeKind::Output if n.icon.as_deref().is_some_and(|i| i.contains("headphone") || i.contains("headset")) => ICON_HEADPHONES,
        NodeKind::Output => ICON_SPEAKER,
    }
}

fn node_row(n: &Node, max: u32, choosable: bool) -> W {
    let id = n.id;
    let mut hint = Vec::new();
    if n.default {
        hint.push("по умолчанию");
    }
    if n.running {
        hint.push(if n.kind == NodeKind::Input { "идёт запись" } else { "играет" });
    }
    if n.muted {
        hint.push("без звука");
    }
    let mut col = Column::new().gap(8.0).class("row-text").child(Text::new(n.label.clone()).class("row-label"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint.join(" · ")).class("row-hint"));
    }
    let mut ctl = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(p) = n.percent {
        ctl = ctl.child(
            Slider::new()
                .range(0.0, max as f32)
                .step(1.0)
                .value(p.min(max) as f32)
                .show_value(0)
                .width(180.0)
                .on_change(move |v| act(move || sound::set_node_volume(id, v.round() as u32, max))),
        );
    }
    let muted = n.muted;
    let mute_icon = match (n.kind, muted) {
        (NodeKind::Input, true) => ICON_MIC_OFF,
        (NodeKind::Input, false) => ICON_MIC,
        (_, true) => ICON_MUTED,
        (_, false) => ICON_VOLUME,
    };
    ctl = ctl.child(icon_button(mute_icon, move || act(move || sound::set_node_mute(id, !muted))));
    if choosable && !n.default {
        ctl = ctl.child(button("Выбрать", move || act(move || sound::set_default(id))));
    }
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("setting-row")
            .child(Icon::new(node_icon(n)).class("wifi-signal"))
            .child(col)
            .child(ctl),
    )
}

fn phone_group(p: &PhoneAudio, profiles: &[DolbyProfile]) -> W {
    let out = match p.output.as_str() {
        "headphones" => "наушники",
        "headset" => "гарнитура",
        _ => "динамики",
    };
    let jack = match (p.headphones_plugged, p.headset_mic) {
        (true, true) => "Гарнитура с микрофоном подключена",
        (true, false) => "Наушники подключены",
        _ => "Наушники не подключены (USB-C)",
    };
    let route = Dropdown::new()
        .width(220.0)
        .item(DropdownItem::new("auto", "Автоматически"))
        .item(DropdownItem::new("speaker", "Всегда динамики"))
        .item(DropdownItem::new("headphones", "Наушники"))
        .selected(p.route.as_str())
        .on_change(|v: &str| {
            let r = match v {
                "speaker" => Route::Speaker,
                "headphones" => Route::Headphones,
                _ => Route::Auto,
            };
            act(move || {
                sound::set_route(r);
            });
        });
    let mut rows = vec![row("Куда выводить", &format!("Сейчас: {out}. {jack}"), route)];
    if let Some(on) = p.dolby {
        rows.push(row_inline(
            "Dolby Atmos",
            "Обработка Dolby в сигнальном процессоре: объём, диалоги, выравнивание громкости, бас",
            Toggle::with_state(on).on_change(|on| {
                act(move || {
                    sound::set_dolby(on);
                });
                state::toast(if on { "Dolby Atmos включён" } else { "Dolby Atmos выключен" });
            }),
        ));
        if on && !profiles.is_empty() {
            let mut dd = Dropdown::new().width(220.0).item(DropdownItem::new("-1", "По умолчанию"));
            for pr in profiles {
                dd = dd.item(DropdownItem::new(pr.id.to_string(), pr.title()));
            }
            rows.push(row(
                "Профиль Dolby",
                "Настройки звука Xiaomi для динамиков и наушников",
                dd.selected(p.dolby_profile.to_string()).on_change(|v: &str| {
                    let id: i32 = v.parse().unwrap_or(-1);
                    act(move || {
                        sound::set_dolby_profile(id);
                    });
                }),
            ));
        }
    }
    group("Телефон", rows)
}

pub fn audio() -> W {
    let s = sig();
    refresh(s);
    let body = Reactive::new(move || -> Vec<W> {
        let Some(snap) = s.get() else { return vec![note("Чтение состояния звука…")] };
        let Some(nodes) = snap.nodes else {
            return vec![note("PipeWire не отвечает: звук сеанса не запущен (pipewire, wireplumber).")];
        };
        let max = snap.max;
        let mut col = Column::new().gap(18.0);
        let pick = |k: NodeKind| nodes.iter().filter(move |n| n.kind == k);
        let outs: Vec<W> = pick(NodeKind::Output).map(|n| node_row(n, max, true)).collect();
        col = col.child(group("Вывод", if outs.is_empty() { vec![note("Нет устройств вывода.")] } else { outs }));
        if let Some(p) = &snap.phone {
            col = col.child(phone_group(p, &snap.profiles));
        }
        let ins: Vec<W> = pick(NodeKind::Input).map(|n| node_row(n, max, true)).collect();
        col = col.child(group("Ввод", if ins.is_empty() { vec![note("Нет микрофонов.")] } else { ins }));
        let apps: Vec<W> = pick(NodeKind::AppStream).map(|n| node_row(n, max, false)).collect();
        if !apps.is_empty() {
            col = col.child(group("Программы", apps));
        }
        col = col.child(group(
            "",
            vec![switch_row(
                "Громкость выше 100 %",
                "Усиление до 150 % для тихих записей и музыки; громкие звуки при этом искажаются",
                op!["sound", "overamplify"],
                max > 100,
            )],
        ));
        vec![boxed(col)]
    });
    page("Звук", "Динамики, наушники, микрофон, громкость программ, Dolby Atmos.", vec![boxed(body)])
}
