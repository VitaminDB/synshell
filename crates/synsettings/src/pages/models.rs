use std::sync::atomic::{AtomicBool, Ordering};

use syngui::prelude::*;
use synaptixd::{Client, DeviceKind, LoadState, ModelEntry, ModelKind, SettingsChange, Status};

use crate::state;
use crate::ui::*;

#[derive(Clone, PartialEq, Default)]
struct Snapshot {
    status: Option<Status>,
    models: Vec<ModelEntry>,
    error: Option<String>,
}

thread_local! {
    static LIVE: std::cell::Cell<Option<RwSignal<Snapshot>>> = const { std::cell::Cell::new(None) };
}

static ACTIVE: AtomicBool = AtomicBool::new(false);

fn fetch() -> Snapshot {
    let r = Client::connect().and_then(|mut c| Ok((c.status()?, c.list()?)));
    match r {
        Ok((status, models)) => Snapshot { status: Some(status), models, error: None },
        Err(e) => Snapshot { error: Some(e.to_string()), ..Default::default() },
    }
}

fn apply(sig: RwSignal<Snapshot>, s: Snapshot) {
    if sig.get_untracked() != s {
        sig.set(s);
    }
}

fn live() -> RwSignal<Snapshot> {
    LIVE.with(|l| match l.get() {
        Some(v) => v,
        None => {
            let v = use_signal(fetch());
            l.set(Some(v));
            std::thread::Builder::new()
                .name("synaptixd".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    syngui::async_runtime::run_on_main_thread(|| {
                        ACTIVE.store(state::ctx().page.get_untracked() == "models", Ordering::Relaxed);
                    });
                    if ACTIVE.load(Ordering::Relaxed) {
                        let s = fetch();
                        syngui::async_runtime::run_on_main_thread(move || apply(v, s));
                    }
                })
                .ok();
            v
        }
    })
}

fn act(sig: RwSignal<Snapshot>, f: impl FnOnce(&mut Client) -> synaptixd::Result<()> + Send + 'static) {
    std::thread::spawn(move || {
        let r = Client::connect().and_then(|mut c| f(&mut c));
        let s = fetch();
        syngui::async_runtime::run_on_main_thread(move || {
            if let Err(e) = r {
                state::toast(e.to_string());
            }
            apply(sig, s);
        });
    });
}

fn mb(b: u64) -> String {
    if b >= 1 << 30 {
        t!("{v} ГБ", v = format!("{:.1}", b as f64 / (1u64 << 30) as f64))
    } else {
        t!("{v} МБ", v = b >> 20)
    }
}

fn device_label(d: DeviceKind) -> String {
    match d {
        DeviceKind::Auto => t!("Авто"),
        DeviceKind::Cpu => "CPU".into(),
        DeviceKind::Cuda => "CUDA".into(),
        DeviceKind::Gpu => t!("GPU (Vulkan)"),
        DeviceKind::Npu => "NPU".into(),
    }
}

fn kind_label(k: ModelKind) -> String {
    match k {
        ModelKind::Ctc => t!("Распознавание звуков (CTC)"),
        ModelKind::Mfa => t!("Выравнивание речи (MFA)"),
        ModelKind::Asr => t!("Распознавание речи"),
        ModelKind::Llm => t!("Языковая модель"),
        ModelKind::Tts => t!("Синтез речи"),
        ModelKind::Embed => t!("Эмбеддинги"),
    }
}

fn state_text(m: &ModelEntry) -> String {
    match m.state {
        LoadState::Loaded => {
            let dev = m.device.map(device_label).unwrap_or_default();
            let mut s = t!("Загружена: {dev}, {mem}", dev = dev, mem = mb(m.mem_bytes));
            if m.vram_bytes > 0 {
                s.push_str(&t!(" + видеопамять {v}", v = mb(m.vram_bytes)));
            }
            if !m.precision.is_empty() {
                s.push_str(&format!(" · {}", tl(synaptixd::precision_label(&m.precision))));
            }
            if m.busy {
                s.push_str(&t!(" · работает"));
            }
            if m.clients > 0 {
                s.push_str(&t!(" · программ: {n}", n = m.clients));
            }
            s
        }
        LoadState::Loading => t!("Загружается…"),
        LoadState::Failed => t!("Ошибка: {e}", e = m.error.clone().unwrap_or_default()),
        LoadState::Unloaded => t!("Не загружена — загрузится при первом обращении программы"),
    }
}

const IDLE: &[(i64, &str)] = &[
    (-1, n_!("По умолчанию")),
    (0, n_!("Только при нехватке памяти")),
    (2, n_!("2 минуты")),
    (5, n_!("5 минут")),
    (10, n_!("10 минут")),
    (30, n_!("30 минут")),
    (60, n_!("1 час")),
];

