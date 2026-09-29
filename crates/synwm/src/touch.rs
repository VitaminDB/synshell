//! Жесты сенсорного экрана телефона (`[gestures]`): касание в зоне края
//! экрана задерживается, пока не станет ясно, что это:
//!
//! - свайп от левого/правого края внутрь — `edge_left`/`edge_right`
//!   («назад»), по отпусканию;
//! - свайп снизу вверх — `edge_bottom` («домой»), с задержкой пальца перед
//!   отпусканием — `edge_bottom_hold` («Недавние»);
//! - вдоль нижнего края в режиме страниц — листание приложений пальцем;
//! - свайп сверху вниз — `edge_top` (шторка), сразу по порогу;
//! - не жест (тап у края, движение вдоль края) — касание воспроизводится
//!   приложению или панели как было: сначала `down`, потом движения.
//!
//! Касания вне краёв и второй палец идут клиентам без задержки.

use std::time::{Duration, Instant};

use smithay::backend::input::TouchSlot;
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use synshell_common::config::FormFactor;
use synshell_common::Action;

use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Undecided,
    /// Жест от края: действие по отпусканию (сверху — уже выполнено).
    Edge,
    /// Листание страниц вдоль нижнего края.
    PageDrag,
    /// Не жест — касание отдано клиенту, дальше как обычно.
    Client,
}

pub struct Pending {
    slot: TouchSlot,
    edge: Edge,
    start: Point<f64, Logical>,
    last: Point<f64, Logical>,
    start_time: u32,
    last_move: Instant,
    decision: Decision,
    /// Точки для скорости броска: (время, x).
    samples: Vec<(Instant, f64)>,
    /// Ширина вывода — для порога листания.
    width: f64,
}

#[derive(Default)]
pub struct TouchGestures {
    pub pending: Option<Pending>,
}

impl State {
    fn gestures_on(&self) -> bool {
        self.core.form_factor == FormFactor::Phone && self.core.config.gestures.enabled && !self.core.is_locked()
    }

    fn edge_at(&self, pos: Point<f64, Logical>) -> Option<(Edge, f64)> {
        let output = self.core.space.outputs().find(|o| {
            self.core.space.output_geometry(o).is_some_and(|g| g.to_f64().contains(pos))
        })?;
        let g = self.core.space.output_geometry(output)?.to_f64();
        let e = self.core.config.gestures.edge_size.max(4) as f64;
        let (x, y) = (pos.x - g.loc.x, pos.y - g.loc.y);
        let edge = if y >= g.size.h - e {
            Edge::Bottom
        } else if y <= e {
            Edge::Top
        } else if x <= e {
            Edge::Left
        } else if x >= g.size.w - e {
            Edge::Right
        } else {
            return None;
        };
        Some((edge, g.size.w))
    }

    /// Палец коснулся. `true` — касание задержано жестом.
    pub fn gesture_touch_down(&mut self, slot: TouchSlot, pos: Point<f64, Logical>, time: u32) -> bool {
        if !self.gestures_on() || self.core.touch_gestures.pending.is_some() {
            return false;
        }
        let Some((edge, width)) = self.edge_at(pos) else { return false };
        self.core.touch_gestures.pending = Some(Pending {
            slot,
            edge,
            start: pos,
            last: pos,
            start_time: time,
            last_move: Instant::now(),
            decision: Decision::Undecided,
            samples: vec![(Instant::now(), pos.x)],
            width,
        });
        true
    }

