//! Жесты сенсорного экрана телефона (`[gestures]`): касание в зоне края
//! экрана задерживается, пока не станет ясно, что это:
//!
//! - свайп от левого/правого края внутрь — `edge_left`/`edge_right`
//!   («назад»), по отпусканию;
//! - свайп снизу вверх — `edge_bottom` («домой»); поднять палец выше
//!   [`HOLD_RISE`] высоты экрана и задержать ([`HOLD_TIME`], дрожание до
//!   [`HOLD_SLOP`] px не в счёт) — `edge_bottom_hold` («Недавние»), как в
//!   Android: срабатывает по таймеру, не дожидаясь отпускания;
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

/// Жест снизу с удержанием: насколько (доля высоты экрана) поднять палец.
const HOLD_RISE: f64 = 0.18;
/// Сколько держать палец почти на месте.
const HOLD_TIME: Duration = Duration::from_millis(220);
/// Дрожание пальца, которое удержанию не мешает (сенсор шумит).
const HOLD_SLOP: f64 = 14.0;
/// Шаг проверки удержания таймером (пока палец стоит, событий нет).
const HOLD_POLL: Duration = Duration::from_millis(40);

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
    /// Высота вывода — для порога удержания снизу.
    height: f64,
    /// Удержание: где палец остановился и с какого момента.
    hold_at: Point<f64, Logical>,
    hold_since: Instant,
    /// «Недавние» уже открыты удержанием — отпускание ничего не делает.
    held: bool,
}

#[derive(Default)]
pub struct TouchGestures {
    pub pending: Option<Pending>,
}

impl State {
    fn gestures_on(&self) -> bool {
        self.core.form_factor == FormFactor::Phone && self.core.config.gestures.enabled && !self.core.is_locked()
    }

    fn edge_at(&self, pos: Point<f64, Logical>) -> Option<(Edge, f64, f64)> {
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
        Some((edge, g.size.w, g.size.h))
    }

