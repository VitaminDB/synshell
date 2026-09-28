//! Поверхности оверлея (по одной на вывод), подсказка сверху и панель
//! действий у выделения.

use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::mss::StyleValue;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceSpec};

use crate::overlay::Overlay;
use crate::sel::{self, Choice};
use crate::shot::R;

type W = Box<dyn Widget>;

fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

mod glyph {
    pub const COPY: &str = "\u{e14d}";
    pub const SAVE: &str = "\u{e161}";
    pub const LINK: &str = "\u{e157}";
    pub const OPEN: &str = "\u{e89e}";
    pub const CLOSE: &str = "\u{e5cd}";
    pub const CROP: &str = "\u{e3be}";
    pub const SCREEN: &str = "\u{e30c}";
    pub const ALL: &str = "\u{e162}";
}

/// Ширина полосы, в которой по центру стоит панель действий, и её высота
/// (для выбора места у выделения).
const BAR_W: f32 = 640.0;
const BAR_H: f32 = 46.0;
const GAP: f32 = 12.0;
const HINT_TOP: f32 = 28.0;

pub const STYLES: &str = include_str!("../styles/screenshot.mss");

/// Таблица стилей: палитра рабочего стола + свои правила.
pub fn stylesheet(cfg: &syndesktop_common::Config) -> String {
    let a = &cfg.appearance;
    let mut out = a.mss_variables();
    out.push_str(&a.theme_mss_variables());
    out.push_str(STYLES);
    if !a.font.trim().is_empty() {
        out.push_str(&format!("Text, Button {{\n  font-family: \"{}\";\n}}\n", a.font.trim()));
    }
    out
}

pub fn open_surfaces() {
    let shot = sel::shot();
    for (i, f) in shot.frames.iter().enumerate() {
        let spec = SurfaceSpec {
            namespace: "syndesktop-screenshot".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::Exclusive,
            output: Some(f.name.clone()),
            clear_color: [0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        };
        let hooks = SurfaceHooks {
            on_key: Some(Box::new(sel::key)),
            on_closed: Some(Box::new(sel::cancel)),
            ..Default::default()
        };
        syngui_layer::create_surface_with(spec, hooks, move || boxed(surface_root(i)));
    }
}

fn surface_root(i: usize) -> impl Widget {
    let shot = sel::shot();
    let f = &shot.frames[i];
    let s = sel::sig();
    Stack::new()
        .fit(StackFit::Expand)
        .child(
            Image::from_rgba_shared(format!("shot:{}", f.name), f.width, f.height, f.rgba.clone())
                .fit(ImageFit::Fill)
                .placeholder(false),
        )
        .child(Reactive::new(move || -> Vec<W> { vec![boxed(Overlay::new(i, s.rev.get()))] }))
        .child(Reactive::new(move || -> Vec<W> { hint_layer(i) }))
        .child(Reactive::new(move || -> Vec<W> { toolbar_layer(i) }))
}

fn button(glyph: &str, label: &str, tip: &str, class: &str, choice: Choice) -> W {
    boxed(
        Tooltip::new(
            Button::new(label).leading_icon(glyph).on_click(move || sel::finish(choice)).class(&format!("shot-btn {class}")),
            tip.to_string(),
        )
        .delay_ms(600),
    )
}

fn close_button() -> W {
    boxed(ToolButton::new(glyph::CLOSE).tooltip("Отмена (Esc)").on_click(sel::cancel).class("shot-close"))
}

fn separator() -> W {
    boxed(DecoratedBox::new().class("shot-sep"))
}

/// Подсказка вверху вывода под указателем, пока ничего не выделено.
fn hint_layer(i: usize) -> Vec<W> {
    let s = sel::sig();
    if s.settled.get().is_some() || s.dragging.get() || s.pointer_frame.get() != i {
        return Vec::new();
    }
    let many = sel::shot().frames.len() > 1;
    let mut row = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(glyph::CROP).class("icon shot-hint-icon"))
        .child(
            Column::new()
                .gap(1.0)
                .child(Text::new("Выделите область или щёлкните по окну").max_lines(1).class("shot-hint-text"))
                .child(Text::new("Enter — весь экран · Ctrl+C — копировать · Esc — отмена").max_lines(1).class("shot-hint-keys")),
        )
        .child(separator())
        .child(Button::new("Весь экран").leading_icon(glyph::SCREEN).on_click(sel::select_output).class("shot-btn"));
    if many {
        row = row.child(Button::new("Все экраны").leading_icon(glyph::ALL).on_click(sel::select_all).class("shot-btn"));
    }
    row = row.child(close_button());
    let width = sel::shot().frames[i].geo.w as f32;
    let strip = Row::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .child(DecoratedBox::new().class("shot-bar").child(row))
        .style("width", StyleValue::px(width));
    vec![boxed(Positioned::new(strip).at(0.0, HINT_TOP))]
}

/// Панель действий под выделением (не влезает — над ним, иначе внутри).
fn toolbar_layer(i: usize) -> Vec<W> {
    let s = sel::sig();
    let Some(r) = s.settled.get() else { return Vec::new() };
    let shot = sel::shot();
    let anchor = (r.x + r.w / 2.0, r.bottom() - 1.0);
    if shot.nearest_frame(anchor) != i {
        return Vec::new();
    }
    let (x, y) = bar_position(&r, &shot.frames[i].geo);
    let bar = DecoratedBox::new().class("shot-bar").child(
        Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(button(glyph::COPY, "Копировать", "Копировать снимок в буфер обмена (Ctrl+C)", "", Choice::Copy))
            .child(button(glyph::SAVE, "Сохранить", "Сохранить в папку снимков (Enter, Ctrl+S)", "primary", Choice::Save))
            .child(button(glyph::LINK, "Копировать путь", "Сохранить и скопировать путь к файлу (Ctrl+Shift+C)", "", Choice::CopyPath))
            .child(button(glyph::OPEN, "Открыть", "Сохранить и открыть (Ctrl+O)", "", Choice::Open))
            .child(separator())
            .child(close_button()),
    );
    let strip = Row::new().main_axis_alignment(MainAxisAlignment::Center).child(bar).style("width", StyleValue::px(BAR_W));
    vec![boxed(Positioned::new(strip).at(x, y))]
}

/// Левый верхний угол полосы панели в координатах вывода `frame`.
fn bar_position(r: &R, frame: &R) -> (f32, f32) {
    let (fw, fh) = (frame.w as f32, frame.h as f32);
    let cx = (r.x + r.w / 2.0 - frame.x) as f32;
    let x = (cx - BAR_W / 2.0).clamp(0.0, (fw - BAR_W).max(0.0));
    let below = (r.bottom() - frame.y) as f32 + GAP;
    let above = (r.y - frame.y) as f32 - GAP - BAR_H;
    let y = if below + BAR_H <= fh - 4.0 {
        below
    } else if above >= 4.0 {
        above
    } else {
        ((r.bottom() - frame.y) as f32 - GAP - BAR_H).clamp(4.0, (fh - BAR_H - 4.0).max(4.0))
    };
    (x, y)
}
