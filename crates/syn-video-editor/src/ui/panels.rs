//! Панели дока: медиатека, эффекты, просмотр, инспектор, ИИ.


use std::sync::Mutex;

use serde_json::json;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::containers::Positioned;

use super::{ed, icons, timeline, AiLine, Pick, St};
use syngui::widgets::overlay::MenuItem;
use crate::doc::{tc, MediaKind};

pub fn content(st: St, panel: &str) -> Box<dyn Widget> {
    match panel {
        "media" => Box::new(media(st)),
        "templates" => Box::new(super::templates::view(st)),
        "effects" => Box::new(effects(st)),
        "luts" => Box::new(super::luts::view(st)),
        "layers" => Box::new(layers(st)),
        "viewer" => Box::new(viewer(st)),
        "timeline" => Box::new(timeline::view(st)),
        "inspector" => Box::new(super::inspector::view(st)),
        "ai" => Box::new(ai(st)),
        "graph" => super::graph::panel(),
        "curves" => Box::new(super::curves::view(st)),
        "color" => Box::new(super::color::view(st)),
        "media_view" => Box::new(media_view_panel(st)),
        "projects" => Box::new(super::projects::panel(st)),
        _ => Box::new(DecoratedBox::new()),
    }
}

fn empty(text: &str) -> Box<dyn Widget> {
    Box::new(Center::new().child(Text::new(text).max_lines(4).class("panel-empty")))
}

// ── медиатека ───────────────────────────────────────────────────────────

thread_local! {
    static MEDIA_FILTER: std::cell::RefCell<Option<RwSignal<Option<MediaKind>>>> = const { std::cell::RefCell::new(None) };
}

/// Вид файлов, показанных в медиатеке (`None` — все).
fn media_filter() -> RwSignal<Option<MediaKind>> {
    MEDIA_FILTER.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(None)))
}

thread_local! {
    static DOCS_ONLY: std::cell::RefCell<Option<RwSignal<bool>>> = const { std::cell::RefCell::new(None) };
}

/// В медиатеке — только документы проекта.
fn docs_only() -> RwSignal<bool> {
    DOCS_ONLY.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(false)))
}

/// Плитка документа проекта (Markdown): двойной щелчок — вкладка «Медиа» с правкой.
fn doc_tile(st: St, path: std::path::PathBuf) -> Box<dyn Widget> {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string();
    let info = format!("документ · {} строк", text.lines().count());
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb doc-thumb").child(Center::new().child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::DESCRIPTION).class("tile-icon")).child(Text::new(first).max_lines(2).elide(Elide::End).class("doc-thumb-text"))))))
        .child(Text::new(format!("{name}.md")).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(info).max_lines(1).class("row-sub"));
    Box::new(GestureDetector::new().on_double_click(move || open_doc(st, path.clone())).child(DecoratedBox::new().class("tile").child(body)))
}

/// Открыть документ проекта во вкладке «Медиа».
pub fn open_doc(st: St, path: std::path::PathBuf) {
    st.media_preview.set(None);
    st.fx_preview.set(None);
    st.doc_preview.set(Some(path));
    st.open_panel("media_view");
    st.dock.activate("media_view");
}

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
    // папка проекта: где лежат все его файлы — путь, открыть, перенести, переименовать
    let folder = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let Some(path) = ed().path.clone() else { return vec![] };
        let dir = crate::store::media_dir(&path);
        let home = std::env::var("HOME").unwrap_or_default();
        let shown = dir.display().to_string();
        let shown = if !home.is_empty() && shown.starts_with(&home) { format!("~{}", &shown[home.len()..]) } else { shown };
        let open_dir = dir.clone();
        vec![Box::new(
            DecoratedBox::new().class("media-folder").child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::FOLDER_OPEN).class("head-icon"))
                    .child(DecoratedBox::new().class("grow").child(Text::new(shown).max_lines(1).elide(Elide::Middle).class("row-sub")))
                    .child(ToolButton::new(icons::FOLDER_OPEN).tooltip("Открыть папку проекта").on_click(move || {
                        let _ = std::fs::create_dir_all(&open_dir);
                        let _ = std::process::Command::new("xdg-open").arg(&open_dir).spawn();
                    }))
                    .child(ToolButton::new(icons::EDIT).tooltip("Переименовать проект (и папку)").on_click(move || st.run_menu("project.rename")))
                    .child(ToolButton::new("\u{E2C7}").tooltip("Перенести папку проекта…").on_click(move || {
                        std::thread::spawn(move || {
                            let start = crate::store::projects_dir();
                            if let Some(d) = rfd::FileDialog::new().set_title("Куда перенести папку проекта").set_directory(start).pick_folder() {
                                syngui::async_runtime::run_on_main_thread(move || {
                                    st.exec("project.move", json!({"dir": d.display().to_string()}));
                                });
                            }
                        });
                    })),
            ),
        )]
    });
    let kind_filter = media_filter();
    let docs_flag = docs_only();
    // чипы-фильтры по виду файлов: «Все» и виды, которые есть в медиатеке
    let chips = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let cur = kind_filter.get();
        let e = ed();
        let n = |k: MediaKind| e.project.media.iter().filter(|m| m.kind == k).count();
        let total = e.project.media.len();
        if total == 0 && crate::cmd::doc::list_docs(e.path.as_deref()).is_empty() {
            return vec![];
        }
        let docs = crate::cmd::doc::list_docs(e.path.as_deref()).len();
        let only_docs = docs_flag.get();
        let mut row = Flex::row().wrap().gap(6.0);
        row = row.child(Chip::new(format!("Все · {}", total + docs)).selected(cur.is_none() && !only_docs).on_click(move || {
            kind_filter.set(None);
            docs_flag.set(false);
        }));
        for (k, title, icon) in [(MediaKind::Video, "Видео", icons::MOVIE), (MediaKind::Image, "Картинки", icons::IMAGE), (MediaKind::Audio, "Звук", icons::AUDIO), (MediaKind::Lottie, "Анимация", icons::SHAPE)] {
            let c = n(k);
            if c == 0 {
                continue;
            }
            row = row.child(Chip::new(format!("{title} · {c}")).icon(icon).selected(cur == Some(k) && !only_docs).on_click(move || {
                docs_flag.set(false);
                kind_filter.set(if kind_filter.get_untracked() == Some(k) { None } else { Some(k) });
            }));
        }
        if docs > 0 {
            row = row.child(Chip::new(format!("Документы · {docs}")).icon(icons::DESCRIPTION).selected(only_docs).on_click(move || {
                kind_filter.set(None);
                docs_flag.set(!docs_flag.get_untracked());
            }));
        }
        vec![Box::new(DecoratedBox::new().class("media-chips").child(row))]
    });
    let list = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let _ = st.thumbs.get();
        let tile_size = st.tile_size.get();
        let only = kind_filter.get();
        let only_docs = docs_flag.get();
        let docs: Vec<std::path::PathBuf> = if only.is_none() { crate::cmd::doc::list_docs(ed().path.as_deref()) } else { Vec::new() };
        let items: Vec<(u64, MediaKind, String, String, std::path::PathBuf, f64, f32)> = ed()
            .project
            .media
            .iter()
            .filter(|m| !only_docs && only.is_none_or(|k| m.kind == k))
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
        let gen = st.gen_tiles.get();
        if items.is_empty() && gen.is_empty() && docs.is_empty() {
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
        // выделение: убрать исчезнувшие файлы
        let ids_now: Vec<u64> = items.iter().map(|i| i.0).collect();
        if st.media_sel.get_untracked().iter().any(|i| !ids_now.contains(i)) {
            st.media_sel.update(|v| v.retain(|i| ids_now.contains(i)));
        }
        let _ = st.media_sel.get();
        // масонская раскладка: плитки по пропорциям роликов, ширина колонки — ползунком
        let mut grid = Grid::new(16).min_column_width(tile_size).gap(8.0).masonry(true);
        // что агент генерирует прямо сейчас — плитками с ходом, до готового файла
        let label = st.gen_label.get();
        for g in gen {
            let frac = if g.total > 0 { g.step as f32 / g.total as f32 } else { 0.0 };
            let what = match g.op.as_str() { "image" => "картинка", "video" => "ролик", "music" => "музыка", "speak" => "голос", _ => "звук" };
            let item = g.item.map(|(k, n)| format!(" {k} из {n}")).unwrap_or_default();
            let speech = g.op == "speak";
            let step = if g.total > 0 { format!("шаг {}/{}", g.step, g.total) } else if speech { "синтез речи…".into() } else { "подготовка…".into() };
            // место в фильме от агента («Сцена 3 из 12: …») — вместо «ролик 1 из 1»
            let title = match (&label, g.item) {
                (Some(l), Some((k, n))) if n > 1 => format!("{l} · {what} {k} из {n}"),
                (Some(l), _) => l.clone(),
                (None, _) => format!("Генерируется {what}{item}"),
            };
            let bar = if g.total == 0 { ProgressBar::with_value(0.0).indeterminate() } else { ProgressBar::with_value(frac) };
            grid = grid.child(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(AspectRatio::new(16.0 / 9.0).child(match &g.preview {
                        // превью по ходу: картинка проявляется на плитке
                        Some(p) => Box::new(DecoratedBox::new().class("tile-thumb gen-thumb").child(Image::new(p.display().to_string()).fit(ImageFit::Cover))) as Box<dyn Widget>,
                        None => Box::new(DecoratedBox::new().class("tile-thumb gen-thumb").child(Center::new().child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(match g.op.as_str() { "video" => icons::MOVIE, "speak" | "music" => icons::AUDIO, _ => icons::IMAGE }).class("media-empty-icon")).child(Text::new(step.clone()).class("row-sub"))))),
                    }))
                    .child(Text::new(step).class("row-sub"))
                    .child(bar.class("gen-bar"))
                    .child(Text::new(title).max_lines(2).elide(Elide::End).class("row-title"))
                    .child(Text::new(g.node.clone()).max_lines(1).elide(Elide::End).class("row-sub"))
                    .class("tile gen-tile"),
            );
        }
        // документы проекта (сценарий, диалоги) — первыми
        for d in docs {
            grid = grid.child(doc_tile(st, d));
        }
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
        .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(header).child(folder).child(chips).child(DecoratedBox::new().class("grow").child(list)))
}

// ── слои (вид «Моушн») ─────────────────────────────────────────────────

struct LayerRow {
    cid: u64,
    tid: u64,
    track_name: String,
    hidden: bool,
    depth: usize,
    badge: &'static str,
    label: String,
    blend: String,
    keys: usize,
    has_mask: bool,
    selected: bool,
}

