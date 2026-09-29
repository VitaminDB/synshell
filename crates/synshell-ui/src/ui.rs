//! Мелкие виджеты оболочки, которых нет в syngui.
//!
//! [`InputArea`] — как `GestureDetector`, но видит все кнопки мыши (панель
//! задач: средняя — закрыть, правая — меню окна), колесо (громкость, столы)
//! и умеет «поглощать» нажатия внутри себя — содержимое всплывающих окон
//! не должно пропускать клик к подложке, которая закрывает окно.

use std::any::Any;
use std::sync::{Arc, Mutex};
use syngui::core::{Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{
    DirtyFlags, Element, ElementId, ElementTree, EventContextExt, LayoutHint, StyledElement, UpdateContext, Widget,
};
use syngui::widgets::IntoWidget;

type PressCb = Arc<Mutex<dyn FnMut(MouseButton, Point, Rect) + Send>>;
type WheelCb = Arc<Mutex<dyn FnMut(f32) + Send>>;
type HoverCb = Arc<Mutex<dyn FnMut(bool) + Send>>;
type ButtonCb = Arc<Mutex<dyn FnMut(MouseButton) + Send>>;

/// Сколько пикселей надо протащить с нажатой кнопкой, чтобы это стало
/// перетаскиванием, а не щелчком.
const DRAG_THRESHOLD: f32 = 6.0;

/// Область ввода вокруг одного ребёнка.
pub struct InputArea {
    child: Option<Box<dyn Widget>>,
    on_press: Option<PressCb>,
    on_click: Option<PressCb>,
    on_wheel: Option<WheelCb>,
    on_hover: Option<HoverCb>,
    on_double_click: Option<ButtonCb>,
    on_drag: Option<ButtonCb>,
    absorb: bool,
    cursor: CursorIcon,
    buttons: Option<&'static [MouseButton]>,
    classes: Vec<String>,
}

impl InputArea {
    pub fn new<M>(child: impl IntoWidget<M>) -> Self {
        Self {
            child: Some(child.into_widget()),
            on_press: None,
            on_click: None,
            on_wheel: None,
            on_hover: None,
            on_double_click: None,
            on_drag: None,
            absorb: false,
            cursor: CursorIcon::Default,
            buttons: None,
            classes: Vec::new(),
        }
    }

    /// Нажимать только этими кнопками: остальные уходят родителю (правый
    /// клик по кнопке апплета открывает меню панели, а не пропадает).
    pub fn buttons(mut self, b: &'static [MouseButton]) -> Self {
        self.buttons = Some(b);
        self
    }

    /// Нажатие любой кнопки (срабатывает на отпускании внутри области):
    /// кнопка, точка и границы области — в координатах поверхности.
    pub fn on_click(mut self, f: impl FnMut(MouseButton, Point, Rect) + Send + 'static) -> Self {
        self.on_click = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Нажатие (на MouseDown) — для подложек, закрывающих окно.
    pub fn on_press(mut self, f: impl FnMut(MouseButton, Point, Rect) + Send + 'static) -> Self {
        self.on_press = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Колесо: `dy > 0` — вверх.
    pub fn on_wheel(mut self, f: impl FnMut(f32) + Send + 'static) -> Self {
        self.on_wheel = Some(Arc::new(Mutex::new(f)));
        self
    }

    pub fn on_hover(mut self, f: impl FnMut(bool) + Send + 'static) -> Self {
        self.on_hover = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Двойной щелчок (второе нажатие; щелчок по отпусканию тоже придёт).
    pub fn on_double_click(mut self, f: impl FnMut(MouseButton) + Send + 'static) -> Self {
        self.on_double_click = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Начало перетаскивания: кнопку нажали здесь и сдвинули указатель
    /// дальше порога. Щелчка по отпусканию после этого не будет.
    pub fn on_drag(mut self, f: impl FnMut(MouseButton) + Send + 'static) -> Self {
        self.on_drag = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Поглощать все события мыши внутри области.
    pub fn absorb(mut self) -> Self {
        self.absorb = true;
        self
    }

    pub fn pointer(mut self) -> Self {
        self.cursor = CursorIcon::Pointer;
        self
    }

    pub fn class(mut self, c: impl Into<String>) -> Self {
        syngui::widget::push_classes(&mut self.classes, c.into());
        self
    }
}

impl Widget for InputArea {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(InputAreaElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            hovered: false,
            pressed: None,
            on_press: self.on_press.clone(),
            on_click: self.on_click.clone(),
            on_wheel: self.on_wheel.clone(),
            on_hover: self.on_hover.clone(),
            on_double_click: self.on_double_click.clone(),
            on_drag: self.on_drag.clone(),
            press_at: None,
            absorb: self.absorb,
            cursor: self.cursor,
            buttons: self.buttons,
            child_id: None,
            classes: self.classes.clone(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            mss: MssFields::new(),
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

    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        if let Some(child) = &self.child {
            let el = child.create_element();
            let id = tree.insert_with_type_id(el, Some(parent_id), child.as_any().type_id());
            child.mount(tree, id);
        }
    }

    fn child_widgets(&self) -> Vec<&dyn Widget> {
        self.child.as_ref().map(|c| vec![c.as_ref() as &dyn Widget]).unwrap_or_default()
    }

    fn widget_classes(&self) -> &[String] {
        &self.classes
    }
}

struct InputAreaElement {
    id: ElementId,
    bounds: Rect,
    hovered: bool,
    pressed: Option<MouseButton>,
    on_press: Option<PressCb>,
    on_click: Option<PressCb>,
    on_wheel: Option<WheelCb>,
    on_hover: Option<HoverCb>,
    on_double_click: Option<ButtonCb>,
    on_drag: Option<ButtonCb>,
    /// Где нажали (для порога перетаскивания).
    press_at: Option<Point>,
    absorb: bool,
    cursor: CursorIcon,
    buttons: Option<&'static [MouseButton]>,
    child_id: Option<ElementId>,
    classes: Vec<String>,
    dirty: DirtyFlags,
    mss: MssFields,
}

fn call1(cb: &Option<ButtonCb>, b: MouseButton) {
    if let Some(cb) = cb {
        if let Ok(mut f) = cb.lock() {
            f(b);
        }
    }
}

fn call3(cb: &Option<PressCb>, b: MouseButton, p: Point, r: Rect) {
    if let Some(cb) = cb {
        if let Ok(mut f) = cb.lock() {
            f(b, p, r);
        }
    }
}

impl Element for InputAreaElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<InputArea>() {
            self.on_press = w.on_press.clone();
            self.on_click = w.on_click.clone();
            self.on_wheel = w.on_wheel.clone();
            self.on_hover = w.on_hover.clone();
            self.on_double_click = w.on_double_click.clone();
            self.on_drag = w.on_drag.clone();
            self.absorb = w.absorb;
            self.cursor = w.cursor;
            self.buttons = w.buttons;
        }
    }

    fn layout(&mut self, c: Constraints) -> Size {
        let w = if c.max_width.is_finite() { c.max_width } else { 0.0 };
        let h = if c.max_height.is_finite() { c.max_height } else { 0.0 };
        let size = Size::new(w, h);
        self.bounds = Rect::new(Point::zero(), size);
        size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        let absorbed = |inside: bool| if inside && (self.absorb) { EventResult::Handled } else { EventResult::Ignored };
        if let (Some(allowed), Event::MouseDown { button, .. } | Event::MouseUp { button, .. } | Event::DoubleClick { button, .. }) =
            (self.buttons, event)
        {
            if !allowed.contains(button) {
                return EventResult::Ignored;
            }
        }
        match event {
            Event::MouseMove(pos) => {
                if let (Some(b), Some(at), true) = (self.pressed, self.press_at, self.on_drag.is_some()) {
                    if (pos.x - at.x).abs() > DRAG_THRESHOLD || (pos.y - at.y).abs() > DRAG_THRESHOLD {
                        self.pressed = None;
                        self.press_at = None;
                        call1(&self.on_drag, b);
                        return EventResult::Handled;
                    }
                }
                let inside = self.bounds.contains(*pos);
                if inside != self.hovered {
                    self.hovered = inside;
                    if let Some(cb) = &self.on_hover {
                        if let Ok(mut f) = cb.lock() {
                            f(inside);
                        }
                    }
                    ctx.request_paint();
                }
                if inside && self.cursor != CursorIcon::Default {
                    ctx.set_cursor(self.cursor);
                }
                EventResult::Ignored
            }
            Event::MouseDown { button, position } => {
                let inside = self.bounds.contains(*position);
                if inside && (self.on_click.is_some() || self.on_press.is_some()) {
                    self.pressed = Some(*button);
                    self.press_at = Some(*position);
                    call3(&self.on_press, *button, *position, self.bounds);
                    return EventResult::Handled;
                }
                absorbed(inside)
            }
            Event::MouseUp { button, position } => {
                let inside = self.bounds.contains(*position);
                if self.pressed == Some(*button) {
                    self.pressed = None;
                    if inside {
                        call3(&self.on_click, *button, *position, self.bounds);
                    }
                    return EventResult::Handled;
                }
                absorbed(inside)
            }
            Event::DoubleClick { position, button } => {
                // Двойной клик приходит вместо второго MouseDown — считаем
                // его нажатием, иначе быстрые повторные клики теряются.
                let inside = self.bounds.contains(*position);
                if inside && (self.on_click.is_some() || self.on_press.is_some()) {
                    self.pressed = Some(*button);
                    self.press_at = Some(*position);
                    call3(&self.on_press, *button, *position, self.bounds);
                    call1(&self.on_double_click, *button);
                    return EventResult::Handled;
                }
                absorbed(inside)
            }
            Event::MouseWheel { delta, position, .. } => {
                let inside = self.bounds.contains(*position);
                if inside {
                    if let Some(cb) = &self.on_wheel {
                        if let Ok(mut f) = cb.lock() {
                            f(*delta);
                        }
                        return EventResult::Handled;
                    }
                }
                absorbed(inside)
            }
            _ => EventResult::Ignored,
        }
    }

    fn children(&self) -> &[ElementId] {
        match self.child_id {
            Some(ref id) => std::slice::from_ref(id),
            None => &[],
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn set_content_size(&mut self, size: Size) {
        self.bounds.size = size;
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn mount(&mut self, tree: &mut ElementTree) {
        self.child_id = tree.children_of(self.id).first().copied();
    }
    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn element_type_name(&self) -> &str {
        "InputArea"
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, style: &ComputedStyle) {
        self.mss.apply(style);
        self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
    }
}

impl StyledElement for InputAreaElement {
    fn apply_style(&mut self, _style: &ComputedStyle) {}
    fn classes(&self) -> &[String] {
        &self.classes
    }
    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
    }
}

/// Глиф Material Icons (виджет `Icon`: квадрат `icon-size`, цвет
/// `icon-color`/`color`) с классом `icon`.
pub fn icon(glyph: &str) -> syngui::widget::StyledWidget<syngui::widgets::Icon> {
    use syngui::widget::WidgetExt;
    syngui::widgets::Icon::new(glyph).class("icon")
}

/// Коробка с классом (частый случай).
pub fn boxed<M>(class: &str, child: impl IntoWidget<M>) -> syngui::widget::StyledWidget<syngui::widgets::DecoratedBox> {
    use syngui::widget::WidgetExt;
    syngui::widgets::DecoratedBox::new().child(child).class(class.to_string())
}

/// Коды Material Icons, которыми рисуются апплеты.
pub mod mi {
    pub const APPS: &str = "\u{E5C3}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const POWER: &str = "\u{E8AC}";
    pub const LOCK: &str = "\u{E897}";
    pub const LOGOUT: &str = "\u{E9BA}";
    pub const RESTART: &str = "\u{F053}";
    pub const SLEEP: &str = "\u{EF44}"; // bedtime
    pub const VOLUME_UP: &str = "\u{E050}";
    pub const VOLUME_DOWN: &str = "\u{E04D}";
    pub const VOLUME_MUTE: &str = "\u{E04F}";
    pub const VOLUME_OFF: &str = "\u{E04F}";
    pub const MIC: &str = "\u{E029}";
    pub const MIC_OFF: &str = "\u{E02B}";
    pub const BRIGHTNESS: &str = "\u{E1AE}";
    pub const BELL: &str = "\u{E7F4}";
    pub const BELL_OFF: &str = "\u{E7F6}";
    pub const WIFI: &str = "\u{E63E}";
    pub const WIFI_OFF: &str = "\u{E648}";
    pub const ETHERNET: &str = "\u{EB54}";
    pub const NET_OFF: &str = "\u{E1DA}";
    pub const BATTERY_FULL: &str = "\u{E1A4}";
    pub const BATTERY_CHARGING: &str = "\u{E1A3}";
    pub const BATTERY_ALERT: &str = "\u{E19C}";
    pub const BATTERY_STD: &str = "\u{E1A5}";
    pub const KEYBOARD: &str = "\u{E312}";
    pub const DESKTOP: &str = "\u{E30C}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const MINIMIZE: &str = "\u{E931}";
    pub const MAXIMIZE: &str = "\u{E3C6}";
    /// filter_none — «восстановить размер».
    pub const RESTORE: &str = "\u{E3E0}";
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const PUSH_PIN: &str = "\u{F10D}";
    pub const ARROW_UP: &str = "\u{E5D8}";
    pub const LAYERS: &str = "\u{E53B}";
    pub const GRID: &str = "\u{E9B0}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const WALLPAPER: &str = "\u{E1BC}";
    pub const HOME: &str = "\u{E88A}";
    pub const BACK: &str = "\u{E5C4}";
    pub const TERMINAL: &str = "\u{EB8E}";
    pub const CALC: &str = "\u{EA5F}";
    pub const PLAY: &str = "\u{E037}";
    pub const PAUSE: &str = "\u{E034}";
    pub const NEXT: &str = "\u{E044}";
    pub const PREV: &str = "\u{E045}";
    pub const MEMORY: &str = "\u{E322}";
    pub const CPU: &str = "\u{E30D}";
    pub const STAR: &str = "\u{E838}";
    pub const HISTORY: &str = "\u{E889}";
    pub const CLEAR_ALL: &str = "\u{E0B8}";
    pub const TILE: &str = "\u{E871}";
    pub const FLOAT: &str = "\u{E069}";
    pub const WINDOW: &str = "\u{F088}";
    pub const INFO: &str = "\u{E88E}";
    pub const PLAY_ARROW: &str = "\u{E037}";
    pub const RUN: &str = "\u{E5C8}";
}

/// Полоса-индикатор 0..100: `.meter` > `.meter-fill` + `.meter-rest`.
pub fn meter(percent: u32) -> syngui::widgets::Row {
    use syngui::mss::StyleValue;
    use syngui::widget::WidgetExt;
    use syngui::widgets::{DecoratedBox, Row};
    let p = percent.min(100) as f32;
    let mut row = Row::new().gap(0.0).class("meter");
    if p > 0.0 {
        row = row.child(DecoratedBox::new().class("meter-fill").style("flex-grow", StyleValue::Number(p)));
    }
    if p < 100.0 {
        row = row.child(DecoratedBox::new().class("meter-rest").style("flex-grow", StyleValue::Number(100.0 - p)));
    }
    row
}

/// Реактивный ребёнок из замыкания, возвращающего `Box<dyn Widget>`
/// (разные типы в ветках).
pub fn rx(f: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static) -> syngui::widgets::Reactive {
    syngui::widgets::Reactive::new(move || vec![f()])
}

/// Центрировать содержимое по вертикали в полной высоте родителя (апплеты
/// растягиваются на толщину панели, а содержимое разной высоты).
pub fn vcenter<M>(child: impl IntoWidget<M>) -> syngui::widgets::Column {
    syngui::widgets::Column::new()
        .main_axis_alignment(syngui::widgets::MainAxisAlignment::Center)
        .cross_axis_alignment(syngui::widgets::CrossAxisAlignment::Center)
        .child(child)
}
