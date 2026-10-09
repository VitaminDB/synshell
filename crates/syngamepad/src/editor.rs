//! Редактор раскладки — прямо поверх игры, в поверхности контроллера: элементы тащатся пальцем,
//! у выбранного — размер, ширина, назначение, удаление; добавить элемент, прозрачность, раскладки
//! (своя у приложения: `[osk] app_layouts`). Правка идёт в черновике, «Готово» — сохранить в файл.

use syngui::containers::{Positioned, TouchPhase, TouchPoint};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::{GestureDetector, Grid, ScrollView};

use crate::layout::{Element, Kind, Layout};
use crate::ui::Pad;

/// Ширина панели инструментов (не больше экрана).
const BAR_W: f32 = 560.0;
const BAR_H: f32 = 196.0;
const PICKER_H: f32 = 330.0;
/// Шаги размера, ширины и прозрачности.
const SIZE_STEP: f32 = 0.02;
const ASPECT_STEP: f32 = 0.2;
const OPACITY_STEP: f32 = 0.1;

#[derive(Clone, Copy)]
pub struct Editor {
    pub on: RwSignal<bool>,
    pub draft: RwSignal<Layout>,
    pub selected: RwSignal<Option<usize>>,
    /// Открыт выбор назначения выбранного элемента.
    pub picker: RwSignal<bool>,
    /// Панель вверху (иначе внизу) — убрать с дороги элементы под ней.
    pub bar_top: RwSignal<bool>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            on: use_signal(false),
            draft: use_signal(Layout::default()),
            selected: use_signal(None),
            picker: use_signal(false),
            bar_top: use_signal(false),
        }
    }
}

thread_local! {
    /// Перетаскивание: палец → (элемент, сдвиг пальца от центра элемента).
    static DRAG: std::cell::RefCell<Option<(u64, usize, f32, f32)>> = const { std::cell::RefCell::new(None) };
}

pub fn open(pad: Pad) {
    crate::ui::release_all();
    pad.folded.set(false);
    pad.ed.draft.set(pad.layout.get_untracked());
    pad.ed.selected.set(None);
    pad.ed.picker.set(false);
    pad.ed.on.set(true);
}

fn close(pad: Pad, save: bool) {
    if save {
        let draft = pad.ed.draft.get_untracked();
        let id = pad.layout_id.get_untracked();
        if let Err(e) = crate::layout::save(&id, &draft) {
            log::error!("раскладка {id}: {e:#}");
        }
        pad.layout.set(draft);
    }
    pad.ed.on.set(false);
    pad.ed.picker.set(false);
    pad.ed.selected.set(None);
}

/// Прямоугольник панели (выбор назначения — выше).
pub fn panel_rect(pad: Pad, w: f32, h: f32) -> [f32; 4] {
    let pw = BAR_W.min(w - 16.0);
    let ph = if pad.ed.picker.get_untracked() { PICKER_H } else { BAR_H }.min(h - 16.0);
    let y = if pad.ed.bar_top.get_untracked() { 56.0 } else { h - ph - 8.0 };
    [(w - pw) / 2.0, y, pw, ph]
}

pub fn on_touch(pad: Pad, t: TouchPoint) -> bool {
    let (w, h) = (t.size.width, t.size.height);
    let p = (t.position.x, t.position.y);
    match t.phase {
        TouchPhase::Down => {
            let r = panel_rect(pad, w, h);
            if p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3] {
                return false; // кнопкам панели
            }
            let draft = pad.ed.draft.get_untracked();
            let hit = draft.elements.iter().enumerate().rev().find(|(_, e)| {
                let r = e.rect(w, h);
                p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3]
            });
            match hit {
                Some((i, e)) => {
                    pad.ed.selected.set(Some(i));
                    pad.ed.picker.set(false);
                    DRAG.with(|d| *d.borrow_mut() = Some((t.id, i, p.0 - e.x * w, p.1 - e.y * h)));
                }
                None => {
                    pad.ed.selected.set(None);
                    pad.ed.picker.set(false);
                }
            }
            true
        }
        TouchPhase::Move => {
            let Some((id, i, dx, dy)) = DRAG.with(|d| *d.borrow()) else { return true };
            if id != t.id {
                return true;
            }
            pad.ed.draft.update(|l| {
                if let Some(e) = l.elements.get_mut(i) {
                    e.x = ((p.0 - dx) / w).clamp(0.0, 1.0);
                    e.y = ((p.1 - dy) / h).clamp(0.0, 1.0);
                }
            });
            true
        }
        TouchPhase::Up | TouchPhase::Cancel => {
            DRAG.with(|d| {
                let mut d = d.borrow_mut();
                if d.is_some_and(|(id, ..)| id == t.id) {
                    *d = None;
                }
            });
            true
        }
    }
}

