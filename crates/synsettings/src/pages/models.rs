use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use syngui::prelude::*;
use synaptixd::{
    Client, DefaultModel, DeviceKind, EngineProfile, LibraryInfo, LoadState, ModelEntry, ModelKind, ModelSettings, ParamKind, ParamSpec, ParamValue, Params, QuantOption,
    SettingsChange, Status,
};

use crate::state;
use crate::ui::*;

#[derive(Clone, PartialEq, Default)]
struct Snapshot {
    status: Option<Status>,
    models: Vec<ModelEntry>,
    defaults: Vec<DefaultModel>,
    libraries: Vec<LibraryInfo>,
    error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct Cfg {
    devices: Vec<DeviceKind>,
    avail: Vec<DeviceKind>,
    settings: ModelSettings,
    quant: Vec<QuantOption>,
    fallback: bool,
    /// Действующий профиль движка (у языковых моделей).
    engine: Option<EngineProfile>,
}

#[derive(Clone, PartialEq)]
struct Choices {
    defaults: Vec<DefaultModel>,
    models: Vec<(String, String, ModelKind)>,
}

#[derive(Clone, PartialEq)]
struct ParamState {
    schema: Vec<ParamSpec>,
    optimal: Params,
    own: Params,
}

thread_local! {
    static LIVE: std::cell::Cell<Option<RwSignal<Snapshot>>> = const { std::cell::Cell::new(None) };
    static CHOICES: std::cell::Cell<Option<RwSignal<Option<Choices>>>> = const { std::cell::Cell::new(None) };
    static CFG: RefCell<HashMap<String, RwSignal<Option<Cfg>>>> = RefCell::new(HashMap::new());
    static PARAMS: RefCell<HashMap<String, (RwSignal<u64>, Option<ParamState>)>> = RefCell::new(HashMap::new());
}

static ACTIVE: AtomicBool = AtomicBool::new(false);

const SUB: &str = "models/";

fn fetch() -> Snapshot {
    let r = Client::connect().and_then(|mut c| Ok((c.status()?, c.list()?, c.defaults()?, c.libraries().unwrap_or_default())));
    match r {
        Ok((status, models, defaults, libraries)) => Snapshot { status: Some(status), models, defaults, libraries, error: None },
        Err(e) => Snapshot { error: Some(e.to_string()), ..Default::default() },
    }
}

fn cfg_signal(id: &str) -> RwSignal<Option<Cfg>> {
    CFG.with(|m| *m.borrow_mut().entry(id.to_string()).or_insert_with(|| use_signal(None)))
}

fn choices_signal() -> RwSignal<Option<Choices>> {
    CHOICES.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            c.set(Some(s));
            s
        }
    })
}

fn params_rev(id: &str) -> RwSignal<u64> {
    PARAMS.with(|m| m.borrow_mut().entry(id.to_string()).or_insert_with(|| (use_signal(0u64), None)).0)
}

fn params_state(id: &str) -> Option<ParamState> {
    PARAMS.with(|m| m.borrow().get(id).and_then(|(_, s)| s.clone()))
}

fn set_params_state(id: &str, st: ParamState, bump: bool) {
    let rev = params_rev(id);
    PARAMS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(id) {
            e.1 = Some(st);
        }
    });
    if bump {
        rev.set(rev.get_untracked() + 1);
    }
}

fn apply(sig: RwSignal<Snapshot>, s: Snapshot) {
    let avail = s.status.as_ref().map(|st| st.devices.clone()).unwrap_or_default();
    for m in &s.models {
        let cfg = Cfg {
            devices: m.devices.clone(),
            avail: avail.clone(),
            settings: ModelSettings { params: Params::default(), ..m.settings.clone() },
            quant: m.quant_options.clone(),
            fallback: m.fallback,
            engine: m.engine.clone(),
        };
        let c = cfg_signal(&m.id);
        if c.get_untracked().as_ref() != Some(&cfg) {
            c.set(Some(cfg));
        }
        let st = ParamState { schema: m.schema.clone(), optimal: m.defaults.clone(), own: m.settings.params.clone() };
        params_rev(&m.id);
        if params_state(&m.id).as_ref() != Some(&st) {
            set_params_state(&m.id, st, true);
        }
    }
    if s.status.is_some() {
        let choices = Choices { defaults: s.defaults.clone(), models: s.models.iter().map(|m| (m.id.clone(), m.title.clone(), m.kind)).collect() };
        let c = choices_signal();
        if c.get_untracked().as_ref() != Some(&choices) {
            c.set(Some(choices));
        }
    }
    if sig.get_untracked() != s {
        sig.set(s);
    }
}

fn live() -> RwSignal<Snapshot> {
    LIVE.with(|l| match l.get() {
        Some(v) => v,
        None => {
            let v = use_signal(Snapshot::default());
            l.set(Some(v));
            apply(v, fetch());
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

fn change(sig: RwSignal<Snapshot>, id: &str, ch: SettingsChange) {
    let id = id.to_string();
    act(sig, move |c| c.set(&id, ch).map(|_| ()));
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
        ModelKind::Image => t!("Генерация картинок"),
        ModelKind::Video => t!("Генерация видео"),
        ModelKind::Music => t!("Генерация музыки"),
        ModelKind::Diarize => t!("Разметка говорящих"),
        ModelKind::Rerank => t!("Переранжирование"),
    }
}

fn group_label(k: ModelKind) -> String {
    match k {
        ModelKind::Llm => t!("Языковые модели"),
        ModelKind::Image => t!("Картинки"),
        ModelKind::Video => t!("Видео"),
        ModelKind::Music => t!("Музыка"),
        ModelKind::Tts => t!("Синтез речи"),
        ModelKind::Asr => t!("Распознавание речи"),
        ModelKind::Embed => t!("Эмбеддинги"),
        ModelKind::Ctc => t!("Распознавание звуков (CTC)"),
        ModelKind::Mfa => t!("Выравнивание речи (MFA)"),
        ModelKind::Diarize => t!("Разметка говорящих"),
        ModelKind::Rerank => t!("Переранжирование"),
    }
}

fn task_label(k: ModelKind) -> String {
    match k {
        ModelKind::Llm => t!("Чат и текст"),
        ModelKind::Image => t!("Картинки"),
        ModelKind::Video => t!("Видео"),
        ModelKind::Music => t!("Музыка"),
        ModelKind::Tts => t!("Синтез речи"),
        ModelKind::Asr => t!("Распознавание речи"),
        ModelKind::Embed => t!("Поиск по смыслу (эмбеддинги)"),
        other => kind_label(other),
    }
}

fn task_hint(k: ModelKind) -> String {
    match k {
        ModelKind::Llm => t!("Ассистент, ответы, обработка текста"),
        ModelKind::Image => t!("SDXL, FLUX, Qwen-Image…"),
        ModelKind::Video => t!("LTX, MiniMax H3…"),
        ModelKind::Music => t!("ACE-Step, YuE…"),
        ModelKind::Tts => t!("Озвучка текста"),
        ModelKind::Asr => t!("Голосовой ввод и расшифровка"),
        ModelKind::Embed => t!("Поиск файлов и заметок по смыслу"),
        _ => String::new(),
    }
}

fn kind_icon(k: ModelKind) -> &'static str {
    match k {
        ModelKind::Llm => "\u{e0b7}",
        ModelKind::Image => "\u{e3f4}",
        ModelKind::Video => "\u{e04b}",
        ModelKind::Music => "\u{e405}",
        ModelKind::Tts => "\u{e050}",
        ModelKind::Asr => "\u{e029}",
        ModelKind::Embed => "\u{e8b6}",
        ModelKind::Ctc | ModelKind::Mfa => "\u{e1b8}",
        ModelKind::Diarize => "\u{e7ef}",
        ModelKind::Rerank => "\u{e164}",
    }
}

const SCHEMA_TEXT: &[&str] = &[
    n_!("Стартовый промпт"),
    n_!("Системное сообщение в начале диалога, если программа не задала своё"),
    n_!("Длина ответа, токенов"),
    n_!("Предел длины одного ответа"),
    n_!("Температура"),
    n_!("Выше — разнообразнее, ниже — точнее"),
    n_!("Доля вероятности, из которой выбирается слово"),
    n_!("Из скольких самых вероятных слов выбирать; 0 — без ограничения"),
    n_!("Отсекать слова вероятностью ниже этой доли от лучшего"),
    n_!("Штраф за повторы"),
    n_!("1 — без штрафа"),
    n_!("Штраф за присутствие"),
    n_!("Против повторов уже сказанного; 0 — без штрафа"),
    n_!("Размышления"),
    n_!("Модель сначала рассуждает, потом отвечает — точнее и дольше"),
    n_!("Глубина размышлений"),
    n_!("Сколько модель думает перед ответом"),
    n_!("Зерно (seed)"),
    n_!("−1 — случайное; одно зерно даёт повторяемый результат"),
    n_!("Негативный промпт"),
    n_!("Чего не должно быть в результате"),
    n_!("Шаги"),
    n_!("Больше — качественнее и дольше"),
    n_!("Шаги диффузии"),
    n_!("Шаги маскированного декода; авто — 16 на NPU, 32 на остальных"),
    n_!("Насколько строго следовать промпту"),
    n_!("Ширина"),
    n_!("Высота"),
    n_!("В пикселях"),
    n_!("Длительность, с"),
    n_!("Длина результата в секундах"),
    n_!("Кадров в секунду"),
    n_!("Частота кадров видео"),
    n_!("Язык"),
    n_!("Код языка (ru, en, kk); пусто — определять по тексту"),
    n_!("Код языка (ru, en, kk); пусто — определять самой"),
    n_!("Голос"),
    n_!("Описание голоса: пол, возраст, манера"),
    n_!("Скорость речи"),
    n_!("1 — обычная"),
    n_!("Текст песни"),
    n_!("Пусто — инструментал"),
    n_!("Куплеты и припевы"),
];

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
        LoadState::Unloaded if !m.runnable => t!("Служба пока не исполняет модели этого вида"),
        LoadState::Unloaded => t!("Не загружена — загрузится при первом обращении программы"),
    }
}

