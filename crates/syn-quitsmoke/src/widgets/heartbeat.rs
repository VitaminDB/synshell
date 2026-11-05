//! Невидимый виджет-пульс: раз в секунду вызывает `AppState::tick`.
//!
//! Держит кадровый цикл живым для счётчиков и анимаций и продвигает
//! логику времени/уведомлений.

use std::any::Any;
use std::time::Duration;

use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult};
use syngui::layout::Constraints;
use syngui::prelude::{DirtyFlags, Element, ElementId, ElementTree, EventContext, UpdateContext, Widget};
use syngui::render::DisplayList;

use crate::app_state::AppState;

/// Виджет-пульс.
pub struct Heartbeat {
    state: AppState,
}

impl Heartbeat {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl Widget for Heartbeat {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(HeartbeatElement {
            id: ElementId::new(),
            state: self.state,
            accum: 0.0,
            dirty_flags: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
        })
    }

    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct HeartbeatElement {
    id: ElementId,
    state: AppState,
    accum: f32,
    dirty_flags: DirtyFlags,
}

impl Element for HeartbeatElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(h) = widget.as_any().downcast_ref::<Heartbeat>() {
            self.state = h.state;
        }
    }

    fn layout(&mut self, _constraints: Constraints) -> Size {
        Size::zero()
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn animate(&mut self, dt: Duration) -> bool {
        self.accum += dt.as_secs_f32();
        if self.accum >= 1.0 {
            self.accum = 0.0;
            self.state.tick();
        }
        // Продолжаем тикать всегда — держим часы и анимации живыми.
        true
    }

    fn handle_event(&mut self, _event: &Event, _ctx: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }

    fn children(&self) -> &[ElementId] {
        &[]
    }

    fn bounds(&self) -> Rect {
        Rect::zero()
    }

    fn set_position(&mut self, _pos: Point) {}

    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty_flags |= flags;
    }

    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty_flags.remove(flags);
    }

    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty_flags.contains(flags)
    }

    fn id(&self) -> ElementId {
        self.id
    }

    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }

    fn passthrough_hit_test(&self) -> bool {
        true
    }

    fn mount(&mut self, _tree: &mut ElementTree) {}
}
