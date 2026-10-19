//! Заметки на рабочем столе: виджеты из syn-notes и вид стола «Заметки».
//!
//! Данные — сводка хозяина заметок (`syn_notes_proto::desk`): повестка дня,
//! доски с карточками и разделы страницы стола (`[notes] desk_page`)
//! основного хранилища. Опрос — в фоне, пока стол виден; тап по строке
//! открывает страницу в syn-notes.
//!
//! Виджеты: `notes-agenda` (просрочено, сегодня, завтра, события),
//! `notes-board` (доска: `board` — id или страница, `column` — одна колонка),
//! `notes-page` (страница `page` текстом). Вид «Заметки» раскладывает
//! разделы страницы стола карточками в колонки: текст — markdown, врезка
//! доски — доска, календарь — повестка.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use synshell_common::config::DeskWidget;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::visual::MarkdownView;
use syngui::GestureDetector;

use super::views::Cx;
use crate::ui::{icon, rx};
use crate::ShellCtx;

/// Сводка: `None` — хозяин заметок не отвечает.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotesSnap {
    pub data: Option<Value>,
    /// Хозяин отвечает, но открытого проекта нет (заняты другой программой).
    pub no_project: bool,
}

thread_local! {
    static SNAP: Cell<Option<RwSignal<NotesSnap>>> = const { Cell::new(None) };
}

/// Запросить сводку сейчас (после правки, переключения вида).
static KICK: AtomicBool = AtomicBool::new(false);
/// Вид «Заметки» показан — опрашивать и без виджетов заметок.
static NOTES_VIEW: AtomicBool = AtomicBool::new(false);

pub fn snap() -> RwSignal<NotesSnap> {
    SNAP.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(NotesSnap::default());
            s.set(Some(x));
            start(x);
            x
        }
    })
}

pub fn refresh() {
    KICK.store(true, Ordering::Relaxed);
}

fn fetch() -> NotesSnap {
    let (cfg, _) = synshell_common::Config::load();
    let page = cfg.notes.desk_page.trim().to_string();
    match syn_notes_proto::desk((!page.is_empty()).then_some(page.as_str())) {
        Ok(d) => NotesSnap { data: Some(d), no_project: false },
        Err(e) => NotesSnap { data: None, no_project: e.contains("no notes project") },
    }
}

fn start(sig: RwSignal<NotesSnap>) {
    std::thread::Builder::new()
        .name("desk-notes".into())
        .spawn(move || {
            let mut last: Option<NotesSnap> = None;
            let mut waited = Duration::ZERO;
            loop {
                let due = last.is_none() || waited >= Duration::from_secs(20) || KICK.swap(false, Ordering::Relaxed);
                if due && (super::data::visible() || NOTES_VIEW.load(Ordering::Relaxed)) {
                    waited = Duration::ZERO;
                    let s = fetch();
                    if last.as_ref() != Some(&s) {
                        last = Some(s.clone());
                        syngui::async_runtime::run_on_main_thread(move || sig.set(s));
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
                waited += Duration::from_millis(500);
            }
        })
        .ok();
}

/// Открыть страницу (id) в syn-notes; без страницы — само окно заметок.
pub fn open(page: Option<String>) {
    std::thread::spawn(move || {
        let r = match page {
            Some(p) => syn_notes_proto::open(&p),
            None => syn_notes_proto::ensure_running(Duration::from_secs(20))
                .and_then(|_| syn_notes_proto::request(&serde_json::json!({"cmd": "show"}), Duration::from_secs(5)).map(drop).map_err(|e| e.to_string())),
        };
        if let Err(e) = r {
            log::warn!("заметки: {e}");
        }
        refresh();
    });
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn offline(cx: &Cx) -> Box<dyn Widget> {
    if snap().get_untracked().no_project {
        return Box::new(
            GestureDetector::new().on_click(|| open(None)).child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(t!("Нет открытого проекта заметок")).class("desk-title"))
                    .child(Text::new(t!("Проект может быть открыт в другой программе (synthos). Коснитесь, чтобы открыть заметки")).max_lines(3).class("desk-k"))
                    .style("height", StyleValue::px(cx.h)),
            ),
        );
    }
    Box::new(
        GestureDetector::new().on_click(|| open(None)).child(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(Text::new(t!("Заметки не запущены")).class("desk-title"))
                .child(Text::new(t!("Коснитесь, чтобы открыть syn-notes")).max_lines(2).class("desk-k"))
                .style("height", StyleValue::px(cx.h)),
        ),
    )
}

/// Строка со ссылкой на страницу.
fn link_row(dot: &'static str, title: String, sub: String, page: Option<String>) -> Box<dyn Widget> {
    let mut text = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).max_lines(2).class("desk-notes-title"));
    if !sub.is_empty() {
        text = text.child(Text::new(sub).max_lines(1).class("desk-k"));
    }
    Box::new(
        GestureDetector::new().on_click(move || open(page.clone())).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("desk-notes-row")
                .child(DecoratedBox::new().class(format!("desk-notes-dot {dot}")))
                .child(DecoratedBox::new().class("grow").child(text)),
        ),
    )
}

