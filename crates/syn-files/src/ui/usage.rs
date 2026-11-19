//! Панель «Занятое место» справа (кнопка в строке состояния, Ctrl+Shift+U):
//! кольцо папок (как Baobab/Filelight) открытой папки и список её содержимого
//! по размеру. Щелчок по папке — панель файлов переходит в неё (кольцо идёт
//! следом), по файлу — он выделяется в панели, по центру — на уровень вверх.
//! Правая кнопка и корзина в строке — чистка (в корзину или навсегда).

use std::path::{Path, PathBuf};

use syngui::prelude::*;
use syngui::widgets::charts::types::palette_color;
use syngui::widgets::*;

use super::{boxed, bx, icons, W};
use crate::loc::Location;
use crate::model;
use crate::ops::{self, Op};
use crate::state::{self, Dialog};
use crate::usage::{self, Tree};

/// Колец в кольце.
const LEVELS: usize = 4;
/// Доля папки меньше этой — в кольцо не идёт.
const MIN_SHARE: f64 = 0.003;

// Сектор/строка: папка — индекс узла; файл — флаг, узел папки и номер в её
// `top`; «другие файлы» — флаг и узел.
const FILE: u64 = 1 << 63;
const REST: u64 = 1 << 62;

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Dir(u32),
    File(u32, usize),
    Rest(u32),
}

fn item_of(id: u64) -> Item {
    if id & FILE != 0 {
        Item::File(((id & !FILE) >> 8) as u32, (id & 0xff) as usize)
    } else if id & REST != 0 {
        Item::Rest((id & !REST) as u32)
    } else {
        Item::Dir(id as u32)
    }
}

fn file_id(dir: u32, k: usize) -> u64 {
    FILE | ((dir as u64) << 8) | k as u64
}

/// Путь, имя и размер элемента.
fn describe(t: &Tree, it: &Item) -> Option<(PathBuf, String, u64)> {
    match *it {
        Item::Dir(i) => {
            let n = t.nodes.get(i as usize).filter(|n| !n.gone)?;
            Some((t.path(i), if i == 0 { crate::archive::display(&t.root).display().to_string() } else { n.name.clone() }, n.bytes))
        }
        Item::File(d, k) => {
            let (name, b) = t.nodes.get(d as usize)?.top.get(k)?.clone();
            Some((t.path(d).join(&name), name, b))
        }
        Item::Rest(d) => {
            let n = t.nodes.get(d as usize)?;
            Some((t.path(d), t!("Другие файлы: {v}", v = n.rest.0), n.rest.1))
        }
    }
}

fn file_color() -> Color {
    Color::from_hex("#8b919c")
}

fn rest_color() -> Color {
    Color::from_hex("#5d626b")
}

/// Дети узла для кольца: папки по размеру, крупные файлы, «другие файлы».
fn chart_nodes(t: &Tree, i: u32, total: u64, depth: usize, top_level: bool) -> Vec<SunburstNode> {
    if depth == 0 || total == 0 {
        return Vec::new();
    }
    let min = (total as f64 * MIN_SHARE) as u64;
    let mut v = Vec::new();
    for (k, d) in t.sorted_dirs(i).into_iter().enumerate() {
        let n = t.node(d);
        if n.bytes <= min {
            break;
        }
        let mut node = SunburstNode::new(d as u64, n.name.clone(), n.bytes as f64).children(chart_nodes(t, d, total, depth - 1, false));
        if top_level {
            node = node.color(palette_color(k));
        }
        v.push(node);
    }
    let n = t.node(i);
    for (k, (name, b)) in n.top.iter().enumerate() {
        if *b <= min {
            break;
        }
        v.push(SunburstNode::new(file_id(i, k), name.clone(), *b as f64).color(file_color()));
    }
    if n.rest.1 > min {
        v.push(SunburstNode::new(REST | i as u64, String::new(), n.rest.1 as f64).color(rest_color()));
    }
    // Кольцо — по убыванию, вперемешку папки и файлы (как в списке).
    v.sort_by(|a, b| b.value.total_cmp(&a.value));
    v
}

/// Строки списка: (элемент, имя, байт, цвет точки).
fn list_items(t: &Tree, i: u32) -> Vec<(Item, String, u64, Color)> {
    let mut v = Vec::new();
    for (k, d) in t.sorted_dirs(i).into_iter().enumerate() {
        let n = t.node(d);
        v.push((Item::Dir(d), n.name.clone(), n.bytes, palette_color(k)));
    }
    let n = t.node(i);
    for (k, (name, b)) in n.top.iter().enumerate() {
        v.push((Item::File(i, k), name.clone(), *b, file_color()));
    }
    if n.rest.0 > 0 {
        v.push((Item::Rest(i), t!("Другие файлы: {v}", v = n.rest.0), n.rest.1, rest_color()));
    }
    v.sort_by(|a, b| b.2.cmp(&a.2));
    v
}