fn short_state(m: &ModelEntry) -> String {
    match m.state {
        LoadState::Loaded => {
            let dev = m.device.map(device_label).unwrap_or_default();
            let mut s = t!("Загружена · {dev} · {mem}", dev = dev, mem = mb(m.mem_bytes));
            if m.busy {
                s.push_str(&t!(" · работает"));
            }
            s
        }
        LoadState::Loading => t!("Загружается…"),
        LoadState::Failed => t!("Ошибка: {e}", e = m.error.clone().unwrap_or_default()),
        LoadState::Unloaded if !m.runnable => t!("Пока не исполняется службой"),
        LoadState::Unloaded => t!("Не загружена · {v} на диске", v = mb(m.disk_bytes)),
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

const KV_KEEP: &[(i64, &str)] = &[
    (-1, n_!("По умолчанию")),
    (0, n_!("Не держать")),
    (10, n_!("10 минут")),
    (30, n_!("30 минут")),
    (60, n_!("1 час")),
    (180, n_!("3 часа")),
    (720, n_!("12 часов")),
];

fn nav_item(ctx: state::Ctx, key: String, title: String, summary: String, icon: &'static str, fault: bool) -> W {
    let t = title.clone();
    boxed(
        syngui::GestureDetector::new().on_click(move || ctx.sub.set(Some((format!("{SUB}{key}"), t.clone())))).child(
            DecoratedBox::new().class("phone-nav-item").child(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        DecoratedBox::new()
                            .class(if fault { "hw-badge hw-badge-fault" } else { "hw-badge" })
                            .child(Icon::new(if fault { "\u{E002}" } else { icon }).class(if fault { "hw-icon hw-icon-fault" } else { "hw-icon" })),
                    )
                    .child(
                        Column::new()
                            .gap(2.0)
                            .class("grow")
                            .child(Text::new(title).class("phone-nav-label"))
                            .child(Text::new(summary).max_lines(1).class(if fault { "hw-sum hw-sum-fault" } else { "hw-sum" })),
                    )
                    .child(Icon::new(icons::CHEVRON_RIGHT).class("phone-nav-chevron")),
            ),
        ),
    )
}

fn card(items: Vec<W>) -> W {
    let mut card = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
    for (n, w) in items.into_iter().enumerate() {
        if n > 0 {
            card = card.child(DecoratedBox::new().class("row-sep"));
        }
        card = card.child(w);
    }
    boxed(card)
}

fn titled(title: String, body: W) -> W {
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("group-title")).child(body))
}

fn title_of(snap: &Snapshot, id: &str) -> String {
    snap.models.iter().find(|m| m.id == id).map(|m| m.title.clone()).unwrap_or_else(|| id.to_string())
}

