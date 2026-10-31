//! Панель инструментов в духе Onyx: строка (или столбец) крупных кнопок —
//! перо, ластик, лассо, фигуры, текст (у рисунка ещё заливка, пипетка,
//! слои), толщина, цвет, отмена/повтор. Повторный тап по выбранному
//! инструменту открывает его ряд настроек (кисти, режимы ластика, фигуры);
//! толщина и цвет — свои всплывающие ряды. При выделении лассо — действия
//! над выделенным. Без анимаций.

use syngui::core::{Point, Rect, Size};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::overlay::menu::PopupAnchor;
use syngui::widgets::overlay::popup_panel::PopupPanel;
use syngui::widgets::ToolButton;
use syngui::containers::GestureDetector;

use super::icons as ic;
use super::model::{Brush, DocKind, Rgba, ShapeKind, Template, P2};
use super::{EraserMode, InkHandle, Tool, PALETTE};

/// Что раскрыто во всплывающем ряду.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pop {
    Pen,
    Eraser,
    Shape,
    Width,
    Color,
    Layers,
    Template,
    More,
}

/// Построитель панели: `InkToolbar::new(h).vertical(true).build()`.
pub struct InkToolbar {
    handle: InkHandle,
    vertical: bool,
    /// Свои кнопки в конце (например, «страницы» у блокнота).
    extra: Vec<Box<dyn Fn() -> Box<dyn Widget> + Send + Sync>>,
}

impl InkToolbar {
    pub fn new(handle: InkHandle) -> Self {
        Self { handle, vertical: false, extra: Vec::new() }
    }

    /// Столбцом (плавающая панель слева).
    pub fn vertical(mut self, v: bool) -> Self {
        self.vertical = v;
        self
    }

    /// Добавить свою кнопку в конец панели.
    pub fn extra(mut self, f: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static) -> Self {
        self.extra.push(Box::new(f));
        self
    }

    pub fn build(self) -> impl Widget {
        super::register_i18n();
        let h = self.handle.clone();
        let vertical = self.vertical;
        let pop: RwSignal<Option<Pop>> = use_signal(None);
        let open = use_signal(false);
        let anchor = use_signal(Rect::zero());
        // Закрытие панели — сброс раскрытого ряда.
        let extra = std::sync::Arc::new(self.extra);
        let bar = {
            let h = h.clone();
            Reactive::new(move || -> Vec<Box<dyn Widget>> {
                let ts = h.tool.get();
                let sel = h.selection.get();
                let _ = h.revision.get();
                let _ = h.page.get();
                let p = pop.get();
                vec![bar(&h, &ts, !sel.is_empty(), vertical, p, pop, open, anchor, &extra)]
            })
        };
        let panel = {
            let h = h.clone();
            PopupPanel::new()
                .is_open(open)
                .anchor_rect(anchor)
                .anchor(if vertical { PopupAnchor::EndCenter } else { PopupAnchor::BottomStart })
                .min_width(120.0)
                .max_width(560.0)
                .max_height(640.0)
                .on_close(move || pop.set(None))
                .class("ink-pop")
                .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    let ts = h.tool.get();
                    let _ = h.revision.get();
                    let _ = h.page.get();
                    match pop.get() {
                        Some(p) if open.get() => vec![popup(&h, p, &ts, pop, open)],
                        _ => vec![Box::new(DecoratedBox::new())],
                    }
                }))
        };
        Stack::new().clip(false).child(bar).child(panel)
    }
}

/// Кнопка, открывающая ряд `which` (повторный тап — закрыть).
fn opener(
    btn: ToolButton,
    which: Pop,
    pop: RwSignal<Option<Pop>>,
    open: RwSignal<bool>,
    anchor: RwSignal<Rect>,
    before: impl Fn() -> bool + Send + 'static,
) -> impl Widget {
    btn.on_click_with_bounds(move |_, b| {
        // `before` — сменить инструмент; true — сразу раскрыть ряд.
        if !before() {
            return;
        }
        if pop.get_untracked() == Some(which) && open.get_untracked() {
            open.set(false);
            pop.set(None);
        } else {
            anchor.set(b);
            pop.set(Some(which));
            open.set(true);
        }
    })
    .class("ink-tool")
}