/// Вид правки: затемнение, элементы черновика (выбранный — с рамкой), панель или выбор назначения.
pub fn view(pad: Pad, w: f32, h: f32) -> Vec<Box<dyn Widget>> {
    let draft = pad.ed.draft.get();
    let selected = pad.ed.selected.get();
    let picker = pad.ed.picker.get();
    let _ = pad.ed.bar_top.get();
    let mut out: Vec<Box<dyn Widget>> = vec![Box::new(
        DecoratedBox::new().class("gp-edit-bg").style("width", StyleValue::px(w)).style("height", StyleValue::px(h)),
    )];
    for (i, e) in draft.elements.iter().enumerate() {
        out.push(element_preview(e, w, h, selected == Some(i)));
    }
    let [x, y, pw, ph] = panel_rect(pad, w, h);
    let panel: Box<dyn Widget> = if picker { Box::new(picker_view(pad, selected)) } else { Box::new(toolbar(pad, &draft, selected)) };
    out.push(Box::new(
        Positioned::new(DecoratedBox::new().child(panel).class("gp-panel").style("width", StyleValue::px(pw)).style("height", StyleValue::px(ph))).at(x, y),
    ));
    out
}

fn element_preview(e: &Element, w: f32, h: f32, selected: bool) -> Box<dyn Widget> {
    let [x, y, ew, eh] = e.rect(w, h);
    let label = match e.kind {
        Kind::Stick => match e.bind.as_str() {
            "right" => "R".to_string(),
            "wasd" => "WASD".into(),
            "arrows" => "Стрелки".into(),
            "mouse" => "Мышь".into(),
            _ => "L".into(),
        },
        Kind::Dpad => match e.bind.as_str() {
            "wasd" => "WASD".into(),
            "arrows" => "Стрелки".into(),
            _ => "✚".into(),
        },
        Kind::Trackpad => "Тачпад".into(),
        Kind::Button => e.label.clone(),
    };
    let radius = if e.kind == Kind::Trackpad { 18.0 } else { eh / 2.0 };
    let body = DecoratedBox::new()
        .child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(crate::ui::label_view(&label))
                .class("gp-fill"),
        )
        .class(if selected { "gp-btn gp-selected" } else { "gp-btn" })
        .style("border-radius", StyleValue::px(radius))
        .style("width", StyleValue::px(ew))
        .style("height", StyleValue::px(eh));
    Box::new(Positioned::new(body).at(x, y))
}

fn btn(label: impl Into<String>, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new()
        .child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(label.into()).class("gp-tool-label"))
                        .class("gp-fill"),
                )
                .class("gp-tool"),
        )
        .on_click(move || f())
        .style("flex-grow", 1.0_f32)
}

/// Кнопка заданной высоты (в сетке выбора назначения кнопки иначе по высоте текста).
fn btn_h(label: impl Into<String>, height: f32, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    btn(label, f).style("height", StyleValue::px(height))
}