// ─── Повестка ──────────────────────────────────────────────────────────────

fn agenda_list(data: &Value) -> Column {
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let items = data.get("agenda").and_then(Value::as_array).cloned().unwrap_or_default();
    let groups: [(&str, &'static str, String); 5] = [
        ("overdue", "overdue", t!("Просрочено")),
        ("today", "today", t!("Сегодня")),
        ("event", "event", t!("События сегодня")),
        ("tomorrow", "tomorrow", t!("Завтра")),
        ("event_tomorrow", "tomorrow", t!("События завтра")),
    ];
    let mut any = false;
    for (kind, dot, label) in groups {
        let list: Vec<&Value> = items.iter().filter(|i| s(i, "kind") == kind).collect();
        if list.is_empty() {
            continue;
        }
        any = true;
        col = col.child(Text::new(format!("{label} · {}", list.len())).class("desk-notes-group"));
        for i in list {
            let page = i.get("page").and_then(Value::as_str).map(String::from);
            col = col.child(link_row(dot, s(i, "title").to_string(), s(i, "sub").to_string(), page));
        }
    }
    if !any {
        col = col.child(Text::new(t!("На сегодня и завтра сроков нет")).max_lines(2).class("desk-k"));
    }
    col
}

pub(super) fn agenda(cx: Cx) -> Box<dyn Widget> {
    let sig = snap();
    Box::new(rx(move || {
        let snap = sig.get();
        let Some(data) = snap.data else { return offline(&cx) };
        Box::new(ScrollView::new().vertical().child(agenda_list(&data)).style("height", StyleValue::px(cx.h)))
    }))
}

// ─── Доска ─────────────────────────────────────────────────────────────────

/// Доска по `board` (id объекта или название страницы); пусто — первая.
fn find_board<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    let boards = data.get("boards")?.as_array()?;
    let key = key.trim().to_lowercase();
    if key.is_empty() {
        return boards.first();
    }
    boards.iter().find(|b| s(b, "id") == key || s(b, "page") == key || s(b, "page_title").to_lowercase() == key)
}

