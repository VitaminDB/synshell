//! Панели дока: медиатека, эффекты, просмотр, инспектор, ИИ.


use std::sync::Mutex;

use serde_json::{json, Value};
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::containers::Positioned;

use super::{ed, icons, timeline, AiLine, Pick, St};
use syngui::widgets::overlay::MenuItem;
use crate::doc::{tc, MediaKind, Source};

pub fn content(st: St, panel: &str) -> Box<dyn Widget> {
    match panel {
        "media" => Box::new(media(st)),
        "effects" => Box::new(effects(st)),
        "viewer" => Box::new(viewer(st)),
        "timeline" => Box::new(timeline::view(st)),
        "inspector" => Box::new(inspector(st)),
        "ai" => Box::new(ai(st)),
        "graph" => super::graph::panel(),
        _ => Box::new(DecoratedBox::new()),
    }
}

fn empty(text: &str) -> Box<dyn Widget> {
    Box::new(Center::new().child(Text::new(text).max_lines(4).class("panel-empty")))
}

// ── медиатека ───────────────────────────────────────────────────────────

fn media(st: St) -> impl Widget {
    let header = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("panel-head")
        .child(Button::new("Импорт…").on_click(move || st.dialog(Pick::ImportFiles)).class("head-btn"))
        .child(Button::new("Папка…").on_click(move || st.dialog(Pick::ImportFolder)).class("head-btn"))
        .child(DecoratedBox::new().class("grow"))
        .child(Icon::new(icons::IMAGE).class("head-icon"))
        .child(Slider::new().range(90.0, 360.0).value(st.tile_size.get_untracked()).width(110.0).on_change(move |v| st.tile_size.set(v)));
    let list = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let _ = st.thumbs.get();
        let tile_size = st.tile_size.get();
        let items: Vec<(u64, MediaKind, String, String, std::path::PathBuf, f64, f32)> = ed()
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
                let aspect = if m.width > 0 && m.height > 0 { m.width as f32 / m.height as f32 } else { 16.0 / 9.0 };
                (m.id, m.kind, m.name(), info.join(" · "), m.path.clone(), m.duration, aspect)
            })
            .collect();
        if items.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("media-empty")
                    .child(Icon::new(icons::MOVIE).class("media-empty-icon"))
                    .child(Text::new("Медиатека пуста").class("row-title"))
                    .child(Button::new("Импорт медиа…").on_click(move || st.dialog(Pick::ImportFiles)).class("primary-btn"))
                    .child(Text::new("Перетащите сюда файлы или папку, нажмите Ctrl+I\nили попросите агента: «импортируй ролики из ~/Videos/…»").max_lines(3).class("panel-empty")),
            )];
        }
        // масонская раскладка: плитки по пропорциям роликов, ширина колонки — ползунком
        let mut grid = Grid::new(16).min_column_width(tile_size).gap(8.0).masonry(true);
        for (id, kind, name, info, path, dur, aspect) in items {
            grid = grid.child(tile(st, id, kind, name, info, path, dur, aspect));
        }
        vec![Box::new(DecoratedBox::new().child(grid).class("panel-list"))]
    }));
    let panel_menu = vec![MenuItem::new("import", "Импорт медиа…"), MenuItem::new("import_folder", "Импорт папки…")];
    ContextMenu::new()
        .items(panel_menu)
        .on_select(move |a: &str| match a {
            "import" => st.dialog(Pick::ImportFiles),
            "import_folder" => st.dialog(Pick::ImportFolder),
            _ => {}
        })
        .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(header).child(DecoratedBox::new().class("grow").child(list)))
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

/// Кадры для просмотра: читатели клипов живут между кадрами (воспроизведение
/// декодирует подряд), размер — до 1280 по большей стороне.
fn preview_frames(p: &crate::doc::Project, t: f64) -> crate::render::compose::Frames {
    static SRC: Mutex<Option<crate::render::compose::FrameSource>> = Mutex::new(None);
    let mut s = SRC.lock().unwrap_or_else(|e| e.into_inner());
    s.get_or_insert_with(|| crate::render::compose::FrameSource::new(1280, 1280)).frames(p, t)
}

/// Просмотр — кадр композитора (тот же, что при экспорте) в масштабе панели.
fn viewer(st: St) -> impl Widget {
    let area = use_signal(syngui::prelude::Size::new(640.0, 360.0));
    EventHook::new().fill().on_resize(move |s| area.set(s)).child(DecoratedBox::new().class("viewer").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let a = area.get();
        let (p, t) = {
            let e = ed();
            (e.project.clone(), e.playhead)
        };
        let k = ((a.width - 16.0) / p.settings.width as f32).min((a.height - 16.0) / p.settings.height as f32).max(0.02);
        let frames = preview_frames(&p, t);
        let frame = crate::render::compose::frame_widget(&p, t, &frames, k);
        vec![Box::new(Center::new().child(Column::new().children(vec![frame])))]
    })))
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

