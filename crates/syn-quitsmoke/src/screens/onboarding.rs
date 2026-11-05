//! Онбординг: пошаговый мастер настройки.

use syngui::mgui;
use syngui::prelude::*;
use syngui::GestureDetector;

use crate::app_state::AppState;
use crate::model::profile::{QuitMode, UserProfile};
use crate::util::{local_civil_to_epoch_ms, now_local};
use crate::widgets::components::{primary_button, soft_button};

const STEPS: usize = 7;

/// Сигналы формы (все `RwSignal` → `Copy`).
#[derive(Clone, Copy)]
struct Form {
    step: RwSignal<usize>,
    mode: RwSignal<usize>, // 0 = постепенно, 1 = сразу
    cigs_day: RwSignal<f64>,
    cigs_pack: RwSignal<f64>,
    price: RwSignal<f64>,
    years: RwSignal<f64>,
    waking: RwSignal<f64>,
    taper_days: RwSignal<f64>,
    quit_date: RwSignal<Date>,
    quit_time: RwSignal<Time>,
}

pub fn build(state: AppState) -> impl Widget {
    let (y, mo, d, h, mi) = now_local();
    let f = Form {
        step: use_signal(0usize),
        mode: use_signal(0usize),
        cigs_day: use_signal(20.0),
        cigs_pack: use_signal(20.0),
        price: use_signal(200.0),
        years: use_signal(5.0),
        waking: use_signal(16.0),
        taper_days: use_signal(30.0),
        quit_date: use_signal(Date::new(y, mo, d)),
        quit_time: use_signal(Time::new(h, mi)),
    };

    let root = mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("onb-root") => [
            move || header(state, f),
            ScrollView::new()
                .vertical()
                .child(Padding::symmetric(22.0, 20.0).child(Reactive::new(move || vec![step_body(state, f)])))
                .class("grow"),
        ]
    };

    // Фон красит DecoratedBox (Column сам фон не рисует). Системный «назад»
    // (Event::BackPressed) ловит GestureDetector.on_back: шаг назад / отмена.
    DecoratedBox::new()
        .child(
            GestureDetector::new()
                .on_back(move || {
                    go_back(state, f);
                    true
                })
                .child(root),
        )
        .class("app-bg")
}

/// Назад: на предыдущий шаг, а с первого — отмена мастера (на главный).
fn go_back(state: AppState, f: Form) {
    let s = f.step.get_untracked();
    if s > 0 {
        f.step.set(s - 1);
    } else {
        state.cancel_onboarding();
    }
}

fn header(state: AppState, f: Form) -> impl Widget {
    let step = f.step.get();
    let prog = (step as f32) / ((STEPS - 1) as f32);

    let back_btn = DecoratedBox::new()
        .child(
            GestureDetector::new()
                .child(Text::new(crate::icons::ARROW_BACK).class("onb-back-icon"))
                .on_click(move || go_back(state, f)),
        )
        .class("onb-back");

    let top = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(back_btn)
        .child(
            Text::new(format!("{} {} / {}", state.tr("app.title"), step + 1, STEPS))
                .class("onb-progress-label"),
        );

    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("onb-header")
        .child(top)
        .child(ProgressBar::with_value(prog).class("onb-progress"))
}

fn step_body(state: AppState, f: Form) -> Box<dyn Widget> {
    match f.step.get() {
        0 => Box::new(step_welcome(state, f)),
        1 => Box::new(step_mode(state, f)),
        2 => Box::new(step_habits(state, f)),
        3 => Box::new(step_plan(state, f)),
        4 => Box::new(step_notif(state, f)),
        5 => Box::new(step_quit_moment(state, f)),
        _ => Box::new(step_finish(state, f)),
    }
}

fn nav_row(state: AppState, f: Form, next_label: &str, on_next: impl FnMut() + Send + 'static) -> impl Widget {
    mgui! {
        Row::new().gap(12.0).main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::Center) => [
            soft_button(state.tr("common.back"), move || go_back(state, f)),
            primary_button(next_label, on_next),
        ]
    }
}

fn advance(f: Form) {
    let s = f.step.get_untracked();
    if s + 1 < STEPS {
        f.step.set(s + 1);
    }
}

fn step_welcome(state: AppState, f: Form) -> impl Widget {
    mgui! {
        Column::new().gap(20.0).cross_axis_alignment(CrossAxisAlignment::Center).class("onb-step") => [
            Text::new(crate::icons::LEAF).class("onb-hero-emoji"),
            Text::new(state.tr("onb.welcome.title")).class("onb-title"),
            Text::new(state.tr("onb.welcome.body")).class("onb-body"),
            primary_button(state.tr("onb.welcome.cta"), move || advance(f)),
        ]
    }
}

fn step_mode(state: AppState, f: Form) -> impl Widget {
    let mode = f.mode;
    let seg = SegmentedButton::new(vec![state.tr("onb.mode.gradual"), state.tr("onb.mode.immediate")])
        .selected(mode.get())
        .on_change(move |i| mode.set(i))
        .class("onb-seg");

    let desc_key = if mode.get() == 0 {
        "onb.mode.gradual.desc"
    } else {
        "onb.mode.immediate.desc"
    };

    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("onb-step") => [
            Text::new(state.tr("onb.mode.title")).class("onb-title"),
            seg,
            DecoratedBox::new().child(Text::new(state.tr(desc_key)).class("card-body")).class("card card-accent"),
            nav_row(state, f, state.tr("common.next"), move || advance(f)),
        ]
    }
}

