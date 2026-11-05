//! Экран восстановления здоровья — вертикальный таймлайн вех.

use syngui::mgui;
use syngui::prelude::*;

use crate::app_state::AppState;
use crate::model::health::{self, HealthMilestone};
use crate::widgets::components::screen_title;

pub fn build(state: AppState) -> impl Widget {
    Column::new().child(move || body(state))
}

fn body(state: AppState) -> impl Widget {
    let now = state.now_tick.get();
    let profile = state.profile.get();
    let baseline = profile.health_baseline();
    let in_progress = now < baseline;
    let streak = if in_progress {
        0
    } else {
        (now - baseline) / 1000
    };

    let mut col = Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(screen_title(state.tr("health.title")));

    if in_progress {
        col = col.child(
            DecoratedBox::new()
                .child(Text::new(state.tr("health.in_progress")).class("card-body"))
                .class("card card-accent"),
        );
    }

    for (i, m) in health::HEALTH_MILESTONES.iter().enumerate() {
        col = col.child(milestone_row(state, i, m, streak));
    }
    col
}

fn milestone_row(state: AppState, index: usize, m: &HealthMilestone, streak: u64) -> impl Widget {
    let reached = streak >= m.offset_secs;
    let prog = health::milestone_progress(streak, index) as f32;
    let title = state.tr(&format!("health.{}.title", m.key));
    let body_text = state.tr(&format!("health.{}.body", m.key));

    let texts = mgui! {
        Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Start).class("grow") => [
            Text::new(title).class("ms-title"),
            Text::new(body_text).class("ms-body"),
        ]
    };

    let mut row = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(m.icon).class("ms-icon"))
        .child(texts);
    row = if reached {
        row.child(Text::new(crate::icons::CHECK).class("ms-check"))
    } else {
        row.child(
            CircularProgress::with_value(prog)
                .size(34.0)
                .stroke_width(5.0)
                .class("ms-ring"),
        )
    };

    let cls = if reached { "card ms-row ms-row-done" } else { "card ms-row" };
    DecoratedBox::new().child(row).class(cls)
}
