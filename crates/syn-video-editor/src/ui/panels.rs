//! Панели дока: медиатека, эффекты, просмотр, инспектор, ИИ.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, icons, timeline, AiLine, St};
use crate::doc::{tc, MediaKind, Source, TrackKind};

pub fn content(st: St, panel: &str) -> Box<dyn Widget> {
    match panel {
        "media" => Box::new(media(st)),
        "effects" => Box::new(effects(st)),
        "viewer" => Box::new(viewer(st)),
        "timeline" => Box::new(timeline::view(st)),
        "inspector" => Box::new(inspector(st)),
        "ai" => Box::new(ai(st)),
        _ => Box::new(DecoratedBox::new()),
    }
}

fn empty(text: &str) -> Box<dyn Widget> {
    Box::new(Center::new().child(Text::new(text).max_lines(4).class("panel-empty")))
}

// ── медиатека ───────────────────────────────────────────────────────────

fn media(st: St) -> impl Widget {
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let items: Vec<(u64, MediaKind, String, String)> = ed()
            .project
            .media
            .iter()
            .map(|m| {
                let mut info = Vec::new();
                if m.duration > 0.0 {
                    info.push(tc(m.duration));
                }
                if m.width > 0 {
                    info.push(format!("{}×{}", m.width, m.height));
                }
                if m.fps > 0.0 {
                    info.push(format!("{} fps", m.fps));
                }
                (m.id, m.kind, m.name(), info.join(" · "))
            })
            .collect();
        if items.is_empty() {
            return vec![empty("Медиатека пуста.\nПопросите агента: «импортируй ролики из ~/Videos/…»")];
        }
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("panel-list");
        for (id, kind, name, info) in items {
            let icon = match kind {
                MediaKind::Video => icons::MOVIE,
                MediaKind::Audio => icons::AUDIO,
                MediaKind::Image => icons::IMAGE,
            };
            let row = Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class("row-icon"))
                .child(DecoratedBox::new().class("grow").child(Column::new().child(Text::new(name).max_lines(1).elide(Elide::End).class("row-title")).child(Text::new(format!("m{id} · {info}")).max_lines(1).class("row-sub"))))
                .child(ToolButton::new(icons::ADD).tooltip("На таймлайн (в конец дорожки)").on_click(move || {
                    st.exec("clip.add", json!({"media": id}));
                }));
            col = col.child(GestureDetector::new().on_double_click(move || {
                st.exec("clip.add", json!({"media": id}));
            }).child(DecoratedBox::new().class("list-row").child(row)));
        }
        vec![Box::new(col)]
    }))
}

// ── эффекты ─────────────────────────────────────────────────────────────

fn effects(st: St) -> impl Widget {
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let sel = ed().selection.clone();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("panel-list");
        col = col.child(Text::new(if sel.is_empty() { "Выделите клип на таймлайне, затем выберите эффект" } else { "Щелчок — добавить эффект выделенным клипам" }).max_lines(2).class("panel-hint"));
        for (kind, help) in crate::cmd::EFFECTS {
            let k = *kind;
            let ids: Vec<Value> = sel.iter().map(|i| json!(i)).collect();
            col = col.child(
                GestureDetector::new()
                    .on_click(move || {
                        if ids.is_empty() {
                            st.toast.set(Some("Сначала выделите клип".into()));
                        } else if k == "lut" {
                            st.dock.open("ai", Some("inspector"));
                            st.toast.set(Some("LUT: скажите агенту, какой файл .cube".into()));
                        } else {
                            st.exec("effect.add", json!({"ids": ids, "kind": k}));
                        }
                    })
                    .child(DecoratedBox::new().class("list-row").child(Column::new().child(Text::new(k).class("row-title")).child(Text::new(*help).max_lines(2).class("row-sub")))),
            );
        }
        vec![Box::new(col)]
    }))
}

// ── просмотр ────────────────────────────────────────────────────────────

/// Кадр источника для позиции (кэш по файлу и времени до 1/10 с).
fn frame(path: &str, at_frac: f64) -> Option<(u32, u32, std::sync::Arc<Vec<u8>>)> {
    static CACHE: Mutex<Option<HashMap<(String, i64), (u32, u32, std::sync::Arc<Vec<u8>>)>>> = Mutex::new(None);
    let key = (path.to_string(), (at_frac * 1000.0).round() as i64);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = c.get_or_insert_with(HashMap::new);
    if let Some(v) = map.get(&key) {
        return Some(v.clone());
    }
    let t = syngui::video::thumbnail(path, 960, at_frac).ok()?;
    let v = (t.width, t.height, std::sync::Arc::new(t.rgba));
    if map.len() > 64 {
        map.clear();
    }
    map.insert(key, v.clone());
    Some(v)
}