#[allow(clippy::too_many_arguments)]
fn bar(
    h: &InkHandle,
    ts: &super::ToolState,
    has_sel: bool,
    vertical: bool,
    p: Option<Pop>,
    pop: RwSignal<Option<Pop>>,
    open: RwSignal<bool>,
    anchor: RwSignal<Rect>,
    extra: &[Box<dyn Fn() -> Box<dyn Widget> + Send + Sync>],
) -> Box<dyn Widget> {
    let drawing = h.kind() == DocKind::Drawing;
    let overlay = h.kind() == DocKind::Overlay;
    let mut items: Vec<Box<dyn Widget>> = Vec::new();

    // Инструмент: первый тап выбирает, второй — раскрывает настройки.
    let tool_btn = |tool: Tool, icon: &'static str, title: &str, which: Option<Pop>| -> Box<dyn Widget> {
        let active = ts.tool == tool;
        let hh = h.clone();
        let b = ToolButton::new(icon).tooltip(tr(title)).active(active);
        match which {
            Some(w) => Box::new(opener(b, w, pop, open, anchor, move || {
                if hh.tool.get_untracked().tool == tool {
                    true
                } else {
                    hh.set_tool(|t| t.tool = tool);
                    open.set(false);
                    false
                }
            })),
            None => Box::new(
                b.on_click(move || {
                    hh.set_tool(|t| t.tool = tool);
                    open.set(false);
                })
                .class("ink-tool"),
            ),
        }
    };

    items.push(tool_btn(Tool::Pen, ic::brush(ts.brush), "notes.ink.tool.pen", Some(Pop::Pen)));
    items.push(tool_btn(Tool::Eraser, ic::ERASER, "notes.ink.tool.eraser", Some(Pop::Eraser)));
    items.push(tool_btn(Tool::Lasso, ic::LASSO, "notes.ink.tool.lasso", None));
    items.push(tool_btn(Tool::Shape, ic::shape(ts.shape), "notes.ink.tool.shape", Some(Pop::Shape)));
    if !overlay {
        items.push(tool_btn(Tool::Text, ic::TEXT, "notes.ink.tool.text", None));
    }
    if drawing {
        items.push(tool_btn(Tool::Fill, ic::FILL, "notes.ink.tool.fill", None));
        items.push(tool_btn(Tool::Eyedropper, ic::EYEDROPPER, "notes.ink.tool.eyedropper", None));
        items.push(tool_btn(Tool::Hand, ic::HAND, "notes.ink.tool.hand", None));
    }
    items.push(sep(vertical));
    items.push(Box::new(opener(
        ToolButton::new(ic::WIDTH).tooltip(tr("notes.ink.width")).active(p == Some(Pop::Width)),
        Pop::Width,
        pop,
        open,
        anchor,
        || true,
    )));
    // Цвет: кружок текущего цвета.
    items.push(Box::new(
        GestureDetector::new()
            .on_click_with_bounds(move |_, b| {
                if pop.get_untracked() == Some(Pop::Color) && open.get_untracked() {
                    open.set(false);
                    pop.set(None);
                } else {
                    anchor.set(b);
                    pop.set(Some(Pop::Color));
                    open.set(true);
                }
            })
            .child(
                DecoratedBox::new()
                    .class("ink-color-btn")
                    .child(swatch(ts.color, 22.0, false)),
            ),
    ));
    if drawing {
        items.push(Box::new(opener(
            ToolButton::new(ic::LAYERS).tooltip(tr("notes.ink.layers")).active(p == Some(Pop::Layers)),
            Pop::Layers,
            pop,
            open,
            anchor,
            || true,
        )));
    }
    items.push(sep(vertical));
    let (hu, hr) = (h.clone(), h.clone());
    items.push(Box::new(ToolButton::new(ic::UNDO).tooltip(tr("notes.ink.undo")).disabled(!h.can_undo()).on_click(move || hu.undo()).class("ink-tool")));
    items.push(Box::new(ToolButton::new(ic::REDO).tooltip(tr("notes.ink.redo")).disabled(!h.can_redo()).on_click(move || hr.redo()).class("ink-tool")));

    if has_sel {
        items.push(sep(vertical));
        let (a, b, c, d) = (h.clone(), h.clone(), h.clone(), h.clone());
        items.push(Box::new(ToolButton::new(ic::COPY).tooltip(tr("notes.ink.copy")).on_click(move || a.copy_selection()).class("ink-tool")));
        items.push(Box::new(ToolButton::new(ic::CUT).tooltip(tr("notes.ink.cut")).on_click(move || b.cut_selection()).class("ink-tool")));
        items.push(Box::new(ToolButton::new(ic::DELETE).tooltip(tr("notes.ink.delete")).on_click(move || c.delete_selection()).class("ink-tool")));
        items.push(Box::new(ToolButton::new(ic::CLOSE).tooltip(tr("notes.ink.deselect")).on_click(move || d.clear_selection()).class("ink-tool")));
    } else if h.has_clipboard() && ts.tool == Tool::Lasso {
        let a = h.clone();
        items.push(Box::new(ToolButton::new(ic::PASTE).tooltip(tr("notes.ink.paste")).on_click(move || a.paste(P2::new(20.0, 20.0))).class("ink-tool")));
    }
    for f in extra {
        items.push(f());
    }
    items.push(Box::new(opener(
        ToolButton::new(ic::MORE).tooltip(tr("notes.ink.more")).active(p == Some(Pop::More)),
        Pop::More,
        pop,
        open,
        anchor,
        || true,
    )));

    if vertical {
        Box::new(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).children(items).class("ink-toolbar").class("ink-toolbar-v"))
    } else {
        Box::new(Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).children(items).class("ink-toolbar"))
    }
}