fn number_field(label: &str, sig: RwSignal<f64>, min: f64, max: f64, _step: f64) -> impl Widget {
    // get_untracked: поле не перерисовывает весь шаг при вводе (иначе поле
    // пересоздаётся, теряя фокус/IME и вызывая двойную подсветку фокуса).
    let value = sig.get_untracked();
    let text = if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    };
    mgui! {
        Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("field") => [
            Text::new(label).class("field-label"),
            // TextField — вызывает системную клавиатуру; парсим значение защитно.
            TextField::with_text(text).on_change(move |s: &str| {
                let cleaned: String = s.trim().replace(',', ".");
                if let Ok(v) = cleaned.parse::<f64>() {
                    sig.set(v.clamp(min, max));
                }
            }).class("onb-input"),
        ]
    }
}

fn step_habits(state: AppState, f: Form) -> impl Widget {
    mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("onb-step") => [
            Text::new(state.tr("onb.habits.title")).class("onb-title"),
            number_field(state.tr("onb.habits.cigs_per_day"), f.cigs_day, 1.0, 80.0, 1.0),
            number_field(state.tr("onb.habits.cigs_per_pack"), f.cigs_pack, 1.0, 40.0, 1.0),
            number_field(state.tr("onb.habits.price"), f.price, 1.0, 5000.0, 10.0),
            number_field(state.tr("onb.habits.years"), f.years, 0.0, 70.0, 1.0),
            nav_row(state, f, state.tr("common.next"), move || advance(f)),
        ]
    }
}

fn step_plan(state: AppState, f: Form) -> impl Widget {
    let gradual = f.mode.get() == 0;
    let mut col = Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("onb-step")
        .child(Text::new(state.tr("onb.plan.title")).class("onb-title"))
        .child(number_field(state.tr("onb.plan.waking"), f.waking, 8.0, 24.0, 1.0));

    if gradual {
        col = col
            .child(number_field(state.tr("onb.plan.taper_days"), f.taper_days, 3.0, 120.0, 1.0))
            .child(DecoratedBox::new().child(Text::new(state.tr("onb.plan.preview")).class("card-body")).class("card card-accent"));
    }

    col.child(nav_row(state, f, state.tr("common.next"), move || advance(f)))
}

fn step_notif(state: AppState, f: Form) -> impl Widget {
    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("onb-step") => [
            Text::new(crate::icons::BELL).class("onb-hero-emoji"),
            Text::new(state.tr("onb.notif.title")).class("onb-title"),
            Text::new(state.tr("onb.notif.body")).class("onb-body"),
            primary_button(state.tr("onb.notif.allow"), move || {
                crate::notify::request_permission();
                advance(f);
            }),
            soft_button(state.tr("common.skip"), move || advance(f)),
        ]
    }
}

fn step_quit_moment(state: AppState, f: Form) -> impl Widget {
    let qd = f.quit_date;
    let qt = f.quit_time;
    let date = DatePicker::new()
        .selected(qd.get())
        .on_change(move |d| {
            if let Some(d) = d {
                qd.set(d);
            }
        })
        .class("onb-picker");
    let time = TimePicker::new()
        .selected(qt.get())
        .on_change(move |t| {
            if let Some(t) = t {
                qt.set(t);
            }
        })
        .class("onb-picker");

    mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("onb-step") => [
            Text::new(state.tr("onb.quit.title")).class("onb-title"),
            Text::new(state.tr("onb.quit.body")).class("onb-body"),
            Text::new(state.tr("onb.quit.date")).class("field-label"),
            date,
            Text::new(state.tr("onb.quit.time")).class("field-label"),
            time,
            nav_row(state, f, state.tr("common.next"), move || advance(f)),
        ]
    }
}

fn step_finish(state: AppState, f: Form) -> impl Widget {
    mgui! {
        Column::new().gap(20.0).cross_axis_alignment(CrossAxisAlignment::Center).class("onb-step") => [
            Text::new(crate::icons::PARTY).class("onb-hero-emoji"),
            Text::new(state.tr("onb.finish.title")).class("onb-title"),
            Text::new(state.tr("onb.finish.body")).class("onb-body"),
            primary_button(state.tr("onb.finish.cta"), move || {
                let mode = if f.mode.get_untracked() == 0 { QuitMode::Gradual } else { QuitMode::Immediate };
                let qd = f.quit_date.get_untracked();
                let qt = f.quit_time.get_untracked();
                let profile = UserProfile {
                    cigs_per_day: f.cigs_day.get_untracked().round().max(1.0) as u32,
                    cigs_per_pack: f.cigs_pack.get_untracked().round().max(1.0) as u32,
                    price_per_pack: f.price.get_untracked(),
                    years_smoking: f.years.get_untracked() as f32,
                    quit_started_at: local_civil_to_epoch_ms(qd.year, qd.month, qd.day, qt.hour, qt.minute),
                    mode,
                    waking_hours: f.waking.get_untracked() as f32,
                    taper_days: f.taper_days.get_untracked().round().max(1.0) as u32,
                };
                state.complete_onboarding(profile);
            }),
        ]
    }
}