/// `h` — высота виджета (со своей прокруткой); `None` — по содержимому
/// (карточка вида «Заметки», прокручивается вся страница).
fn board_view(b: &Value, column: &str, horizontal: bool, h: Option<f32>) -> Box<dyn Widget> {
    let page = Some(s(b, "page").to_string());
    let columns = b.get("columns").and_then(Value::as_array).cloned().unwrap_or_default();
    let column = column.trim().to_lowercase();
    let count = |c: &Value| c.get("cards").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let mut shown: Vec<&Value> = columns
        .iter()
        .filter(|c| if column.is_empty() { !c.get("done").and_then(Value::as_bool).unwrap_or(false) } else { s(c, "name").to_lowercase() == column || s(c, "id") == column })
        .collect();
    // Пустые колонки — шум; все пустые — показать первую («нет задач»).
    if column.is_empty() && shown.iter().any(|c| count(c) > 0) {
        shown.retain(|c| count(c) > 0);
    } else if column.is_empty() {
        shown.truncate(1);
    }
    let col_view = |c: &Value| -> Column {
        let cards = c.get("cards").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut list = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("desk-notes-dot").style("background-color", StyleValue::Color(super::views::mss_color(syngui::core::Color::from_hex(if s(c, "color").starts_with('#') { s(c, "color") } else { "#8b95a6" })))))
                .child(Text::new(format!("{} · {}", s(c, "name"), cards.len())).max_lines(1).class("desk-notes-group")),
        );
        for card in cards {
            let overdue = card.get("overdue").and_then(Value::as_bool).unwrap_or(false);
            let due = s(&card, "due").to_string();
            let sub = if due.is_empty() { String::new() } else { t!("срок {due}", due = due) };
            list = list.child(link_row(if overdue { "overdue" } else { "card" }, s(&card, "title").to_string(), sub, page.clone()));
        }
        list
    };
    let body: Box<dyn Widget> = if horizontal && shown.len() > 1 {
        let mut row = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start);
        for c in &shown {
            row = row.child(DecoratedBox::new().style("width", StyleValue::px(220.0)).child(col_view(c)));
        }
        Box::new(ScrollView::new().horizontal().child(row))
    } else {
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for c in &shown {
            col = col.child(col_view(c));
        }
        match h {
            Some(_) => Box::new(ScrollView::new().vertical().child(col)),
            None => Box::new(col),
        }
    };
    match h {
        Some(h) => Box::new(DecoratedBox::new().style("height", StyleValue::px(h)).child(body)),
        None => body,
    }
}

pub(super) fn board(w: &DeskWidget, cx: Cx) -> Box<dyn Widget> {
    let sig = snap();
    let key = w.str_or("board", "").to_string();
    let column = w.str_or("column", "").to_string();
    Box::new(rx(move || {
        let snap = sig.get();
        let Some(data) = snap.data else { return offline(&cx) };
        match find_board(&data, &key) {
            Some(b) => board_view(b, &column, cx.w > 520.0, Some(cx.h)),
            None => Box::new(Text::new(t!("Доска не найдена")).class("desk-k")),
        }
    }))
}

// ─── Страница ──────────────────────────────────────────────────────────────

pub(super) fn page(w: &DeskWidget, cx: Cx) -> Box<dyn Widget> {
    let sig = snap();
    let name = w.str_or("page", "").to_string();
    // Страница виджета запрашивается отдельно, если это не страница стола.
    let own = use_signal(None::<Value>);
    if !name.is_empty() {
        let n = name.clone();
        std::thread::spawn(move || {
            let d = syn_notes_proto::desk(Some(&n)).ok();
            syngui::async_runtime::run_on_main_thread(move || own.set(d));
        });
    }
    Box::new(rx(move || {
        let data = own.get().or_else(|| sig.get().data);
        let Some(data) = data else { return offline(&cx) };
        let Some(p) = data.get("page").filter(|p| !p.is_null()) else {
            return Box::new(Text::new(t!("Страница не найдена")).class("desk-k"));
        };
        let md: Vec<String> = p
            .get("sections")
            .and_then(Value::as_array)
            .map(|v| v.iter().filter(|s2| s(s2, "kind") == "text").map(|s2| s(s2, "md").to_string()).collect())
            .unwrap_or_default();
        let id = Some(s(p, "id").to_string());
        Box::new(
            GestureDetector::new().on_click(move || open(id.clone())).child(
                ScrollView::new()
                    .vertical()
                    .child(MarkdownView::new(md.join("\n\n")).class("desk-notes-md"))
                    .style("height", StyleValue::px(cx.h)),
            ),
        )
    }))
}

// ─── Вид стола «Заметки» ───────────────────────────────────────────────────

