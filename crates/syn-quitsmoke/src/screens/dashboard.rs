//! Главный экран — дашборд (разный для режимов «сразу» и «постепенно»).

use syngui::mgui;
use syngui::prelude::*;

use crate::app_state::{AppState, Screen};
use crate::icons;
use crate::model::health;
use crate::model::profile::QuitMode;
use crate::util::{fmt_hms, fmt_money, group_int};
use crate::widgets::components::{card, primary_button, ring, soft_button, stat_card};

/// Точка входа экрана.
pub fn build(state: AppState) -> impl Widget {
    Column::new().child(Reactive::new(move || vec![body(state)]))
}

fn body(state: AppState) -> Box<dyn Widget> {
    let cig: Box<dyn Widget> = match state.profile.get().mode {
        QuitMode::Immediate => Box::new(immediate(state)),
        QuitMode::Gradual => Box::new(gradual(state)),
    };
    if state.vape_enabled.get() {
        // Box<dyn Widget> не проходит по blanket-иму IntoWidget (dyn не удовлетворяет
        // W: Widget + 'static), но Column::children принимает готовый Vec<Box<dyn Widget>>.
        let vape_widget: Box<dyn Widget> = Box::new(vape_block(state));
        Box::new(
            Column::new()
                .gap(18.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![cig, vape_widget]),
        )
    } else {
        cig
    }
}

// ── Режим «сразу» ──────────────────────────────────────────────────────────

fn immediate(state: AppState) -> impl Widget {
    let s = state.stats_now();
    let settings = state.settings.get();

    let streak = s.streak_secs;
    let next_idx = health::next_index(streak);
    let (ring_progress, milestone_title) = match next_idx {
        Some(i) => (
            health::milestone_progress(streak, i) as f32,
            state.tr(&format!("health.{}.title", health::HEALTH_MILESTONES[i].key)),
        ),
        None => (1.0, state.tr(&format!("health.{}.title", health::FINAL_KEY))),
    };

    let hero = ring(
        ring_progress,
        state.tr("dash.streak"),
        &fmt_hms(streak),
        "ring-big",
        &format!("→ {milestone_title}"),
        220.0,
        "ring-green",
    );

    let stats = mgui! {
        Grid::new(2).gap(12.0) => [
            stat_card(icons::MONEY, fmt_money(s.money_saved, &settings.currency_symbol), state.tr("dash.money_saved"), "tint-amber"),
            stat_card(icons::SMOKE_FREE, group_int(s.cigs_avoided as i64), state.tr("dash.cigs_avoided"), "tint-green"),
            stat_card(icons::TIME, fmt_hms(s.time_regained_secs as u64), state.tr("dash.time_regained"), "tint-blue"),
            stat_card(icons::FIRE, fmt_hms(streak), state.tr("dash.streak"), "tint-rose"),
        ]
    };

    let milestone_card = {
        let (prog, title, body_text) = match next_idx {
            Some(i) => {
                let m = &health::HEALTH_MILESTONES[i];
                (
                    health::milestone_progress(streak, i) as f32,
                    state.tr(&format!("health.{}.title", m.key)),
                    state.tr(&format!("health.{}.body", m.key)),
                )
            }
            None => (1.0, state.tr(&format!("health.{}.title", health::FINAL_KEY)), state.tr(&format!("health.{}.body", health::FINAL_KEY))),
        };
        let inner = mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(state.tr("dash.next_milestone")).class("card-eyebrow"),
                Text::new(title).class("card-title"),
                Text::new(body_text).class("card-body"),
                ProgressBar::with_value(prog).class("milestone-bar"),
            ]
        };
        card("card-accent", inner)
    };

    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Padding::only(0.0, 6.0, 0.0, 0.0).child(
                DecoratedBox::new().child(hero).class("hero-wrap")
            ),
            stats,
            milestone_card,
            primary_button(state.tr("dash.craving_btn"), move || state.go(Screen::Craving)),
            soft_button(state.tr("dash.log_slip"), move || state.log_cigarette()),
        ]
    }
}

// ── Режим «постепенно» ──────────────────────────────────────────────────────

