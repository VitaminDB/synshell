use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use syngui::prelude::*;
use synaptixd::{
    Client, DefaultModel, DeviceKind, ImportScan, Job, JobState, LibraryInfo, LoadState, ModelEntry, ModelKind, ModelSettings, ParamKind, ParamSpec, ParamValue, Params, QuantOption,
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
    jobs: Vec<Job>,
    error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct Cfg {
    devices: Vec<DeviceKind>,
    avail: Vec<DeviceKind>,
    settings: ModelSettings,
    quant: Vec<QuantOption>,
    fallback: bool,
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
    let r = Client::connect().and_then(|mut c| Ok((c.status()?, c.list()?, c.defaults()?, c.libraries().unwrap_or_default(), c.jobs().unwrap_or_default())));
    match r {
        Ok((status, models, defaults, libraries, jobs)) => Snapshot { status: Some(status), models, defaults, libraries, jobs, error: None },
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
                snap.libraries.iter().any(|l| !l.online) || snap.jobs.iter().any(|j| j.state == JobState::Failed),
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
        for kind in ModelKind::ALL {
            let items: Vec<W> = snap
                .models
                .iter()
                .filter(|m| m.kind == kind)
                .map(|m| nav_item(ctx, format!("m/{}", m.id), m.title.clone(), short_state(m), kind_icon(kind), m.state == LoadState::Failed))
                .collect();
            if !items.is_empty() {
                out.push(titled(group_label(kind), card(items)));
            }
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

fn back(ctx: state::Ctx) -> Option<W> {
    (!narrow()).then(|| boxed(Row::new().child(button(t!("‹ Нейросети"), move || ctx.sub.set(None)))))
}

fn sub_page(ctx: state::Ctx, title: String, subtitle: String, body: Vec<W>) -> W {
    let mut parts: Vec<W> = back(ctx).into_iter().collect();
    parts.extend(body);
    page(title, subtitle, parts)
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
                rows.push(row_inline(task_label(kind), task_hint(kind), Text::new(t!("Нет установленных моделей")).class("row-value")));
                continue;
            }
            let auto = match (&d.model, &d.resolved) {
                (None, Some(r)) => t!("Авто — {m}", m = title(r)),
                _ => t!("Авто"),
            };
            let mut dd = Dropdown::new().item(DropdownItem::new("", auto));
            for (id, t, _) in &models {
                dd = dd.item(DropdownItem::new(id.clone(), t.clone()));
            }
            let dd = dd.selected(d.model.clone().unwrap_or_default()).on_change(move |v: &str| {
                let model = (!v.is_empty()).then(|| v.to_string());
                act(sig, move |c| c.set_default(kind, model.as_deref()).map(|_| ()));
            });
            rows.push(row(task_label(kind), task_hint(kind), dd));
        }
        vec![group(t!("Модель для задачи"), rows)]
    });
    sub_page(
        ctx,
        t!("Модели по умолчанию"),
        t!("Их берут программы и ассистент, если не выбрали модель сами. «Авто» — уже загруженная или лучшая для устройств этой машины."),
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
    static SCAN: std::cell::Cell<Option<RwSignal<Option<(String, ImportScan)>>>> = const { std::cell::Cell::new(None) };
    static IMPORT_DIR: RefCell<String> = RefCell::new(String::new());
    static NEW_LIB: RefCell<String> = RefCell::new(String::new());
}

fn scan_signal() -> RwSignal<Option<(String, ImportScan)>> {
    SCAN.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            c.set(Some(s));
            s
        }
    })
}

fn usage_bar(used: u64, total: u64) -> W {
    let v = if total > 0 { used as f32 / total as f32 } else { 0.0 };
    boxed(DecoratedBox::new().class("lib-bar-wrap").child(ProgressBar::with_value(v).class("lib-bar")))
}

