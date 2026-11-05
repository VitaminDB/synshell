//! Android-реализация уведомлений (гибрид: foreground-бар + хронометр + будильники).

use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};

use syngui::app::notification as n;
use syngui::app::notification::{ChannelImportance, ChronometerStyle, NotificationBuilder};

use crate::app_state::AppState;
use crate::i18n::t;
use crate::model::health;
use crate::model::profile::QuitMode;
use crate::model::settings::ReminderStyle;
use crate::model::stats;
use crate::model::taper::TaperSchedule;
use crate::util::now_ms;

const CH_STATUS: &str = "status_timer"; // ongoing, тихий (Low)
const CH_MILESTONES: &str = "milestones"; // важные (High)
const CH_REMINDERS: &str = "reminders"; // напоминания (Default)

const ID_MAIN: i32 = 1001;
const ALARM_NEXT: i32 = 2001;
const ALARM_MILESTONE: i32 = 2002;

// Кэш последнего опубликованного состояния — чтобы не перепубликовывать каждую секунду.
static LAST_MODE: AtomicU8 = AtomicU8::new(255);
static LAST_DEADLINE: AtomicI64 = AtomicI64::new(i64::MIN);
static LAST_BASE: AtomicI64 = AtomicI64::new(i64::MIN);

fn reset_cache() {
    LAST_MODE.store(255, Ordering::SeqCst);
    LAST_DEADLINE.store(i64::MIN, Ordering::SeqCst);
    LAST_BASE.store(i64::MIN, Ordering::SeqCst);
}

pub fn init(state: &AppState) {
    let locale = state.settings.get_untracked().locale;
    n::create_channel(CH_STATUS, "Статус", "Текущий отсчёт и стрик", ChannelImportance::Low);
    n::create_channel(CH_MILESTONES, "Вехи здоровья", "Достижения и вехи", ChannelImportance::High);
    n::create_channel(CH_REMINDERS, "Напоминания", "Сигналы «можно курить»", ChannelImportance::Default);
    let _ = locale;
    request_permission();
    reset_cache();
    sync(state);
}

pub fn request_permission() {
    if !n::has_permission() {
        n::request_permission();
    }
}

pub fn clear() {
    n::cancel(ID_MAIN);
    n::stop_foreground_timer();
    n::cancel_alarm(ALARM_NEXT);
    n::cancel_alarm(ALARM_MILESTONE);
    reset_cache();
}

pub fn sync(state: &AppState) {
    let settings = state.settings.get_untracked();
    if !settings.notifications_enabled || !state.onboarded.get_untracked() {
        clear();
        return;
    }
    let profile = state.profile.get_untracked();
    let locale = settings.locale;
    let now = now_ms() as i64;

    let mode_u8 = match profile.mode {
        QuitMode::Gradual => 0,
        QuitMode::Immediate => 1,
    };
    // Переключение режима — сбрасываем всё старое.
    if LAST_MODE.swap(mode_u8, Ordering::SeqCst) != mode_u8 {
        n::cancel(ID_MAIN);
        n::stop_foreground_timer();
        n::cancel_alarm(ALARM_NEXT);
        n::cancel_alarm(ALARM_MILESTONE);
        LAST_DEADLINE.store(i64::MIN, Ordering::SeqCst);
        LAST_BASE.store(i64::MIN, Ordering::SeqCst);
    }

    match profile.mode {
        QuitMode::Gradual => sync_gradual(state, &settings, locale, now),
        QuitMode::Immediate => sync_immediate(state, &settings, locale, now),
    }
}