fn menu(ctx: state::Ctx, sig: RwSignal<Snapshot>) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        let Some(st) = snap.status.as_ref() else {
            return vec![note(t!(
                "Служба нейросетей synaptixd не отвечает ({e}). Включите её: systemctl enable --now synaptixd",
                e = snap.error.clone().unwrap_or_default()
            ))];
        };
        let mut out: Vec<W> = Vec::new();
        let active: Vec<W> = snap
            .models
            .iter()
            .filter(|m| matches!(m.state, LoadState::Loaded | LoadState::Loading))
            .map(|m| {
                let id = m.id.clone();
                let mut hint = short_state(m);
                if m.vram_bytes > 0 {
                    hint.push_str(&t!(" · видеопамять {v}", v = mb(m.vram_bytes)));
                }
                let control: W = if m.state == LoadState::Loaded {
                    boxed(button(t!("Выгрузить"), move || {
                        let i = id.clone();
                        act(sig, move |c| c.unload(&i));
                    }))
                } else {
                    boxed(Text::new(""))
                };
                row_inline(m.title.clone(), hint, control)
            })
            .collect();
        if !active.is_empty() {
            out.push(titled(t!("Загруженные модели"), card(active)));
        }
        let picked: Vec<String> = snap
            .defaults
            .iter()
            .filter_map(|d| d.resolved.as_ref().map(|id| format!("{}: {}", task_label(d.kind), title_of(&snap, id))))
            .collect();
        let summary = if picked.is_empty() { t!("Модели не установлены") } else { picked.join(" · ") };
        out.push(card(vec![
            nav_item(ctx, "defaults".into(), t!("Модели по умолчанию"), summary, "\u{e838}", false),
            nav_item(
                ctx,
                "storage".into(),
                t!("Хранилище моделей"),
                if snap.libraries.is_empty() {
                    t!("Библиотек нет — добавьте папку для моделей")
                } else {
                    let used: u64 = snap.libraries.iter().map(|l| l.used).sum();
                    let models: u32 = snap.libraries.iter().map(|l| l.models).sum();
                    t!("Библиотек: {n} · моделей: {m} · {u}", n = snap.libraries.len(), m = models, u = mb(used))
                },
                "\u{E1DB}",
                snap.libraries.iter().any(|l| !l.online),
            ),
            nav_item(
                ctx,
                "service".into(),
                t!("Служба и память"),
                t!("Свободно {a} из {b} · загружено моделей: {n}", a = mb(st.mem_available), b = mb(st.mem_total), n = st.loaded),
                "\u{e322}",
                false,
            ),
        ]));
        if snap.models.is_empty() {
            out.push(note(t!("Моделей нет. Добавьте библиотеку и импортируйте модели в «Хранилище моделей».")));
        }
        let cats: Vec<W> = ModelKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let ms: Vec<&ModelEntry> = snap.models.iter().filter(|m| m.kind == kind).collect();
                if ms.is_empty() {
                    return None;
                }
                let loaded = ms.iter().filter(|m| m.state == LoadState::Loaded).count();
                let mut sum = t!("Моделей: {n}", n = ms.len());
                if loaded > 0 {
                    sum.push_str(&t!(" · загружено: {n}", n = loaded));
                }
                if let Some(id) = snap.defaults.iter().find(|d| d.kind == kind).and_then(|d| d.resolved.as_ref()) {
                    sum.push_str(&t!(" · по умолчанию {m}", m = title_of(&snap, id)));
                }
                let fault = ms.iter().any(|m| m.state == LoadState::Failed);
                Some(nav_item(ctx, format!("k/{}", kind.id()), group_label(kind), sum, kind_icon(kind), fault))
            })
            .collect();
        if !cats.is_empty() {
            out.push(titled(t!("Модели"), card(cats)));
        }
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(out))]
    });
    let actions = Row::new()
        .gap(8.0)
        .child(button(t!("Обновить список моделей"), move || act(sig, |c| c.call(synaptixd::Request::Rescan, &[]).map(|_| ()))));
    page(
        t!("Нейросети"),
        t!("Системные модели: общие для всех программ, загружаются по требованию и выгружаются при нехватке памяти."),
        vec![boxed(actions), boxed(body)],
    )
}

fn back(ctx: state::Ctx, to: Option<(String, String)>) -> Option<W> {
    let label = match &to {
        Some((_, t)) => format!("‹ {t}"),
        None => t!("‹ Нейросети"),
    };
    (!narrow()).then(|| boxed(Row::new().child(button(label, move || ctx.sub.set(to.clone())))))
}

fn sub_page(ctx: state::Ctx, title: String, subtitle: String, body: Vec<W>) -> W {
    sub_page_in(ctx, None, title, subtitle, body)
}

fn sub_page_in(ctx: state::Ctx, parent: Option<(String, String)>, title: String, subtitle: String, body: Vec<W>) -> W {
    let mut parts: Vec<W> = back(ctx, parent).into_iter().collect();
    parts.extend(body);
    page(title, subtitle, parts)
}

fn kind_key(kind: ModelKind) -> (String, String) {
    (format!("{SUB}k/{}", kind.id()), group_label(kind))
}

pub fn sub_parent(key: &str) -> Option<(String, String)> {
    let id = key.strip_prefix(SUB)?.strip_prefix("m/")?;
    let sig = LIVE.with(|l| l.get())?;
    sig.get_untracked().models.iter().find(|m| m.id == id).map(|m| kind_key(m.kind))
}

fn kind_page(ctx: state::Ctx, sig: RwSignal<Snapshot>, kind: ModelKind) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        if snap.status.is_none() {
            return vec![note(t!("Служба нейросетей не отвечает"))];
        }
        let items: Vec<W> = snap
            .models
            .iter()
            .filter(|m| m.kind == kind)
            .map(|m| nav_item(ctx, format!("m/{}", m.id), m.title.clone(), short_state(m), kind_icon(kind), m.state == LoadState::Failed))
            .collect();
        if items.is_empty() {
            return vec![note(t!("Моделей этого вида нет"))];
        }
        vec![card(items)]
    });
    sub_page(ctx, group_label(kind), task_hint(kind), vec![boxed(body)])
}

fn task_row(kind: ModelKind, control: W, open: Option<W>) -> W {
    let badge = DecoratedBox::new().class("hw-badge").child(Icon::new(kind_icon(kind)).class("hw-icon"));
    let text = Column::new()
        .gap(2.0)
        .class("grow")
        .child(Text::new(task_label(kind)).class("row-label"))
        .child(Text::new(task_hint(kind)).class("row-hint"));
    let mut controls = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(control);
    if let Some(o) = open {
        controls = controls.child(o);
    }
    let head = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(badge).child(text);
    if narrow() {
        return boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("setting-row-wide").child(head).child(controls));
    }
    boxed(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).class("setting-row").child(head.class("grow")).child(controls))
}

fn defaults_page(ctx: state::Ctx, sig: RwSignal<Snapshot>) -> W {
    let choices = choices_signal();
    let body = Reactive::new(move || -> Vec<W> {
        let Some(ch) = choices.get() else { return vec![note(t!("Служба нейросетей не отвечает"))] };
        let mut rows: Vec<W> = Vec::new();
        for d in &ch.defaults {
            let kind = d.kind;
            let models: Vec<&(String, String, ModelKind)> = ch.models.iter().filter(|m| m.2 == kind).collect();
            let title = |id: &str| ch.models.iter().find(|m| m.0 == id).map(|m| m.1.clone()).unwrap_or_else(|| id.to_string());
            if models.is_empty() {
                rows.push(task_row(kind, boxed(Text::new(t!("Нет установленных моделей")).class("row-value")), None));
                continue;
            }
            let auto = match (&d.model, &d.resolved) {
                (None, Some(r)) => t!("Авто — {m}", m = title(r)),
                _ => t!("Авто"),
            };
            let mut dd = Dropdown::new().width(280.0).item(DropdownItem::new("", auto));
            for (id, t, _) in &models {
                dd = dd.item(DropdownItem::new(id.clone(), t.clone()));
            }
            let dd = dd.selected(d.model.clone().unwrap_or_default()).on_change(move |v: &str| {
                let model = (!v.is_empty()).then(|| v.to_string());
                act(sig, move |c| c.set_default(kind, model.as_deref()).map(|_| ()));
            });
            let open = d.resolved.clone().map(|id| {
                let t = title(&id);
                boxed(
                    Button::new("")
                        .icon(icons::TUNE)
                        .class("icon-btn")
                        .on_click(move || ctx.sub.set(Some((format!("{SUB}m/{id}"), t.clone())))),
                )
            });
            rows.push(task_row(kind, boxed(dd), open));
        }
        vec![group(t!("Модель для задачи"), rows)]
    });
    sub_page(
        ctx,
        t!("Модели по умолчанию"),
        t!("Их берут программы и ассистент, если не выбрали модель сами. «Авто» — уже загруженная или лучшая для устройств этой машины. Кнопка справа открывает настройки выбранной модели."),
        vec![boxed(body)],
    )
}

