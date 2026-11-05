//! Корневой каркас: переключение экранов + Heartbeat (+ оверлей частиц — Фаза 3).

use syngui::prelude::*;
use syngui::widgets::ParticleSystem;

use crate::app_state::{AppState, Screen};
use crate::screens;
use crate::widgets::components::scaffold;
use crate::widgets::heartbeat::Heartbeat;

/// Корневой виджет приложения.
pub fn build_app(state: AppState) -> impl Widget {
    Stack::new()
        .fit(StackFit::Expand)
        .child(Reactive::new(move || vec![current_screen(state)]))
        // Оверлей конфетти: всплеск при изменении celebrate (поверх экрана, кликопрозрачный).
        .child(Reactive::new(move || {
            vec![Box::new(ParticleSystem::confetti(state.celebrate.get())) as Box<dyn Widget>]
        }))
        .child(Heartbeat::new(state))
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