fn tr(key: &str) -> String {
    syngui::i18n::tr(key)
}

fn sep(vertical: bool) -> Box<dyn Widget> {
    Box::new(DecoratedBox::new().class(if vertical { "ink-sep-h" } else { "ink-sep-v" }))
}

/// Кружок цвета.
fn swatch(c: Rgba, d: f32, selected: bool) -> impl Widget {
    let col = syngui::core::Color::from_srgb(c.0[0], c.0[1], c.0[2], 1.0);
    DecoratedBox::new()
        .class("ink-swatch")
        .class(if selected { "selected" } else { "plain" })
        .style("width", StyleValue::px(d))
        .style("height", StyleValue::px(d))
        .style("border-radius", StyleValue::px(d / 2.0))
        .style("background-color", StyleValue::from(col))
}

/// Подпись в ряду.
fn caption(s: String) -> Box<dyn Widget> {
    Box::new(Text::new(s).class("ink-pop-caption"))
}

fn popup(h: &InkHandle, which: Pop, ts: &super::ToolState, pop: RwSignal<Option<Pop>>, open: RwSignal<bool>) -> Box<dyn Widget> {
    let close = move || {
        open.set(false);
        pop.set(None);
    };
    let mut col: Vec<Box<dyn Widget>> = Vec::new();
    match which {
        Pop::Pen => {
            col.push(caption(tr("notes.ink.brush")));
            let mut row: Vec<Box<dyn Widget>> = Vec::new();
            for b in Brush::ALL {
                let hh = h.clone();
                row.push(Box::new(
                    ToolButton::new(ic::brush(b))
                        .text(tr(&format!("notes.ink.brush.{}", b.key())))
                        .active(ts.brush == b)
                        .on_click(move || {
                            hh.set_tool(|t| {
                                t.brush = b;
                                t.tool = Tool::Pen;
                            });
                        })
                        .class("ink-pop-item"),
                ));
            }
            col.push(Box::new(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(row)));
            col.push(caption(tr("notes.ink.width")));
            col.push(widths_row(h, ts));
            col.push(caption(tr("notes.ink.color")));
            col.push(colors_grid(h, ts, close));
        }
        Pop::Width => {
            col.push(widths_row(h, ts));
        }
        Pop::Color => {
            col.push(colors_grid(h, ts, close));
        }
        Pop::Eraser => {
            for (m, key) in [(EraserMode::Stroke, "stroke"), (EraserMode::Point, "point"), (EraserMode::Area, "area")] {
                let hh = h.clone();
                col.push(Box::new(
                    ToolButton::new(if ts.eraser == m { ic::CHECK } else { ic::ERASER })
                        .text(tr(&format!("notes.ink.eraser.{key}")))
                        .active(ts.eraser == m)
                        .on_click(move || hh.set_tool(|t| {
                            t.eraser = m;
                            t.tool = Tool::Eraser;
                        }))
                        .class("ink-pop-item"),
                ));
            }
            col.push(caption(tr("notes.ink.eraser.size")));
            let mut row: Vec<Box<dyn Widget>> = Vec::new();
            for r in [4.0f32, 8.0, 14.0, 24.0, 40.0] {
                let hh = h.clone();
                let sel = (ts.eraser_r - r).abs() < 0.5;
                row.push(Box::new(
                    GestureDetector::new()
                        .on_click(move || hh.set_tool(|t| t.eraser_r = r))
                        .child(DecoratedBox::new().class("ink-width-cell").class(if sel { "selected" } else { "plain" }).child(Center::new().child(ring(r.min(30.0))))),
                ));
            }
            col.push(Box::new(Row::new().gap(4.0).children(row)));
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(ic::CLEAR_PAGE)
                    .text(tr("notes.ink.clear_page"))
                    .on_click(move || {
                        hh.clear_page();
                        close();
                    })
                    .class("ink-pop-item"),
            ));
        }
        Pop::Shape => {
            let kinds = [ShapeKind::Line, ShapeKind::Arrow, ShapeKind::Rect, ShapeKind::Ellipse, ShapeKind::Triangle];
            for k in kinds {
                let hh = h.clone();
                col.push(Box::new(
                    ToolButton::new(ic::shape(k))
                        .text(tr(&format!("notes.ink.shape.{}", k.key())))
                        .active(ts.shape == k)
                        .on_click(move || {
                            hh.set_tool(|t| {
                                t.shape = k;
                                t.tool = Tool::Shape;
                            });
                            close();
                        })
                        .class("ink-pop-item"),
                ));
            }
            if h.kind() == DocKind::Drawing {
                let hh = h.clone();
                let filled = ts.shape_fill.is_some();
                col.push(Box::new(
                    ToolButton::new(ic::FILL)
                        .text(tr("notes.ink.shape.filled"))
                        .active(filled)
                        .on_click(move || hh.set_tool(|t| t.shape_fill = if t.shape_fill.is_some() { None } else { Some(t.color.with_alpha(255)) }))
                        .class("ink-pop-item"),
                ));
            }
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(ic::MAGIC)
                    .text(tr("notes.ink.hold_to_shape"))
                    .active(ts.hold_to_shape)
                    .on_click(move || hh.set_tool(|t| t.hold_to_shape = !t.hold_to_shape))
                    .class("ink-pop-item"),
            ));
        }
        Pop::Layers => col.push(layers(h)),
        Pop::Template => col.push(templates(h, close)),
        Pop::More => {
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(if ts.finger_draw { ic::TOUCH } else { ic::NO_TOUCH })
                    .text(tr("notes.ink.finger_draw"))
                    .active(ts.finger_draw)
                    .on_click(move || hh.set_tool(|t| t.finger_draw = !t.finger_draw))
                    .class("ink-pop-item"),
            ));
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(ic::MAGIC)
                    .text(tr("notes.ink.hold_to_shape"))
                    .active(ts.hold_to_shape)
                    .on_click(move || hh.set_tool(|t| t.hold_to_shape = !t.hold_to_shape))
                    .class("ink-pop-item"),
            ));
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(ic::PEN)
                    .text(tr("notes.ink.crisp"))
                    .active(ts.crisp)
                    .on_click(move || hh.set_tool(|t| t.crisp = !t.crisp))
                    .class("ink-pop-item"),
            ));
            if h.kind() != DocKind::Overlay {
                col.push(Box::new(
                    ToolButton::new(ic::TEMPLATE)
                        .text(tr("notes.ink.template"))
                        .on_click(move || pop.set(Some(Pop::Template)))
                        .class("ink-pop-item"),
                ));
            }
            let hh = h.clone();
            col.push(Box::new(
                ToolButton::new(ic::FIT)
                    .text(tr("notes.ink.fit"))
                    .on_click(move || {
                        hh.fit();
                        close();
                    })
                    .class("ink-pop-item"),
            ));
        }
    }
    Box::new(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(col).class("ink-pop-body"))
}