/// Видеослои сверху вниз с вложенностью «родитель → потомок».
fn collect_layers() -> Vec<LayerRow> {
    let e = ed();
    let p = &e.project;
    let mut flat: Vec<(u64, u64, String, bool, Option<u64>, &'static str, String, String, usize, bool)> = Vec::new();
    for tr in p.tracks.iter().rev().filter(|t| t.kind == crate::doc::TrackKind::Video) {
        for c in &tr.clips {
            let (badge, title) = match &c.source {
                crate::doc::Source::Title { text, .. } => ("T", text.lines().next().unwrap_or("Титр").to_string()),
                crate::doc::Source::Shape { kind, .. } => ("◆", crate::doc::shape_title(kind).to_string()),
                crate::doc::Source::Color { color } => ("■", color.clone()),
                crate::doc::Source::Media { media } => {
                    let m = p.media(*media);
                    let b = if m.is_some_and(|x| x.kind == MediaKind::Image) { "🖼" } else { "▶" };
                    (b, m.map(|x| x.name()).unwrap_or_else(|| format!("m{media}")))
                }
            };
            let label = if c.name.is_empty() { format!("c{} {title}", c.id) } else { format!("c{} {}", c.id, c.name) };
            let keys = c.keys.values().map(Vec::len).sum();
            let has_mask = c.effects.iter().any(|ef| ef.kind == "mask");
            flat.push((c.id, tr.id, tr.name.clone(), tr.hidden, c.parent, badge, label, c.blend.clone(), keys, has_mask));
        }
    }
    let ids: std::collections::HashSet<u64> = flat.iter().map(|x| x.0).collect();
    let mut out = Vec::with_capacity(flat.len());
    let mut visited = std::collections::HashSet::new();
    fn visit(
        parent: Option<u64>,
        depth: usize,
        flat: &[(u64, u64, String, bool, Option<u64>, &'static str, String, String, usize, bool)],
        ids: &std::collections::HashSet<u64>,
        sel: &[u64],
        visited: &mut std::collections::HashSet<u64>,
        out: &mut Vec<LayerRow>,
    ) {
        for item in flat {
            let eff_parent = item.4.filter(|pid| ids.contains(pid));
            if eff_parent != parent || !visited.insert(item.0) {
                continue;
            }
            out.push(LayerRow {
                cid: item.0,
                tid: item.1,
                track_name: item.2.clone(),
                hidden: item.3,
                depth,
                badge: item.5,
                label: item.6.clone(),
                blend: item.7.clone(),
                keys: item.8,
                has_mask: item.9,
                selected: sel.contains(&item.0),
            });
            if depth < 8 {
                visit(Some(item.0), depth + 1, flat, ids, sel, visited, out);
            }
        }
    }
    visit(None, 0, &flat, &ids, &e.selection, &mut visited, &mut out);
    // на случай нестандартных связей — добрать не попавшие
    for item in &flat {
        if visited.insert(item.0) {
            out.push(LayerRow {
                cid: item.0,
                tid: item.1,
                track_name: item.2.clone(),
                hidden: item.3,
                depth: 0,
                badge: item.5,
                label: item.6.clone(),
                blend: item.7.clone(),
                keys: item.8,
                has_mask: item.9,
                selected: e.selection.contains(&item.0),
            });
        }
    }
    out
}

fn layers(st: St) -> impl Widget {
    let add_btn = move |label: &'static str, f: fn(St)| Button::new(label).on_click(move || f(st)).class("head-btn");
    let header = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("panel-head")
        .child(add_btn("+ Титр", |st| {
            if let Some(o) = st.exec("title.add", json!({"text": "Заголовок"})) {
                if let Some(id) = o.data.get("id").and_then(|v| v.as_u64()) {
                    st.exec("selection.set", json!({"ids": [id]}));
                }
            }
        }))
        .child(add_btn("+ Фигура", |st| {
            if let Some(o) = st.exec("shape.add", json!({"kind": "rect"})) {
                if let Some(id) = o.data.get("id").and_then(|v| v.as_u64()) {
                    st.exec("selection.set", json!({"ids": [id]}));
                }
            }
        }))
        .child(add_btn("+ Звезда", |st| {
            if let Some(o) = st.exec("shape.add", json!({"kind": "star"})) {
                if let Some(id) = o.data.get("id").and_then(|v| v.as_u64()) {
                    st.exec("selection.set", json!({"ids": [id]}));
                }
            }
        }))
        .child(add_btn("+ Маска", |st| {
            let sel = ed().selection.clone();
            if sel.is_empty() {
                st.toast.set(Some("Сначала выберите слой".into()));
            } else {
                st.exec("effect.add", json!({"ids": sel, "kind": "mask"}));
            }
        }));
    let list = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let items = collect_layers();
        if items.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("media-empty")
                    .child(Text::new("Нет видеослоёв").class("row-title"))
                    .child(Text::new("Добавьте титр, фигуру или перетащите видео на таймлайн").max_lines(2).class("panel-empty")),
            )];
        }
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("panel-list");
        for item in items {
            let cid = item.cid;
            let tid = item.tid;
            let hidden = item.hidden;
            let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if item.depth > 0 {
                row = row.child(DecoratedBox::new().style("width", (item.depth * 14) as f32)).child(Text::new("↳").class("row-sub"));
            }
            row = row
                .child(Text::new(item.badge).class("layer-kind"))
                .child(DecoratedBox::new().class("grow").child(Text::new(item.label).max_lines(1).elide(Elide::End).class("row-title")));
            if item.has_mask {
                row = row.child(Text::new("◯").class("layer-badge"));
            }
            if !item.blend.is_empty() && item.blend != "normal" {
                row = row.child(Text::new(item.blend).class("layer-badge"));
            }
            if item.keys > 0 {
                row = row.child(Text::new(format!("◈{}", item.keys)).class("layer-key"));
            }
            row = row
                .child(Text::new(item.track_name).class("row-sub"))
                .child(
                    ToolButton::new(if hidden { icons::HIDDEN } else { icons::VISIBLE })
                        .tooltip(if hidden { "Показать дорожку" } else { "Скрыть дорожку" })
                        .on_click(move || {
                            st.exec("track.set", json!({"id": tid, "hidden": !hidden}));
                        }),
                );
            let cls = format!("layer-row{}{}", if item.selected { " layer-row-on" } else { "" }, if hidden { " layer-row-off" } else { "" });
            let item_w = GestureDetector::new()
                .on_click(move || {
                    st.exec("selection.set", json!({"ids": [cid]}));
                })
                .child(DecoratedBox::new().class(cls).child(row));
            col = col.child(item_w);
        }
        vec![Box::new(col)]
    }));
    Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(header).child(DecoratedBox::new().class("grow").child(list))
}

// ── эффекты и переходы ──────────────────────────────────────────────────

/// Подпись эффекта для людей.
fn fx_title(kind: &str) -> &'static str {
    match kind {
        "fade_in" => "Проявление",
        "fade_out" => "Затухание",
        "audio_fade_in" => "Нарастание звука",
        "audio_fade_out" => "Затухание звука",
        "lut" => "LUT",
        "grade" => "Цветокоррекция",
        "color" => "Цвет",
        "blur" => "Размытие",
        "sharpen" => "Резкость",
        "vignette" => "Виньетка",
        "transform" => "Положение и масштаб",
        "mask" => "Маска",
        "glass" => "Матовое стекло",
        "upscale" => "Нейроувеличение",
        "interpolate" => "Плавность RIFE",
        _ => "Эффект",
    }
}

/// Плитки движений, переходов и эффектов: превью — лента того же композитора,
/// ведение курсора — промотка; щелчок — применить к выделенному клипу.
fn effects(st: St) -> impl Widget {
    super::thumbs::ensure_fx_strips();
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let _ = st.rev.get();
        let size = st.tile_size.get();
        let has_sel = !ed().selection.is_empty();
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("panel-list");
        col = col.child(Text::new(if has_sel { "Щелчок — применить к выделенному клипу; ведите курсор по плитке для превью" } else { "Выделите клип на таймлайне, затем щёлкните движение, переход или эффект" }).max_lines(2).class("panel-hint"));
        for (section, prefix, kind) in [("ДВИЖЕНИЕ", "motion:", FxKind::Motion), ("ПЕРЕХОДЫ", "transition:", FxKind::Transition), ("ЭФФЕКТЫ", "effect:", FxKind::Effect)] {
            col = col.child(Text::new(section).class("section-title"));
            let mut grid = Grid::new(16).min_column_width(size).gap(8.0);
            for (key, label) in crate::render::preview::kinds().into_iter().filter(|(k, _)| k.starts_with(prefix)) {
                let name = key[prefix.len()..].to_string();
                let title = if kind == FxKind::Effect { fx_title(&name).to_string() } else { label };
                grid = grid.child(fx_tile(st, key, name, title, kind));
            }
            col = col.child(grid);
        }
        col = col.child(Text::new("БЛОКИ").class("section-title"));
        let mut grid = Grid::new(16).min_column_width(size).gap(8.0);
        for b in crate::blocks::all() {
            grid = grid.child(block_tile(st, b));
        }
        col = col.child(grid);
        vec![Box::new(col)]
    }))
}

/// Превью блока на демо-кадре (кэш по имени и содержимому).
fn block_thumb(b: &crate::blocks::Block) -> syngui::video::RgbaFrame {
    use std::collections::HashMap;
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, syngui::video::RgbaFrame>>> = std::sync::OnceLock::new();
    let key = format!("{}:{}", b.name, serde_json::to_string(&b.graph).unwrap_or_default());
    let m = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = m.lock().unwrap().get(&key) {
        return f.clone();
    }
    let base = super::luts::demo();
    let params = crate::blocks::node_params(b, None);
    let out = match crate::blocks::inner(&params) {
        Some(g) => {
            let im = g.eval(base.width, base.height, &base.rgba, base.height as f32 / 1080.0, &mut |_, _| None);
            syngui::video::RgbaFrame { width: im.width, height: im.height, pts: 0.0, rgba: std::sync::Arc::new(im.to_rgba8()) }
        }
        None => base,
    };
    m.lock().unwrap().insert(key, out.clone());
    out
}

