//! Выделение: создание рамкой, перенос, ручки, щелчок по окну, клавиши.
//! Координаты — глобальные логические; все оверлеи (по одному на вывод)
//! рисуют одно и то же выделение.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use syngui::input::{CursorIcon, Key};
use syngui::signal::{use_signal, RwSignal};
use syngui_layer::KeyInfo;

use crate::shot::{Shot, R};

/// Насколько далеко от края ещё берётся ручка.
pub const GRAB: f64 = 8.0;
/// Сдвиг указателя, после которого нажатие — рамка, а не щелчок по окну.
const CLICK_SLOP: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub l: bool,
    pub t: bool,
    pub r: bool,
    pub b: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    None,
    /// Новая рамка от точки нажатия; пока `moved == false` — это щелчок.
    Create { from: (f64, f64), moved: bool },
    Move { grab: (f64, f64), orig: R },
    Resize { edges: Edges, grab: (f64, f64), orig: R },
}

pub struct Sel {
    pub rect: Option<R>,
    pub drag: Drag,
    pub pointer: (f64, f64),
}

/// Что сделать с выделенным, когда оверлей закроется.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    Copy,
    CopyPath,
    Open,
}

#[derive(Clone, Copy)]
pub struct Signals {
    /// Любое изменение выделения или указателя — перерисовать оверлеи.
    pub rev: RwSignal<u64>,
    /// Выделение, пока его не тащат: к нему встаёт панель действий.
    pub settled: RwSignal<Option<R>>,
    /// Вывод под указателем — на нём подсказка.
    pub pointer_frame: RwSignal<usize>,
    /// Рамку тянут — подсказка и панель прячутся.
    pub dragging: RwSignal<bool>,
}

thread_local! {
    static SHOT: RefCell<Option<Rc<Shot>>> = const { RefCell::new(None) };
    static SEL: RefCell<Sel> = const { RefCell::new(Sel { rect: None, drag: Drag::None, pointer: (0.0, 0.0) }) };
    static SIGNALS: Cell<Option<Signals>> = const { Cell::new(None) };
    static RESULT: Cell<Option<(Choice, R)>> = const { Cell::new(None) };
}

pub fn init(shot: Shot) {
    let pointer = shot.pointer;
    let frame = shot.nearest_frame(pointer);
    SHOT.with(|s| *s.borrow_mut() = Some(Rc::new(shot)));
    SEL.with(|s| s.borrow_mut().pointer = pointer);
    SIGNALS.with(|s| s.set(Some(Signals { rev: use_signal(0), settled: use_signal(None), pointer_frame: use_signal(frame), dragging: use_signal(false) })));
}

pub fn shot() -> Rc<Shot> {
    SHOT.with(|s| s.borrow().clone().expect("кадр не загружен"))
}

pub fn sig() -> Signals {
    SIGNALS.with(|s| s.get().expect("сигналы не созданы"))
}

pub fn with<T>(f: impl FnOnce(&Sel) -> T) -> T {
    SEL.with(|s| f(&s.borrow()))
}

fn update(f: impl FnOnce(&mut Sel)) {
    SEL.with(|s| f(&mut s.borrow_mut()));
    changed();
}

/// Разослать изменение: перерисовка, панель действий, вывод с подсказкой.
fn changed() {
    let s = sig();
    let (rect, dragging, pointer) = with(|s| (s.rect, s.drag != Drag::None, s.pointer));
    s.rev.update(|r| *r += 1);
    let settled = if dragging { None } else { rect };
    if s.settled.get_untracked() != settled {
        s.settled.set(settled);
    }
    if s.dragging.get_untracked() != dragging {
        s.dragging.set(dragging);
    }
    let frame = shot().nearest_frame(pointer);
    if s.pointer_frame.get_untracked() != frame {
        s.pointer_frame.set(frame);
    }
}

/// Итог работы: действие и область (после выхода из цикла).
pub fn result() -> Option<(Choice, R)> {
    RESULT.with(|r| r.get())
}

/// Закончить: действие над выделением (нет выделения — весь вывод под
/// указателем).
pub fn finish(choice: Choice) {
    let shot = shot();
    let rect = with(|s| s.rect).unwrap_or_else(|| shot.frames[shot.nearest_frame(with(|s| s.pointer))].geo);
    RESULT.with(|r| r.set(Some((choice, rect))));
    syngui_layer::quit();
}