    /// Движение пальца. `true` — съедено жестом.
    pub fn gesture_touch_motion(&mut self, slot: TouchSlot, pos: Point<f64, Logical>, time: u32) -> bool {
        let threshold = self.core.config.gestures.threshold.max(8) as f64;
        let pages = self.core.wm.mobile.pages_mode();
        let Some(p) = self.core.touch_gestures.pending.as_mut().filter(|p| p.slot == slot) else { return false };
        if (pos.x - p.last.x).abs() + (pos.y - p.last.y).abs() > 2.0 {
            p.last_move = Instant::now();
        }
        p.last = pos;
        let now = Instant::now();
        p.samples.push((now, pos.x));
        p.samples.retain(|(t, _)| now.duration_since(*t) < Duration::from_millis(120));
        let (dx, dy) = (pos.x - p.start.x, pos.y - p.start.y);
        match p.decision {
            Decision::Undecided => {
                if dx.abs().max(dy.abs()) < threshold {
                    return true;
                }
                let horizontal = dx.abs() > dy.abs();
                let d = match p.edge {
                    Edge::Left if horizontal && dx > 0.0 => Decision::Edge,
                    Edge::Right if horizontal && dx < 0.0 => Decision::Edge,
                    Edge::Bottom if !horizontal && dy < 0.0 => Decision::Edge,
                    Edge::Bottom if horizontal && pages => Decision::PageDrag,
                    Edge::Top if !horizontal && dy > 0.0 => Decision::Edge,
                    _ => Decision::Client,
                };
                p.decision = d;
                match d {
                    Decision::Client => {
                        self.gesture_replay(slot, time);
                        return true;
                    }
                    Decision::PageDrag => {
                        self.page_drag_begin();
                        self.page_drag_update(dx);
                    }
                    Decision::Edge if p.edge == Edge::Top => {
                        let a = self.core.config.gestures.edge_top.clone();
                        self.do_action(a);
                    }
                    _ => {}
                }
                true
            }
            Decision::PageDrag => {
                self.page_drag_update(dx);
                true
            }
            Decision::Edge => true,
            Decision::Client => false,
        }
    }

    /// Палец поднят. `true` — съедено жестом.
    pub fn gesture_touch_up(&mut self, slot: TouchSlot, time: u32) -> bool {
        let Some(p) = self.core.touch_gestures.pending.take_if(|p| p.slot == slot) else { return false };
        match p.decision {
            Decision::Undecided => {
                // Тап у края — касание приложению целиком.
                self.core.touch_gestures.pending = Some(p);
                self.gesture_replay(slot, time);
                self.core.touch_gestures.pending = None;
                if let Some(touch) = self.core.seat.get_touch() {
                    touch.up(self, &UpEvent { slot, serial: SERIAL_COUNTER.next_serial(), time });
                    touch.frame(self);
                }
                true
            }
            Decision::Client => false,
            Decision::PageDrag => {
                let v = match (p.samples.first(), p.samples.last()) {
                    (Some(a), Some(b)) if b.0 > a.0 => (b.1 - a.1) / b.0.duration_since(a.0).as_secs_f64() / 1000.0,
                    _ => 0.0,
                };
                self.page_drag_end(v, p.width);
                true
            }
            Decision::Edge => {
                let g = &self.core.config.gestures;
                let action: Option<Action> = match p.edge {
                    Edge::Left => Some(g.edge_left.clone()),
                    Edge::Right => Some(g.edge_right.clone()),
                    Edge::Bottom => {
                        // Задержал палец перед отпусканием — «Недавние».
                        let held = p.last_move.elapsed() > Duration::from_millis(250);
                        Some(if held { g.edge_bottom_hold.clone() } else { g.edge_bottom.clone() })
                    }
                    Edge::Top => None,
                };
                let _ = p.start_time;
                if let Some(a) = action {
                    self.do_action(a);
                }
                true
            }
        }
    }

    /// Отдать задержанное касание клиенту: `down` в начальной точке и
    /// движение до текущей.
    fn gesture_replay(&mut self, slot: TouchSlot, time: u32) {
        let Some(p) = self.core.touch_gestures.pending.as_mut() else { return };
        p.decision = Decision::Client;
        let (start, last) = (p.start, p.last);
        let Some(touch) = self.core.seat.get_touch() else { return };
        let under = self.under(start);
        if let crate::input::Under::Surface(s, _) = &under {
            self.click_focus(&s.0.clone());
        }
        touch.down(self, under.focus(), &DownEvent { slot, location: start, serial: SERIAL_COUNTER.next_serial(), time });
        if last != start {
            let under = self.under(last);
            touch.motion(self, under.focus(), &MotionEvent { slot, location: last, time });
        }
        touch.frame(self);
    }
}