fn block_tile(st: St, b: crate::blocks::Block) -> Box<dyn Widget> {
    let f = block_thumb(&b);
    let name = b.name.clone();
    let img = Image::from_rgba_shared(format!("block:{}:{:x}", b.name, std::sync::Arc::as_ptr(&f.rgba) as usize), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false);
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb").child(img)))
        .child(Text::new(b.name.clone()).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(if b.builtin { b.description.clone() } else { format!("свой · {}", b.description) }).max_lines(1).elide(Elide::End).class("row-sub"));
    Box::new(GestureDetector::new().on_click(move || {
        if ed().selection.is_empty() {
            st.toast.set(Some("Сначала выделите клип на таймлайне".into()));
            return;
        }
        st.exec("block.apply", json!({"name": name}));
        st.dock.activate("inspector");
    }).child(DecoratedBox::new().class("tile").child(body)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FxKind {
    Motion,
    Transition,
    Effect,
}

fn fx_tile(st: St, key: String, name: String, title: String, kind: FxKind) -> Box<dyn Widget> {
    let hover = use_signal(None::<f32>);
    let width = use_signal(150.0f32);
    let path = crate::render::preview::strip_path(&key);
    let key2 = key.clone();
    let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let frac = hover.get();
        let mut stack = Stack::new().fit(StackFit::Expand);
        match super::thumbs::strip_file(&path) {
            Some(strip) if !strip.is_empty() => {
                let n = strip.len();
                // без наведения — показательный кадр в разгаре движения/перехода/эффекта
                let idle = if kind == FxKind::Motion { n / 3 } else { n / 2 };
                let i = frac.map(|f| ((f * n as f32) as usize).min(n - 1)).unwrap_or(idle);
                let f = &strip[i];
                stack = stack.child(Image::from_rgba_shared(format!("fx:{key2}:{i}"), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false));
            }
            _ => {
                let glyph = match kind {
                    FxKind::Motion => "\u{E922}",
                    FxKind::Transition => "\u{E8D4}",
                    FxKind::Effect => "\u{E3A1}",
                };
                stack = stack.child(Center::new().child(Icon::new(glyph).class("tile-icon")));
            }
        }
        if let Some(f) = frac {
            let w = width.get_untracked();
            stack = stack.child(Positioned::new(DecoratedBox::new().class("tile-scrub")).at((f * w - 1.0).max(0.0), 0.0).dimensions(2.0, 4000.0));
        }
        vec![Box::new(stack)]
    });
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(EventHook::new().fill().on_hover_at(move |p| {
            if let Some((_, s)) = p {
                width.set(s.width);
            }
            hover.set(p.map(|(p, s)| (p.x / s.width.max(1.0)).clamp(0.0, 0.999)))
        }).child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb").child(thumb))))
        .child(Text::new(title.clone()).max_lines(1).elide(Elide::End).class("row-title"));
    let (pkey, ptitle) = (key.clone(), title.clone());
    // щелчок применяет с задержкой двойного щелчка: двойной — только просмотр
    // во вкладке «Медиа», без применения к выделенному клипу
    let click = move || {
        let at = std::time::Instant::now();
        let name = name.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(FX_CLICK_DELAY_MS));
            let dbl = *FX_DBL.lock().unwrap_or_else(|e| e.into_inner());
            if dbl.is_some_and(|d| d + std::time::Duration::from_millis(700) > at) {
                return;
            }
            syngui::async_runtime::run_on_main_thread(move || fx_apply(st, kind, &name));
        });
    };
    Box::new(
        GestureDetector::new()
            .on_click(click)
            .on_double_click(move || {
                *FX_DBL.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                open_fx_preview(st, pkey.clone(), ptitle.clone());
            })
            .child(DecoratedBox::new().class("tile").child(body)),
    )
}

const FX_CLICK_DELAY_MS: u64 = 380;
static FX_DBL: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Показать движение, переход или эффект вкладкой «Медиа» (лента по кругу).
pub fn open_fx_preview(st: St, key: String, title: String) {
    st.media_preview.set(None);
    st.doc_preview.set(None);
    st.fx_preview.set(Some((key, title)));
    st.open_panel("media_view");
    st.dock.activate("media_view");
}

fn fx_kind(key: &str) -> Option<(FxKind, &str)> {
    for (prefix, kind) in [("motion:", FxKind::Motion), ("transition:", FxKind::Transition), ("effect:", FxKind::Effect)] {
        if let Some(n) = key.strip_prefix(prefix) {
            return Some((kind, n));
        }
    }
    None
}

/// Применить движение/переход/эффект к выделенным клипам.
fn fx_apply(st: St, kind: FxKind, name: &str) {
    let name = name.to_string();
    {
        let sel = ed().selection.clone();
        if sel.is_empty() {
            st.toast.set(Some("Сначала выделите клип на таймлайне".into()));
            return;
        }
        // при выделенной связанной паре [видео, звук] движения и видеопереходы вешаем на видеоклип
        let video_sel = || {
            let e = ed();
            let v: Vec<u64> = sel
                .iter()
                .copied()
                .filter(|id| e.project.find_clip(*id).is_some_and(|(ti, _)| e.project.tracks[ti].kind == crate::doc::TrackKind::Video))
                .collect();
            if v.is_empty() { sel.clone() } else { v }
        };
        match kind {
            FxKind::Motion => {
                st.exec("motion.apply", json!({"preset": name, "ids": video_sel()}));
            }
            FxKind::Transition => {
                for id in video_sel() {
                    st.exec("transition.set", json!({"id": id, "kind": name}));
                }
            }
            FxKind::Effect if name == "lut" => {
                st.dock.open("ai", Some("inspector"));
                st.toast.set(Some("LUT: скажите агенту, какой файл .cube".into()));
            }
            FxKind::Effect => {
                st.exec("effect.add", json!({"ids": sel, "kind": name}));
            }
        }
    }
}

// ── просмотр ────────────────────────────────────────────────────────────

static VIEWER_FIT_K: Mutex<f32> = Mutex::new(0.5);
static VIEWER_PT: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
static VIEWER_AREA: Mutex<(f32, f32)> = Mutex::new((640.0, 360.0));
static VIEWER_PAN0: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
static VIEWER_PINCH_Z0: Mutex<Option<f32>> = Mutex::new(None);

pub fn viewer_fit_k() -> f32 {
    *VIEWER_FIT_K.lock().unwrap_or_else(|e| e.into_inner())
}

fn zoom_viewer_at(st: St, next_k: f32, anchor: Option<(f32, f32)>) {
    let next_k = next_k.clamp(0.1, 8.0);
    let old_opt = st.viewer_zoom.get_untracked();
    let old_k = old_opt.unwrap_or_else(viewer_fit_k).max(0.02);
    if let Some((mx, my)) = anchor {
        let (aw, ah) = *VIEWER_AREA.lock().unwrap_or_else(|e| e.into_inner());
        let (px, py) = if old_opt.is_some() { st.viewer_pan.get_untracked() } else { (0.0, 0.0) };
        let cx = aw / 2.0 + px;
        let cy = ah / 2.0 + py;
        let ratio = next_k / old_k;
        let nx = mx - aw / 2.0 - (mx - cx) * ratio;
        let ny = my - ah / 2.0 - (my - cy) * ratio;
        st.viewer_pan.set((nx, ny));
    }
    st.viewer_zoom.set(Some(next_k));
}

fn viewer_bg_label(mode: u8) -> &'static str {
    match mode {
        1 => "Поле просмотра: серое (нажмите для смены)",
        2 => "Поле просмотра: светлое (нажмите для смены)",
        3 => "Поле просмотра: шахматка прозрачности (нажмите для смены)",
        _ => "Поле просмотра: тёмное (нажмите для смены)",
    }
}

/// Кадры для просмотра: читатели клипов живут между кадрами (воспроизведение
/// декодирует подряд), размер — до 1280 по большей стороне.
fn preview_frames(p: &crate::doc::Project, t: f64, compare: bool) -> crate::render::compose::Frames {
    static SRC: Mutex<Option<crate::render::compose::FrameSource>> = Mutex::new(None);
    let mut s = SRC.lock().unwrap_or_else(|e| e.into_inner());
    let src = s.get_or_insert_with(|| crate::render::compose::FrameSource::new(1280, 1280));
    // «до/после» панели «Цвет»: коррекция и LUT — правая половина кадра
    src.compare = compare.then_some(0.5);
    src.frames(p, t)
}

/// Просмотр — кадр композитора (тот же, что при экспорте) с зумом,
/// панорамированием и видимыми границами кадра проекта.
fn viewer(st: St) -> impl Widget {
    Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("grow").child(viewer_frame(st))).child(transport(st))
}

/// Управление воспроизведением и масштабом под кадром.
fn transport(st: St) -> impl Widget {
    let phone = crate::layout::is_phone();
    let btn = move |icon: &'static str, tip: &'static str, f: fn(St)| ToolButton::new(icon).tooltip(tip).on_click(move || f(st));
    DecoratedBox::new().class("transport").child(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                let _ = st.rev.get();
                let t = st.ph.get();
                let (d, w, h) = {
                    let e = ed();
                    (e.project.duration(), e.project.settings.width, e.project.settings.height)
                };
                let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
                // телефон: только позиция (длительность и кадр — в строке состояния)
                row = row.child(Text::new(if phone { tc(t) } else { format!("{} / {}", tc(t), tc(d)) }).max_lines(1).class("transport-time"));
                if !phone {
                    row = row.child(Text::new(format!("{w}×{h}")).class("transport-res"));
                }
                vec![Box::new(row)]
            })))
            .child(btn("\u{E045}", "В начало (Home)", |st| st.seek_live(0.0)))
            .child(btn("\u{E020}", "Кадр назад (←)", |st| st.step(-1.0)))
            .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                let on = st.playing.get();
                vec![Box::new(ToolButton::new(if on { icons::PAUSE } else { icons::PLAY }).tooltip(if on { "Пауза (Пробел)" } else { "Воспроизвести (Пробел)" }).on_click(move || st.toggle_play()).class("transport-play"))]
            }))
            .child(btn("\u{E047}", "Стоп", |st| st.stop()))
            .child(btn("\u{E01F}", "Кадр вперёд (→)", |st| st.step(1.0)))
            .child(btn("\u{E044}", "В конец (End)", |st| {
                let d = ed().project.duration();
                st.seek_live(d);
            }))
            .child((move || -> Box<dyn Widget> {
                if phone {
                    // масштаб просмотра на телефоне — щипком
                    return Box::new(DecoratedBox::new().class("grow"));
                }
                Box::new(DecoratedBox::new().class("grow").child(
                    Row::new()
                        .gap(2.0)
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                            let mode = st.viewer_bg.get();
                            vec![Box::new(
                                ToolButton::new(if mode == 3 { "\u{E3EC}" } else { "\u{E3A9}" })
                                    .tooltip(viewer_bg_label(mode))
                                    .on_click(move || st.viewer_bg.update(|b| *b = (*b + 1) % 4)),
                            )]
                        }))
                        .child(ToolButton::new(icons::ZOOM_OUT).tooltip("Уменьшить просмотр (колесо вниз)").on_click(move || st.viewer_zoom_step(1.0 / 1.25)))
                        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                            let z = st.viewer_zoom.get();
                            let presets: &[(&str, &str, Option<f32>)] = &[
                                ("fit", "Вписать", None),
                                ("25", "25%", Some(0.25)),
                                ("50", "50%", Some(0.50)),
                                ("75", "75%", Some(0.75)),
                                ("100", "100%", Some(1.00)),
                                ("150", "150%", Some(1.50)),
                                ("200", "200%", Some(2.00)),
                                ("400", "400%", Some(4.00)),
                            ];
                            let mut items: Vec<DropdownItem> = presets.iter().map(|(k, l, _)| DropdownItem::new(*k, *l)).collect();
                            let sel = match z {
                                None => "fit".to_string(),
                                Some(v) => {
                                    let pct = (v * 100.0).round() as i32;
                                    match presets.iter().find(|(_, _, pv)| pv.is_some_and(|x| ((x * 100.0).round() as i32) == pct)) {
                                        Some((k, _, _)) => (*k).to_string(),
                                        None => {
                                            let key = format!("custom:{pct}");
                                            items.insert(0, DropdownItem::new(key.clone(), format!("{pct}%")));
                                            key
                                        }
                                    }
                                }
                            };
                            vec![Box::new(
                                Dropdown::new()
                                    .items(items)
                                    .selected(sel)
                                    .on_change(move |k: &str| match k {
                                        "fit" => st.viewer_fit(),
                                        s => {
                                            if let Ok(pct) = s.trim_start_matches("custom:").parse::<f32>() {
                                                st.viewer_zoom.set(Some((pct / 100.0).clamp(0.1, 8.0)));
                                                st.viewer_pan.set((0.0, 0.0));
                                            }
                                        }
                                    })
                                    .class("viewer-zoom-select"),
                            )]
                        }))
                        .child(ToolButton::new(icons::ZOOM_IN).tooltip("Увеличить просмотр (колесо вверх)").on_click(move || st.viewer_zoom_step(1.25)))
                        .child(ToolButton::new(icons::FULLSCREEN).tooltip("Вписать кадр").on_click(move || st.viewer_fit())),
                ))
            })()),
    )
}