/// Пять ступеней толщины — точки растущего размера.
fn widths_row(h: &InkHandle, ts: &super::ToolState) -> Box<dyn Widget> {
    let mut row: Vec<Box<dyn Widget>> = Vec::new();
    let ws = ts.brush.widths();
    let max = ws[4];
    for (k, w) in ws.iter().enumerate() {
        let hh = h.clone();
        let d = 4.0 + 20.0 * (w / max).sqrt();
        let sel = ts.width_step() == k;
        row.push(Box::new(
            GestureDetector::new()
                .on_click(move || {
                    hh.set_tool(|t| t.set_width_step(k));
                    if !hh.selection.get_untracked().is_empty() {
                        let w = hh.tool.get_untracked().width();
                        hh.rewidth_selection(w);
                    }
                })
                .child(
                    DecoratedBox::new()
                        .class("ink-width-cell")
                        .class(if sel { "selected" } else { "plain" })
                        .child(Center::new().child(swatch(Rgba::BLACK, d, false))),
                ),
        ));
    }
    Box::new(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).children(row))
}

fn ring(d: f32) -> impl Widget {
    DecoratedBox::new()
        .class("ink-ring")
        .style("width", StyleValue::px(d))
        .style("height", StyleValue::px(d))
        .style("border-radius", StyleValue::px(d / 2.0))
}

