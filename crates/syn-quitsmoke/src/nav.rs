//! Корневой каркас: переключение экранов + Heartbeat (+ оверлей частиц — Фаза 3).
//! На компьютере — своя рамка окна: заголовок со знаком и кнопками окна из настроек
//! рабочего стола, изменение размера за края; на телефоне synshell — заголовок с кнопками окна
//! без рамки и изменения размера; на Android и iPhone — ничего своего.

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::ParticleSystem;

use crate::app_state::{AppState, Screen};
use crate::screens;
use crate::widgets::components::scaffold;
use crate::widgets::heartbeat::Heartbeat;

/// Корневой виджет приложения.
pub fn build_app(state: AppState) -> Box<dyn Widget> {
    let content = Stack::new()
        .fit(StackFit::Expand)
        .child(Reactive::new(move || vec![current_screen(state)]))
        // Оверлей конфетти: всплеск при изменении celebrate (поверх экрана, кликопрозрачный).
        .child(Reactive::new(move || {
            vec![Box::new(ParticleSystem::confetti(state.celebrate.get())) as Box<dyn Widget>]
        }))
        .child(Heartbeat::new(state));
    if !window_chrome() {
        return Box::new(content);
    }
    let inner = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(titlebar(state))
        .child(DecoratedBox::new().child(content).class("grow"));
    let phone = synshell_phone();
    let frame = DecoratedBox::new().class(if phone { "window-frame flat" } else { "window-frame" }).clip(true).child(inner);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!phone).child(frame))
}

/// Свой заголовок с кнопками окна: везде, где окнами управляет synwm (и на телефоне synshell).
/// На Android и iPhone окно — весь экран системы, заголовка нет.
pub fn window_chrome() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

/// Телефон synshell (`SYNSHELL_FORM_FACTOR=phone`): окно во весь экран, без рамки и изменения размера.
fn synshell_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

/// Заголовок окна в цвет фона экранов: знак и название слева (за них, как и за пустое место,
/// окно перетаскивается), кнопки окна справа.
fn titlebar(state: AppState) -> impl Widget {
    let controls = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let w = state.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let brand = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(crate::icons::SMOKE_FREE).class("tb-logo-icon")).class("tb-logo"))
        .child(Text::new("QuitSmoke").class("tb-title"));
    DecoratedBox::new()
        .child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("grow tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().child(brand).class("tb-drag-space"))))
                .child(DecoratedBox::new().child(controls).class("window-controls")),
        )
        .class("titlebar")
}

/// Выбор активного экрана (реактивно по `state.screen`).
fn current_screen(state: AppState) -> Box<dyn Widget> {
    match state.screen.get() {
        Screen::Onboarding => Box::new(screens::onboarding::build(state)),
        Screen::Home => Box::new(scaffold(state, screens::dashboard::build(state))),
        Screen::Stats => Box::new(scaffold(state, screens::statistics::build(state))),
        Screen::Health => Box::new(scaffold(state, screens::health_timeline::build(state))),
        Screen::Achievements => {
            Box::new(scaffold(state, screens::achievements::build(state)))
        }
        Screen::Settings => Box::new(scaffold(state, screens::settings::build(state))),
        Screen::Craving => Box::new(screens::craving::build(state)),
    }
}