fn viewer(st: St) -> impl Widget {
    DecoratedBox::new().class("viewer").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let (pic, titles, aspect) = {
            let e = ed();
            let p = &e.project;
            let t = e.playhead;
            let aspect = p.settings.width as f32 / p.settings.height.max(1) as f32;
            let mut pic = None;
            let mut titles = Vec::new();
            // верхние дорожки — поверх: идём сверху вниз
            for tr in p.tracks.iter().rev().filter(|tr| tr.kind == TrackKind::Video && !tr.hidden) {
                for c in tr.clips.iter().filter(|c| c.start <= t && t < c.end()) {
                    match &c.source {
                        Source::Title { text, .. } => titles.push(text.clone()),
                        Source::Media { media } if pic.is_none() => {
                            if let Some(m) = p.media(*media) {
                                let src = c.offset + (t - c.start) * c.speed;
                                let frac = if m.duration > 0.0 { (src / m.duration).clamp(0.0, 0.999) } else { 0.0 };
                                pic = Some((m.path.display().to_string(), frac, m.kind));
                            }
                        }
                        _ => {}
                    }
                }
            }
            (pic, titles, aspect)
        };
        let mut stack = Stack::new().fit(StackFit::Expand);
        match pic {
            Some((path, frac, MediaKind::Video)) => {
                if let Some((w, h, rgba)) = frame(&path, frac) {
                    stack = stack.child(Center::new().child(Image::from_rgba(format!("frame:{path}:{frac:.3}"), w, h, (*rgba).clone()).fit(ImageFit::Contain)));
                }
            }
            Some((path, _, MediaKind::Image)) => {
                if let Ok(bytes) = std::fs::read(&path) {
                    stack = stack.child(Center::new().child(Image::from_bytes(path.clone(), bytes).fit(ImageFit::Contain)));
                }
            }
            _ => {
                stack = stack.child(Center::new().child(Text::new(format!("Кадр {}:{}", if aspect >= 1.0 { "16" } else { "9" }, if aspect >= 1.0 { "9" } else { "16" })).class("panel-empty")));
            }
        }
        if !titles.is_empty() {
            stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("viewer-title").child(Text::new(titles.join("\n")).max_lines(4).class("viewer-title-text"))));
        }
        vec![Box::new(stack)]
    }))
}

// ── инспектор ───────────────────────────────────────────────────────────

fn kv(k: &str, v: impl Into<String>) -> impl Widget {
    Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("kv-row").child(Text::new(k).class("kv-key")).child(DecoratedBox::new().class("grow").child(Text::new(v.into()).max_lines(3).class("kv-value")))
}

fn inspector(st: St) -> impl Widget {
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let e = ed();
        let p = &e.project;
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("inspector");
        let Some(id) = e.selection.first().copied() else {
            col = col.child(Text::new("ПРОЕКТ").class("section-title"));
            col = col.child(kv("Название", p.name.clone()));
            col = col.child(kv("Кадр", format!("{}×{}", p.settings.width, p.settings.height)));
            col = col.child(kv("Частота", format!("{} fps", p.settings.fps)));
            col = col.child(kv("Длительность", tc(p.duration())));
            col = col.child(kv("Медиа", p.media.len().to_string()));
            col = col.child(kv("Дорожки", p.tracks.len().to_string()));
            return vec![Box::new(col)];
        };
        let Some(c) = p.clip(id) else { return vec![] };
        col = col.child(Text::new(format!("КЛИП c{id}")).class("section-title"));
        let what = match &c.source {
            Source::Media { media } => p.media(*media).map(|m| format!("m{media} {}", m.name())).unwrap_or_default(),
            Source::Title { text, .. } => format!("титр «{text}»"),
            Source::Color { color } => format!("цвет {color}"),
        };
        col = col.child(kv("Источник", what));
        col = col.child(kv("Начало", tc(c.start)));
        col = col.child(kv("Конец", tc(c.end())));
        col = col.child(kv("Длительность", tc(c.duration)));
        col = col.child(kv("Из источника с", tc(c.offset)));
        col = col.child(kv("Скорость", format!("{}×", c.speed)));
        col = col.child(kv("Громкость", format!("{:.0} %", c.volume * 100.0)));
        if !c.effects.is_empty() {
            col = col.child(Text::new("ЭФФЕКТЫ").class("section-title"));
            for ef in &c.effects {
                let eid = ef.id;
                let params = if ef.params.is_empty() { String::new() } else { Value::Object(ef.params.clone()).to_string() };
                col = col.child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").child(kv(&ef.kind, params)))
                        .child(ToolButton::new("\u{E5CD}").tooltip("Убрать эффект").on_click(move || {
                            st.exec("effect.remove", json!({"id": eid}));
                        })),
                );
            }
        }
        vec![Box::new(col)]
    }))
}

// ── ИИ ──────────────────────────────────────────────────────────────────

fn ai(st: St) -> impl Widget {
    let input_gen = use_signal(0u64);
    let draft = use_signal(String::new());
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ai-panel")
        .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().follow_end(true).child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let log = st.ai_log.get();
            if log.is_empty() {
                return vec![empty("Агент монтирует сам: импорт, нарезка, титры, эффекты.\nНапример: «собери ролик на 30 секунд из ~/Videos/Test, в начале титр»")];
            }
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("ai-log");
            for l in log {
                let (class, text) = match l {
                    AiLine::User(t) => ("ai-user", t),
                    AiLine::Text(t) => ("ai-text", t),
                    AiLine::Tool(t) => ("ai-tool", format!("→ {t}")),
                    AiLine::Result(t) => ("ai-result", t),
                    AiLine::Error(t) => ("ai-error", t),
                };
                col = col.child(DecoratedBox::new().class(class).child(Text::new(text).selectable(true).class(format!("{class}-text"))));
            }
            vec![Box::new(col)]
        }))))
        .child(DecoratedBox::new().class("ai-input").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let _ = input_gen.get();
            let busy = st.ai_busy.get();
            vec![Box::new(Row::new().child(DecoratedBox::new().class("grow").child(
                TextField::new()
                    .text(draft.get_untracked())
                    .on_change(move |s: &str| draft.set(s.to_string()))
                    .placeholder(if busy { "Агент работает…" } else { "Задача для агента… (Enter)" })
                    .prefix_icon(icons::AI)
                    .disabled(busy)
                    .on_submit(move |q: &str| {
                        st.ask_ai(q.to_string());
                        draft.set(String::new());
                        input_gen.update(|g| *g += 1);
                    })
                    .class("ai-field"),
            )))]
        })))
}
