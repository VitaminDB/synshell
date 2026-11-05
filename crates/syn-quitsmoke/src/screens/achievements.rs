//! Экран достижений — сетка бейджей.

use syngui::mgui;
use syngui::prelude::*;

use crate::app_state::AppState;
use crate::model::achievements::{self, ACHIEVEMENTS};
use crate::widgets::components::screen_title;

pub fn build(state: AppState) -> impl Widget {
    Column::new().child(move || body(state))
}

fn body(state: AppState) -> impl Widget {
    let states = state.achievements.get();

    let mut grid = Grid::new(2).gap(12.0);
    for def in ACHIEVEMENTS.iter() {
        let unlocked = achievements::is_unlocked(&states, def.id);
        let title = state.tr(&format!("ach.{}.title", def.id));
        let desc = state.tr(&format!("ach.{}.desc", def.id));
        let icon = if unlocked { def.icon } else { crate::icons::LOCK };

        let inner = mgui! {
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(icon).class("ach-icon"),
                Text::new(title).class("ach-title"),
                Text::new(desc).class("ach-desc"),
            ]
        };
        let cls = if unlocked {
            "card ach-card ach-unlocked"
        } else {
            "card ach-card ach-locked"
        };
        grid = grid.child(DecoratedBox::new().child(inner).clip(true).class(cls));
    }

    mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            screen_title(state.tr("ach.title")),
            grid,
        ]
    }
}