fn kind_short(k: ModelKind) -> String {
    group_label(k)
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
    let scan = scan_signal();
    let libs_view = Reactive::new(move || -> Vec<W> {
        let snap = sig.get();
        let mut out: Vec<W> = Vec::new();
        if snap.libraries.is_empty() {
            out.push(note(t!("Библиотека — папка на любом диске, где служба хранит модели. Одинаковые файлы хранятся один раз; на том же разделе btrfs импорт не тратит место.")));
        } else {
            out.push(group(t!("Библиотеки"), library_rows(sig, &snap.libraries)));
        }
        let jobs: Vec<W> = snap
            .jobs
            .iter()
            .map(|j| {
                let hint = match j.state {
                    JobState::Running => format!("{} / {} · {}", mb(j.done), mb(j.total), j.detail),
                    JobState::Done => t!("Готово"),
                    JobState::Failed => t!("Ошибка: {e}", e = j.error.clone().unwrap_or_default()),
                };
                boxed(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(row_inline(j.title.clone(), hint, Text::new("")))
                        .child(usage_bar(j.done, j.total.max(1))),
                )
            })
            .collect();
        if !jobs.is_empty() {
            out.push(group(t!("Задачи"), jobs));
        }
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(out))]
    });
    let add_field = TextField::with_text(NEW_LIB.with(|v| v.borrow().clone()))
        .placeholder(t!("/путь/к/папке"))
        .width(320.0)
        .on_change(|s| NEW_LIB.with(|v| *v.borrow_mut() = s.to_string()));
    let add = group(
        t!("Добавить библиотеку"),
        vec![row_wide(
            t!("Папка"),
            t!("Будет создана, если её нет; лучше быстрый диск — модели читаются при загрузке"),
            Row::new().gap(8.0).child(add_field).child(button(t!("Добавить"), move || {
                let path = NEW_LIB.with(|v| v.borrow().trim().to_string());
                if path.is_empty() {
                    return;
                }
                act(sig, move |c| c.call(synaptixd::Request::AddLibrary { path, name: None }, &[]).map(|_| ()));
            })),
        )],
    );
    let dir_field = TextField::with_text(IMPORT_DIR.with(|v| v.borrow().clone()))
        .placeholder(t!("Папка с файлами .syn"))
        .width(320.0)
        .on_change(|s| IMPORT_DIR.with(|v| *v.borrow_mut() = s.to_string()));
    let find = button(t!("Найти модели"), move || {
        let dir = IMPORT_DIR.with(|v| v.borrow().trim().to_string());
        if dir.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            let r = Client::connect().and_then(|mut c| c.import_scan(&dir));
            syngui::async_runtime::run_on_main_thread(move || match r {
                Ok(s) => scan.set(Some((dir, s))),
                Err(e) => state::toast(e.to_string()),
            });
        });
    });
    let import_head = row_wide(t!("Импорт"), t!("Служба узнаёт модели synaptix по файлам и собирает их из частей"), Row::new().gap(8.0).child(dir_field).child(find));
    let found = Reactive::new(move || -> Vec<W> {
        let Some((dir, s)) = scan.get() else { return vec![] };
        let libs = sig.get_untracked().libraries;
        let mut rows: Vec<W> = Vec::new();
        let fresh: Vec<String> = s.models.iter().filter(|m| !m.installed).map(|m| m.id.clone()).collect();
        for m in &s.models {
            let size: u64 = m.files.iter().map(|f| f.2).sum();
            let parts = if m.files.len() > 1 { t!(" · частей: {n}", n = m.files.len()) } else { String::new() };
            let state = if m.installed { t!("уже установлена") } else { mb(size) };
            rows.push(row_inline(m.title.clone(), format!("{} · {}{parts}", kind_short(m.kind), m.description), Text::new(state).class("row-value")));
        }
        for (p, why) in &s.skipped {
            rows.push(row_inline(p.rsplit('/').next().unwrap_or(p).to_string(), why.clone(), Text::new(t!("пропущено")).class("row-value")));
        }
        if !fresh.is_empty() && !libs.is_empty() {
            let mut dd = Dropdown::new();
            for l in &libs {
                dd = dd.item(DropdownItem::new(l.path.clone(), l.name.clone()));
            }
            let def = libs.iter().find(|l| l.default).or(libs.first()).map(|l| l.path.clone()).unwrap_or_default();
            let target = Arc::new(std::sync::Mutex::new(def.clone()));
            let t2 = target.clone();
            let dd = dd.selected(def).on_change(move |v: &str| *t2.lock().unwrap() = v.to_string());
            let n = fresh.len();
            let go = primary_button(t!("Импортировать: {n}", n = n), move || {
                let (dir, lib, ids) = (dir.clone(), target.lock().unwrap().clone(), fresh.clone());
                scan.set(None);
                act(sig, move |c| c.import(&dir, &lib, Some(ids)).map(|_| ()));
            });
            rows.push(row(t!("В библиотеку"), "", Row::new().gap(8.0).child(dd).child(go)));
        } else if libs.is_empty() {
            rows.push(note(t!("Сначала добавьте библиотеку")));
        }
        vec![group(t!("Найдено"), rows)]
    });
    sub_page(
        ctx,
        t!("Хранилище моделей"),
        t!("Библиотеки моделей на любых дисках: перенос между ними, импорт, общие файлы без копий."),
        vec![
            boxed(libs_view),
            add,
            group(t!("Импорт моделей"), vec![import_head]),
            boxed(found),
        ],
    )
}

fn status_card(m: &ModelEntry) -> W {
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
        (Some(_), Some(o)) if !matches!(kind, ParamKind::Text { multiline: true } | ParamKind::Bool) => hint.push_str(&t!(" · оптимальное: {v}", v = o)),
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
            let others: Vec<&LibraryInfo> = libs.iter().filter(|l| &l.path != path && l.online).collect();
            if others.is_empty() {
                rows.push(row_inline(t!("Где лежит"), path.clone(), Text::new(name).class("row-value")));
            } else {
                let mut dd = Dropdown::new().item(DropdownItem::new("", name));
                for l in others {
                    dd = dd.item(DropdownItem::new(l.path.clone(), t!("Перенести в «{n}»", n = l.name.clone())));
                }
                let id = m.id.clone();
                let dd = dd.selected("").on_change(move |v: &str| {
                    if v.is_empty() {
                        return;
                    }
                    let (id, lib) = (id.clone(), v.to_string());
                    act(sig, move |c| c.move_model(&id, &lib).map(|_| ()));
                });
                rows.push(row(t!("Где лежит"), path.clone(), dd));
            }
            let id = m.id.clone();
            rows.push(row_inline(
                t!("Удалить модель"),
                t!("Файлы, которые не нужны другим моделям, будут удалены"),
                button(t!("Удалить"), move || {
                    let id = id.clone();
                    act(sig, move |c| c.delete_model(&id));
                }),
            ));
        }
    }
    group(t!("Хранение"), rows)
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
    let settings = Reactive::new(move || -> Vec<W> { cfg.get().map(|c| settings_card(sig, &i, &c)).into_iter().collect() });
    let title = title_of(&sig.get_untracked(), &id);
    let note_text = t!("Программы могут передать свои значения в запросе — они действуют только на этот запрос.");
    sub_page(
        ctx,
        title,
        note_text,
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(status).child(settings).child(params_card(sig, &id)))],
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