fn checkerboard(w: f32, h: f32) -> Box<dyn Widget> {
    let canvas = Canvas::new(move |cx, _| {
        let cell = 12.0f32;
        let light = Color::from_srgb(74, 78, 90, 1.0);
        let dark = Color::from_srgb(48, 52, 62, 1.0);
        cx.set_color(dark);
        cx.fill_rect(0.0, 0.0, w, h);
        cx.set_color(light);
        let cols = (w / cell).ceil() as i32;
        let rows = (h / cell).ceil() as i32;
        for r in 0..rows {
            for c in 0..cols {
                if (r + c) % 2 == 0 {
                    cx.fill_rect(c as f32 * cell, r as f32 * cell, cell, cell);
                }
            }
        }
    })
    .size(w, h);
    Box::new(canvas)
}

fn viewer_frame(st: St) -> impl Widget {
    let area = use_signal(syngui::prelude::Size::new(640.0, 360.0));
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let _ = st.ph.get();
        let a = area.get();
        let zoom_opt = st.viewer_zoom.get();
        let pan = if zoom_opt.is_some() { st.viewer_pan.get() } else { (0.0, 0.0) };
        let bg_mode = st.viewer_bg.get();
        let (p, t) = {
            let e = ed();
            (e.project.clone(), e.playhead)
        };
        let mut p = p;
        // рамку тянут — кадр с новыми значениями
        if let Some(d) = st.xf_drag.get() {
            super::xform::apply_preview(&mut p, &d, t);
        }
        let pw = p.settings.width.max(2) as f32;
        let ph = p.settings.height.max(2) as f32;
        let fit_k = ((a.width - 28.0).max(32.0) / pw).min((a.height - 28.0).max(32.0) / ph).max(0.02);
        *VIEWER_FIT_K.lock().unwrap_or_else(|e| e.into_inner()) = fit_k;
        let k = zoom_opt.unwrap_or(fit_k).clamp(0.05, 8.0);
        let (fw, fh) = ((pw * k).max(4.0), (ph * k).max(4.0));
        let ox = (a.width - fw) / 2.0 + pan.0;
        let oy = (a.height - fh) / 2.0 + pan.1;

        let compare = st.color_compare.get();
        let frames = preview_frames(&p, t, compare);
        let frame = crate::render::compose::frame_widget(&p, t, &frames, k);
        let show_checker = bg_mode == 3 || p.settings.bg.eq_ignore_ascii_case("transparent") || p.settings.bg.eq_ignore_ascii_case("none");

        let mut frame_stack = Stack::new();
        if show_checker {
            frame_stack = frame_stack.child(Positioned::new(checkerboard(fw, fh)).at(0.0, 0.0).dimensions(fw, fh));
        }
        frame_stack = frame_stack.child(Positioned::new(frame).at(0.0, 0.0).dimensions(fw, fh));
        if compare {
            // линия раздела и подписи «до | после»
            frame_stack = frame_stack
                .child(Positioned::new(DecoratedBox::new().class("cmp-line")).at(fw / 2.0 - 1.0, 0.0).dimensions(2.0, fh))
                .child(Positioned::new(DecoratedBox::new().class("cmp-tag").child(Text::new("До").class("cmp-tag-text"))).at(fw / 2.0 - 46.0, 8.0))
                .child(Positioned::new(DecoratedBox::new().class("cmp-tag").child(Text::new("После").class("cmp-tag-text"))).at(fw / 2.0 + 8.0, 8.0));
        }
        if !st.playing.get() {
            for w in super::xform::overlay(st, &p, t, k, fw, fh) {
                frame_stack = frame_stack.child(w);
            }
        }

        // Чёткая рамка границы кадра проекта поверх поля (чтобы чёрный кадр был виден)
        let border_col = if bg_mode == 2 { Color::from_srgb(0, 0, 0, 0.35) } else { Color::from_srgb(255, 255, 255, 0.32) };
        let frame_wrap = DecoratedBox::new()
            .class("viewer-frame-box")
            .style("border-color", border_col)
            .child(DecoratedBox::new().style("width", fw).style("height", fh).child(frame_stack));

        let pasteboard_bg = match bg_mode {
            1 => Color::from_srgb(56, 60, 72, 1.0),
            2 => Color::from_srgb(200, 205, 216, 1.0),
            _ => Color::from_srgb(28, 30, 38, 1.0),
        };
        // Перетаскивание по пустому полю панорамирует увеличенный кадр; двойной клик — «Вписать»
        let backdrop = GestureDetector::new()
            .on_double_click(move || st.viewer_fit())
            .on_pan_start(move |_| {
                *VIEWER_PAN0.lock().unwrap_or_else(|e| e.into_inner()) = st.viewer_pan.get_untracked();
            })
            .on_pan_update(move |u: syngui::widgets::PanUpdate| {
                if st.viewer_zoom.get_untracked().is_some() {
                    let (x0, y0) = *VIEWER_PAN0.lock().unwrap_or_else(|e| e.into_inner());
                    st.viewer_pan.set((x0 + u.total.x, y0 + u.total.y));
                }
            })
            .child(DecoratedBox::new().style("background-color", pasteboard_bg).style("width", a.width).style("height", a.height));

        let viewport = Stack::new()
            .clip(true)
            .child(Positioned::new(backdrop).at(0.0, 0.0).dimensions(a.width, a.height))
            .child(Positioned::new(frame_wrap).at(ox - 1.0, oy - 1.0).dimensions(fw + 2.0, fh + 2.0));
        vec![Box::new(viewport)]
    });

    let pinch = GestureDetector::new()
        .on_pinch(move |u: syngui::widgets::PinchUpdate| {
            let mut z0 = VIEWER_PINCH_Z0.lock().unwrap_or_else(|e| e.into_inner());
            let base = *z0.get_or_insert_with(|| st.viewer_zoom.get_untracked().unwrap_or_else(viewer_fit_k));
            let pt = *VIEWER_PT.lock().unwrap_or_else(|e| e.into_inner());
            zoom_viewer_at(st, base * u.scale, Some(pt));
        })
        .on_pinch_end(move || *VIEWER_PINCH_Z0.lock().unwrap_or_else(|e| e.into_inner()) = None)
        .child(DecoratedBox::new().class("viewer").child(body));

    EventHook::new()
        .fill()
        .on_resize(move |s| {
            *VIEWER_AREA.lock().unwrap_or_else(|e| e.into_inner()) = (s.width, s.height);
            area.set(s);
        })
        .on_hover_at(move |p| {
            if let Some((pt, _)) = p {
                *VIEWER_PT.lock().unwrap_or_else(|e| e.into_inner()) = (pt.x, pt.y);
            }
        })
        .on_wheel(move |_dx, dy, _pos| {
            if dy == 0.0 {
                return false;
            }
            let cur = st.viewer_zoom.get_untracked().unwrap_or_else(viewer_fit_k);
            let factor = 1.12f32.powf(dy / 40.0);
            let pt = *VIEWER_PT.lock().unwrap_or_else(|e| e.into_inner());
            zoom_viewer_at(st, cur * factor, Some(pt));
            true
        })
        .child(pinch)
}

// ── Съёмочная группа: чипы ролей, доска, записки ─────────────────────────

/// Ряд чипов над чатом ИИ: режиссёр (главный чат), каждая роль, что уже
/// работала (её чат; идущая сейчас — с крутилкой), доска сцен и записки.
/// Пока группа не работала (нет ролей и записок) — ряда нет.
fn crew_chips(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let roles = st.role_logs.get();
        let active = st.role_active.get();
        let view = st.ai_view.get();
        let _ = st.rev.get();
        let project = ed().path.clone();
        let notes_open = project.as_deref().map(|p| crate::crew::notes(p).iter().filter(|n| n.status != crate::crew::Status::Closed).count()).unwrap_or(0);
        let has_crew = !roles.is_empty() || project.as_deref().is_some_and(|p| crate::store::media_dir(p).join("правила.md").is_file());
        if !has_crew {
            return Vec::new();
        }
        let chip = |label: String, on: bool, busy: bool, v: Option<String>| -> Box<dyn Widget> {
            let text = if busy { format!("● {label}") } else { label };
            Box::new(GestureDetector::new().on_press(move |_| st.ai_view.set(v.clone())).child(DecoratedBox::new().class(if on { "ai-chip ai-chip-on" } else { "ai-chip" }).child(Text::new(text).max_lines(1).class(if on { "ai-chip-text-on" } else { "ai-chip-text" }))))
        };
        let mut chips: Vec<Box<dyn Widget>> = vec![chip("Режиссёр".into(), view.is_none(), st.ai_busy.get() && active.is_none(), None)];
        // чей шаг следующий по доске (автор текущего шага или ждущий проверяющий)
        let next_role: Option<&'static str> = project.as_deref().and_then(|p| {
            let e = ed();
            let n = crate::cmd::crew::scene_count(p);
            (1..=n).map(|s| (s, crate::crew::stage(p, s, Some(&e.project)))).find(|(_, st)| *st != crate::crew::Stage::Done).map(|(s, st)| {
                crate::crew::who(p, crate::crew::review_due(p, s, st).and_then(|w| crate::crew::REVIEWS.iter().find(|r| r.0 == w)).map(|r| r.2).unwrap_or(st.roles().0))
            })
        });
        let notes = project.as_deref().map(crate::crew::notes).unwrap_or_default();
        for (id, title, log) in &roles {
            let busy = active.as_deref() == Some(id.as_str());
            let to_me = notes.iter().filter(|x| &x.to == id && x.status != crate::crew::Status::Closed).count();
            let state = if busy {
                "думает".to_string()
            } else if to_me > 0 {
                format!("замечание {to_me}")
            } else if matches!(log.last(), Some(AiLine::Error(_))) {
                "ошибка".to_string()
            } else if next_role == Some(id.as_str()) {
                "ждёт".to_string()
            } else {
                "готово".to_string()
            };
            chips.push(chip(format!("{title} · {state}"), view.as_deref() == Some(id.as_str()), busy, Some(id.clone())));
        }
        // роли, которые ещё не работали, но чей шаг следующий, — тоже чипом
        if let Some(nr) = next_role.filter(|nr| !roles.iter().any(|(id, _, _)| id == nr)) {
            if let Some(r) = crate::crew::role(nr) {
                chips.push(chip(format!("{} · ждёт", r.title), view.as_deref() == Some(nr), false, Some(nr.to_string())));
            }
        }
        chips.push(chip("Доска".into(), view.as_deref() == Some("board"), false, Some("board".into())));
        chips.push(chip(if notes_open > 0 { format!("Записки · {notes_open}") } else { "Записки".into() }, view.as_deref() == Some("notes"), false, Some("notes".into())));
        vec![Box::new(DecoratedBox::new().class("ai-chips").child(Flex::new().gap(4.0).wrap().children(chips)))]
    })
}