/// Папка панели, которую анализируем (или причина, почему нельзя).
fn target(loc: &Location) -> std::result::Result<PathBuf, String> {
    match loc {
        Location::Dir(d) if crate::archive::root_of(d).is_some() => Err(t!("Архивы не анализируются")),
        Location::Dir(d) if usage::skipped(d) => Err(t!("Эта папка не анализируется")),
        Location::Dir(d) if !d.as_os_str().is_empty() => Ok(d.clone()),
        _ => Err(t!("Откройте папку, чтобы увидеть, что занимает место")),
    }
}

fn act(it: Item) {
    let Some(s) = usage::current() else { return };
    let Some((path, _, _)) = describe(&s.tree.lock().unwrap(), &it) else { return };
    let p = state::pane();
    match it {
        Item::Dir(_) => state::navigate(p, Location::Dir(path), true),
        Item::File(..) => {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
            if p.loc.get_untracked().dir() != Some(parent.as_path()) {
                state::navigate(p, Location::Dir(parent), true);
            }
            state::select_later(p, path);
        }
        Item::Rest(_) => {}
    }
}

fn trash(path: PathBuf, permanent: bool) {
    if permanent || state::ctx().cfg.get_untracked().files.confirm_trash {
        state::ctx().dialog.set(Some(Dialog::ConfirmDelete { paths: vec![path], permanent }));
        return;
    }
    ops::start(Op::Trash { srcs: vec![path.clone()] });
    usage::forget_path(&path);
    state::toast_undo(t!("В корзину: {v}", v = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
}

fn menu(it: Item, at: Point) {
    let Some(s) = usage::current() else { return };
    let Some((path, _, _)) = describe(&s.tree.lock().unwrap(), &it) else { return };
    if matches!(it, Item::Rest(_)) {
        return;
    }
    let dir = matches!(it, Item::Dir(_));
    let mi = |id: &str, label: String, glyph: &str| MenuItem::new(id, label).icon(glyph);
    let mut items = vec![
        mi("open", if dir { t!("Перейти") } else { t!("Открыть") }, icons::OPEN),
        mi("show", t!("Показать в папке"), icons::FOLDER_OPEN),
        mi("copy-path", t!("Копировать путь"), icons::COPY_PATH),
        MenuItem::separator(),
        mi("trash", t!("Удалить в корзину"), icons::DELETE).shortcut("Delete"),
        mi("delete", t!("Удалить навсегда"), icons::DELETE_FOREVER).shortcut("Shift+Delete"),
    ];
    if dir && it == Item::Dir(0) {
        items.truncate(3);
    }
    state::show_menu(items, at, move |id| {
        let p = state::pane();
        match id {
            "open" if dir => state::navigate(p, Location::Dir(path.clone()), true),
            "open" => {
                if let Some(e) = model::Entry::from_path(&path) {
                    crate::actions::open_entry(p, &e, false);
                }
            }
            "show" => {
                if let Some(parent) = path.parent() {
                    state::navigate(p, Location::Dir(parent.to_path_buf()), true);
                    state::select_later(p, path.clone());
                }
            }
            "copy-path" => crate::actions::copy_paths_text(std::slice::from_ref(&path)),
            "trash" => trash(path.clone(), false),
            "delete" => trash(path.clone(), true),
            _ => {}
        }
    });
}

fn pct(part: u64, whole: u64) -> String {
    if whole == 0 {
        return String::new();
    }
    let v = part as f64 / whole as f64 * 100.0;
    if v >= 10.0 {
        format!("{v:.0} %")
    } else {
        format!("{v:.1} %")
    }
}

/// Заголовок: название, путь, кнопки «вверх», «пересчитать», «закрыть»; строка итогов.
fn header() -> W {
    let top = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let _ = ctx.usage_rev.get();
        let p = state::tab_tracked().pane_tracked();
        let loc = p.loc.get();
        let dir = target(&loc).ok();
        if let Some(d) = &dir {
            usage::ensure(d);
        }
        let status = match (usage::current(), &dir) {
            (Some(s), Some(_)) => {
                let t = s.tree.lock().unwrap();
                let root = t.node(0);
                if t.done {
                    let mut line = t!(
                        "{size} · файлов: {files} · папок: {dirs}",
                        size = model::format_size(root.bytes),
                        files = root.files,
                        dirs = t.dirs
                    );
                    if t.denied > 0 {
                        line.push_str(&t!(" · без доступа: {n}", n = t.denied));
                    }
                    line
                } else {
                    t!("Подсчёт… {size}, файлов: {files}", size = model::format_size(root.bytes), files = root.files)
                }
            }
            _ => String::new(),
        };
        let scanning = usage::current().is_some_and(|s| !s.tree.lock().unwrap().done);
        let root_label = usage::current().map(|s| crate::archive::display(&s.tree.lock().unwrap().root).display().to_string()).unwrap_or_default();
        let can_up = loc.parent().is_some() && dir.is_some();
        let d2 = dir.clone();
        let mut row = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icons::DONUT).class("icon us-title-icon"))
            .child(
                Column::new()
                    .gap(1.0)
                    .class("grow")
                    .child(Text::new(t!("Занятое место")).max_lines(1).class("us-title"))
                    .child(Text::new(if dir.is_some() { root_label } else { String::new() }).max_lines(1).class("us-sub")),
            );
        if scanning {
            row = row.child(bx("us-spinner", CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0)));
        }
        row = row
            .child(super::icon_button(icons::UP, &t!("На уровень вверх"), "small", can_up, move || state::go_up(state::pane())))
            .child(super::icon_button(icons::REFRESH, &t!("Пересчитать"), "small", dir.is_some(), move || {
                if let Some(d) = &d2 {
                    usage::rescan(d);
                }
            }))
            .child(super::icon_button(icons::CLOSE, &t!("Закрыть (Ctrl+Shift+U)"), "small", true, toggle));
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(row);
        if !status.is_empty() {
            col = col.child(Text::new(status).max_lines(1).class("us-status"));
        }
        vec![boxed(col)]
    });
    bx("us-head", top)
}