fn service_page(ctx: state::Ctx, sig: RwSignal<Snapshot>) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        let Some(st) = snap.status.as_ref() else { return vec![note(t!("Служба нейросетей не отвечает"))] };
        let devs: Vec<String> = st.devices.iter().map(|d| device_label(*d)).collect();
        vec![group(
            t!("Служба"),
            vec![
                row_inline(
                    t!("Память"),
                    t!("Свободно {a} из {b}; запас {c} МБ — меньше выгружаются неиспользуемые", a = mb(st.mem_available), b = mb(st.mem_total), c = st.min_available_mb),
                    Text::new(t!("моделей {n}, {m}", n = st.loaded, m = mb(st.models_bytes))).class("row-value"),
                ),
                row_inline(t!("Устройства"), t!("Где служба может исполнять модели"), Text::new(devs.join(", ")).class("row-value")),
                row_inline(t!("Выгружено при нехватке"), t!("С запуска службы"), Text::new(st.evictions.to_string()).class("row-value")),
                row_inline(
                    t!("Модели"),
                    t!("Хранятся в библиотеках — «Хранилище моделей»"),
                    button(t!("Обновить список"), move || act(sig, |c| c.call(synaptixd::Request::Rescan, &[]).map(|_| ()))),
                ),
            ],
        )]
    });
    sub_page(ctx, t!("Служба и память"), String::new(), vec![boxed(body)])
}

thread_local! {
    static NEW_LIB: RefCell<String> = RefCell::new(String::new());
}

fn usage_bar(used: u64, total: u64) -> W {
    let v = if total > 0 { used as f32 / total as f32 } else { 0.0 };
    boxed(DecoratedBox::new().class("lib-bar-wrap").child(ProgressBar::with_value(v).class("lib-bar")))
}

fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn library_rows(sig: RwSignal<Snapshot>, libs: &[LibraryInfo]) -> Vec<W> {
    let mut rows: Vec<W> = Vec::new();
    for l in libs {
        let hint = if l.online {
            t!("{p} · моделей: {n} · занято {u} · свободно {f} из {t}", p = l.path.clone(), n = l.models, u = mb(l.used), f = mb(l.free), t = mb(l.total))
        } else {
            t!("{p} · диск не подключён — модели отсюда недоступны", p = l.path.clone())
        };
        let mut actions = Row::new().gap(8.0);
        let po = l.path.clone();
        actions = actions.child(button(t!("Открыть папку"), move || open_folder(&po)));
        if l.default {
            actions = actions.child(Text::new(t!("Основная")).class("row-value"));
        } else {
            let p = l.path.clone();
            actions = actions.child(button(t!("Сделать основной"), move || {
                let p = p.clone();
                act(sig, move |c| c.call(synaptixd::Request::SetDefaultLibrary { path: p }, &[]).map(|_| ()));
            }));
        }
        let p = l.path.clone();
        actions = actions.child(button(t!("Убрать"), move || {
            let p = p.clone();
            act(sig, move |c| c.call(synaptixd::Request::RemoveLibrary { path: p }, &[]).map(|_| ()));
        }));
        rows.push(boxed(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(row_wide(l.name.clone(), hint, actions))
                .child(usage_bar(l.total.saturating_sub(l.free), l.total)),
        ));
    }
    rows
}

fn storage_page(ctx: state::Ctx, sig: RwSignal<Snapshot>) -> W {
    let libs_view = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        if snap.libraries.is_empty() {
            return vec![note(t!("Библиотека — обычная папка с моделями. Добавьте её ниже."))];
        }
        vec![group(t!("Библиотеки"), library_rows(sig, &snap.libraries))]
    });
    let add_field = TextField::with_text(NEW_LIB.with(|v| v.borrow().clone()))
        .placeholder(t!("/путь/к/папке"))
        .width(320.0)
        .on_change(|s| NEW_LIB.with(|v| *v.borrow_mut() = s.to_string()));
    let add = group(
        t!("Добавить библиотеку"),
        vec![row_wide(
            t!("Папка"),
            t!("В ней будут созданы папки категорий; лучше быстрый диск — модели читаются при загрузке"),
            Row::new().gap(8.0).child(add_field).child(button(t!("Добавить"), move || {
                let path = NEW_LIB.with(|v| v.borrow().trim().to_string());
                if path.is_empty() {
                    return;
                }
                act(sig, move |c| c.call(synaptixd::Request::AddLibrary { path, name: None }, &[]).map(|_| ()));
            })),
        )],
    );
    let how = group(
        t!("Как добавить модель"),
        vec![
            row_wide(
                t!("Положите файл .syn в папку категории"),
                t!("llm — языковые, image — картинки, video — видео, music — музыка, tts — синтез речи, asr — распознавание, embed — поиск по смыслу. Служба узнаёт модель по файлу и сама собирает её из частей (энкодеры, VAE)."),
                Text::new(""),
            ),
            row_wide(
                t!("Модель со своим описанием"),
                t!("Папка с model.toml и файлами весов (например, pronunciation/w2v2-kk) — для моделей, которые synaptix не узнаёт по файлу. Папки, начинающиеся с «_», служба пропускает."),
                Text::new(""),
            ),
            row_inline(
                t!("Обновить список"),
                t!("После добавления или удаления файлов"),
                button(t!("Обновить"), move || act(sig, |c| c.call(synaptixd::Request::Rescan, &[]).map(|_| ()))),
            ),
        ],
    );
    sub_page(
        ctx,
        t!("Хранилище моделей"),
        t!("Модели — обычные файлы в папках библиотек, разложенные по категориям."),
        vec![boxed(libs_view), how, add],
    )
}

fn status_card(m: &ModelEntry) -> W {
    let mut rows: Vec<W> = Vec::new();
    let mut hint = kind_label(m.kind);
    if !m.languages.is_empty() {
        hint.push_str(&format!(" · {}", m.languages.join(", ")));
    }
    hint.push_str(&format!(" · {}", mb(m.disk_bytes)));
    if m.npu_cache {
        hint.push_str(&t!(" · графы NPU предсобраны"));
    }
    if !m.description.is_empty() {
        hint = format!("{} · {hint}", m.description);
    }
    rows.push(row_inline(state_text(m), hint, Text::new("")));
    let files: Vec<String> = m
        .variants
        .iter()
        .map(|v| {
            let devs: Vec<String> = v.devices.iter().map(|d| device_label(*d)).collect();
            format!("{}: {} · {}", devs.join(", "), tl(synaptixd::precision_label(&v.precision)), mb(v.bytes))
        })
        .collect();
    if !files.is_empty() {
        rows.push(row_inline(t!("Файлы весов"), files.join("\n"), Text::new("")));
    }
    if !m.license.is_empty() {
        rows.push(row_inline(t!("Лицензия"), "", Text::new(m.license.clone()).class("row-value")));
    }
    if !m.components.is_empty() {
        rows.push(row_inline(t!("Части модели"), m.components.join(", "), Text::new("")));
    }
    group(t!("Состояние"), rows)
}