/// Доска производства: сцены × шаг конвейера, кто делает и проверяет, записки.
fn crew_board(st: St) -> Box<dyn Widget> {
    let _ = st.rev.get();
    let _ = st.role_active.get();
    let Some(p) = ed().path.clone() else { return empty("Проект не сохранён — доски нет") };
    let n = crate::cmd::crew::scene_count(&p);
    if n == 0 {
        return empty("Сцен пока нет: доска появится после структуры фильма (03_структура)");
    }
    let project = ed().project.clone();
    let notes = crate::crew::notes(&p);
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("ai-log");

    let stages = crate::crew::Stage::ALL;
    let mut current_set = false;
    for s in 1..=n {
        let stg = crate::crew::stage(&p, s, Some(&project));
        let (a, b) = (crate::crew::who(&p, stg.roles().0), crate::crew::who(&p, stg.roles().1));
        let done = stages.iter().position(|x| *x == stg).unwrap_or(0);
        let mut bar = Row::new().gap(2.0);
        for (k, _) in stages.iter().enumerate().take(stages.len() - 1) {
            bar = bar.child(DecoratedBox::new().class(if k < done { "crew-cell crew-cell-done" } else if k == done { "crew-cell crew-cell-now" } else { "crew-cell" }));
        }
        let open = notes.iter().filter(|x| x.scene == s && x.status != crate::crew::Status::Closed).count();
        let current = !current_set && stg != crate::crew::Stage::Done;
        current_set |= current;
        let who = if stg == crate::crew::Stage::Done { "готово".to_string() } else { format!("{} · {} → {}", stg.title(), crate::crew::role(a).map(|r| r.title).unwrap_or(a), crate::crew::role(b).map(|r| r.title).unwrap_or(b)) };
        let opened = st.board_open.get() == Some(s);
        let line = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(format!("{s:>2}")).class(if current { "crew-scene crew-scene-now" } else { "crew-scene" }))
            .child(bar)
            .child(Text::new(if open > 0 { format!("{who} · записок {open}") } else { who }).max_lines(1).class("ai-result-text"));
        col = col.child(GestureDetector::new().on_press(move |_| st.board_open.update(|o| *o = if *o == Some(s) { None } else { Some(s) })).child(line));
        if opened {
            col = col.child(crew_scene_card(&p, s, &notes));
        }
    }
    col = col.child(Text::new(format!("Шаги: {}", stages.iter().map(|s| s.title()).collect::<Vec<_>>().join(" → "))).class("ai-think-text"));
    // вехи согласования: переключатель и согласие пользователя
    let ms = crate::crew::milestones(&p);
    let p2 = p.clone();
    let mut mrow = Flex::new().gap(6.0).wrap().child(Button::new(if ms.enabled { "Вехи согласования: вкл" } else { "Вехи согласования: выкл" }).on_click(move || {
        let mut m = crate::crew::milestones(&p2);
        m.enabled = !m.enabled;
        crate::crew::save_milestones(&p2, &m);
        st.board_open.update(|_| {});
    }).class("head-btn"));
    if let Some(&(id, title, show)) = crate::crew::milestone_due(&p, Some(&project)) {
        let p3 = p.clone();
        mrow = mrow.child(Text::new(format!("Ждёт согласия: {title} — {show}")).max_lines(2).class("ai-result-text")).child(Button::new(format!("Согласовать «{title}»")).on_click(move || {
            let mut m = crate::crew::milestones(&p3);
            if !m.approved.iter().any(|a| a == id) {
                m.approved.push(id.to_string());
            }
            crate::crew::save_milestones(&p3, &m);
            crate::crew::journal(&p3, &format!("веха «{title}»: согласовано пользователем"));
            st.board_open.update(|_| {});
        }).class("head-btn"));
    } else if ms.enabled {
        let done: Vec<&str> = crate::crew::MILESTONES.iter().filter(|m| ms.approved.iter().any(|a| a == m.0)).map(|m| m.1).collect();
        mrow = mrow.child(Text::new(if done.is_empty() { "Вехи: сценарий → лукбук → первая сцена".to_string() } else { format!("Согласовано: {}", done.join(", ")) }).class("ai-result-text"));
    }
    col = col.child(mrow);
    Box::new(col)
}

/// Материалы сцены на доске: документ, ассеты, проверки, записки, ролики.
fn crew_scene_card(p: &std::path::Path, s: u32, notes: &[crate::crew::Note]) -> Box<dyn Widget> {
    let dir = crate::store::media_dir(p);
    let scene = std::fs::read_to_string(dir.join(format!("scene_{s:02}.md"))).unwrap_or_default();
    let mut c = Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let head: String = scene.lines().find(|l| !l.trim().is_empty()).unwrap_or("документа сцены ещё нет").trim_start_matches('#').trim().to_string();
    c = c.child(Text::new(format!("scene_{s:02}: {head}")).max_lines(2).class("ai-result-text"));
    let cat = crate::cmd::doc::catalog(&std::fs::read_to_string(dir.join("04_ассеты.md")).unwrap_or_default());
    let refs = crate::cmd::doc::asset_refs(&scene);
    if !refs.is_empty() {
        let v: Vec<String> = refs.iter().map(|id| format!("@{id}{}", if cat.iter().any(|a| &a.id == id && std::path::Path::new(&a.file).is_file()) { "" } else { " (нет)" })).collect();
        c = c.child(Text::new(format!("Ассеты: {}", v.join(", "))).max_lines(3).class("ai-result-text"));
    }
    for &(what, title, _, _) in crate::crew::REVIEWS {
        let mark = match crate::crew::review_ok(p, s, what) {
            Some(true) => "✓",
            Some(false) => "✗",
            None => "—",
        };
        c = c.child(Text::new(format!("Проверка «{title}»: {mark}")).class("ai-result-text"));
    }
    let rolls = crate::crew::scene_rolls(p, s);
    if !rolls.is_empty() {
        c = c.child(Text::new(format!("Роликов: {} — {}", rolls.len(), rolls.iter().filter_map(|r| r.file_name().map(|x| x.to_string_lossy().into_owned())).collect::<Vec<_>>().join(", "))).max_lines(3).class("ai-result-text"));
    }
    for n in notes.iter().filter(|x| x.scene == s && x.status != crate::crew::Status::Closed) {
        c = c.child(Text::new(crate::crew::note_line(n)).max_lines(3).class("ai-result-text"));
    }
    Box::new(DecoratedBox::new().class("ai-note").child(c))
}

/// Лента записок между ролями: открытые сверху, круги и эскалация.
fn crew_notes(st: St) -> Box<dyn Widget> {
    let _ = st.rev.get();
    let _ = st.role_active.get();
    let Some(p) = ed().path.clone() else { return empty("Проект не сохранён") };
    let mut v = crate::crew::notes(&p);
    if v.is_empty() {
        return empty("Записок нет: роли пишут их друг другу, когда что-то не так");
    }
    v.sort_by_key(|n| (n.status == crate::crew::Status::Closed, std::cmp::Reverse(n.id)));
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("ai-log");
    for n in v {
        let closed = n.status == crate::crew::Status::Closed;
        col = col.child(DecoratedBox::new().class(if closed { "ai-note ai-note-closed" } else if n.to == "user" { "ai-note ai-note-user" } else { "ai-note" }).child(Text::new(crate::crew::note_line(&n)).selectable(true).class("ai-result-text")));
    }
    Box::new(col)
}

// ── ИИ ──────────────────────────────────────────────────────────────────