fn model_card(sig: RwSignal<Snapshot>, m: &ModelEntry, devices: &[DeviceKind]) -> W {
    let id = m.id.clone();
    let mut rows: Vec<W> = Vec::new();
    let mut hint = kind_label(m.kind);
    if !m.languages.is_empty() {
        hint.push_str(&format!(" · {}", m.languages.join(", ")));
    }
    hint.push_str(&format!(" · {} · {}", m.package, mb(m.disk_bytes)));
    if m.npu_cache {
        hint.push_str(&t!(" · графы NPU предсобраны"));
    }
    if !m.description.is_empty() {
        hint = format!("{} · {hint}", m.description);
    }
    rows.push(row_inline(state_text(m), hint, Text::new("")));
    let quant: Vec<String> = m
        .variants
        .iter()
        .map(|v| {
            let devs: Vec<String> = v.devices.iter().map(|d| device_label(*d)).collect();
            format!("{}: {} · {}", devs.join(", "), tl(synaptixd::precision_label(&v.precision)), mb(v.bytes))
        })
        .collect();
    if !quant.is_empty() {
        rows.push(row_inline(t!("Квантование"), quant.join("\n"), Text::new("")));
    }

    let mut dd = Dropdown::new().width(220.0);
    let mut choices = vec![DeviceKind::Auto];
    choices.extend(DeviceKind::ALL.iter().copied().filter(|d| *d != DeviceKind::Auto && m.devices.contains(d)));
    for d in choices {
        let mut label = device_label(d);
        if d != DeviceKind::Auto && !devices.contains(&d) {
            label.push_str(&t!(" — недоступно"));
        }
        dd = dd.item(DropdownItem::new(d.id(), label));
    }
    let i = id.clone();
    let dd = dd.selected(m.settings.device.id().to_string()).on_change(move |v: &str| {
        if let Some(d) = DeviceKind::parse(v) {
            let i = i.clone();
            act(sig, move |c| c.set(&i, SettingsChange { device: Some(d), ..Default::default() }).map(|_| ()));
        }
    });
    let dev_hint = if m.fallback {
        t!("Выбранное устройство не справилось — модель работает на CPU")
    } else {
        t!("Авто: CUDA, затем NPU, затем CPU")
    };
    rows.push(row(t!("Тип выполнения"), dev_hint, dd));

    let i = id.clone();
    rows.push(row_inline(
        t!("Не выгружать"),
        t!("Держать в памяти, даже если её не хватает другим"),
        Toggle::with_state(m.settings.pinned).on_change(move |v| {
            let i = i.clone();
            act(sig, move |c| c.set(&i, SettingsChange { pinned: Some(v), ..Default::default() }).map(|_| ()));
        }),
    ));
    let i = id.clone();
    rows.push(row_inline(
        t!("Загружать при запуске"),
        t!("Модель готова сразу, без задержки при первом обращении"),
        Toggle::with_state(m.settings.preload).on_change(move |v| {
            let i = i.clone();
            act(sig, move |c| c.set(&i, SettingsChange { preload: Some(v), ..Default::default() }).map(|_| ()));
        }),
    ));
    let mut idle = Dropdown::new().width(220.0);
    for (v, l) in IDLE {
        idle = idle.item(DropdownItem::new(v.to_string(), tl(l)));
    }
    let i = id.clone();
    let cur = m.settings.idle_unload_min.map(|v| v as i64).unwrap_or(-1);
    rows.push(row(
        t!("Выгружать при простое"),
        t!("Неиспользуемая модель освобождает память"),
        idle.selected(cur.to_string()).on_change(move |v: &str| {
            let Ok(n) = v.parse::<i64>() else { return };
            let i = i.clone();
            let value = if n < 0 { None } else { Some(n as u32) };
            act(sig, move |c| c.set(&i, SettingsChange { idle_unload_min: Some(value), ..Default::default() }).map(|_| ()));
        }),
    ));
    let (i1, i2) = (id.clone(), id.clone());
    let buttons = Row::new()
        .gap(8.0)
        .child(button(t!("Загрузить"), move || {
            let i = i1.clone();
            act(sig, move |c| c.load(&i).map(|_| ()));
        }))
        .child(button(t!("Выгрузить"), move || {
            let i = i2.clone();
            act(sig, move |c| c.unload(&i));
        }));
    rows.push(row_inline(t!("Память"), "", buttons));
    group(m.title.clone(), rows)
}

pub fn models() -> W {
    let sig = live();
    let body = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        let mut out: Vec<W> = Vec::new();
        let Some(st) = snap.status.as_ref() else {
            out.push(note(t!(
                "Служба нейросетей synaptixd не отвечает ({e}). Включите её: systemctl enable --now synaptixd",
                e = snap.error.clone().unwrap_or_default()
            )));
            return vec![boxed(Column::new().children(out))];
        };
        let devs: Vec<String> = st.devices.iter().map(|d| device_label(*d)).collect();
        out.push(group(
            t!("Служба"),
            vec![
                row_inline(
                    t!("Память"),
                    t!("Свободно {a} из {b}; запас {c} МБ — меньше выгружаются неиспользуемые", a = mb(st.mem_available), b = mb(st.mem_total), c = st.min_available_mb),
                    Text::new(t!("моделей {n}, {m}", n = st.loaded, m = mb(st.models_bytes))).class("row-value"),
                ),
                row_inline(t!("Устройства"), t!("Где служба может исполнять модели"), Text::new(devs.join(", ")).class("row-value")),
                row_inline(
                    t!("Модели"),
                    t!("Ставятся пакетами synaptix-model-* в /usr/share/synaptix/models"),
                    button(t!("Обновить список"), move || act(sig, |c| c.call(synaptixd::Request::Rescan, &[]).map(|_| ()))),
                ),
            ],
        ));
        if snap.models.is_empty() {
            out.push(note(t!("Модели не установлены. Найдите пакеты synaptix-model в «Программах».")));
        }
        for m in &snap.models {
            out.push(model_card(sig, m, &st.devices));
        }
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(out))]
    });
    page(
        t!("Нейросети"),
        t!("Системные модели: общие для всех программ, загружаются по требованию и выгружаются при нехватке памяти."),
        vec![boxed(body)],
    )
}