/// Тип переноса «медиа из медиатеки» (payload — id медиа).
pub const DRAG_MEDIA: &str = "syn-video-editor/media";

/// Плитка медиатеки: превью (ведение курсора — промотка по ленте кадров),
/// длительность, имя; двойной щелчок — на таймлайн, правая кнопка — меню.
#[allow(clippy::too_many_arguments)]
fn tile(st: St, id: u64, kind: MediaKind, name: String, info: String, path: std::path::PathBuf, dur: f64, aspect: f32) -> Box<dyn Widget> {
    let drag_label = name.clone();
    let hover = use_signal(None::<f32>);
    let width = use_signal(150.0f32);
    // ширина плитки — из события наведения (on_resize в сетке приходит не всегда)
    let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let frac = hover.get();
        let mut stack = Stack::new().fit(StackFit::Expand);
        match (kind, super::thumbs::strip(&path)) {
            (MediaKind::Audio, _) => {
                stack = stack.child(Center::new().child(Icon::new(icons::AUDIO).class("tile-icon")));
            }
            (_, Some(strip)) => {
                let i = frac.map(|f| ((f * super::thumbs::FRAMES as f32) as usize).min(super::thumbs::FRAMES - 1)).unwrap_or(super::thumbs::FRAMES / 3);
                let f = &strip[i];
                stack = stack.child(Center::new().child(Image::from_rgba_shared(format!("thumb:{id}:{i}"), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false)));
            }
            (_, None) => {
                stack = stack.child(Center::new().child(Icon::new(if kind == MediaKind::Image { icons::IMAGE } else { icons::MOVIE }).class("tile-icon")));
            }
        }
        if dur > 0.0 {
            let label = match frac {
                Some(f) => format!("{} / {}", tc(dur * f as f64), tc(dur)),
                None => tc(dur),
            };
            stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::End).child(DecoratedBox::new().class("tile-badge").child(Text::new(label).class("tile-badge-text"))));
        }
        if let Some(f) = frac.filter(|_| dur > 0.0) {
            // полоска позиции промотки
            let w = width.get_untracked();
            stack = stack.child(Positioned::new(DecoratedBox::new().class("tile-scrub")).at((f * w - 1.0).max(0.0), 0.0).dimensions(2.0, 4000.0));
        }
        vec![Box::new(stack)]
    });
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(EventHook::new().fill().on_resize(move |s| width.set(s.width)).on_hover_at(move |p| {
            if let Some((_, s)) = p {
                width.set(s.width);
            }
            hover.set(p.map(|(p, s)| (p.x / s.width.max(1.0)).clamp(0.0, 0.999)))
        }).child(AspectRatio::new(aspect.clamp(0.3, 3.0)).child(DecoratedBox::new().class("tile-thumb").child(thumb))))
        .child(Text::new(name).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(format!("m{id} · {info}")).max_lines(1).elide(Elide::End).class("row-sub"));
    let menu = vec![
        MenuItem::new("add", "На таймлайн (в конец)"),
        MenuItem::new("add_here", "На таймлайн в позицию"),
        MenuItem::separator(),
        MenuItem::new("import", "Импорт медиа…"),
        MenuItem::new("import_folder", "Импорт папки…"),
        MenuItem::separator(),
        MenuItem::new("remove", "Убрать из медиатеки"),
    ];
    Box::new(
        ContextMenu::new()
            .items(menu)
            .on_select(move |a: &str| match a {
                "add" => {
                    st.exec("clip.add", json!({"media": id}));
                }
                "add_here" => {
                    let at = ed().playhead;
                    st.exec("clip.add", json!({"media": id, "at": at}));
                }
                "import" => st.dialog(Pick::ImportFiles),
                "import_folder" => st.dialog(Pick::ImportFolder),
                "remove" => {
                    st.exec("media.remove", json!({"id": id}));
                }
                _ => {}
            })
            // тащить на дорожку таймлайна; двойной щелчок — в конец дорожки
            .child(Draggable::new(DRAG_MEDIA, id.to_string()).label(drag_label.clone()).on_double_click(move || {
                st.exec("clip.add", json!({"media": id}));
            }).child(DecoratedBox::new().class("tile").child(body))),
    )
}