/// Кольцо открытой папки.
fn chart() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let _ = ctx.usage_rev.get();
        let hover = ctx.usage_hover.get();
        let p = state::tab_tracked().pane_tracked();
        let loc = p.loc.get();
        let dir = match target(&loc) {
            Ok(d) => d,
            Err(why) => return vec![placeholder(icons::DONUT, why)],
        };
        let Some(s) = usage::current() else { return vec![] };
        let t = s.tree.lock().unwrap();
        let Some(f) = t.find(&dir) else {
            return vec![placeholder(icons::HOURGLASS, t!("Папка ещё считается…"))];
        };
        let n = t.node(f);
        let total = n.bytes;
        if total == 0 {
            let why = if n.read { t!("Папка пуста") } else { t!("Папка ещё считается…") };
            return vec![placeholder(icons::FOLDER_OPEN, why)];
        }
        let nodes = chart_nodes(&t, f, total, LEVELS, true);
        let name = if f == 0 { loc.title() } else { n.name.clone() };
        let center = vec![model::format_size(total), name];
        drop(t);
        vec![boxed(
            SunburstChart::new()
                .levels(LEVELS)
                .total(total as f64)
                .nodes(nodes)
                .center_text(center)
                .highlight(hover)
                .on_hover(move |id| state::ctx().usage_hover.set(id))
                .on_click(move |id| act(item_of(id)))
                .on_center_click(move || state::go_up(state::pane()))
                .on_secondary_click(move |id, at| {
                    if let Some(id) = id {
                        menu(item_of(id), at);
                    }
                })
                .class("us-chart"),
        )]
    }))
}

/// Под кольцом: что под указателем (или итог папки).
fn hover_line() -> W {
    bx(
        "us-hover",
        Reactive::new(move || -> Vec<W> {
            let ctx = state::ctx();
            let _ = ctx.usage_rev.get();
            let hover = ctx.usage_hover.get();
            let p = state::tab_tracked().pane_tracked();
            let Ok(dir) = target(&p.loc.get()) else { return vec![] };
            let Some(s) = usage::current() else { return vec![] };
            let t = s.tree.lock().unwrap();
            let Some(f) = t.find(&dir) else { return vec![] };
            let whole = t.node(f).bytes;
            let (title, sub) = match hover.map(item_of).and_then(|it| describe(&t, &it).map(|d| (it, d))) {
                Some((it, (path, name, b))) => {
                    let mut sub = format!("{} · {}", model::format_size(b), pct(b, whole));
                    if let Item::Dir(i) = it {
                        sub.push_str(&t!(" · файлов: {n}", n = t.node(i).files));
                    }
                    let shown = if matches!(it, Item::Rest(_)) { name } else { crate::archive::display(&path).display().to_string() };
                    (shown, sub)
                }
                None => {
                    let n = t.node(f);
                    (t!("Щелчок по кольцу — перейти, по центру — вверх"), t!("{size} · файлов: {files}", size = model::format_size(n.bytes), files = n.files))
                }
            };
            vec![boxed(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(title).max_lines(1).class("us-hover-title"))
                    .child(Text::new(sub).max_lines(1).class("us-hover-sub")),
            )]
        }),
    )
}