fn ai(st: St) -> impl Widget {
    let input_gen = use_signal(0u64);
    let draft = use_signal(String::new());
    // развёрнутые блоки вызовов (номера строк лога); по умолчанию свёрнуты
    let expanded = use_signal(std::collections::BTreeSet::<usize>::new());
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ai-panel")
        .child(super::ai_chat::head(st))
        .child(crew_chips(st))
        .child(DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            // доска и записки — своя прокрутка с начала; чаты — следом за концом
            match st.ai_view.get().as_deref() {
                Some("board") => return vec![Box::new(ScrollView::new().vertical().child(crew_board(st)))],
                Some("notes") => return vec![Box::new(ScrollView::new().vertical().child(crew_notes(st)))],
                _ => {}
            }
            vec![Box::new(ScrollView::new().vertical().follow_end(true).child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            // чат роли, доска или записки группы — вместо главного чата
            let view = st.ai_view.get();
            let log = match view.as_deref() {
                None => st.ai_log.get(),
                Some("board") => return vec![crew_board(st)],
                Some("notes") => return vec![crew_notes(st)],
                Some(r) => st.role_logs.get().into_iter().find(|(id, _, _)| id == r).map(|x| x.2).unwrap_or_default(),
            };
            let open = expanded.get();
            if log.is_empty() {
                return vec![empty("Агент монтирует сам: импорт, нарезка, титры, эффекты.\nНапример: «собери ролик на 30 секунд из ~/Videos/Test, в начале титр»")];
            }
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("ai-log");
            let last = log.len().saturating_sub(1);
            for (i, l) in log.into_iter().enumerate() {
                let (class, text) = match l {
                    AiLine::User(t) => ("ai-user", t),
                    // ответ агента — Markdown (списки, код, выделение)
                    AiLine::Text(t) => {
                        // ответ из одних пробелов (после размышлений) — не пустым блоком
                        if t.trim().is_empty() {
                            continue;
                        }
                        col = col.child(DecoratedBox::new().class("ai-text").child(MarkdownView::new(t).with_copy_code(true).selectable(true).class("ai-text-text")));
                        continue;
                    }
                    // размышления — свёрнутый блок: идут — крутилка и «Думает…», готовы — «Думал N с»
                    AiLine::Thinking { text, done, secs, .. } => {
                        let is_open = open.contains(&i);
                        let title = if !done {
                            "Думает…".to_string()
                        } else if secs > 0.0 {
                            format!("Думал {}", if secs < 60.0 { format!("{secs:.0} с") } else { format!("{} мин {:.0} с", (secs / 60.0) as u32, secs % 60.0) })
                        } else {
                            "Размышления".to_string()
                        };
                        let mut head = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(ToolButton::new(if is_open { "\u{E5CE}" } else { "\u{E5CF}" }).tooltip(if is_open { "Свернуть" } else { "Показать размышления" }).on_click(move || {
                            expanded.update(|e| {
                                if !e.remove(&i) {
                                    e.insert(i);
                                }
                            })
                        }));
                        if !done {
                            head = head.child(CircularProgress::new().indeterminate().size(12.0).stroke_width(2.0));
                        }
                        head = head.child(Text::new(title).class("ai-think-title"));
                        let mut block = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
                        if is_open {
                            block = block.child(Text::new(text.trim().to_string()).selectable(true).class("ai-think-text"));
                        }
                        col = col.child(DecoratedBox::new().class("ai-think").child(block));
                        continue;
                    }
                    // вызов ещё собирается — крутилка и имя инструмента, когда известно
                    AiLine::Preparing(name) => {
                        let label = if name.is_empty() { "Готовлюсь к запуску команды…".to_string() } else { format!("Готовлюсь к запуску команды {name}…") };
                        col = col.child(DecoratedBox::new().class("ai-prep").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(12.0).stroke_width(2.0)).child(Text::new(label).class("ai-prep-text"))));
                        continue;
                    }
                    AiLine::Tool(t) => ("ai-tool", format!("→ {t}")),
                    AiLine::Result(t) => ("ai-result", t),
                    AiLine::Error(t) => ("ai-error", t),
                    AiLine::Diff { lines, rev, undone } => {
                        let can = !undone && ed().revision == rev && i == last;
                        let mut card = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(if undone { "Ход откачен" } else { "Изменения хода" }).class("section-title"));
                        for t in &lines {
                            card = card.child(Text::new(t.clone()).max_lines(3).class("ai-result-text"));
                        }
                        if can {
                            card = card.child(Row::new().child(Button::new("Откатить ход").on_click(move || {
                                if st.exec("edit.undo", json!({})).is_some() {
                                    st.ai_log.update(|l| {
                                        if let Some(AiLine::Diff { undone, .. }) = l.get_mut(i) {
                                            *undone = true;
                                        }
                                    });
                                }
                            }).class("head-btn")));
                        }
                        col = col.child(DecoratedBox::new().class("ai-diff").child(card));
                        continue;
                    }
                };
                // длинный вызов или результат — свёрнут до 2 строк, щелчок по
                // стрелке раскрывает целиком (иначе огромный JSON рецепта
                // заполняет чат)
                let long = matches!(class, "ai-tool" | "ai-result") && (text.chars().count() > 160 || text.lines().count() > 2);
                if long {
                    let is_open = open.contains(&i);
                    // заголовок — первая строка (одной строкой); тело: свёрнуто —
                    // одна строка продолжения (если есть), развёрнуто — весь текст
                    let (title, rest) = match text.split_once('\n') {
                        Some((t, r)) => (t.to_string(), Some(r.to_string())),
                        None => (text.clone(), None),
                    };
                    let body: Option<Box<dyn Widget>> = match (is_open, rest) {
                        (true, _) => Some(Box::new(Text::new(text.clone()).selectable(true).class(format!("{class}-text")))),
                        (false, Some(r)) => Some(Box::new(Text::new(r.replace('\n', " ")).max_lines(1).elide(Elide::End).class(format!("{class}-text")))),
                        (false, None) => None,
                    };
                    let head = Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(ToolButton::new(if is_open { "\u{E5CE}" } else { "\u{E5CF}" }).tooltip(if is_open { "Свернуть" } else { "Развернуть" }).on_click(move || {
                            expanded.update(|e| {
                                if !e.remove(&i) {
                                    e.insert(i);
                                }
                            })
                        }))
                        .child(DecoratedBox::new().class("grow").child(Text::new(if is_open { String::new() } else { title }).max_lines(1).elide(Elide::End).class(format!("{class}-text"))));
                    let mut block = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
                    if let Some(b) = body {
                        block = block.child(b);
                    }
                    col = col.child(DecoratedBox::new().class(class).child(block));
                    continue;
                }
                col = col.child(DecoratedBox::new().class(class).child(Text::new(text).selectable(true).class(format!("{class}-text"))));
            }
            vec![Box::new(col)]
        })))]
        })))
        .child(DecoratedBox::new().class("ai-input").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let _ = input_gen.get();
            let busy = st.ai_busy.get();
            let stop: Option<Box<dyn Widget>> = busy.then(|| -> Box<dyn Widget> {
                Box::new(Button::new("Стоп").on_click(|| super::AI_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst)).class("head-btn"))
            });
            // вложения к следующему сообщению: плашки с именем и крестиком
            let files = st.ai_files.get();
            let chips: Vec<Box<dyn Widget>> = files
                .iter()
                .enumerate()
                .map(|(i, f)| -> Box<dyn Widget> {
                    let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone());
                    Box::new(
                        DecoratedBox::new().class("ai-attach-chip").child(
                            Row::new()
                                .gap(4.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Text::new(format!("{} {name}", super::icons::ATTACH)).max_lines(1).elide(Elide::Middle).class("ai-attach-text"))
                                .child(ToolButton::new(super::icons::CLOSE).tooltip("Убрать вложение").on_click(move || st.ai_files.update(|v| {
                                    if i < v.len() {
                                        v.remove(i);
                                    }
                                }))),
                        ),
                    )
                })
                .collect();
            let attach = ToolButton::new(super::icons::ATTACH).tooltip("Прикрепить файлы (PDF, текст, картинки, звук, видео) — или перетащите их в поле").disabled(busy).on_click(move || {
                if let Some(paths) = rfd::FileDialog::new().set_title("Прикрепить к сообщению агенту").pick_files() {
                    st.ai_files.update(|v| v.extend(paths.into_iter().map(|p| p.display().to_string())));
                }
            });
            let row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(stop.into_iter().collect::<Vec<Box<dyn Widget>>>()).child(attach).child(DecoratedBox::new().class("grow").child(
                TextField::new()
                    .text(draft.get_untracked())
                    .on_change(move |s: &str| draft.set(s.to_string()))
                    .placeholder(if busy {
                        "Агент работает…".to_string()
                    } else {
                        match st.ai_view.get().filter(|r| r != "board" && r != "notes").and_then(|r| crate::crew::role(&r)) {
                            Some(r) => format!("Сообщение роли «{}»… (Enter)", r.title),
                            None => "Задача для агента… (Enter)".to_string(),
                        }
                    })
                    .prefix_icon(icons::AI)
                    .disabled(busy)
                    .on_submit(move |q: &str| {
                        // открыт чат роли — сообщение ей напрямую, иначе — режиссёру (главный агент)
                        match st.ai_view.get_untracked() {
                            Some(r) if r != "board" && r != "notes" => st.ask_role(r, q.to_string()),
                            _ => st.ask_ai(q.to_string()),
                        }
                        draft.set(String::new());
                        input_gen.update(|g| *g += 1);
                    })
                    .class("ai-field"),
            ));
            // файлы из проводника, брошенные на поле, — вложения (а не импорт в медиатеку)
            let input = DropArea::new()
                .accept_types(vec![syngui::DragData::TYPE_FILE.to_string()])
                .on_drop(move |d: syngui::DragData| {
                    let paths: Vec<String> = d.payload.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                    st.ai_files.update(|v| v.extend(paths));
                })
                .child(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start).children(chips)).child(row));
            vec![Box::new(input)]
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
        let frac = hover.get();
        let mut stack = Stack::new().fit(StackFit::Expand);
        if kind == MediaKind::Audio {
            stack = stack.child(Center::new().child(Icon::new(icons::AUDIO).class("tile-icon")));
        } else {
            let _ = st.thumbs.get();
            match super::thumbs::strip(&path) {
                Some(strip) => {
                    let i = frac.map(|f| ((f * super::thumbs::FRAMES as f32) as usize).min(super::thumbs::FRAMES - 1)).unwrap_or(super::thumbs::FRAMES / 3);
                    let f = &strip[i];
                    stack = stack.child(Center::new().child(Image::from_rgba_shared(format!("thumb:{id}:{i}:{:x}", std::sync::Arc::as_ptr(&f.rgba) as *const u8 as usize), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false)));
                }
                None => {
                    stack = stack.child(Center::new().child(Icon::new(if kind == MediaKind::Image { icons::IMAGE } else { icons::MOVIE }).class("tile-icon")));
                }
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
    let selected = st.media_sel.get_untracked().contains(&id);
    let n_sel = st.media_sel.get_untracked().len();
    let many = if selected && n_sel > 1 { format!(" ({n_sel})") } else { String::new() };
    let is_video = kind == MediaKind::Video;
    let has_sound = kind == MediaKind::Audio || ed().project.media(id).is_some_and(|m| m.has_audio);
    let mut menu = vec![
        MenuItem::new("add", format!("На таймлайн (в конец){many}")),
        MenuItem::new("add_here", format!("На таймлайн в позицию{many}")),
        MenuItem::separator(),
    ];
    if has_sound {
        menu.push(MenuItem::new("transcribe", format!("Расшифровать речь{many}")));
    }
    if is_video {
        menu.push(MenuItem::new("vfx", "Нейрофильтр").children(vec![MenuItem::new("vfx_up", "Увеличить ×2"), MenuItem::new("vfx_rife", "Плавность до 60 fps")]));
    }
    menu.extend([
        MenuItem::new("reveal", "Показать в папке"),
        MenuItem::new("copy_path", "Копировать путь"),
        MenuItem::new("props", "Свойства…"),
        MenuItem::separator(),
        MenuItem::new("import", "Импорт медиа…"),
        MenuItem::new("import_folder", "Импорт папки…"),
        MenuItem::separator(),
        MenuItem::new("remove", format!("Убрать из медиатеки{many}")),
    ]);
    // меню действует на выделение, если щёлкнули по выделенному; иначе — на этот файл
    let targets = move || -> Vec<u64> {
        let sel = st.media_sel.get_untracked();
        if sel.contains(&id) { sel } else { vec![id] }
    };
    Box::new(
        ContextMenu::new()
            .items(menu)
            .on_select(move |a: &str| {
                let ids = targets();
                if !st.media_sel.get_untracked().contains(&id) {
                    st.media_sel.set(vec![id]);
                }
                match a {
                    "add" => {
                        for m in &ids {
                            st.exec("clip.add", json!({"media": m}));
                        }
                    }
                    "add_here" => {
                        let mut at = ed().playhead;
                        for m in &ids {
                            if let Some(o) = st.exec("clip.add", json!({"media": m, "at": at})) {
                                let _ = o;
                                at = ed().project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.media() == Some(*m)).map(|c| c.end()).fold(at, f64::max);
                            }
                        }
                    }
                    "transcribe" => transcribe_bg(st, ids),
                    "vfx_up" | "vfx_rife" => {
                        let names: Vec<String> = ids.iter().map(|m| format!("m{m}")).collect();
                        let what = if a == "vfx_up" { "kind upscale, scale 2" } else { "kind interpolate, fps 60" };
                        st.open_panel("ai");
                        st.ask_ai(format!("Обработай нейрофильтром media.vfx ({what}) файлы медиатеки: {}.", names.join(", ")));
                    }
                    "reveal" => {
                        if let Some(dir) = ed().project.media(id).and_then(|m| m.path.parent().map(|d| d.to_path_buf())) {
                            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                        }
                    }
                    "copy_path" => {
                        let paths: Vec<String> = ids.iter().filter_map(|m| ed().project.media(*m).map(|x| x.path.display().to_string())).collect();
                        syngui::clipboard::copy(&paths.join("\n"));
                        st.toast.set(Some(format!("Скопировано путей: {}", paths.len())));
                    }
                    "props" => st.media_props.set(Some(id)),
                    "import" => st.dialog(Pick::ImportFiles),
                    "import_folder" => st.dialog(Pick::ImportFolder),
                    "remove" => {
                        for m in &ids {
                            st.exec("media.remove", json!({"id": m}));
                        }
                        st.media_sel.set(Vec::new());
                    }
                    _ => {}
                }
            })
            // щелчок — выделить (Ctrl — добавить/снять, Shift — диапазон); тащить на
            // дорожку таймлайна; двойной щелчок — в конец дорожки
            .child(Draggable::new(DRAG_MEDIA, id.to_string()).label(drag_label.clone()).on_click(move || {
                let m = syngui::input::current_modifiers();
                st.media_sel.update(|sel| {
                    if m.ctrl {
                        if let Some(i) = sel.iter().position(|x| *x == id) {
                            sel.remove(i);
                        } else {
                            sel.push(id);
                        }
                    } else if m.shift && !sel.is_empty() {
                        let order: Vec<u64> = ed().project.media.iter().map(|x| x.id).collect();
                        let a = order.iter().position(|x| Some(x) == sel.last()).unwrap_or(0);
                        let b = order.iter().position(|x| *x == id).unwrap_or(0);
                        let (lo, hi) = (a.min(b), a.max(b));
                        for x in &order[lo..=hi] {
                            if !sel.contains(x) {
                                sel.push(*x);
                            }
                        }
                    } else {
                        *sel = vec![id];
                    }
                });
            }).on_double_click(move || {
                // просмотр этого файла вкладкой «Медиа» (картинка — зум и поворот,
                // видео — плеер); на таймлайн — кнопкой в ней или пунктом меню
                st.fx_preview.set(None);
                st.doc_preview.set(None);
                st.media_preview.set(Some(id));
                st.open_panel("media_view");
                st.dock.activate("media_view");
            }).child(DecoratedBox::new().class(if selected { "tile tile-on" } else { "tile" }).child(body))),
    )
}

