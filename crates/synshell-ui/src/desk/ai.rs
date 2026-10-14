//! Виджеты службы нейросетей: «Языковые модели» (скорость, кто сейчас
//! генерирует, история) и «Модели в памяти» (загруженные модели по
//! устройствам, их память и программы, выгрузка, закрепление, остановка
//! работы). Данные — запрос `Metrics` и список моделей synaptixd раз в
//! секунду, пока виджеты на экране.

use std::time::Duration;

use synaptixd::{Activity, ActivityState, DeviceKind, LoadState, Metrics, ModelEntry, ModelKind, SettingsChange, Status};
use syngui::containers::Keyed;
use syngui::mss::StyleValue;
use syngui::core::Color;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::data;
use super::views::{bar, big_text, human_bytes, line_view, mss_color, short_gpu, sparkline, Cx};
use crate::ui::{icon, mi, rx};

/// Снимок службы для виджетов.
#[derive(Clone, Default, PartialEq)]
pub struct AiSnap {
    pub online: bool,
    pub status: Status,
    pub models: Vec<ModelEntry>,
    pub metrics: Metrics,
}

impl AiSnap {
    fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    fn title(&self, id: &str) -> String {
        self.model(id).map(|m| m.title.clone()).unwrap_or_else(|| id.to_string())
    }

    /// Скорость всех языковых моделей сейчас, ток/с.
    fn tps_now(&self) -> f32 {
        let v: f32 = self.metrics.running.iter().filter(|a| a.op == "generate" && a.state == ActivityState::Running).map(|a| a.tps).sum();
        v.max(0.0)
    }
}

thread_local! {
    static SNAP: std::cell::Cell<Option<RwSignal<AiSnap>>> = const { std::cell::Cell::new(None) };
}

/// Сигнал снимка; первый вызов запускает сборщик.
pub fn snap() -> RwSignal<AiSnap> {
    SNAP.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(AiSnap::default());
            s.set(Some(x));
            start(x);
            x
        }
    })
}

fn fetch() -> AiSnap {
    let r = synaptixd::Client::connect().and_then(|mut c| Ok((c.status()?, c.list()?, c.metrics(true)?)));
    match r {
        Ok((status, models, metrics)) => AiSnap { online: true, status, models, metrics },
        Err(_) => AiSnap::default(),
    }
}