pub fn cancel() {
    RESULT.with(|r| r.set(None));
    syngui_layer::quit();
}

/// Ручки под точкой (углы и стороны рамки).
pub fn edges_at(r: &R, p: (f64, f64)) -> Option<Edges> {
    let outer = R::new(r.x - GRAB, r.y - GRAB, r.w + 2.0 * GRAB, r.h + 2.0 * GRAB);
    if !outer.contains(p) {
        return None;
    }
    let near = |a: f64, b: f64| (a - b).abs() <= GRAB;
    let (mut l, mut rr) = (near(p.0, r.x), near(p.0, r.right()));
    let (mut t, mut b) = (near(p.1, r.y), near(p.1, r.bottom()));
    // Маленькая рамка: обе стороны рядом — берётся ближняя.
    if l && rr {
        let left = (p.0 - r.x).abs() < (p.0 - r.right()).abs();
        l = left;
        rr = !left;
    }
    if t && b {
        let top = (p.1 - r.y).abs() < (p.1 - r.bottom()).abs();
        t = top;
        b = !top;
    }
    (l || rr || t || b).then_some(Edges { l, t, r: rr, b })
}

/// Указатель над точкой: ручка, перенос или перекрестие.
pub fn cursor_at(p: (f64, f64)) -> CursorIcon {
    let (rect, drag) = with(|s| (s.rect, s.drag));
    let edges = match drag {
        Drag::Resize { edges, .. } => Some(edges),
        Drag::Move { .. } => return CursorIcon::Grabbing,
        Drag::Create { .. } => return CursorIcon::Crosshair,
        Drag::None => rect.and_then(|r| edges_at(&r, p)),
    };
    match edges {
        Some(Edges { l: true, t: true, .. }) => CursorIcon::NwResize,
        Some(Edges { r: true, t: true, .. }) => CursorIcon::NeResize,
        Some(Edges { l: true, b: true, .. }) => CursorIcon::SwResize,
        Some(Edges { r: true, b: true, .. }) => CursorIcon::SeResize,
        Some(Edges { l: true, .. }) => CursorIcon::WResize,
        Some(Edges { r: true, .. }) => CursorIcon::EResize,
        Some(Edges { t: true, .. }) => CursorIcon::NResize,
        Some(Edges { b: true, .. }) => CursorIcon::SResize,
        _ if rect.is_some_and(|r| r.contains(p)) => CursorIcon::Move,
        _ => CursorIcon::Crosshair,
    }
}

pub fn press(p: (f64, f64)) {
    update(|s| {
        s.pointer = p;
        s.drag = match s.rect {
            Some(r) => match edges_at(&r, p) {
                Some(edges) => Drag::Resize { edges, grab: p, orig: r },
                None if r.contains(p) => Drag::Move { grab: p, orig: r },
                None => Drag::Create { from: p, moved: false },
            },
            None => Drag::Create { from: p, moved: false },
        };
    });
}

pub fn motion(p: (f64, f64)) {
    let bounds = shot().bounds;
    update(|s| {
        s.pointer = p;
        match s.drag {
            Drag::None => {}
            Drag::Create { from, moved } => {
                let moved = moved || (p.0 - from.0).abs() > CLICK_SLOP || (p.1 - from.1).abs() > CLICK_SLOP;
                s.drag = Drag::Create { from, moved };
                if moved {
                    let clamp = |q: (f64, f64)| (q.0.clamp(bounds.x, bounds.right()), q.1.clamp(bounds.y, bounds.bottom()));
                    s.rect = Some(R::from_points(clamp(from), clamp(p)).round());
                }
            }
            Drag::Move { grab, orig } => {
                let r = R::new(orig.x + p.0 - grab.0, orig.y + p.1 - grab.1, orig.w, orig.h);
                s.rect = Some(r.round().clamp_into(&bounds));
            }
            Drag::Resize { edges, grab, orig } => {
                let (dx, dy) = (p.0 - grab.0, p.1 - grab.1);
                let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
                if edges.l {
                    x0 += dx;
                }
                if edges.r {
                    x1 += dx;
                }
                if edges.t {
                    y0 += dy;
                }
                if edges.b {
                    y1 += dy;
                }
                let r = R::from_points((x0, y0), (x1, y1)).round();
                s.rect = r.intersect(&bounds);
            }
        }
    });
}

