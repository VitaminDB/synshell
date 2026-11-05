//! Экран поддержки при тяге — дыхательная практика 4-7-8.

use syngui::mgui;
use syngui::prelude::*;

use crate::app_state::{AppState, Screen};
use crate::widgets::breathing_orb::{breathing_orb, phase_key};
use crate::widgets::components::{primary_button, soft_button};

pub fn build(state: AppState) -> impl Widget {
    let accent = Color::from_hex("#2fbf71");

    let top = mgui! {
        Row::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Center).class("craving-top") => [
            soft_button("←", move || state.go(Screen::Home)),
        ]
    };

    let content = mgui! {
        Column::new().gap(22.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("craving-center") => [
            Text::new(state.tr("craving.title")).class("craving-title"),
            breathing_orb(accent, 260.0),
            move || {
                let now = state.now_tick.get();
                Text::new(state.tr(phase_key(now))).class("craving-phase")
            },
            Text::new(state.tr("craving.instr")).class("craving-instr"),
            mgui! {
                Row::new().gap(12.0).main_axis_alignment(MainAxisAlignment::Center) => [
                    primary_button(state.tr("craving.resisted"), move || {
                        state.trigger_celebrate();
                        state.go(Screen::Home);
                    }),
                    soft_button(state.tr("craving.slip"), move || {
                        state.log_cigarette();
                        state.go(Screen::Home);
                    }),
                ]
            },
        ]
    };

    mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("craving-root") => [
            top,
            ScrollView::new().vertical().child(Padding::symmetric(20.0, 12.0).child(content)).class("grow"),
        ]
    }
}