fn settings_card(sig: RwSignal<Snapshot>, id: &str, cfg: &Cfg) -> W {
    let mut rows: Vec<W> = Vec::new();
    let mut dd = Dropdown::new().width(220.0);
    let mut choices = vec![DeviceKind::Auto];
    choices.extend(DeviceKind::ALL.iter().copied().filter(|d| *d != DeviceKind::Auto && cfg.devices.contains(d)));
    for d in choices {
        let mut label = device_label(d);
        if d != DeviceKind::Auto && !cfg.avail.contains(&d) {
            label.push_str(&t!(" — недоступно"));
        }
        dd = dd.item(DropdownItem::new(d.id(), label));
    }
    let i = id.to_string();
    let dd = dd.selected(cfg.settings.device.id().to_string()).on_change(move |v: &str| {
        if let Some(d) = DeviceKind::parse(v) {
            change(sig, &i, SettingsChange { device: Some(d), ..Default::default() });
        }
    });
    let order: Vec<String> = [DeviceKind::Cuda, DeviceKind::Npu, DeviceKind::Cpu]
        .into_iter()
        .filter(|d| cfg.devices.contains(d) && cfg.avail.contains(d))
        .map(device_label)
        .collect();
    let dev_hint = if cfg.fallback {
        t!("Выбранное устройство не справилось — модель работает на CPU")
    } else {
        t!("Авто: {order}", order = order.join(" → "))
    };
    rows.push(row(t!("Устройство"), dev_hint, dd));

    let usable: Vec<&QuantOption> = cfg.quant.iter().filter(|q| q.devices.iter().any(|d| cfg.avail.contains(d))).collect();
    if usable.len() > 1 {
        let mut qd = Dropdown::new().width(220.0);
        for q in &usable {
            let mut label = if q.id == "auto" { t!("Оптимальное") } else { tl(synaptixd::precision_label(&q.id)) };
            if q.id != "auto" && q.devices.len() < cfg.devices.len() {
                let devs: Vec<String> = q.devices.iter().map(|d| device_label(*d)).collect();
                label.push_str(&format!(" ({})", devs.join(", ")));
            }
            qd = qd.item(DropdownItem::new(q.id.clone(), label));
        }
        let i = id.to_string();
        let qd = qd.selected(cfg.settings.quant.clone().unwrap_or_else(|| "auto".into())).on_change(move |v: &str| {
            let q = (v != "auto").then(|| v.to_string());
            change(sig, &i, SettingsChange { quant: Some(q), ..Default::default() });
        });
        rows.push(row(t!("Квантование"), t!("Оптимальное подбирается под устройство; смена перезагрузит модель"), qd));
    }

    let i = id.to_string();
    rows.push(row_inline(
        t!("Не выгружать"),
        t!("Держать в памяти, даже если её не хватает другим"),
        Toggle::with_state(cfg.settings.pinned).on_change(move |v| change(sig, &i, SettingsChange { pinned: Some(v), ..Default::default() })),
    ));
    let i = id.to_string();
    rows.push(row_inline(
        t!("Загружать при запуске"),
        t!("Модель готова сразу, без задержки при первом обращении"),
        Toggle::with_state(cfg.settings.preload).on_change(move |v| change(sig, &i, SettingsChange { preload: Some(v), ..Default::default() })),
    ));
    let mut idle = Dropdown::new().width(220.0);
    for (v, l) in IDLE {
        idle = idle.item(DropdownItem::new(v.to_string(), tl(l)));
    }
    let i = id.to_string();
    let cur = cfg.settings.idle_unload_min.map(|v| v as i64).unwrap_or(-1);
    rows.push(row(
        t!("Выгружать при простое"),
        t!("Неиспользуемая модель освобождает память"),
        idle.selected(cur.to_string()).on_change(move |v: &str| {
            let Ok(n) = v.parse::<i64>() else { return };
            let value = if n < 0 { None } else { Some(n as u32) };
            change(sig, &i, SettingsChange { idle_unload_min: Some(value), ..Default::default() });
        }),
    ));
    if cfg.engine.is_some() {
        let mut keep = Dropdown::new().width(220.0);
        for (v, l) in KV_KEEP {
            keep = keep.item(DropdownItem::new(v.to_string(), tl(l)));
        }
        let i = id.to_string();
        let cur = cfg.settings.kv_keep_min.map(|v| v as i64).unwrap_or(-1);
        rows.push(row(
            t!("Держать контекст чатов"),
            t!("После выгрузки KV-кэш диалогов ждёт в оперативной памяти: модель загрузится снова — чат продолжится без пересчёта истории"),
            keep.selected(cur.to_string()).on_change(move |v: &str| {
                let Ok(n) = v.parse::<i64>() else { return };
                let value = if n < 0 { None } else { Some(n as u32) };
                change(sig, &i, SettingsChange { kv_keep_min: Some(value), ..Default::default() });
            }),
        ));
    }
    let (i1, i2) = (id.to_string(), id.to_string());
    let buttons = Row::new()
        .gap(8.0)
        .child(button(t!("Загрузить в память"), move || {
            let i = i1.clone();
            act(sig, move |c| c.load(&i).map(|_| ()));
        }))
        .child(button(t!("Выгрузить из памяти"), move || {
            let i = i2.clone();
            act(sig, move |c| c.unload(&i));
        }));
    rows.push(row_inline(t!("Память"), "", buttons));
    group(t!("Выполнение"), rows)
}

const ENGINE_WEIGHTS: [(&str, &str); 7] = [
    ("nvfp4", "NVFP4 — 4 бита"),
    ("mxfp8", "MXFP8 — 8 бит"),
    ("sq8", "SQ8 — 8 бит"),
    ("sq4", "SQ4 — 4 бита"),
    ("bf16", "BF16"),
    ("f16", "F16"),
    ("f32", "F32"),
];
const ENGINE_COMPUTE: [(&str, &str); 3] = [("bf16", "BF16"), ("f16", "F16"), ("f32", "F32")];
const ENGINE_KV: [(&str, &str); 4] = [("fp8e4m3", "MXFP8 — вдвое меньше памяти"), ("bf16", "BF16"), ("f16", "F16"), ("f32", "F32")];
const ENGINE_SYNC: [(&str, &str); 3] = [("auto", "Авто — на префилле"), ("on", "После каждого слоя"), ("off", "Не синхронизировать")];

/// Правка профиля движка: любая правка делает профиль своим.
fn set_engine(sig: RwSignal<Snapshot>, id: &str, f: impl FnOnce(&mut EngineProfile)) {
    let Some(cfg) = cfg_signal(id).get_untracked() else { return };
    let mut e = cfg.settings.engine;
    e.custom = true;
    f(&mut e);
    change(sig, id, SettingsChange { engine: Some(e), ..Default::default() });
}

fn engine_choice(
    sig: RwSignal<Snapshot>,
    id: &str,
    items: &[(&str, &str)],
    cur: Option<String>,
    set: impl Fn(&mut EngineProfile, String) + Send + Sync + 'static,
) -> W {
    let mut dd = Dropdown::new().width(220.0);
    let cur = cur.unwrap_or_default();
    for (v, l) in items {
        dd = dd.item(DropdownItem::new(*v, tl(*l)));
    }
    if !cur.is_empty() && !items.iter().any(|(v, _)| *v == cur) {
        dd = dd.item(DropdownItem::new(cur.clone(), cur.to_uppercase()));
    }
    let i = id.to_string();
    let set = std::sync::Arc::new(set);
    boxed(dd.selected(cur).on_change(move |v: &str| {
        let (v, set) = (v.to_string(), set.clone());
        set_engine(sig, &i, move |e| set(e, v));
    }))
}

fn engine_toggle(sig: RwSignal<Snapshot>, id: &str, on: bool, set: impl Fn(&mut EngineProfile, bool) + Send + Sync + 'static) -> W {
    let i = id.to_string();
    let set = std::sync::Arc::new(set);
    boxed(Toggle::with_state(on).on_change(move |v| {
        let set = set.clone();
        set_engine(sig, &i, move |e| set(e, v));
    }))
}