fn start(sig: RwSignal<AiSnap>) {
    std::thread::Builder::new()
        .name("desk-ai".into())
        .spawn(move || {
            let mut last = AiSnap { online: true, ..Default::default() };
            loop {
                if data::visible() {
                    let s = fetch();
                    if s != last {
                        last = s.clone();
                        syngui::async_runtime::run_on_main_thread(move || sig.set(s));
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .ok();
}

/// Действие со службой в фоне; ошибка — в журнал (root и wheel могут всё,
/// остальным служба откажет — виджет не притворяется, что вышло).
fn act(f: impl FnOnce(&mut synaptixd::Client) -> synaptixd::Result<()> + Send + 'static) {
    std::thread::spawn(move || {
        if let Err(e) = synaptixd::Client::connect().and_then(|mut c| f(&mut c)) {
            log::warn!("служба нейросетей: {e}");
        }
    });
}

fn open_settings(page: &str) {
    crate::actions::spawn(&format!("synsettings {page}"));
}

fn ago(now: i64, then: i64) -> String {
    let s = ((now - then) / 1000).max(0);
    match s {
        0..=59 => t!("{n} с назад", n = s),
        60..=3599 => t!("{n} мин назад", n = s / 60),
        _ => t!("{n} ч назад", n = s / 3600),
    }
}

fn op_label(op: &str) -> String {
    match op {
        "generate" => t!("ответ"),
        "image" => t!("картинка"),
        "video" => t!("видео"),
        "music" => t!("музыка"),
        "speak" => t!("речь"),
        "transcribe" => t!("расшифровка"),
        "embed" => t!("эмбеддинги"),
        "rerank" => t!("ранжирование"),
        "flow" => t!("граф"),
        "load" => t!("загрузка"),
        other => other.to_string(),
    }
    .into()
}

fn kind_glyph(k: ModelKind) -> &'static str {
    match k {
        ModelKind::Llm => LLM,
        ModelKind::Image => "\u{E3F4}",
        ModelKind::Video => "\u{E04B}",
        ModelKind::Music => "\u{E405}",
        ModelKind::Tts => "\u{E050}",
        ModelKind::Asr | ModelKind::Ctc | ModelKind::Mfa => mi::MIC,
        ModelKind::Embed | ModelKind::Rerank => mi::SEARCH,
        ModelKind::Diarize => "\u{E7EF}",
    }
}

fn key_of(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

/// Значок языковой модели (Material `smart_toy`).
pub const LLM: &str = "\u{F06C}";
const STOP: &str = "\u{E047}";

fn offline(cx: &Cx) -> Box<dyn Widget> {
    Box::new(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(t!("Служба нейросетей не отвечает")).max_lines(2).class("desk-k"))
            .style("height", StyleValue::px(cx.h)),
    )
}

/// Строка работы: программа, модель, ход; «стоп» — остановить.
fn activity_row(s: &AiSnap, a: &Activity, cx: &Cx, stop: bool, with_models: bool) -> Box<dyn Widget> {
    let models: Vec<String> = a.models.iter().map(|m| s.title(m)).collect();
    let mut head = format!("{} · {}", a.app.title(), op_label(&a.op));
    if with_models && !models.is_empty() {
        head.push_str(&format!(" · {}", models.join(" + ")));
    }
    let detail = match (a.state, a.step) {
        (ActivityState::Waiting, _) => t!("ждёт модель или устройство").into(),
        (_, Some((st, total))) => t!("шаг {s} из {t}", s = st, t = total).to_string(),
        _ if a.op == "generate" => t!("{n} ток · {v} ток/с", n = a.tokens, v = format!("{:.0}", a.tps)).to_string(),
        _ => String::new(),
    };
    let mut info = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(Text::new(head).max_lines(1).class("desk-ai-act"))
        .child(Text::new(detail).max_lines(1).class("desk-k"));
    if let Some((st, total)) = a.step {
        info = info.child(bar(st as f32 / total.max(1) as f32 * 100.0, (cx.w - 40.0).max(40.0), 4.0, cx.color));
    }
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(info);
    if stop {
        let id = a.id;
        row = row.child(
            GestureDetector::new()
                .on_click(move || act(move |c| c.cancel_activity(id)))
                .child(DecoratedBox::new().child(icon(STOP).class("desk-ai-btn-icon")).class("desk-ai-btn")),
        );
    }
    Box::new(row)
}

// ─── «Языковые модели» ──────────────────────────────────────────────────────

pub fn llm(cx: Cx) -> Box<dyn Widget> {
    let sig = snap();
    Box::new(rx(move || {
        let s = sig.get();
        if !s.online {
            return offline(&cx);
        }
        let tps = s.tps_now();
        let hist: Vec<f32> = s.metrics.history.iter().map(|h| h.tps).collect();
        let max = hist.iter().copied().fold(1.0f32, f32::max);
        match cx.view.as_str() {
            "line" => {
                let sub = if tps > 0.0 { t!("ток/с сейчас").to_string() } else { t!("ток/с · простаивает").to_string() };
                let tail: Vec<f32> = hist.iter().rev().take(cx.history).rev().copied().collect();
                return Box::new(line_view(&format!("{tps:.0}"), &sub, &[("", tail, cx.color)], Some((max * 1.15) as f64), &cx, |v| format!("{v:.0}")));
            }
            "text" => return Box::new(big_text(&format!("{tps:.0}"), &t!("ток/с"), &cx)),
            _ => {}
        }
        // Карточка: скорость, кто генерирует, остальные работы, недавний ответ, история.
        let talking = s.metrics.running.iter().find(|a| a.op == "generate");
        let (who, what) = match talking {
            Some(a) => (a.app.title(), a.models.first().map(|m| s.title(m)).unwrap_or_default()),
            None => (t!("Простаивает").to_string(), String::new()),
        };
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(format!("{tps:.0}")).class("desk-value"))
                .child(Text::new(t!("ток/с")).class("desk-k"))
                .child(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::End).class("grow").child(Text::new(who).max_lines(1).class("desk-ai-who")).child(Text::new(what).max_lines(1).class("desk-k"))),
        );
        let mut used = 40.0;
        if let Some(a) = talking {
            let ctx = if a.context > 0 { t!(" · контекст {n}", n = a.context).to_string() } else { String::new() };
            col = col.child(Text::new(t!("{n} ток ответа{c}", n = a.tokens, c = ctx)).max_lines(1).class("desk-k"));
            used += 18.0;
        }
        for a in s.metrics.running.iter().filter(|a| a.op != "generate").take(2) {
            if used + 44.0 > cx.h - 24.0 {
                break;
            }
            col = col.child(activity_row(&s, a, &cx, true, true));
            used += 44.0;
        }
        if talking.is_none() {
            if let Some(a) = s.metrics.recent.iter().find(|a| a.op == "generate") {
                let model = a.models.first().map(|m| s.title(m)).unwrap_or_default();
                let when = a.finished_ms.map(|f| ago(s.metrics.now_ms, f)).unwrap_or_default();
                col = col.child(
                    Text::new(t!("Последний ответ: {app} · {m} · {n} ток · {v} ток/с · {w}", app = a.app.title(), m = model, n = a.tokens, v = format!("{:.0}", a.tps), w = when))
                        .max_lines(2)
                        .class("desk-k"),
                );
                used += 34.0;
            }
        }
        let spark_h = cx.h - used - 8.0;
        if spark_h >= 24.0 {
            let tail: Vec<f32> = hist.iter().rev().take(cx.history).rev().map(|v| v / max * 100.0).collect();
            col = col.child(sparkline(tail, cx.history, cx.color, cx.w, spark_h));
        }
        Box::new(col)
    }))
}

// ─── «Модели в памяти» ──────────────────────────────────────────────────────

/// Полоса памяти устройства: всего, занято, из них моделями службы.
fn device_bar(name: String, models: u64, used: u64, total: u64, cx: &Cx) -> impl Widget {
    let p_models = models as f32 / total.max(1) as f32 * 100.0;
    let p_used = used as f32 / total.max(1) as f32 * 100.0;
    let w = cx.w;
    let color = cx.color;
    let canvas = Canvas::new(move |c, _| {
        let h = 8.0;
        c.set_color(color.with_alpha(0.14));
        c.fill_rounded_rect(0.0, 0.0, w, h, h / 2.0);
        let other = (w * (p_used / 100.0).clamp(0.0, 1.0)).max(0.0);
        if other > 0.0 {
            c.set_color(color.with_alpha(0.35));
            c.fill_rounded_rect(0.0, 0.0, other.max(h), h, h / 2.0);
        }
        let m = (w * (p_models / 100.0).clamp(0.0, 1.0)).max(0.0);
        if m > 0.0 {
            c.set_color(color);
            c.fill_rounded_rect(0.0, 0.0, m.max(h), h, h / 2.0);
        }
    })
    .size(w, 8.0);
    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(name).max_lines(1).class("desk-ai-dev"))
                .child(Text::new(t!("модели {m} · {u} из {t}", m = human_bytes(models), u = human_bytes(used), t = human_bytes(total))).max_lines(1).class("desk-k grow")),
        )
        .child(canvas)
}

