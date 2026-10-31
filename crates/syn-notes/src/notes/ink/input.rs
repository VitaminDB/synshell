//! Ввод пера для холста: родные события syngui (`Event::Pen`, tablet-v2 —
//! давление, наклон, ластик, кнопка) и мышь на компьютере (перо с
//! давлением 0,5; правая кнопка — как кнопка пера). Синтезированные из
//! касания пальца щелчки сюда не попадают — пальцы холст разбирает сам.

use syngui::core::Point;
use syngui::input::{Event, MouseButton};
pub use syngui::input::{PenPhase, PenTool};

/// Отсчёт пера для холста.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub position: Point,
    /// 0..1.
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub tool: PenTool,
    /// Нижняя кнопка пера (или правая кнопка мыши).
    pub barrel: bool,
    pub time_ms: u32,
    /// Пришло мышью (давления нет).
    pub from_mouse: bool,
}

fn now_ms() -> u32 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u32
}

fn mouse_sample(position: Point, barrel: bool) -> PenSample {
    PenSample {
        position,
        pressure: 0.5,
        tilt_x: 0.0,
        tilt_y: 0.0,
        tool: PenTool::Pen,
        barrel,
        time_ms: now_ms(),
        from_mouse: true,
    }
}

/// Событие пера из события дерева. `mouse_down` — кнопка мыши, которой
/// сейчас ведётся штрих (движение без нажатия — наведение).
pub fn pen_event(e: &Event, mouse_down: Option<MouseButton>) -> Option<(PenPhase, PenSample)> {
    match e {
        Event::Pen { phase, sample } => Some((
            *phase,
            PenSample {
                position: sample.position,
                pressure: sample.pressure,
                tilt_x: sample.tilt_x,
                tilt_y: sample.tilt_y,
                tool: sample.tool,
                barrel: sample.barrel,
                time_ms: sample.time_ms,
                from_mouse: false,
            },
        )),
        Event::MouseDown { button: b @ (MouseButton::Left | MouseButton::Right), position }
            if !syngui::input::is_synthesized_mouse() =>
        {
            Some((PenPhase::Down, mouse_sample(*position, *b == MouseButton::Right)))
        }
        Event::MouseMove(position) => match mouse_down {
            Some(b) => Some((PenPhase::Move, mouse_sample(*position, b == MouseButton::Right))),
            None => Some((PenPhase::Hover, mouse_sample(*position, false))),
        },
        Event::MouseUp { button, position } if mouse_down == Some(*button) => {
            Some((PenPhase::Up, mouse_sample(*position, *button == MouseButton::Right)))
        }
        _ => None,
    }
}