/// Профиль движка языковой модели: выверенный synaptix под архитектуру и
/// устройство или свой (правки поверх выверенного).
fn engine_card(sig: RwSignal<Snapshot>, id: &str, cfg: &Cfg) -> Option<W> {
    let shown = cfg.engine.clone()?;
    let own = cfg.settings.engine.clone();
    let mut rows: Vec<W> = Vec::new();
    let i = id.to_string();
    rows.push(row_inline(
        t!("Свой профиль"),
        if own.custom { t!("Правки поверх выверенного; пустые поля — как выверено") } else { t!("Сейчас — как выверено движком под архитектуру и устройство") },
        Toggle::with_state(own.custom).on_change(move |v| {
            let Some(cfg) = cfg_signal(&i).get_untracked() else { return };
            let e = EngineProfile { custom: v, ..cfg.settings.engine };
            change(sig, &i, SettingsChange { engine: Some(e), ..Default::default() });
        }),
    ));
    let transcode = shown.transcode == Some(true);
    let weights_hint = if shown.source.is_empty() {
        t!("Плотные веса квантуются в него при загрузке")
    } else if transcode {
        t!("В файле {f} — перекодируются в этот формат", f = shown.source.join(", ").to_uppercase())
    } else {
        t!("В файле {f} — так и считаются; формат — только для плотных тензоров", f = shown.source.join(", ").to_uppercase())
    };
    rows.push(row(t!("Веса"), weights_hint, engine_choice(sig, id, &ENGINE_WEIGHTS, shown.weights.clone(), |e, v| e.weights = Some(v))));
    rows.push(row(t!("Голова (lm_head)"), "", engine_choice(sig, id, &ENGINE_WEIGHTS, shown.lm_head.clone(), |e, v| e.lm_head = Some(v))));
    rows.push(row(t!("Эмбеддинги"), "", engine_choice(sig, id, &ENGINE_WEIGHTS, shown.embed.clone(), |e, v| e.embed = Some(v))));
    rows.push(row(t!("Точность счёта"), "", engine_choice(sig, id, &ENGINE_COMPUTE, shown.compute.clone(), |e, v| e.compute = Some(v))));
    rows.push(row(t!("KV-кэш"), t!("Память на токен контекста"), engine_choice(sig, id, &ENGINE_KV, shown.kv_dtype.clone(), |e, v| e.kv_dtype = Some(v))));
    if !shown.source.is_empty() {
        rows.push(row_inline(
            t!("Перекодировать веса"),
            t!("Двойной квант: быстрее на тензорных ядрах, но теряет точность"),
            engine_toggle(sig, id, transcode, |e, v| e.transcode = Some(v)),
        ));
    }
    rows.push(row_inline(
        t!("Граф декода"),
        t!("Шаг генерации — одним CUDA-графом; меняется без перезагрузки"),
        engine_toggle(sig, id, shown.graph_decode == Some(true), |e, v| e.graph_decode = Some(v)),
    ));
    rows.push(row_inline(
        t!("Спекулятивный декод"),
        t!("MTP или DFlash: несколько токенов за шаг, ответ тот же"),
        engine_toggle(sig, id, shown.speculation == Some(true), |e, v| e.speculation = Some(v)),
    ));
    rows.push(row(t!("Синхронизация слоёв"), "", engine_choice(sig, id, &ENGINE_SYNC, shown.layer_sync.clone(), |e, v| e.layer_sync = Some(v))));
    if own.custom && own != (EngineProfile { custom: true, ..Default::default() }) {
        let i = id.to_string();
        rows.push(row_inline(
            t!("Правки"),
            t!("Веса, счёт, KV и спекуляция перезагружают модель"),
            button(t!("Сбросить"), move || {
                change(sig, &i, SettingsChange { engine: Some(EngineProfile { custom: true, ..Default::default() }), ..Default::default() });
            }),
        ));
    }
    Some(group(t!("Профиль движка"), rows))
}

fn save_param(sig: RwSignal<Snapshot>, id: &str, key: &str, value: Option<ParamValue>) {
    let Some(mut st) = params_state(id) else { return };
    let Some(spec) = st.schema.iter().find(|p| p.key == key).cloned() else { return };
    let mut own = st.own.clone();
    let value = value.and_then(|v| spec.normalize(&v));
    let value = value.filter(|v| Some(v) != st.optimal.get(key));
    own.set(key, value);
    if own == st.own {
        return;
    }
    st.own = own.clone();
    set_params_state(id, st, false);
    change(sig, id, SettingsChange { params: Some(own), ..Default::default() });
}

fn param_row(sig: RwSignal<Snapshot>, id: &str, spec: &ParamSpec, st: &ParamState) -> W {
    let _ = SCHEMA_TEXT;
    let kind = spec.kind.clone();
    let optimal = st.optimal.get(&spec.key).map(|v| v.display(&kind));
    let own = st.own.get(&spec.key).map(|v| v.display(&kind));
    let label = tl(&spec.label);
    let mut hint = tl(&spec.hint);
    match (&own, &optimal) {
        (Some(_), Some(o)) if !matches!(kind, ParamKind::Text { multiline: true } | ParamKind::Bool | ParamKind::File { .. }) => hint.push_str(&t!(" · оптимальное: {v}", v = o)),
        (Some(_), None) => hint.push_str(&t!(" · оптимальное: авто")),
        _ => {}
    }
    let i = id.to_string();
    let key = spec.key.clone();
    match &kind {
        ParamKind::Text { multiline: true } => {
            let gen = Arc::new(AtomicU64::new(0));
            let edit = syngui::widgets::input::MultilineTextEdit::new()
                .text(own.clone().or(optimal.clone()).unwrap_or_default())
                .placeholder(t!("Не задан"))
                .rows(3)
                .max_rows(12)
                .auto_height(true)
                .soft_wrap(true)
                .on_change(move |s: &str| {
                    let n = gen.fetch_add(1, Ordering::Relaxed) + 1;
                    let (gen, i, s, key) = (gen.clone(), i.clone(), s.to_string(), key.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(700));
                        if gen.load(Ordering::Relaxed) == n {
                            syngui::async_runtime::run_on_main_thread(move || save_param(sig, &i, &key, Some(ParamValue::Text(s))));
                        }
                    });
                });
            row_wide(label, hint, edit)
        }
        ParamKind::Bool => {
            let on = st.own.get(&spec.key).or(st.optimal.get(&spec.key)).and_then(ParamValue::as_bool).unwrap_or(false);
            row_inline(label, hint, Toggle::with_state(on).on_change(move |v| save_param(sig, &i, &key, Some(ParamValue::Bool(v)))))
        }
        ParamKind::Choice { options } => {
            let mut dd = Dropdown::new().width(200.0);
            if optimal.is_none() {
                dd = dd.item(DropdownItem::new("", t!("Авто")));
            }
            for o in options {
                let mut l = o.clone();
                if optimal.as_deref() == Some(o.as_str()) {
                    l.push_str(&t!(" (оптимальное)"));
                }
                dd = dd.item(DropdownItem::new(o.clone(), l));
            }
            let cur = own.or(optimal).unwrap_or_default();
            let dd = dd.selected(cur).on_change(move |v: &str| save_param(sig, &i, &key, (!v.is_empty()).then(|| ParamValue::Text(v.to_string()))));
            row(label, hint, dd)
        }
        ParamKind::Text { multiline: false } => {
            let field = TextField::with_text(own.unwrap_or_default())
                .placeholder(optimal.unwrap_or_else(|| t!("Авто")))
                .width(160.0)
                .submit_on_focus_lost(true)
                .on_submit(move |s| {
                    let v = s.trim();
                    save_param(sig, &i, &key, (!v.is_empty()).then(|| ParamValue::Text(v.to_string())));
                });
            row(label, hint, field)
        }
        ParamKind::File { patterns } => {
            let (i2, key2) = (i.clone(), key.clone());
            let field = TextField::with_text(own.clone().unwrap_or_default())
                .placeholder(t!("Не задан"))
                .width(260.0)
                .submit_on_focus_lost(true)
                .on_submit(move |s| {
                    let v = s.trim();
                    save_param(sig, &i, &key, (!v.is_empty()).then(|| ParamValue::Text(v.to_string())));
                });
            let patterns = patterns.clone();
            let start = own.unwrap_or_default();
            let title = label.clone();
            let pick = icon_button(icons::FOLDER, move || {
                let (i, key, patterns, start, title) = (i2.clone(), key2.clone(), patterns.clone(), start.clone(), title.clone());
                std::thread::spawn(move || {
                    let dir = std::path::Path::new(&start).parent().filter(|p| p.is_dir()).map(|p| p.to_path_buf()).unwrap_or_else(|| synshell_common::paths::expand_tilde("~"));
                    let pats: Vec<&str> = patterns.iter().map(String::as_str).collect();
                    let filters = [synsystem::portal_files::PickFilter { name: &title, patterns: &pats }];
                    let picked = match synsystem::portal_files::pick(&title, false, &dir, if pats.is_empty() { &[] } else { &filters }) {
                        Ok(p) => p.map(|p| p.display().to_string()),
                        Err(_) => crate::sys::pick_path(false, &dir.display().to_string()),
                    };
                    if let Some(path) = picked {
                        syngui::async_runtime::run_on_main_thread(move || save_param(sig, &i, &key, Some(ParamValue::Text(path))));
                    }
                });
            });
            row(label, hint, Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(field).child(pick))
        }
        ParamKind::Int { .. } | ParamKind::Float { .. } => {
            let field = TextField::with_text(own.unwrap_or_default())
                .placeholder(optimal.unwrap_or_else(|| t!("Авто")))
                .width(140.0)
                .submit_on_focus_lost(true)
                .input_filter(|c: char| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))
                .on_submit(move |s| {
                    let v = s.trim();
                    save_param(sig, &i, &key, (!v.is_empty()).then(|| ParamValue::Text(v.to_string())));
                });
            row(label, hint, field)
        }
    }
}