/// Расшифровка речи файлов в своём потоке (окно не ждёт), затем — в медиатеку.
fn transcribe_bg(st: St, ids: Vec<u64>) {
    let todo: Vec<(u64, std::path::PathBuf)> = ids.iter().filter_map(|m| ed().project.media(*m).map(|x| (*m, x.path.clone()))).collect();
    st.toast.set(Some(format!("Расшифровка: {} файл(ов) в фоне…", todo.len())));
    std::thread::spawn(move || {
        let mut done = Vec::new();
        let mut errs = Vec::new();
        for (m, p) in todo {
            match crate::cmd::audio::transcribe_file(&p, None) {
                Ok(t) => done.push((m, t)),
                Err(e) => errs.push(e),
            }
        }
        syngui::async_runtime::run_on_main_thread(move || {
            let n = done.len();
            {
                let mut e = ed();
                if n > 0 {
                    e.begin("Расшифровка");
                }
                for (m, t) in done {
                    if let Some(x) = e.project.media.iter_mut().find(|x| x.id == m) {
                        x.transcript = Some(t);
                    }
                }
                st.rev.set(e.revision);
            }
            st.toast.set(Some(if errs.is_empty() { format!("Расшифровано файлов: {n}") } else { format!("Расшифровано: {n}; ошибки: {}", errs.join("; ")) }));
        });
    });
}

/// Окно «Свойства» файла медиатеки.
pub fn media_props_view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let Some(id) = st.media_props.get() else { return Vec::new() };
        let (m, used) = {
            let e = ed();
            let Some(m) = e.project.media(id).cloned() else { return Vec::new() };
            let used = e.project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.media() == Some(id)).count();
            (m, used)
        };
        let close = move || st.media_props.set(None);
        let size = std::fs::metadata(&m.path).map(|x| format!("{:.1} МБ", x.len() as f64 / 1_048_576.0)).unwrap_or_else(|_| "файла нет".into());
        let mut rows: Vec<(&str, String)> = vec![
            ("Файл", m.path.display().to_string()),
            ("Вид", match m.kind { MediaKind::Video => "видео", MediaKind::Audio => "звук", MediaKind::Image => "картинка", MediaKind::Lottie => "анимация Lottie" }.into()),
            ("Размер файла", size),
        ];
        if m.duration > 0.0 {
            rows.push(("Длительность", tc(m.duration)));
        }
        if m.width > 0 {
            rows.push(("Кадр", format!("{}×{}", m.width, m.height)));
        }
        if m.fps > 0.0 {
            rows.push(("Частота", format!("{} fps", m.fps)));
        }
        if matches!(m.kind, MediaKind::Video | MediaKind::Audio) {
            rows.push(("Звук", if m.has_audio || m.kind == MediaKind::Audio { "есть".into() } else { "нет".into() }));
        }
        rows.push(("Расшифровка", m.transcript.as_ref().map(|t| format!("{} слов{}", t.words.len(), t.language.as_ref().map(|l| format!(", {l}")).unwrap_or_default())).unwrap_or_else(|| "нет".into())));
        rows.push(("На таймлайне", format!("клипов: {used}")));
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("m{id} · {}", m.name())).max_lines(1).elide(Elide::Middle).class("fp-title")).child(DecoratedBox::new().class("grow")).child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(close)),
        );
        for (k, v) in rows {
            col = col.child(Row::new().gap(12.0).child(Text::new(k).class("row-sub").style("width", 130.0)).child(DecoratedBox::new().class("grow").child(Text::new(v).selectable(true).max_lines(3).elide(Elide::Middle).class("row-title"))));
        }
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(EventHook::new().on_key_down(move |k, _| {
                    if k == Key::Escape {
                        close();
                        return KeyReply::Handled;
                    }
                    KeyReply::Ignore
                }).child(DecoratedBox::new().class("fp-card").style("width", 560.0).child(Padding::all(16.0).child(col))))),
        )]
    })
}


// ── просмотр медиа по двойному щелчку ───────────────────────────────────

/// Плеер открытого в просмотре звукового файла (один на окно).
static PREVIEW_AUDIO: std::sync::Mutex<Option<(u64, std::sync::Arc<std::sync::Mutex<syngui::video::AudioFilePlayer>>)>> = std::sync::Mutex::new(None);

/// Плеер открытого в просмотре видео/звука (один на окно).
static PREVIEW_PLAYER: std::sync::Mutex<Option<(u64, std::sync::Arc<std::sync::Mutex<syngui::video::VideoPlayer>>)>> = std::sync::Mutex::new(None);

thread_local! {
    static PV: std::cell::RefCell<Option<(RwSignal<i32>, RwSignal<(bool, bool)>, RwSignal<(u64, ImageViewCommand)>)>> = const { std::cell::RefCell::new(None) };
}

fn pv_state() -> (RwSignal<i32>, RwSignal<(bool, bool)>, RwSignal<(u64, ImageViewCommand)>) {
    PV.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(0), use_signal((false, false)), use_signal((0, ImageViewCommand::Fit)))))
}

/// Заголовок вкладки просмотра: «Медиа: имя файла» / «Переход: название».
pub fn media_view_title(st: St) -> String {
    if let Some(p) = st.doc_preview.get() {
        return format!("Документ: {}", p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    }
    if let Some((key, title)) = st.fx_preview.get() {
        let what = match fx_kind(&key).map(|k| k.0) {
            Some(FxKind::Motion) => "Движение",
            Some(FxKind::Transition) => "Переход",
            _ => "Эффект",
        };
        return format!("{what}: {title}");
    }
    match st.media_preview.get().and_then(|id| ed().project.media(id).map(|m| m.name())) {
        Some(n) => format!("Медиа: {n}"),
        None => "Медиа".into(),
    }
}

/// Поколение ленты эффекта: смена просмотра останавливает прежний таймер.
static FX_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

thread_local! {
    /// (кадр ленты, пауза)
    static FXV: std::cell::RefCell<Option<(RwSignal<usize>, RwSignal<bool>)>> = const { std::cell::RefCell::new(None) };
    /// SVG в правке: (файл медиатеки, текст, сохранён ли)
    static SVGV: std::cell::RefCell<Option<(RwSignal<Option<u64>>, RwSignal<String>, RwSignal<bool>)>> = const { std::cell::RefCell::new(None) };
}

fn fxv() -> (RwSignal<usize>, RwSignal<bool>) {
    FXV.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(0usize), use_signal(false))))
}

fn svgv() -> (RwSignal<Option<u64>>, RwSignal<String>, RwSignal<bool>) {
    SVGV.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(None), use_signal(String::new()), use_signal(true))))
}

/// Просмотр движения/перехода/эффекта: его лента на демо-кадрах по кругу,
/// пауза, покадровый шаг и «Применить к выделенному».
fn fx_view(st: St, key: String, title: String) -> Box<dyn Widget> {
    let (frame, paused) = fxv();
    let gen = FX_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    frame.set(0);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(83));
        if FX_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
            break;
        }
        syngui::async_runtime::run_on_main_thread(move || {
            if FX_GEN.load(std::sync::atomic::Ordering::SeqCst) == gen && !paused.get_untracked() {
                frame.update(|f| *f = f.wrapping_add(1));
            }
        });
    });
    let path = crate::render::preview::strip_path(&key);
    let k2 = key.clone();
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        match super::thumbs::strip_file(&path) {
            Some(strip) if !strip.is_empty() => {
                let i = frame.get() % strip.len();
                let f = &strip[i];
                vec![Box::new(Image::from_rgba_shared(format!("fxv:{k2}:{i}"), f.width, f.height, f.rgba.clone()).fit(ImageFit::Contain).placeholder(false).class("pv-fill"))]
            }
            _ => vec![empty("Превью ещё готовится…")],
        }
    });
    let kind = fx_kind(&key);
    let mut tools = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title).max_lines(1).elide(Elide::End).class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(ToolButton::new("\u{E045}").tooltip("Кадр назад").on_click(move || {
            paused.set(true);
            frame.update(|f| *f = f.wrapping_sub(1));
        }))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let p = paused.get();
            vec![Box::new(ToolButton::new(if p { "\u{E037}" } else { "\u{E034}" }).tooltip(if p { "Играть" } else { "Пауза" }).on_click(move || paused.update(|x| *x = !*x)))]
        }))
        .child(ToolButton::new("\u{E044}").tooltip("Кадр вперёд").on_click(move || {
            paused.set(true);
            frame.update(|f| *f = f.wrapping_add(1));
        }));
    if let Some((kind, name)) = kind {
        let name = name.to_string();
        tools = tools.child(Button::new("Применить к выделенному").on_click(move || fx_apply(st, kind, &name)).class("head-btn"));
    }
    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(tools.class("panel-head"))
            .child(DecoratedBox::new().class("grow").class("media-preview-body").child(body)),
    )
}

thread_local! {
    /// Документ в правке: (файл, текст, сохранён ли, показ — просмотр Markdown)
    static DOCV: std::cell::RefCell<Option<(RwSignal<Option<std::path::PathBuf>>, RwSignal<String>, RwSignal<bool>, RwSignal<bool>)>> = const { std::cell::RefCell::new(None) };
}

fn docv() -> (RwSignal<Option<std::path::PathBuf>>, RwSignal<String>, RwSignal<bool>, RwSignal<bool>) {
    DOCV.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(None), use_signal(String::new()), use_signal(true), use_signal(false))))
}