/// Палитра 4×4. Выбор цвета красит и выделенное.
fn colors_grid(h: &InkHandle, ts: &super::ToolState, close: impl Fn() + Clone + Send + 'static) -> Box<dyn Widget> {
    let mut rows: Vec<Box<dyn Widget>> = Vec::new();
    for chunk in PALETTE.chunks(4) {
        let mut row: Vec<Box<dyn Widget>> = Vec::new();
        for c in chunk {
            let c = *c;
            let hh = h.clone();
            let close = close.clone();
            let sel = ts.color.with_alpha(255) == c;
            row.push(Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        hh.set_tool(|t| {
                            t.color = c;
                            if t.shape_fill.is_some() {
                                t.shape_fill = Some(c);
                            }
                        });
                        if !hh.selection.get_untracked().is_empty() {
                            hh.recolor_selection(c);
                        }
                        close();
                    })
                    .child(DecoratedBox::new().class("ink-color-cell").class(if sel { "selected" } else { "plain" }).child(Center::new().child(swatch(c, 30.0, sel)))),
            ));
        }
        rows.push(Box::new(Row::new().gap(6.0).children(row)));
    }
    Box::new(Column::new().gap(6.0).children(rows))
}

/// Слои текущей страницы: видимость, блок, вверх/вниз, удалить, добавить.
fn layers(h: &InkHandle) -> Box<dyn Widget> {
    let ci = h.current();
    let (layers, active) = {
        let d = h.lock();
        let p = &d.pages[ci.min(d.pages.len() - 1)];
        (p.layers.clone(), p.active())
    };
    let mut col: Vec<Box<dyn Widget>> = Vec::new();
    // Верхний слой — сверху списка.
    for (i, l) in layers.iter().enumerate().rev() {
        let (a, b, c, d, e, f) = (h.clone(), h.clone(), h.clone(), h.clone(), h.clone(), h.clone());
        let n = layers.len();
        let row = Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("ink-layer-row")
            .class(if i == active { "active" } else { "plain" })
            .child(ToolButton::new(if l.visible { ic::VISIBLE } else { ic::HIDDEN }).on_click(move || a.edit_layer(i, |l| l.visible = !l.visible)))
            .child(ToolButton::new(if l.locked { ic::LOCK } else { ic::UNLOCK }).on_click(move || b.edit_layer(i, |l| l.locked = !l.locked)))
            .child(
                GestureDetector::new()
                    .on_click(move || c.set_active_layer(i))
                    .child(Text::new(format!("{}  {} %", l.name, (l.opacity * 100.0).round() as i32)).class("ink-layer-name"))
                    .class("grow"),
            )
            .child(ToolButton::new(ic::UP).disabled(i + 1 >= n).on_click(move || d.move_layer(i, i + 1)))
            .child(ToolButton::new(ic::DOWN).disabled(i == 0).on_click(move || e.move_layer(i, i - 1)))
            .child(ToolButton::new(ic::DELETE).on_click(move || f.delete_layer(i)));
        col.push(Box::new(row));
    }
    // Непрозрачность активного слоя.
    let mut op_row: Vec<Box<dyn Widget>> = vec![caption(tr("notes.ink.opacity"))];
    for v in [0.25f32, 0.5, 0.75, 1.0] {
        let hh = h.clone();
        let sel = (layers.get(active).map(|l| l.opacity).unwrap_or(1.0) - v).abs() < 0.01;
        op_row.push(Box::new(
            ToolButton::new("")
                .text(format!("{} %", (v * 100.0) as i32))
                .active(sel)
                .on_click(move || hh.edit_layer(active, |l| l.opacity = v)),
        ));
    }
    col.push(Box::new(Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).children(op_row)));
    let hh = h.clone();
    col.push(Box::new(ToolButton::new(ic::ADD).text(tr("notes.ink.layer.add")).on_click(move || hh.add_layer()).class("ink-pop-item")));
    Box::new(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(col).style("min-width", StyleValue::px(300.0)))
}

/// Шаблоны страницы: этой или всех.
pub fn templates(h: &InkHandle, close: impl Fn() + Clone + Send + 'static) -> Box<dyn Widget> {
    let ci = h.current();
    let cur = h.lock().pages.get(ci).map(|p| p.template.clone()).unwrap_or_default();
    let mut col: Vec<Box<dyn Widget>> = Vec::new();
    for t in Template::presets() {
        let hh = h.clone();
        let hh2 = h.clone();
        let t2 = t.clone();
        let close = close.clone();
        let close2 = close.clone();
        col.push(Box::new(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    ToolButton::new(if cur.key() == t.key() { ic::CHECK } else { ic::TEMPLATE })
                        .text(tr(&format!("notes.ink.template.{}", t.key())))
                        .active(cur.key() == t.key())
                        .on_click(move || {
                            hh.set_template(t.clone(), false);
                            close();
                        })
                        .class("grow"),
                )
                .child(ToolButton::new(ic::PAGES).tooltip(tr("notes.ink.template.all")).on_click(move || {
                    hh2.set_template(t2.clone(), true);
                    close2();
                })),
        ));
    }
    let _ = Point::zero();
    let _ = Size::zero();
    Box::new(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(col))
}