fn sync_gradual(
    state: &AppState,
    settings: &crate::model::settings::Settings,
    locale: crate::model::settings::Locale,
    now: i64,
) {
    let profile = state.profile.get_untracked();
    let log = state.log.get_untracked();
    let sched = TaperSchedule::from_profile(&profile);
    let now_u = now as u64;

    if sched.is_complete(now_u) {
        if LAST_DEADLINE.swap(-2, Ordering::SeqCst) != -2 {
            n::stop_foreground_timer();
            NotificationBuilder::new(ID_MAIN, CH_STATUS, t("app.title", locale), t("dash.completed", locale))
                .ongoing(true)
                .auto_cancel(false)
                .post();
        }
        return;
    }

    let last = log.last_at();
    let Some(next) = sched.next_allowed(now_u, last) else {
        return;
    };
    let next_i = next as i64;
    let start = last.unwrap_or(profile.quit_started_at) as i64;

    // Публикуем/перезапускаем только при смене дедлайна (сервис тикает сам).
    if LAST_DEADLINE.swap(next_i, Ordering::SeqCst) != next_i {
        let title = t("notif.gradual.title", locale);
        let ready = t("notif.gradual.can_smoke", locale);
        match settings.reminder_style {
            ReminderStyle::Chronometer => {
                n::stop_foreground_timer();
                NotificationBuilder::new(ID_MAIN, CH_STATUS, title, "")
                    .chronometer(ChronometerStyle {
                        base_time_ms: next_i,
                        count_down: true,
                    })
                    .ongoing(true)
                    .auto_cancel(false)
                    .post();
            }
            ReminderStyle::ProgressBar | ReminderStyle::Both => {
                // Живой прогресс-бар через foreground-сервис (переживает kill).
                n::start_foreground_timer(CH_STATUS, title, start, next_i, ready, "%s");
            }
        }
        if next_i > now {
            n::schedule_alarm(
                ALARM_NEXT,
                ((next_i - now) / 1000) as u64,
                CH_REMINDERS,
                ready,
                title,
            );
        }
    }
}

fn sync_immediate(
    state: &AppState,
    settings: &crate::model::settings::Settings,
    locale: crate::model::settings::Locale,
    now: i64,
) {
    let profile = state.profile.get_untracked();
    let log = state.log.get_untracked();
    let base = stats::streak_start(&profile, &log) as i64;
    let streak_secs = ((now - base).max(0) / 1000) as u64;
    let next = health::next_index(streak_secs);

    // Ближайшая веха — для будильника (срабатывает по достижении).
    let schedule_milestone = |idx: usize| {
        let m = &health::HEALTH_MILESTONES[idx];
        let delay = (m.offset_secs.saturating_sub(streak_secs)).max(1);
        n::schedule_alarm(
            ALARM_MILESTONE,
            delay,
            CH_MILESTONES,
            t("notif.milestone.title", locale),
            t(&format!("health.{}.title", m.key), locale),
        );
    };

    let use_bar = matches!(
        settings.reminder_style,
        ReminderStyle::ProgressBar | ReminderStyle::Both
    );

    // Прогресс-бар до следующей вехи (живой, переживает kill) — по стилю.
    if use_bar {
        if let Some(idx) = next {
            let m = &health::HEALTH_MILESTONES[idx];
            let prev_off = if idx == 0 {
                0
            } else {
                health::HEALTH_MILESTONES[idx - 1].offset_secs
            };
            let start = base + prev_off as i64 * 1000;
            let deadline = base + m.offset_secs as i64 * 1000;
            // Перепубликуем только при смене цели-вехи (сервис тикает сам).
            if LAST_DEADLINE.swap(deadline, Ordering::SeqCst) != deadline {
                LAST_BASE.store(i64::MIN, Ordering::SeqCst);
                let ms_title = t(&format!("health.{}.title", m.key), locale);
                let title = format!("{} · {}", t("notif.immediate.streak", locale), ms_title);
                let ready = t("notif.milestone.title", locale);
                n::start_foreground_timer(CH_STATUS, &title, start, deadline, &ready, "%s");
                schedule_milestone(idx);
            }
            return;
        }
    }

    // Хронометр времени без курения (счёт вверх) — стиль «Хронометр» или все вехи достигнуты.
    if LAST_BASE.swap(base, Ordering::SeqCst) != base {
        LAST_DEADLINE.store(i64::MIN, Ordering::SeqCst);
        n::stop_foreground_timer();
        NotificationBuilder::new(ID_MAIN, CH_STATUS, t("notif.immediate.streak", locale), "")
            .chronometer(ChronometerStyle {
                base_time_ms: base,
                count_down: false,
            })
            .ongoing(true)
            .auto_cancel(false)
            .post();
        if let Some(idx) = next {
            schedule_milestone(idx);
        }
    }
}

pub fn poll(state: &AppState) {
    // Дренируем действия из кнопок уведомлений (пока действия не назначены).
    while n::poll_action().is_some() {}
    let _ = state;
}