    /// Палец коснулся. `true` — касание задержано жестом.
    pub fn gesture_touch_down(&mut self, slot: TouchSlot, pos: Point<f64, Logical>, time: u32) -> bool {
        if !self.gestures_on() || self.core.touch_gestures.pending.is_some() {
            return false;
        }
        let Some((edge, width, height)) = self.edge_at(pos) else { return false };
        // Кнопки и края рамки у самого края экрана (заголовок развёрнутого
        // окна) — не жест.
        if matches!(self.under(pos), crate::input::Under::Deco(_, crate::deco::DecoHit::Button(_)) | crate::input::Under::Resize(..)) {
            return false;
        }
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
            height,
            hold_at: pos,
            hold_since: Instant::now(),
            held: false,
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
        if (pos.x - p.hold_at.x).abs().max((pos.y - p.hold_at.y).abs()) > HOLD_SLOP {
            p.hold_at = pos;
            p.hold_since = Instant::now();
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
                        self.edge_action(Edge::Top, a);
                    }
                    Decision::Edge if p.edge == Edge::Bottom => self.watch_bottom_hold(slot),
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
                    // «Недавние» уже открыты удержанием.
                    Edge::Bottom if p.held => None,
                    // Таймер мог не успеть: задержка видна и по отпусканию.
                    Edge::Bottom if Self::bottom_held(&p) => Some(g.edge_bottom_hold.clone()),
                    Edge::Bottom => Some(g.edge_bottom.clone()),
                    Edge::Top => None,
                };
                let _ = p.start_time;
                if let Some(a) = action {
                    self.edge_action(p.edge, a);
                }
                true
            }
        }
    }

    /// Палец снизу поднят достаточно высоко и стоит.
    fn bottom_held(p: &Pending) -> bool {
        p.start.y - p.last.y >= p.height * HOLD_RISE && p.hold_since.elapsed() >= HOLD_TIME
    }

    /// Пока идёт жест снизу — проверять удержание таймером: стоящий палец
    /// событий не шлёт, а «Недавние» должны открыться, не дожидаясь
    /// отпускания.
    fn watch_bottom_hold(&mut self, slot: TouchSlot) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        let _ = self.core.loop_handle.insert_source(Timer::from_duration(HOLD_POLL), move |_, _, state| {
            let Some(p) = state.core.touch_gestures.pending.as_mut() else { return TimeoutAction::Drop };
            if p.slot != slot || p.edge != Edge::Bottom || p.decision != Decision::Edge || p.held {
                return TimeoutAction::Drop;
            }
            if !Self::bottom_held(p) {
                return TimeoutAction::ToDuration(HOLD_POLL);
            }
            p.held = true;
            let a = state.core.config.gestures.edge_bottom_hold.clone();
            state.edge_action(Edge::Bottom, a);
            TimeoutAction::Drop
        });
    }

    /// Действие свайпа от края. Команду оболочке — с указанием края: у
    /// спрятанного автоскрытием дока (панели) на этом краю жест сперва
    /// показывает его, а не уводит «домой».
    fn edge_action(&mut self, edge: Edge, action: Action) {
        match action {
            Action::Shell(command) => {
                let edge = match edge {
                    Edge::Left => "left",
                    Edge::Right => "right",
                    Edge::Top => "top",
                    Edge::Bottom => "bottom",
                };
                self.core.ipc.broadcast(&synshell_common::ipc::Event::EdgeGesture { edge: edge.into(), command });
            }
            a => self.do_action(a),
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

// ─── Пальцем по окнам: рамка, кнопки, свободный стол ────────────────────────

use crate::deco::{Button, DecoHit};
use crate::wm::{ResizeEdge, WindowId};

/// Что делает палец, которого не видят клиенты.
pub enum Active {
    /// Тащит окно за заголовок.
    Move { id: WindowId, start: Point<f64, Logical>, loc0: Point<i32, Logical> },
    /// Меняет размер за край рамки.
    Resize { id: WindowId, edges: ResizeEdge, start: Point<f64, Logical>, loc0: Point<i32, Logical>, size0: smithay::utils::Size<i32, Logical>, last: Instant },
    /// Нажал кнопку заголовка — сработает на отпускании над ней.
    Button { id: WindowId, b: Button },
    /// Двумя пальцами двигает виртуальный стол (свободный режим).
    Pan { last: Point<f64, Logical> },
}

#[derive(Default)]
pub struct FingerState {
    /// Все пальцы на экране: слот → точка.
    pub down: Vec<(TouchSlot, Point<f64, Logical>)>,
    /// Пальцы, которыми управляет композитор (клиенты их не видят).
    pub owned: Vec<TouchSlot>,
    pub active: Option<Active>,
}

const BTN_LEFT: u32 = 0x110;

impl State {
    fn free_mode(&self) -> bool {
        self.core.wm.mobile.enabled && self.core.wm.mobile.mode == synshell_common::action::MobileMode::Free
    }

    fn finger_pos(&mut self, slot: TouchSlot, pos: Point<f64, Logical>) {
        let f = &mut self.core.fingers;
        match f.down.iter_mut().find(|(s, _)| *s == slot) {
            Some(e) => e.1 = pos,
            None => f.down.push((slot, pos)),
        }
    }

    fn centroid(&self) -> Option<Point<f64, Logical>> {
        let d = &self.core.fingers.down;
        if d.len() < 2 {
            return None;
        }
        let (a, b) = (d[0].1, d[1].1);
        Some(Point::from(((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)))
    }

    /// Касание рамки окна или второй палец на свободном столе. `true` —
    /// касание забрал композитор.
    pub fn finger_down(&mut self, slot: TouchSlot, pos: Point<f64, Logical>) -> bool {
        self.finger_pos(slot, pos);
        // Второй палец в свободном режиме — пан стола: касание у клиентов
        // отменяется.
        if self.free_mode() && self.core.config.gestures.two_finger_pan && self.core.fingers.down.len() == 2 && !matches!(self.core.fingers.active, Some(Active::Move { .. } | Active::Resize { .. })) {
            if let Some(touch) = self.core.seat.get_touch() {
                touch.cancel(self);
            }
            let c = self.centroid().unwrap_or(pos);
            let slots: Vec<TouchSlot> = self.core.fingers.down.iter().map(|(s, _)| *s).collect();
            self.core.fingers.owned = slots;
            self.core.fingers.active = Some(Active::Pan { last: c });
            return true;
        }
        if self.core.fingers.active.is_some() {
            // Уже что-то тащим — лишние пальцы не мешают.
            self.core.fingers.owned.push(slot);
            return true;
        }
        match self.under(pos) {
            crate::input::Under::Deco(id, DecoHit::Title) => {
                self.focus_window(Some(id));
                let loc0 = self.core.wm.get(id).map(|m| m.loc).unwrap_or_default();
                self.core.fingers.active = Some(Active::Move { id, start: pos, loc0 });
            }
            crate::input::Under::Deco(id, DecoHit::Button(b)) => {
                self.focus_window(Some(id));
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.pressed = Some(b);
                }
                self.core.fingers.active = Some(Active::Button { id, b });
                self.core.queue_redraw_all();
            }
            crate::input::Under::Resize(id, edges) => {
                self.focus_window(Some(id));
                let (loc0, size0) = self.core.wm.get(id).map(|m| (m.loc, m.size())).unwrap_or_default();
                self.core.fingers.active = Some(Active::Resize { id, edges, start: pos, loc0, size0, last: Instant::now() });
            }
            _ => return false,
        }
        self.core.fingers.owned.push(slot);
        true
    }

    pub fn finger_motion(&mut self, slot: TouchSlot, pos: Point<f64, Logical>) -> bool {
        self.finger_pos(slot, pos);
        if !self.core.fingers.owned.contains(&slot) {
            return false;
        }
        match self.core.fingers.active.take() {
            Some(Active::Move { id, start, loc0 }) => {
                let d = Point::<i32, Logical>::from(((pos.x - start.x).round() as i32, (pos.y - start.y).round() as i32));
                // Развёрнутое окно сначала восстановить.
                if self.core.wm.get(id).is_some_and(|m| m.maximized) {
                    self.apply_maximized(id, false);
                }
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.loc = loc0 + d;
                    m.floating = true;
                }
                self.sync_space();
                self.core.queue_redraw_all();
                self.core.fingers.active = Some(Active::Move { id, start, loc0 });
            }
            Some(Active::Resize { id, edges, start, loc0, size0, last }) => {
                let (dx, dy) = ((pos.x - start.x).round() as i32, (pos.y - start.y).round() as i32);
                let mut loc = loc0;
                let mut w = size0.w;
                let mut h = size0.h;
                if edges.contains(ResizeEdge::RIGHT) {
                    w += dx;
                }
                if edges.contains(ResizeEdge::BOTTOM) {
                    h += dy;
                }
                if edges.contains(ResizeEdge::LEFT) {
                    w -= dx;
                    loc.x += dx;
                }
                if edges.contains(ResizeEdge::TOP) {
                    h -= dy;
                    loc.y += dy;
                }
                let (w, h) = (w.max(160), h.max(100));
                let mut last = last;
                // Клиенту — не чаще 30 раз в секунду.
                if last.elapsed() > Duration::from_millis(33) {
                    last = Instant::now();
                    if let Some(m) = self.core.wm.get_mut(id) {
                        m.floating = true;
                    }
                    self.configure_content(id, smithay::utils::Rectangle::new(loc, (w, h).into()), false);
                    self.sync_space();
                    self.core.queue_redraw_all();
                }
                self.core.fingers.active = Some(Active::Resize { id, edges, start, loc0, size0, last });
            }
            Some(Active::Pan { last }) => {
                let c = self.centroid().unwrap_or(last);
                self.shift_desk(c.x - last.x, c.y - last.y);
                self.core.fingers.active = Some(Active::Pan { last: c });
            }
            other => self.core.fingers.active = other,
        }
        true
    }

    pub fn finger_up(&mut self, slot: TouchSlot) -> bool {
        let pos = self.core.fingers.down.iter().find(|(s, _)| *s == slot).map(|(_, p)| *p);
        self.core.fingers.down.retain(|(s, _)| *s != slot);
        let owned = self.core.fingers.owned.contains(&slot);
        self.core.fingers.owned.retain(|s| *s != slot);
        if !owned {
            return false;
        }
        match self.core.fingers.active.take() {
            Some(Active::Button { id, b }) => {
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.pressed = None;
                }
                let over = pos.is_some_and(|p| matches!(self.under(p), crate::input::Under::Deco(i, DecoHit::Button(bb)) if i == id && bb == b));
                if over {
                    self.deco_button_action(id, b, BTN_LEFT);
                }
                self.core.queue_redraw_all();
            }
            Some(Active::Move { id, .. }) | Some(Active::Resize { id, .. }) => {
                self.remember_float(id);
                self.core.ipc_dirty = true;
            }
            Some(Active::Pan { last }) => {
                // Пан до отпускания последнего пальца.
                if self.core.fingers.owned.is_empty() {
                    self.broadcast_mobile();
                } else {
                    self.core.fingers.active = Some(Active::Pan { last });
                }
            }
            None => {}
        }
        true
    }

    pub fn fingers_cancel(&mut self) {
        self.core.fingers = FingerState::default();
    }

    /// Сдвинуть виртуальный стол: окна едут, камера — в границах
    /// `[mobile] desk` (`3x3` — на экран в каждую сторону).
    pub fn shift_desk(&mut self, dx: f64, dy: f64) {
        let Some(out) = self.core.space.outputs().next().cloned() else { return };
        let g = self.core.space.output_geometry(&out).unwrap_or_default();
        let (bx, by) = match self.core.config.mobile.desk.as_str() {
            "2x2" => (g.size.w as f64, g.size.h as f64),
            "infinite" => (f64::INFINITY, f64::INFINITY),
            _ => (g.size.w as f64 * 1.0, g.size.h as f64 * 1.0),
        };
        let (bx, by) = if self.core.config.mobile.desk == "2x2" { (bx / 2.0, by / 2.0) } else { (bx, by) };
        let cam = self.core.wm.mobile.camera;
        let nx = (cam.x - dx).clamp(-bx, bx);
        let ny = (cam.y - dy).clamp(-by, by);
        let (rdx, rdy) = (cam.x - nx, cam.y - ny);
        if rdx.abs() < 0.5 && rdy.abs() < 0.5 {
            return;
        }
        self.core.wm.mobile.camera = Point::from((nx, ny));
        let d = Point::<i32, Logical>::from((rdx.round() as i32, rdy.round() as i32));
        let active = self.core.wm.active;
        for m in &mut self.core.wm.windows {
            if m.mapped && m.workspace == active && !m.sticky && !m.fullscreen {
                m.loc += d;
                if let Some(f) = m.float_geo.as_mut() {
                    f.loc += d;
                }
            }
        }
        self.sync_space();
        self.core.queue_redraw_all();
    }
}