fn model_row(s: &AiSnap, m: &ModelEntry, cx: &Cx) -> Box<dyn Widget> {
    let id = m.id.clone();
    let bytes = if m.device == Some(DeviceKind::Cuda) && m.vram_bytes > 0 { m.vram_bytes } else { m.mem_bytes };
    let mut sub = vec![synaptixd::precision_label(&m.precision), m.device.map(|d| d.id().to_uppercase()).unwrap_or_default()];
    if bytes > 0 {
        sub.push(human_bytes(bytes));
    }
    let apps: Vec<String> = s.metrics.apps.iter().filter(|u| u.models.contains(&m.id)).map(|u| u.app.title()).collect();
    let kv: u64 = s.metrics.kv.iter().filter(|k| k.model == m.id).map(|k| k.bytes).sum();
    let mut who = if apps.is_empty() { t!("не используется").to_string() } else { apps.join(", ") };
    if kv > 0 {
        who.push_str(&t!(" · контекст {v}", v = human_bytes(kv)));
    }
    let state = match m.state {
        LoadState::Loading => Some(t!("загружается").to_string()),
        _ => None,
    };
    let mut info = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(Text::new(m.title.clone()).max_lines(1).class("desk-ai-model"))
        .child(Text::new(state.unwrap_or_else(|| sub.join(" · "))).max_lines(1).class("desk-k"))
        .child(Text::new(who).max_lines(1).class("desk-ai-apps"));
    for a in s.metrics.running.iter().filter(|a| a.models.contains(&m.id)).take(1) {
        info = info.child(activity_row(s, a, &Cx { w: cx.w - 100.0, ..cx.clone() }, true, false));
    }
    let btn = |glyph: &'static str, on: bool, f: Box<dyn Fn() + Send + Sync>| {
        GestureDetector::new().on_click(move || f()).child(
            DecoratedBox::new()
                .child(icon(glyph).class("desk-ai-btn-icon"))
                .class(if on { "desk-ai-btn desk-ai-btn-on" } else { "desk-ai-btn" }),
        )
    };
    let pinned = m.settings.pinned;
    let (pin_id, unload_id) = (id.clone(), id.clone());
    let buttons = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(btn(
            mi::PUSH_PIN,
            pinned,
            Box::new(move || {
                let id = pin_id.clone();
                act(move |c| c.set(&id, SettingsChange { pinned: Some(!pinned), ..Default::default() }).map(|_| ()));
            }),
        ))
        .child(btn(
            mi::CLOSE,
            false,
            Box::new(move || {
                let id = unload_id.clone();
                act(move |c| c.unload(&id));
            }),
        ));
    let row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("desk-ai-badge").child(icon(kind_glyph(m.kind)).class("desk-ai-badge-icon").style("icon-color", StyleValue::Color(mss_color(cx.color)))))
        .child(info)
        .child(buttons);
    Box::new(
        GestureDetector::new()
            .on_click(move || open_settings(&format!("models/m/{id}")))
            .child(DecoratedBox::new().child(row).class(if m.busy { "desk-ai-row desk-ai-row-busy" } else { "desk-ai-row" })),
    )
}