/// Разделы страницы стола карточками в колонках шириной ~380 px.
pub fn desk_view(width: f32, height: f32, top: f32) -> Box<dyn Widget> {
    NOTES_VIEW.store(true, Ordering::Relaxed);
    refresh();
    let sig = snap();
    Box::new(rx(move || {
        let snap = sig.get();
        let pad = 24.0;
        let Some(data) = snap.data else {
            return Box::new(Padding::all(pad).child(DecoratedBox::new().class("desk-card desk-notes-card").style("width", StyleValue::px(360.0)).child(offline(&Cx {
                w: 320.0,
                h: 60.0,
                color: Default::default(),
                view: String::new(),
                history: 0,
            }))));
        };
        let ncols = (((width - pad) / 392.0).floor() as usize).clamp(1, 6);
        let colw = ((width - pad * (ncols as f32 + 1.0)) / ncols as f32).max(240.0);
        let mut cards: Vec<(f32, Box<dyn Widget>)> = Vec::new();
        let page = data.get("page").filter(|p| !p.is_null()).cloned();
        let sections = page.as_ref().and_then(|p| p.get("sections")).and_then(Value::as_array).cloned().unwrap_or_default();
        if page.is_none() {
            // Страницы стола нет — повестка и все доски.
            cards.push((agenda_height(&data), card(t!("Повестка"), None, Box::new(agenda_list(&data)))));
            for b in data.get("boards").and_then(Value::as_array).cloned().unwrap_or_default() {
                cards.push((board_height(&b), card(s(&b, "page_title").to_string(), Some(s(&b, "page").to_string()), board_view(&b, "", false, None))));
            }
        }
        for sec in &sections {
            match s(sec, "kind") {
                "text" => {
                    let md = s(sec, "md").to_string();
                    let est = 40.0 + md.lines().count() as f32 * 22.0;
                    let pid = page.as_ref().map(|p| s(p, "id").to_string());
                    cards.push((est, card(String::new(), pid, Box::new(MarkdownView::new(md).class("desk-notes-md")))));
                }
                "kanban" => {
                    if let Some(b) = find_board(&data, s(sec, "id")) {
                        let h = board_height(b);
                        cards.push((h, card(s(b, "page_title").to_string(), Some(s(b, "page").to_string()), board_view(b, "", false, None))));
                    }
                }
                "calendar" => cards.push((agenda_height(&data), card(t!("Повестка"), None, Box::new(agenda_list(&data))))),
                _ => {}
            }
        }
        // Колонки: каждую карточку — в самую короткую.
        let mut cols: Vec<(f32, Column)> = (0..ncols).map(|_| (0.0, Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch))).collect();
        for (h, w) in cards {
            let i = cols.iter().enumerate().min_by(|a, b| a.1 .0.total_cmp(&b.1 .0)).map(|(i, _)| i).unwrap_or(0);
            let (sum, col) = std::mem::replace(&mut cols[i], (0.0, Column::new()));
            cols[i] = (sum + h + 16.0, col.child(w));
        }
        let mut row = Row::new().gap(pad).cross_axis_alignment(CrossAxisAlignment::Start);
        for (_, c) in cols {
            row = row.child(DecoratedBox::new().style("width", StyleValue::px(colw)).child(c));
        }
        let title = page.as_ref().map(|p| s(p, "title").to_string()).unwrap_or_else(|| s(&data, "project").to_string());
        let head = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(title).class("desk-notes-head"))
            .child(
                GestureDetector::new()
                    .on_click(move || open(page.as_ref().map(|p| s(p, "id").to_string())))
                    .child(icon("\u{E89E}").class("desk-notes-open")),
            );
        Box::new(
            ScrollView::new()
                .vertical()
                .child(
                    Padding::only(pad, top + pad, pad, pad + 56.0)
                        .child(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Start).child(head).child(row)),
                )
                .style("height", StyleValue::px(height)),
        )
    }))
}

/// Вид «Заметки» спрятан — опрос только ради виджетов.
pub fn desk_view_hidden() {
    NOTES_VIEW.store(false, Ordering::Relaxed);
}

fn agenda_height(data: &Value) -> f32 {
    let n = data.get("agenda").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    60.0 + n as f32 * 44.0 + 120.0
}

