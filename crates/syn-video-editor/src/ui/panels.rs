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
        let gen = st.gen_tiles.get();
        if items.is_empty() && gen.is_empty() {
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
        for g in gen {
            let frac = if g.total > 0 { g.step as f32 / g.total as f32 } else { 0.0 };
            let what = match g.op.as_str() { "image" => "картинка", "video" => "ролик", "music" => "музыка", _ => "звук" };
            let item = g.item.map(|(k, n)| format!(" {k} из {n}")).unwrap_or_default();
            let step = if g.total > 0 { format!("шаг {}/{}", g.step, g.total) } else { "подготовка…".into() };
            grid = grid.child(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(AspectRatio::new(16.0 / 9.0).child(match &g.preview {
                        // превью по ходу: картинка проявляется на плитке
                        Some(p) => Box::new(DecoratedBox::new().class("tile-thumb gen-thumb").child(Image::new(p.display().to_string()).fit(ImageFit::Cover))) as Box<dyn Widget>,
                        None => Box::new(DecoratedBox::new().class("tile-thumb gen-thumb").child(Center::new().child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(if g.op == "video" { icons::MOVIE } else { icons::IMAGE }).class("media-empty-icon")).child(Text::new(step.clone()).class("row-sub"))))),
                    }))
                    .child(Text::new(step).class("row-sub"))
                    .child(ProgressBar::with_value(frac).class("gen-bar"))
                    .child(Text::new(format!("Генерируется {what}{item}")).max_lines(1).elide(Elide::End).class("row-title"))
                    .child(Text::new(g.node.clone()).max_lines(1).elide(Elide::End).class("row-sub"))
                    .class("tile gen-tile"),
            );
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
        .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(header).child(DecoratedBox::new().class("grow").child(list)))
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
        .child(Text::new(title).max_lines(1).elide(Elide::End).class("row-title"));
    let apply = move || {
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
    };
    Box::new(GestureDetector::new().on_click(apply).child(DecoratedBox::new().class("tile").child(body)))
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

// ── ИИ ──────────────────────────────────────────────────────────────────

fn ai(st: St) -> impl Widget {
    let input_gen = use_signal(0u64);
    let draft = use_signal(String::new());
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ai-panel")
        .child(super::ai_chat::head(st))
        .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().follow_end(true).child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let log = st.ai_log.get();
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
                        col = col.child(DecoratedBox::new().class("ai-text").child(MarkdownView::new(t).with_copy_code(true).selectable(true).class("ai-text-text")));
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
                col = col.child(DecoratedBox::new().class(class).child(Text::new(text).selectable(true).class(format!("{class}-text"))));
            }
            vec![Box::new(col)]
        }))))
        .child(DecoratedBox::new().class("ai-input").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let _ = input_gen.get();
            let busy = st.ai_busy.get();
            let stop: Option<Box<dyn Widget>> = busy.then(|| -> Box<dyn Widget> {
                Box::new(Button::new("Стоп").on_click(|| super::AI_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst)).class("head-btn"))
            });
            vec![Box::new(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(stop.into_iter().collect::<Vec<Box<dyn Widget>>>()).child(DecoratedBox::new().class("grow").child(
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
                // просмотр этого файла (картинка — зум и поворот, видео — плеер);
                // на таймлайн — кнопкой в окне или пунктом меню
                st.media_preview.set(Some(id));
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

/// Плеер открытого в просмотре видео/звука (один на окно).
static PREVIEW_PLAYER: std::sync::Mutex<Option<(u64, std::sync::Arc<std::sync::Mutex<syngui::video::VideoPlayer>>)>> = std::sync::Mutex::new(None);

thread_local! {
    static PV: std::cell::RefCell<Option<(RwSignal<i32>, RwSignal<(bool, bool)>, RwSignal<(u64, ImageViewCommand)>)>> = const { std::cell::RefCell::new(None) };
}

fn pv_state() -> (RwSignal<i32>, RwSignal<(bool, bool)>, RwSignal<(u64, ImageViewCommand)>) {
    PV.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(0), use_signal((false, false)), use_signal((0, ImageViewCommand::Fit)))))
}

/// Окно просмотра файла медиатеки: картинка — масштаб, сдвиг, поворот и
/// отражение; видео и звук — плеер именно этого файла.
pub fn media_preview_view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(id) = st.media_preview.get() else {
            // закрыто — плеер остановить и отпустить
            if let Ok(mut g) = PREVIEW_PLAYER.lock() {
                *g = None;
            }
            return Vec::new();
        };
        let Some(m) = ed().project.media(id).cloned() else { return Vec::new() };
        let (turns, flip, cmd) = pv_state();
        let close = move || {
            st.media_preview.set(None);
            turns.set(0);
            flip.set((false, false));
        };
        let path = m.path.display().to_string();
        let is_image = m.kind == MediaKind::Image;
        let body: Box<dyn Widget> = if is_image {
            let (t, (fh, fv), (seq, c)) = (turns.get(), flip.get(), cmd.get());
            let mut v = ImageViewport::new(path.clone()).quarter_turns(t).flip(fh, fv).command(seq, c);
            if m.width > 0 && m.height > 0 {
                v = v.natural_size(m.width, m.height);
            }
            Box::new(v)
        } else if matches!(m.kind, MediaKind::Video | MediaKind::Audio) {
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
            }).class("head-btn"))
            .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(close));
        let card = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(tools).child(DecoratedBox::new().class("grow").class("media-preview-body").child(body));
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("fp-scrim")))
                .child(Padding::all(48.0).child(EventHook::new().on_key_down(move |k, _| {
                    if k == Key::Escape {
                        close();
                        return KeyReply::Handled;
                    }
                    KeyReply::Ignore
                }).child(DecoratedBox::new().class("fp-card").class("grow").child(Padding::all(12.0).child(card))))),
        )]
    })
}