fn row(items: Vec<Box<dyn Widget>>) -> impl Widget {
    let mut r = Row::new().gap(6.0).height(38.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for i in items {
        r = r.child(i);
    }
    r
}

fn boxed(w: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(w)
}

fn edit_selected(pad: Pad, f: impl FnOnce(&mut Element)) {
    let Some(i) = pad.ed.selected.get_untracked() else { return };
    pad.ed.draft.update(|l| {
        if let Some(e) = l.elements.get_mut(i) {
            f(e);
        }
    });
}

fn add(pad: Pad, e: Element) {
    let mut i = 0;
    pad.ed.draft.update(|l| {
        l.elements.push(e);
        i = l.elements.len() - 1;
    });
    pad.ed.selected.set(Some(i));
}

fn toolbar(pad: Pad, draft: &Layout, selected: Option<usize>) -> impl Widget {
    let name = draft.name.clone();
    let sel = selected.and_then(|i| draft.elements.get(i)).cloned();
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(row(vec![
        boxed(btn("◀", move || switch_layout(pad, -1))),
        boxed(Text::new(format!("{name} · {:.0}%", draft.opacity * 100.0)).class("gp-tool-title").style("flex-grow", 3.0_f32)),
        boxed(btn("▶", move || switch_layout(pad, 1))),
        boxed(btn("Новая", move || new_layout(pad))),
    ]));
    match sel {
        Some(e) => {
            let mut items = vec![
                boxed(btn("−", move || edit_selected(pad, |e| e.size = (e.size - SIZE_STEP).max(0.05)))),
                boxed(btn("+", move || edit_selected(pad, |e| e.size = (e.size + SIZE_STEP).min(0.8)))),
            ];
            if matches!(e.kind, Kind::Button | Kind::Trackpad) {
                items.push(boxed(btn("↔−", move || edit_selected(pad, |e| e.aspect = (e.aspect - ASPECT_STEP).max(0.6)))));
                items.push(boxed(btn("↔+", move || edit_selected(pad, |e| e.aspect = (e.aspect + ASPECT_STEP).min(4.0)))));
            }
            if e.kind == Kind::Stick {
                let floating = e.floating;
                items.push(boxed(btn(if floating { "Плавающий ✓" } else { "Плавающий" }, move || edit_selected(pad, |e| e.floating = !e.floating))));
            }
            if e.kind == Kind::Button {
                let toggle = e.toggle;
                items.push(boxed(btn(if toggle { "Залипает ✓" } else { "Залипает" }, move || edit_selected(pad, |e| e.toggle = !e.toggle))));
            }
            col = col.child(row(items));
            col = col.child(row(vec![
                boxed(btn(format!("Назначение: {}", bind_title(&e)), move || pad.ed.picker.set(true))),
                boxed(btn("Удалить", move || {
                    if let Some(i) = pad.ed.selected.get_untracked() {
                        pad.ed.selected.set(None);
                        pad.ed.draft.update(|l| {
                            if i < l.elements.len() {
                                l.elements.remove(i);
                            }
                        });
                    }
                })),
            ]));
        }
        None => {
            col = col.child(row(vec![
                boxed(btn("+ Кнопка", move || add(pad, Element { label: "A".into(), bind: "pad:a".into(), ..Default::default() }))),
                boxed(btn("+ Стик", move || add(pad, Element { kind: Kind::Stick, size: 0.3, bind: "left".into(), ..Default::default() }))),
                boxed(btn("+ Крестовина", move || add(pad, Element { kind: Kind::Dpad, size: 0.26, bind: "pad".into(), ..Default::default() }))),
                boxed(btn("+ Тачпад", move || {
                    add(pad, Element { kind: Kind::Trackpad, size: 0.3, aspect: 1.6, bind: "mouse:left".into(), ..Default::default() })
                })),
            ]));
            col = col.child(row(vec![
                boxed(btn("Прозрачнее", move || pad.ed.draft.update(|l| l.opacity = (l.opacity - OPACITY_STEP).max(0.15)))),
                boxed(btn("Плотнее", move || pad.ed.draft.update(|l| l.opacity = (l.opacity + OPACITY_STEP).min(1.0)))),
                boxed(btn("Сбросить", move || {
                    pad.ed.draft.set(crate::layout::standard());
                    pad.ed.selected.set(None);
                })),
            ]));
        }
    }
    col.child(row(vec![
        boxed(btn(if pad.ed.bar_top.get_untracked() { "Панель вниз" } else { "Панель вверх" }, move || {
            pad.ed.bar_top.set(!pad.ed.bar_top.get_untracked())
        })),
        boxed(btn("Отмена", move || close(pad, false))),
        boxed(btn("Готово", move || close(pad, true))),
    ]))
    .style("padding", 8.0_f32)
}

/// Подпись назначения элемента.
fn bind_title(e: &Element) -> String {
    let opts = options(e.kind);
    opts.iter().find(|o| o.1 == e.bind).map(|o| o.0.to_string()).unwrap_or_else(|| e.bind.clone())
}

/// Варианты назначения: (подпись в списке, назначение, подпись на кнопке).
fn options(kind: Kind) -> Vec<(&'static str, &'static str, &'static str)> {
    match kind {
        Kind::Stick => vec![
            ("Левый стик", "left", ""),
            ("Правый стик", "right", ""),
            ("WASD", "wasd", ""),
            ("Стрелки", "arrows", ""),
            ("Мышь", "mouse", ""),
        ],
        Kind::Dpad => vec![("Крестовина", "pad", ""), ("Стрелки", "arrows", ""), ("WASD", "wasd", "")],
        Kind::Trackpad => vec![("Тап — левая", "mouse:left", ""), ("Тап — правая", "mouse:right", "")],
        Kind::Button => vec![
            ("A", "pad:a", "A"),
            ("B", "pad:b", "B"),
            ("X", "pad:x", "X"),
            ("Y", "pad:y", "Y"),
            ("LB", "pad:lb", "LB"),
            ("RB", "pad:rb", "RB"),
            ("LT", "pad:lt", "LT"),
            ("RT", "pad:rt", "RT"),
            ("Back", "pad:back", "\u{E3E0}"),
            ("Start", "pad:start", "\u{E5D2}"),
            ("Guide", "pad:guide", "\u{E338}"),
            ("L3", "pad:l3", "L3"),
            ("R3", "pad:r3", "R3"),
            ("ЛКМ", "mouse:left", "ЛКМ"),
            ("ПКМ", "mouse:right", "ПКМ"),
            ("СКМ", "mouse:middle", "СКМ"),
            ("Колесо ↑", "mouse:wheelup", "\u{E5CE}"),
            ("Колесо ↓", "mouse:wheeldown", "\u{E5CF}"),
            ("Пробел", "key:space", "Space"),
            ("Enter", "key:enter", "\u{E31B}"),
            ("Esc", "key:esc", "Esc"),
            ("Tab", "key:tab", "Tab"),
            ("Shift", "key:shift", "\u{E5D8}"),
            ("Ctrl", "key:ctrl", "Ctrl"),
            ("Alt", "key:alt", "Alt"),
            ("W", "key:w", "W"),
            ("A", "key:a", "A"),
            ("S", "key:s", "S"),
            ("D", "key:d", "D"),
            ("E", "key:e", "E"),
            ("Q", "key:q", "Q"),
            ("R", "key:r", "R"),
            ("F", "key:f", "F"),
            ("G", "key:g", "G"),
            ("1", "key:1", "1"),
            ("2", "key:2", "2"),
            ("3", "key:3", "3"),
            ("4", "key:4", "4"),
            ("5", "key:5", "5"),
            ("↑", "key:up", "\u{E5C7}"),
            ("↓", "key:down", "\u{E5C5}"),
            ("←", "key:left", "\u{E5DE}"),
            ("→", "key:right", "\u{E5DF}"),
        ],
    }
}

fn picker_view(pad: Pad, selected: Option<usize>) -> impl Widget {
    let kind = selected.and_then(|i| pad.ed.draft.get_untracked().elements.get(i).map(|e| e.kind)).unwrap_or_default();
    let mut grid = Grid::new(6).gap(6.0);
    for (title, bind, label) in options(kind) {
        grid = grid.child(btn_h(title, 36.0, move || {
            edit_selected(pad, |e| {
                e.bind = bind.to_string();
                if e.kind == Kind::Button {
                    e.label = label.to_string();
                }
            });
            pad.ed.picker.set(false);
        }));
    }
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(row(vec![
            boxed(Text::new("Назначение").class("gp-tool-title").style("flex-grow", 3.0_f32)),
            boxed(btn("Назад", move || pad.ed.picker.set(false))),
        ]))
        .child(ScrollView::new().vertical().child(grid).style("flex-grow", 1.0_f32))
        .style("padding", 8.0_f32)
}

/// Соседняя раскладка; выбор запоминается для приложения в фокусе.
fn switch_layout(pad: Pad, step: i32) {
    let list = crate::layout::list();
    if list.is_empty() {
        return;
    }
    let cur = pad.layout_id.get_untracked();
    let i = list.iter().position(|(id, _)| *id == cur).unwrap_or(0) as i32;
    let n = list.len() as i32;
    let (id, _) = &list[((i + step) % n + n) as usize % n as usize];
    use_layout(pad, id);
}

fn use_layout(pad: Pad, id: &str) {
    pad.set_layout(id);
    pad.ed.draft.set(pad.layout.get_untracked());
    pad.ed.selected.set(None);
    let value = toml_edit::Value::from(id);
    let r = match pad.app.get_untracked() {
        Some(app) => synshell_common::config_edit::set_value(&["osk", "app_layouts", &app], value),
        None => synshell_common::config_edit::set_value(&["osk", "gamepad_layout"], value),
    };
    if let Err(e) = r {
        log::warn!("[osk] раскладка: {e:#}");
    }
}

/// Новая раскладка — копия черновика под свободным id `custom-N`.
fn new_layout(pad: Pad) {
    let taken: Vec<String> = crate::layout::list().into_iter().map(|(id, _)| id).collect();
    let n = (1..).find(|n| !taken.contains(&format!("custom-{n}"))).unwrap_or(1);
    let id = format!("custom-{n}");
    let mut l = pad.ed.draft.get_untracked();
    l.name = format!("Раскладка {n}");
    if let Err(e) = crate::layout::save(&id, &l) {
        log::error!("раскладка {id}: {e:#}");
        return;
    }
    use_layout(pad, &id);
}