pub fn release(p: (f64, f64)) {
    let shot = shot();
    update(|s| {
        s.pointer = p;
        match s.drag {
            // Щелчок без рамки — окно под указателем (или весь вывод).
            Drag::Create { moved: false, .. } => s.rect = Some(shot.candidate_at(p)),
            _ => {
                if s.rect.is_some_and(|r| r.is_empty()) {
                    s.rect = None;
                }
            }
        }
        s.drag = Drag::None;
    });
}

pub fn double_click(p: (f64, f64)) {
    if with(|s| s.rect.is_some_and(|r| r.contains(p))) {
        finish(Choice::Save);
    } else {
        press(p);
    }
}

/// Правая кнопка: снять выделение, без выделения — выйти.
pub fn secondary() {
    if with(|s| s.rect.is_some()) {
        update(|s| {
            s.rect = None;
            s.drag = Drag::None;
        });
    } else {
        cancel();
    }
}

fn select(r: R) {
    update(|s| {
        s.rect = Some(r);
        s.drag = Drag::None;
    });
}

/// Клавиши оверлея; `true` — съедена.
pub fn key(k: &KeyInfo) -> bool {
    if !k.pressed {
        return false;
    }
    let (ctrl, shift) = (k.modifiers.ctrl, k.modifiers.shift);
    let shot = shot();
    match k.key {
        Key::Escape => {
            if with(|s| s.drag != Drag::None) {
                update(|s| s.drag = Drag::None);
            } else {
                cancel();
            }
        }
        Key::Enter => finish(Choice::Save),
        Key::S if ctrl => finish(Choice::Save),
        Key::C if ctrl && shift => finish(Choice::CopyPath),
        Key::C if ctrl => finish(Choice::Copy),
        Key::O if ctrl => finish(Choice::Open),
        Key::A if ctrl && shift => select(shot.bounds),
        Key::A if ctrl => select(shot.frames[shot.nearest_frame(with(|s| s.pointer))].geo),
        Key::Delete | Key::Backspace => update(|s| s.rect = None),
        Key::Left | Key::Right | Key::Up | Key::Down => {
            let Some(r) = with(|s| s.rect) else { return true };
            let step = if ctrl { 10.0 } else { 1.0 };
            let (dx, dy) = match k.key {
                Key::Left => (-step, 0.0),
                Key::Right => (step, 0.0),
                Key::Up => (0.0, -step),
                _ => (0.0, step),
            };
            let next = if shift {
                // Shift — размер: правый и нижний край.
                R::new(r.x, r.y, (r.w + dx).max(1.0), (r.h + dy).max(1.0)).intersect(&shot.bounds).unwrap_or(r)
            } else {
                R::new(r.x + dx, r.y + dy, r.w, r.h).clamp_into(&shot.bounds)
            };
            select(next);
        }
        _ => return false,
    }
    true
}

/// Подсветка под указателем, пока выделения нет: что выберет щелчок.
pub fn hover_candidate() -> Option<R> {
    let (rect, drag, p) = with(|s| (s.rect, s.drag, s.pointer));
    if rect.is_some() || drag != Drag::None {
        return None;
    }
    Some(shot().candidate_at(p))
}

/// Показывать ли лупу: рамку тянут или меняют размер.
pub fn loupe_visible() -> bool {
    with(|s| matches!(s.drag, Drag::Create { moved: true, .. } | Drag::Resize { .. }))
}

/// Выделение для рисования (во время создания — текущая рамка).
pub fn current_rect() -> Option<R> {
    with(|s| match s.drag {
        Drag::Create { moved: false, .. } => None,
        _ => s.rect,
    })
}

pub fn is_creating() -> bool {
    with(|s| matches!(s.drag, Drag::Create { .. }))
}

/// Выделить весь вывод под указателем или все выводы (кнопки подсказки).
pub fn select_output() {
    let shot = shot();
    select(shot.frames[shot.nearest_frame(with(|s| s.pointer))].geo);
}

pub fn select_all() {
    select(shot().bounds);
}