pub fn models(cx: Cx) -> Box<dyn Widget> {
    let sig = snap();
    let dsig = data::snap();
    Box::new(rx(move || {
        let s = sig.get();
        if !s.online {
            return offline(&cx);
        }
        let d = dsig.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let loaded: Vec<&ModelEntry> = s.models.iter().filter(|m| matches!(m.state, LoadState::Loaded | LoadState::Loading)).collect();
        // Видеопамять: занято на карте всего (data) и из него — модели службы.
        let vram_models: u64 = loaded.iter().filter(|m| m.device == Some(DeviceKind::Cuda)).map(|m| m.vram_bytes).sum();
        if let Some((used, Some(total), name)) = d.vram.clone() {
            col = col.child(device_bar(short_gpu(&name), vram_models, used, total, &cx));
        }
        let ram_models: u64 = loaded.iter().filter(|m| m.device != Some(DeviceKind::Cuda)).map(|m| m.mem_bytes).sum();
        if s.status.mem_total > 0 {
            col = col.child(device_bar(t!("ОЗУ").into(), ram_models, s.status.mem_total.saturating_sub(s.status.mem_available), s.status.mem_total, &cx));
        }
        if loaded.is_empty() {
            col = col.child(Text::new(t!("Нет загруженных моделей — служба загрузит нужную при первом обращении")).max_lines(3).class("desk-k"));
        } else {
            let mut list = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            for m in loaded {
                let (s2, m2, cx2) = (s.clone(), m.clone(), cx.clone());
                list = list.child(Keyed::new(key_of(&m.id), 0, move || model_row(&s2, &m2, &cx2)));
            }
            col = col.child(ScrollView::new().vertical().child(list).class("grow desk-ai-list"));
        }
        // Работы без загруженной модели в списке (ждут загрузки) — внизу.
        let waiting: Vec<&Activity> = s.metrics.running.iter().filter(|a| a.state == ActivityState::Waiting).collect();
        for a in waiting.into_iter().take(1) {
            col = col.child(activity_row(&s, a, &cx, true, true));
        }
        Box::new(col.style("height", StyleValue::px(cx.h)))
    }))
}

/// Тап по виджету — служба и память в «Параметрах».
pub fn open_service() {
    open_settings("models/service");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Снимок виджетов с живой службой: `SHOT_DIR=… cargo test -p synshell-ui shot_ai -- --ignored`.
    #[test]
    #[ignore]
    fn shot_ai() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let cfg = synshell_common::config::Config::default();
        let mss = crate::theme::build(&cfg);
        for (kind, view, w, h) in [("llm", "card", 380.0, 190.0), ("llm", "line", 380.0, 190.0), ("ai_models", "list", 380.0, 470.0)] {
            let out = format!("{dir}/desk-{kind}-{view}.png");
            syngui::shot::render_png(
                move || {
                    let ctx = crate::ShellCtx::new(cfg_clone(), synshell_common::config::FormFactor::Desktop);
                    provide_context(ctx);
                    snap().set(fetch());
                    let vram = synsystem::gpu::devices(&synsystem::Sys::host()).into_iter().find_map(|g| Some((g.mem_used?, g.mem_total, g.name)));
                    data::snap().set(data::Snap { vram, ..Default::default() });
                    let pad = 12.0;
                    let cx = Cx { w: w - 2.0 * pad, h: h - 2.0 * pad - 28.0, color: Color::from_hex("#7aa2f7"), view: view.into(), history: 60 };
                    let body = if kind == "llm" { llm(cx) } else { models(cx) };
                    Box::new(
                        DecoratedBox::new()
                            .child(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(kind).class("desk-title").style("height", StyleValue::px(28.0))).child(body))
                            .style("width", StyleValue::px(w))
                            .style("height", StyleValue::px(h))
                            .style("padding", StyleValue::px(pad))
                            .class(format!("desk-card desk-{kind}")),
                    )
                },
                &mss,
                (w as u32, h as u32),
                2.0,
                &out,
            )
            .unwrap();
        }
    }

    fn cfg_clone() -> synshell_common::config::Config {
        synshell_common::config::Config::default()
    }
}