fn params_card(sig: RwSignal<Snapshot>, id: &str) -> W {
    let rev = params_rev(id);
    let id = id.to_string();
    boxed(Reactive::new(move || -> Vec<W> {
        rev.get();
        let Some(st) = params_state(&id) else { return vec![] };
        if st.schema.is_empty() {
            return vec![];
        }
        let mut rows: Vec<W> = st.schema.iter().map(|k| param_row(sig, &id, k, &st)).collect();
        let i = id.clone();
        let reset = button(t!("Сбросить к оптимальным"), move || {
            let Some(mut st) = params_state(&i) else { return };
            st.own = Params::default();
            set_params_state(&i, st, true);
            change(sig, &i, SettingsChange { params: Some(Params::default()), ..Default::default() });
        });
        rows.push(row_inline(
            t!("Свои значения"),
            if st.own.is_empty() { t!("Не заданы — работают оптимальные значения модели") } else { t!("Пустое поле — оптимальное значение модели") },
            reset,
        ));
        vec![group(t!("Параметры по умолчанию"), rows)]
    }))
}

fn location_card(sig: RwSignal<Snapshot>, m: &ModelEntry, libs: &[LibraryInfo]) -> W {
    let mut rows: Vec<W> = Vec::new();
    match &m.library {
        None => rows.push(row_inline(t!("Где лежит"), t!("Лежит в каталоге из model_dirs службы, вне библиотек"), Text::new(""))),
        Some(path) => {
            let name = libs.iter().find(|l| &l.path == path).map(|l| l.name.clone()).unwrap_or_else(|| path.clone());
            let files: Vec<String> = m.files.iter().map(|f| f.strip_prefix(path.as_str()).unwrap_or(f).trim_start_matches('/').to_string()).collect();
            let folder = m.files.first().and_then(|f| std::path::Path::new(f).parent().map(|p| p.to_string_lossy().into_owned())).unwrap_or_else(|| path.clone());
            rows.push(row_wide(t!("Где лежит"), format!("{name}: {}", files.join(", ")), Row::new().gap(8.0).child(button(t!("Показать в папке"), move || open_folder(&folder)))));
            let id = m.id.clone();
            rows.push(row_inline(
                t!("Удалить модель"),
                t!("Файлы модели, которые не нужны другим моделям, будут удалены с диска"),
                button(t!("Удалить"), move || {
                    let id = id.clone();
                    act(sig, move |c| c.delete_model(&id));
                }),
            ));
        }
    }
    group(t!("Хранение"), rows)
}

/// Идущая проверка озвучки: новый запуск или «Остановить» сменяют поколение.
static TTS_TEST: AtomicU64 = AtomicU64::new(0);