fn gradual(state: AppState) -> impl Widget {
    let now = state.now_tick.get();
    let profile = state.profile.get();
    let log = state.log.get();
    let settings = state.settings.get();
    let sched = state.taper();

    let last = log.last_at();
    let complete = sched.is_complete(now);
    let progress = sched.progress(now, last) as f32;
    let secs_left = sched.secs_until_allowed(now, last);
    let can_smoke = complete || progress >= 1.0;

    let (top, big, bottom) = if complete {
        (
            state.tr("dash.completed"),
            icons::PARTY.to_string(),
            String::new(),
        )
    } else if can_smoke {
        (
            state.tr("dash.until_next"),
            state.tr("dash.can_smoke").to_string(),
            String::new(),
        )
    } else {
        (
            state.tr("dash.until_next"),
            fmt_hms(secs_left),
            String::new(),
        )
    };

    let ring_class = if can_smoke { "ring-amber" } else { "ring-green" };
    // Текстовая подпись («Можно курить») уже, чем таймер, — уменьшаем шрифт,
    // чтобы влезала в кольцо. Иконку (complete) и таймер оставляем крупными.
    let big_class = if can_smoke && !complete { "ring-big-sm" } else { "ring-big" };
    let hero = ring(progress, &top, &big, big_class, &bottom, 220.0, ring_class);

    let allowed_today = sched.allowed_per_day(now);
    let smoked_today = {
        let day_start = now - (now % 86_400_000);
        log.count_in(day_start, now + 1) as u32
    };

    let quota_card = {
        let quota_frac = if allowed_today > 0 {
            (smoked_today as f32 / allowed_today as f32).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let inner = mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(state.tr("dash.today_quota")).class("card-eyebrow"),
                Text::new(format!("{smoked_today} {} {allowed_today}", state.tr("dash.of"))).class("card-title"),
                ProgressBar::with_value(quota_frac).class("milestone-bar"),
            ]
        };
        card("card-accent", inner)
    };

    let stats = mgui! {
        Grid::new(2).gap(12.0) => [
            stat_card(icons::MONEY, fmt_money(state.stats_now().money_saved, &settings.currency_symbol), state.tr("dash.money_saved"), "tint-amber"),
            stat_card(icons::SMOKE_FREE, group_int(state.stats_now().cigs_avoided as i64), state.tr("dash.cigs_avoided"), "tint-green"),
        ]
    };

    let _ = profile;

    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Padding::only(0.0, 6.0, 0.0, 0.0).child(
                DecoratedBox::new().child(hero).class("hero-wrap")
            ),
            quota_card,
            stats,
            primary_button(state.tr("dash.log_cig"), move || state.log_cigarette()),
            soft_button(state.tr("dash.craving_btn"), move || state.go(Screen::Craving)),
        ]
    }
}

// ── Vape-блок ────────────────────────────────────────────────────────────────

/// Второй hero-блок для вейпа — аналог `immediate`/`gradual` для сигарет.
/// Показывается только когда `state.vape_enabled == true`.
fn vape_block(state: AppState) -> impl Widget {
    let s = state.vape_stats_now();
    let settings = state.settings.get();
    let profile = state.profile_vape.get();

    let streak = s.streak_secs;
    let next_idx = health::next_index(streak);
    let (ring_progress, milestone_title) = match next_idx {
        Some(i) => (
            health::milestone_progress(streak, i) as f32,
            state.tr(&format!("health.{}.title", health::HEALTH_MILESTONES[i].key)),
        ),
        None => (1.0, state.tr(&format!("health.{}.title", health::FINAL_KEY))),
    };

    let hero = ring(
        ring_progress,
        state.tr("vape.streak"),
        &fmt_hms(streak),
        "ring-big",
        &format!("→ {milestone_title}"),
        220.0,
        "ring-vape",
    );

    let stats = mgui! {
        Grid::new(2).gap(12.0) => [
            stat_card(icons::MONEY, fmt_money(s.money_saved, &settings.currency_symbol), state.tr("vape.money_saved"), "tint-vape"),
            stat_card(icons::VAPE, group_int(s.cigs_avoided as i64), state.tr("vape.cigs_avoided"), "tint-vape"),
            stat_card(icons::TIME, fmt_hms(s.time_regained_secs as u64), state.tr("vape.time_regained"), "tint-vape"),
            stat_card(icons::FIRE, fmt_hms(streak), state.tr("vape.streak"), "tint-vape"),
        ]
    };

    let milestone_card = {
        let (prog, title, body_text) = match next_idx {
            Some(i) => {
                let m = &health::HEALTH_MILESTONES[i];
                (
                    health::milestone_progress(streak, i) as f32,
                    state.tr(&format!("health.{}.title", m.key)),
                    state.tr(&format!("health.{}.body", m.key)),
                )
            }
            None => (
                1.0,
                state.tr(&format!("health.{}.title", health::FINAL_KEY)),
                state.tr(&format!("health.{}.body", health::FINAL_KEY)),
            ),
        };
        let inner = mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(state.tr("vape.next_milestone")).class("card-eyebrow"),
                Text::new(title).class("card-title"),
                Text::new(body_text).class("card-body"),
                ProgressBar::with_value(prog).class("milestone-bar"),
            ]
        };
        card("card-accent", inner)
    };

    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("vape-block") => [
            Text::new(state.tr("vape.title")).class("vape-block-title"),
            Padding::only(0.0, 6.0, 0.0, 0.0).child(
                DecoratedBox::new().child(hero).class("hero-wrap")
            ),
            stats,
            milestone_card,
            primary_button(state.tr("vape.log_vape"), move || state.log_vape_hit()),
            soft_button(state.tr("vape.craving_btn"), move || state.go(Screen::Craving)),
        ]
    }
}