fn board_height(b: &Value) -> f32 {
    let open: usize = b
        .get("columns")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter(|c| !c.get("done").and_then(Value::as_bool).unwrap_or(false))
                .map(|c| c.get("cards").and_then(Value::as_array).map(Vec::len).unwrap_or(0))
                .map(|n| if n > 0 { n + 1 } else { 0 })
                .sum()
        })
        .unwrap_or(0);
    (40.0 + open as f32 * 46.0).clamp(120.0, 640.0)
}

/// Карточка раздела: заголовок (тап — страница) и тело.
fn card(title: String, page: Option<String>, body: Box<dyn Widget>) -> Box<dyn Widget> {
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !title.is_empty() {
        let p = page.clone();
        col = col.child(GestureDetector::new().on_click(move || open(p.clone())).child(Text::new(title).max_lines(1).class("desk-title")));
    }
    col = col.child(body);
    Box::new(DecoratedBox::new().class("desk-card desk-notes-card").style("padding", StyleValue::px(14.0)).child(col))
}

/// Есть ли смысл в переключателе: syn-notes установлен.
pub fn available() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("syn-notes").is_file())) || syn_notes_proto::running()
}

/// Переключатель «Виджеты / Заметки» внизу слева.
pub fn view_switch(ctx: ShellCtx) -> impl Widget {
    rx(move || {
        let notes = ctx.config.get().desktop.view == "notes";
        let btn = |glyph: &'static str, label: String, on: bool, target: &'static str| -> Box<dyn Widget> {
            Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        if let Err(e) = synshell_common::config_edit::set_value(&["desktop", "view"], toml_edit::Value::from(target)) {
                            log::warn!("вид стола не записан: {e}");
                        }
                        if target == "widgets" {
                            desk_view_hidden();
                        }
                    })
                    .child(
                        Row::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .class(if on { "desk-view-btn on" } else { "desk-view-btn" })
                            .child(icon(glyph).class("desk-view-icon"))
                            .child(Text::new(label).class("desk-view-label")),
                    ),
            )
        };
        Box::new(
            Row::new()
                .gap(2.0)
                .class("desk-view-switch")
                .child(btn("\u{E1BD}", t!("Виджеты"), !notes, "widgets"))
                .child(btn("\u{E244}", t!("Заметки"), notes, "notes")),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syngui::core::Color;

    /// Снимки с живым хозяином заметок: `SHOT_DIR=… cargo test -p synshell-ui shot_notes -- --ignored`.
    #[test]
    #[ignore]
    fn shot_notes() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let cfg = synshell_common::config::Config::default();
        let mss = crate::theme::build(&cfg);
        let data = fetch();
        assert!(data.data.is_some(), "syn-notes не отвечает");
        let cases: [(&str, f32, f32); 4] = [("desk", 1600.0, 1000.0), ("notes-agenda", 330.0, 420.0), ("notes-board", 760.0, 420.0), ("notes-page", 420.0, 320.0)];
        for (kind, w, h) in cases {
            let out = format!("{dir}/desk-{kind}.png");
            let d = data.clone();
            syngui::shot::render_png(
                move || {
                    let ctx = crate::ShellCtx::new(synshell_common::config::Config::default(), synshell_common::config::FormFactor::Desktop);
                    provide_context(ctx);
                    snap().set(d.clone());
                    if kind == "desk" {
                        return Box::new(DecoratedBox::new().style("background-color", StyleValue::Color(super::super::views::mss_color(Color::from_hex("#2b3a55")))).child(desk_view(w, h, 0.0)));
                    }
                    let pad = 12.0;
                    let cx = Cx { w: w - 2.0 * pad, h: h - 2.0 * pad - 28.0, color: Color::from_hex("#7aa2f7"), view: "list".into(), history: 60 };
                    let dw = DeskWidget::new(kind, 0, 0, 1, 1).with("board", "synshell");
                    let body = match kind {
                        "notes-agenda" => agenda(cx),
                        "notes-board" => board(&dw, cx),
                        _ => page(&DeskWidget::new(kind, 0, 0, 1, 1).with("page", "synshell"), cx),
                    };
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
                1.0,
                &out,
            )
            .unwrap();
        }
    }
}