/// Озвучивает текст моделью `id` с её сохранёнными параметрами и ждёт конца
/// воспроизведения (или смены поколения / ухода со страницы моделей).
/// Зерно, которым синтезировано, отдаётся в `seed` (закрепить понравившийся голос).
fn tts_test_blocking(id: &str, text: &str, gen: u64, seed: &dyn Fn(Option<u64>)) -> std::result::Result<(), String> {
    let current = || TTS_TEST.load(Ordering::Acquire) == gen && ACTIVE.load(Ordering::Relaxed);
    let mut c = Client::connect().map_err(|e| e.to_string())?;
    let sp = c.speak_fresh(id, text, &Params::default()).map_err(|e| e.to_string())?;
    seed(sp.seed);
    if !current() {
        return Ok(());
    }
    let pcm: Arc<[f32]> = sp.audio.f32s().into();
    let player = syngui::audio::AudioPlayer::start(pcm, sp.sample_rate).map_err(|e| e.to_string())?;
    while !player.is_done() && current() {
        if let Some(e) = player.error() {
            return Err(e);
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    player.stop();
    Ok(())
}

/// Проверка модели синтеза речи: текст из поля читается этой моделью с
/// заданными на странице параметрами (голос, скорость, шаги…).
/// `presets` — модель понимает описание голоса (VoxCPM2, OmniVoice): над
/// проверкой готовые описания, нажатие подставляет его в «Голос» и сразу озвучивает.
fn tts_test_card(sig: RwSignal<Snapshot>, id: &str, presets: Option<&'static super::voice_presets::Presets>) -> W {
    let text = use_signal(t!("Привет! Так звучит эта модель с заданными настройками.").to_string());
    let busy: RwSignal<Option<bool>> = use_signal(None);
    let error: RwSignal<Option<String>> = use_signal(None);
    let seed: RwSignal<Option<u64>> = use_signal(None);
    let id = id.to_string();
    let start = {
        let id = id.clone();
        move || {
            let s = text.get_untracked();
            if s.trim().is_empty() {
                return;
            }
            let gen = TTS_TEST.fetch_add(1, Ordering::AcqRel) + 1;
            error.set(None);
            busy.set(Some(true));
            let id = id.clone();
            std::thread::spawn(move || {
                let got = move |n: Option<u64>| run_on_main_thread(move || seed.set(n));
                let r = tts_test_blocking(&id, &s, gen, &got);
                run_on_main_thread(move || {
                    if TTS_TEST.load(Ordering::Acquire) != gen {
                        return;
                    }
                    busy.set(None);
                    if let Err(e) = r {
                        error.set(Some(e));
                    }
                });
            });
        }
    };
    let submit = start.clone();
    let preset_row = presets.map(|p| voice_presets_row(sig, &id, p, start.clone()));
    let toggle = Reactive::new(move || -> Vec<W> {
        let on = busy.get().is_some();
        let start = start.clone();
        vec![boxed(button(if on { t!("Остановить") } else { t!("Проверить") }, move || {
            if busy.get_untracked().is_some() {
                TTS_TEST.fetch_add(1, Ordering::AcqRel);
                busy.set(None);
            } else {
                start();
            }
        }))]
    });
    let pin_id = id.clone();
    let state = Reactive::new(move || -> Vec<W> {
        let mut out: Vec<W> = Vec::new();
        if let Some(e) = error.get() {
            out.push(boxed(Text::new(t!("Озвучка не удалась: {e}", e = e)).class("mic-test-state pause")));
        } else if busy.get().is_some() {
            out.push(boxed(Text::new(t!("Озвучиваю… (первый запуск загружает модель)")).class("mic-test-text")));
        }
        // Зерно последней проверки: понравился голос — закрепить его.
        if let Some(n) = seed.get() {
            params_rev(&pin_id).get();
            let pinned = params_state(&pin_id).and_then(|st| st.own.get("seed").cloned()) == Some(ParamValue::Int(n as i64));
            let id = pin_id.clone();
            let mut r = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(
                Text::new(if pinned { t!("Зерно {n} — закреплено", n = n) } else { t!("Зерно этого голоса: {n}", n = n) }).class("mic-test-text"),
            );
            if !pinned {
                r = r.child(button(t!("Закрепить"), move || {
                    save_param(sig, &id, "seed", Some(ParamValue::Int(n as i64)));
                    if let Some(st) = params_state(&id) {
                        set_params_state(&id, st, true);
                    }
                }));
            }
            out.push(boxed(r));
        }
        out
    });
    let field = TextField::with_text(text.get_untracked())
        .placeholder(t!("Текст для проверки"))
        .on_change(move |s| text.set(s.to_string()))
        .on_submit(move |_| submit())
        .class("grow");
    let mut rows: Vec<W> = preset_row.into_iter().collect();
    rows.push(row_wide(
            t!("Проверка озвучки"),
            t!("Текст читается этой моделью с параметрами, заданными выше, каждый раз заново — без голоса прошлых озвучек. Если у модели есть «Зерно»: со случайным голос каждый раз новый, понравившийся можно закрепить"),
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(8.0).child(field).child(toggle))
                .child(state),
        ));
    group(t!("Проверка"), rows)
}

/// Готовые голоса: вкладки категорий и плитки; выбранная — та, что сейчас в «Голосе».
fn voice_presets_row(sig: RwSignal<Snapshot>, id: &str, presets: &'static super::voice_presets::Presets, play: impl Fn() + Clone + Send + Sync + 'static) -> W {
    let cats = presets.categories;
    let current = move |id: &str| -> String {
        params_state(id).and_then(|st| st.own.get("voice").and_then(|v| v.as_str().map(str::to_string))).unwrap_or_default()
    };
    let tab = use_signal(presets.category_of(&current(id)).unwrap_or(0));
    let tabs = SegmentedButton::new(cats.iter().map(|c| tl(c.title)).collect::<Vec<_>>()).selected(tab.get_untracked()).on_change(move |i| tab.set(i));
    let id = id.to_string();
    let chips = Reactive::new(move || -> Vec<W> {
        params_rev(&id).get();
        let cur = current(&id);
        let cat = &cats[tab.get().min(cats.len() - 1)];
        let mut wrap = Flex::row().wrap().gap(6.0);
        for pr in cat.presets {
            let (id, play) = (id.clone(), play.clone());
            wrap = wrap.child(Chip::new(tl(pr.title)).selected(cur == pr.voice).on_click(move || {
                save_param(sig, &id, "voice", Some(ParamValue::Text(pr.voice.to_string())));
                if let Some(st) = params_state(&id) {
                    set_params_state(&id, st, true);
                }
                play();
            }));
        }
        if !cur.is_empty() {
            let id = id.clone();
            wrap = wrap.child(Chip::new(t!("Без описания")).on_click(move || {
                save_param(sig, &id, "voice", None);
                if let Some(st) = params_state(&id) {
                    set_params_state(&id, st, true);
                }
            }));
        }
        let hint = if cur.is_empty() { t!("Описание не задано — голос выбирает модель").to_string() } else { cur };
        vec![boxed(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(cat.note.map(|n| boxed(Text::new(tl(n)).class("row-hint"))))
                .child(wrap)
                .child(Text::new(hint).class("mic-test-text")),
        )]
    });
    row_wide(
        t!("Готовые голоса"),
        tl(presets.hint),
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Start).child(tabs).child(chips),
    )
}

fn model_page(ctx: state::Ctx, sig: RwSignal<Snapshot>, id: String) -> W {
    let cfg = cfg_signal(&id);
    let i = id.clone();
    let status = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        match snap.models.iter().find(|m| m.id == i) {
            Some(m) => vec![status_card(m), location_card(sig, m, &snap.libraries)],
            None => vec![note(t!("Модель {id} не найдена — возможно, удалена из библиотеки", id = i))],
        }
    });
    let i = id.clone();
    let settings = Reactive::new(move || -> Vec<W> {
        let Some(c) = cfg.get() else { return Vec::new() };
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(settings_card(sig, &i, &c));
        if let Some(e) = engine_card(sig, &i, &c) {
            col = col.child(e);
        }
        vec![boxed(col)]
    });
    let snap = sig.get_untracked();
    let title = title_of(&snap, &id);
    let parent = snap.models.iter().find(|m| m.id == id).map(|m| kind_key(m.kind));
    let note_text = t!("Программы могут передать свои значения в запросе — они действуют только на этот запрос.");
    let test = snap
        .models
        .iter()
        .find(|m| m.id == id && m.kind == ModelKind::Tts)
        .map(|m| tts_test_card(sig, &id, super::voice_presets::for_arch(&m.arch)));
    sub_page_in(
        ctx,
        parent,
        title,
        note_text,
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(status).child(settings).child(params_card(sig, &id)).children(test))],
    )
}

fn key_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish() | 2
}

pub fn models() -> W {
    let sig = live();
    ACTIVE.store(true, Ordering::Relaxed);
    let ctx = state::ctx();
    let view = Reactive::new(move || -> Vec<W> {
        let sub = ctx.sub.get().and_then(|(k, _)| k.strip_prefix(SUB).map(str::to_string));
        let key = sub.as_deref().map(key_hash).unwrap_or(1);
        vec![boxed(
            AnimatedSwitcher::new(key, move || match sub.as_deref() {
                Some("defaults") => defaults_page(ctx, sig),
                Some("service") => service_page(ctx, sig),
                Some("storage") => storage_page(ctx, sig),
                Some(k) if k.starts_with("m/") => model_page(ctx, sig, k[2..].to_string()),
                Some(k) if k.starts_with("k/") => match ModelKind::parse(&k[2..]) {
                    Some(kind) => kind_page(ctx, sig, kind),
                    None => menu(ctx, sig),
                },
                _ => menu(ctx, sig),
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(240)
            .exit_duration_ms(160)
            .animate_size(false)
            .class("grow"),
        )]
    });
    boxed(view)
}