/// Документ проекта: правка Markdown с подсветкой, «Просмотр» — как текст
/// выглядит, «Сохранить» — в файл (агент перечитает его через doc.read).
fn doc_view(st: St, path: std::path::PathBuf) -> Box<dyn Widget> {
    use syngui::widgets::input::code_editor::syntax::Language;
    use syngui::widgets::input::code_editor::CodeEditorChange;
    let (cur, text, saved, preview) = docv();
    if cur.get_untracked().as_ref() != Some(&path) {
        cur.set(Some(path.clone()));
        text.set(std::fs::read_to_string(&path).unwrap_or_default());
        saved.set(true);
    }
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if preview.get() {
            return vec![Box::new(ScrollView::new().vertical().child(Padding::all(12.0).child(MarkdownView::new(text.get_untracked()).selectable(true).class("ai-text-text"))))];
        }
        vec![Box::new(
            CodeEditor::new()
                .text(text.get_untracked())
                .language(Language::Markdown)
                .soft_wrap(true)
                .tab_width(2)
                .on_change(move |ch: CodeEditorChange| {
                    text.set(ch.full_text.to_string());
                    saved.set(false);
                })
                .class("set-code"),
        )]
    });
    let save_path = path.clone();
    let save = move || match crate::layout::write_atomic(&save_path, &text.get_untracked()) {
        Ok(()) => {
            saved.set(true);
            st.rev.update(|n| *n += 1);
            st.toast.set(Some(format!("Сохранено: {}", save_path.display())));
        }
        Err(e) => st.toast.set(Some(format!("Не сохранилось: {e}"))),
    };
    let reload = path.clone();
    let tools = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).max_lines(1).elide(Elide::Middle).class("fp-title"))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> { if saved.get() { vec![] } else { vec![Box::new(Text::new("● не сохранено").class("row-sub"))] } }))
        .child(DecoratedBox::new().class("grow"))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let p = preview.get();
            vec![Box::new(Button::new(if p { "Править" } else { "Просмотр" }).on_click(move || preview.update(|x| *x = !*x)).class("head-btn"))]
        }))
        .child(ToolButton::new("\u{E5D5}").tooltip("Отменить правки (перечитать файл)").on_click(move || {
            text.set(std::fs::read_to_string(&reload).unwrap_or_default());
            saved.set(true);
            cur.set(None);
            st.doc_preview.update(|_| {});
        }))
        .child(Button::new("Сохранить").on_click(save).class("head-btn"));
    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(tools.class("panel-head"))
            .child(DecoratedBox::new().class("grow").child(body)),
    )
}

/// Мини-редактор SVG: код (подсветка XML) и живая картинка рядом; «Сохранить»
/// пишет файл, миниатюра и кадры на таймлайне обновляются.
fn svg_view(st: St, id: u64, path: std::path::PathBuf) -> Box<dyn Widget> {
    use syngui::widgets::input::code_editor::syntax::Language;
    use syngui::widgets::input::code_editor::CodeEditorChange;
    let (cur, text, saved) = svgv();
    if cur.get_untracked() != Some(id) {
        cur.set(Some(id));
        text.set(std::fs::read_to_string(&path).unwrap_or_default());
        saved.set(true);
    }
    let editor = CodeEditor::new()
        .text(text.get_untracked())
        .language(Language::Xml)
        .soft_wrap(true)
        .tab_width(2)
        .on_change(move |ch: CodeEditorChange| {
            text.set(ch.full_text.to_string());
            saved.set(false);
        })
        .class("set-code");
    let preview = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let t = text.get();
        match syngui::gpu::rasterize_svg(t.as_bytes(), 1024) {
            Ok((w, h, rgba, _)) => {
                use std::hash::{Hash, Hasher};
                let mut hs = std::collections::hash_map::DefaultHasher::new();
                t.hash(&mut hs);
                vec![Box::new(Image::from_rgba(format!("svgv:{id}:{:x}", hs.finish()), w, h, rgba).fit(ImageFit::Contain).placeholder(false).class("pv-fill"))]
            }
            Err(e) => vec![Box::new(Center::new().child(Text::new(format!("Ошибка SVG: {e}")).max_lines(4).class("panel-empty")))],
        }
    });
    let save_path = path.clone();
    let save = move || {
        match crate::layout::write_atomic(&save_path, &text.get_untracked()) {
            Ok(()) => {
                saved.set(true);
                if let Some((w, h)) = crate::cmd::media::image_size(&save_path) {
                    let mut e = ed();
                    if let Some(m) = e.project.media.iter_mut().find(|m| m.id == id) {
                        (m.width, m.height) = (w, h);
                    }
                }
                super::thumbs::forget(&save_path);
                st.thumbs.update(|n| *n += 1);
                st.rev.update(|n| *n += 1);
                st.toast.set(Some(format!("Сохранено: {}", save_path.display())));
            }
            Err(e) => st.toast.set(Some(format!("Не сохранилось: {e}"))),
        }
    };
    let reload_path = path.clone();
    let tools = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(format!("m{id} · {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())).max_lines(1).elide(Elide::Middle).class("fp-title"))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> { if saved.get() { vec![] } else { vec![Box::new(Text::new("● не сохранено").class("row-sub"))] } }))
        .child(DecoratedBox::new().class("grow"))
        .child(ToolButton::new("\u{E5D5}").tooltip("Отменить правки (перечитать файл)").on_click(move || {
            text.set(std::fs::read_to_string(&reload_path).unwrap_or_default());
            saved.set(true);
            cur.set(None);
            st.media_preview.update(|_| {});
        }))
        .child(Button::new("Сохранить").on_click(save).class("head-btn"))
        .child(Button::new("На таймлайн").on_click(move || {
            st.exec("clip.add", json!({"media": id}));
        }).class("head-btn"));
    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(tools.class("panel-head"))
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .class("grow")
                    .child(DecoratedBox::new().class("grow").child(editor))
                    .child(DecoratedBox::new().class("grow").class("media-preview-body").child(preview)),
            ),
    )
}

/// Вкладка «Медиа» — просмотр файла медиатеки: картинка — масштаб, сдвиг,
/// поворот и отражение; видео и звук — плеер именно этого файла.
pub fn media_view_panel(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let fx = st.fx_preview.get();
        let media = st.media_preview.get();
        if let Some(p) = st.doc_preview.get() {
            if let Ok(mut g) = PREVIEW_PLAYER.lock() {
                *g = None;
            }
            if let Ok(mut g) = PREVIEW_AUDIO.lock() {
                *g = None;
            }
            FX_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return vec![doc_view(st, p)];
        }
        if fx.is_none() {
            // ленту эффекта больше не крутить
            FX_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if media.is_none() || fx.is_some() {
            // плеер прежнего файла остановить и отпустить
            if let Ok(mut g) = PREVIEW_PLAYER.lock() {
                *g = None;
            }
            if let Ok(mut g) = PREVIEW_AUDIO.lock() {
                *g = None;
            }
        }
        if let Some((key, title)) = fx {
            return vec![fx_view(st, key, title)];
        }
        let Some(id) = media else {
            return vec![empty("Двойной щелчок по файлу медиатеки, переходу или эффекту — просмотр здесь")];
        };
        let Some(m) = ed().project.media(id).cloned() else { return vec![empty("Файла больше нет в медиатеке")] };
        if crate::cmd::media::is_svg(&m.path) {
            return vec![svg_view(st, id, m.path.clone())];
        }
        let (turns, flip, cmd) = pv_state();

        let path = m.path.display().to_string();
        let is_image = m.kind == MediaKind::Image;
        let body: Box<dyn Widget> = if is_image {
            let (t, (fh, fv), (seq, c)) = (turns.get(), flip.get(), cmd.get());
            let mut v = ImageViewport::new(path.clone()).quarter_turns(t).flip(fh, fv).command(seq, c);
            if m.width > 0 && m.height > 0 {
                v = v.natural_size(m.width, m.height);
            }
            Box::new(v)
        } else if m.kind == MediaKind::Audio {
            // звук без видео — свой плеер syngui (видеодекодер требует видеопоток)
            if let Ok(mut g) = PREVIEW_PLAYER.lock() {
                *g = None;
            }
            let player = {
                let mut g = PREVIEW_AUDIO.lock().unwrap_or_else(|e| e.into_inner());
                match g.as_ref() {
                    Some((pid, p)) if *pid == id => Some(p.clone()),
                    _ => match syngui::video::AudioFilePlayer::open(&path) {
                        Ok(p) => {
                            let p = std::sync::Arc::new(std::sync::Mutex::new(p));
                            *g = Some((id, p.clone()));
                            Some(p)
                        }
                        Err(e) => {
                            st.toast.set(Some(format!("Просмотр: {e}")));
                            None
                        }
                    },
                }
            };
            match player {
                Some(p) => Box::new(VideoPlayerView::audio_file(p).build()),
                None => Box::new(Center::new().child(Text::new("Не удалось открыть файл").class("panel-empty"))),
            }
        } else if m.kind == MediaKind::Video {
            if let Ok(mut g) = PREVIEW_AUDIO.lock() {
                *g = None;
            }
            let player = {
                let mut g = PREVIEW_PLAYER.lock().unwrap_or_else(|e| e.into_inner());
                match g.as_ref() {
                    Some((pid, p)) if *pid == id => Some(p.clone()),
                    _ => match syngui::video::VideoPlayer::open(&path) {
                        Ok(p) => {
                            let p = std::sync::Arc::new(std::sync::Mutex::new(p));
                            *g = Some((id, p.clone()));
                            Some(p)
                        }
                        Err(e) => {
                            st.toast.set(Some(format!("Просмотр: {e}")));
                            None
                        }
                    },
                }
            };
            match player {
                Some(p) => Box::new(VideoPlayerView::file(p).build()),
                None => Box::new(Center::new().child(Text::new("Не удалось открыть файл").class("panel-empty"))),
            }
        } else {
            Box::new(Center::new().child(Text::new("Просмотр этого вида пока недоступен").class("panel-empty")))
        };
        let mut tools = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("m{id} · {}", m.name())).max_lines(1).elide(Elide::Middle).class("fp-title")).child(DecoratedBox::new().class("grow"));
        if is_image {
            let bump = move |c: ImageViewCommand| cmd.update(|x| *x = (x.0 + 1, c));
            tools = tools
                .child(ToolButton::new("\u{E8D5}").tooltip("Повернуть влево").on_click(move || turns.update(|t| *t = (*t + 3) % 4)))
                .child(ToolButton::new("\u{E41A}").tooltip("Повернуть вправо").on_click(move || turns.update(|t| *t = (*t + 1) % 4)))
                .child(ToolButton::new("\u{E3E8}").tooltip("Отразить по горизонтали").on_click(move || flip.update(|f| f.0 = !f.0)))
                .child(ToolButton::new("\u{E8FF}").tooltip("Крупнее").on_click(move || bump(ImageViewCommand::ZoomIn)))
                .child(ToolButton::new("\u{E900}").tooltip("Мельче").on_click(move || bump(ImageViewCommand::ZoomOut)))
                .child(ToolButton::new("\u{E5D0}").tooltip("Вписать").on_click(move || bump(ImageViewCommand::Fit)))
                .child(Button::new("1:1").on_click(move || bump(ImageViewCommand::Actual)).class("head-btn"));
        }
        tools = tools
            .child(Button::new("На таймлайн").on_click(move || {
                st.exec("clip.add", json!({"media": id}));
            }).class("head-btn"));
        vec![Box::new(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(tools.class("panel-head")).child(DecoratedBox::new().class("grow").class("media-preview-body").child(body)))]
    })
}