fn row(it: Item, name: String, bytes: u64, whole: u64, dot: Color) -> W {
    let id = match it {
        Item::Dir(i) => i as u64,
        Item::File(d, k) => file_id(d, k),
        Item::Rest(d) => REST | d as u64,
    };
    let glyph = match it {
        Item::Dir(_) => icons::FOLDER,
        Item::File(..) => icons::FILE,
        Item::Rest(_) => icons::MORE,
    };
    let frac = if whole > 0 { (bytes as f64 / whole as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    const BAR: f32 = 72.0;
    let bar = DecoratedBox::new()
        .class("us-bar")
        .child(DecoratedBox::new().class("us-bar-fill").style("width", syngui::mss::StyleValue::px((BAR * frac).max(1.0))).style("background-color", syngui::mss::StyleValue::from(dot)));
    let actionable = !matches!(it, Item::Rest(_));
    let mut r = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(glyph).class(if matches!(it, Item::Dir(_)) { "icon us-row-icon dir" } else { "icon us-row-icon" }))
        .child(Text::new(name).max_lines(1).class("us-name grow"))
        .child(Text::new(pct(bytes, whole)).max_lines(1).class("us-pct"))
        .child(bar)
        .child(Text::new(model::format_size(bytes)).max_lines(1).class("us-size"));
    let it2 = it.clone();
    if actionable {
        let it3 = it.clone();
        r = r.child(super::icon_button(icons::DELETE, &t!("Удалить в корзину"), "small us-trash", true, move || {
            // Блокировка дерева — только на время чтения: `trash` берёт её снова.
            let found = usage::current().and_then(|s| describe(&s.tree.lock().unwrap(), &it3));
            if let Some((path, _, _)) = found {
                trash(path, false);
            }
        }));
    }
    boxed(
        GestureDetector::new()
            .on_hover_change(move |h| {
                let hv = state::ctx().usage_hover;
                if h {
                    hv.set(Some(id));
                } else if hv.get_untracked() == Some(id) {
                    hv.set(None);
                }
            })
            .on_click(move || act(it2.clone()))
            .on_secondary_click(move |at| menu(it.clone(), at))
            .child(DecoratedBox::new().class(if actionable { "us-row" } else { "us-row rest" }).child(r)),
    )
}

fn list() -> W {
    let rows = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let _ = ctx.usage_rev.get();
        let p = state::tab_tracked().pane_tracked();
        let Ok(dir) = target(&p.loc.get()) else { return vec![] };
        let Some(s) = usage::current() else { return vec![] };
        let t = s.tree.lock().unwrap();
        let Some(f) = t.find(&dir) else { return vec![] };
        let whole = t.node(f).bytes;
        let items = list_items(&t, f);
        drop(t);
        let mut col = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (it, name, b, dot) in items {
            col = col.child(row(it, name, b, whole, dot));
        }
        vec![boxed(col)]
    });
    boxed(ScrollView::new().vertical().class("grow us-list").child(bx("us-list-body", rows)))
}

fn placeholder(glyph: &str, text: String) -> W {
    boxed(
        Column::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("us-empty grow")
            .child(Icon::new(glyph).class("empty-icon"))
            .child(Text::new(text).class("empty-text")),
    )
}

pub fn panel() -> W {
    let ctx = state::ctx();
    let top = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(bx("grow us-chart-wrap", chart())).child(hover_line());
    let split = SplitView::new(DecoratedBox::new().class("grow").child(top), DecoratedBox::new().class("grow").child(list()))
        .direction(SplitDirection::Vertical)
        .ratio_signal(ctx.usage_chart_ratio)
        .min_size(120.0)
        .class("us-split");
    boxed(DecoratedBox::new().class("usage").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(header()).child(bx("grow", split))))
}

/// Открыть или закрыть панель; закрытая — обход останавливается.
pub fn toggle() {
    let ctx = state::ctx();
    let on = !ctx.usage.get_untracked();
    ctx.usage.set(on);
    ctx.usage_hover.set(None);
    if !on {
        usage::stop();
    }
}

/// Кнопка в строке состояния.
pub fn status_button() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let on = state::ctx().usage.get();
        let face = DecoratedBox::new().class(if on { "us-toggle on" } else { "us-toggle" }).child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::DONUT).class("icon"))
                .child(Text::new(t!("Занятое место")).max_lines(1).class("status-text")),
        );
        vec![boxed(Tooltip::new(GestureDetector::new().on_click(toggle).child(face), t!("Что занимает место: кольцо папок и чистка (Ctrl+Shift+U)")).delay_ms(500))]
    }))
}
