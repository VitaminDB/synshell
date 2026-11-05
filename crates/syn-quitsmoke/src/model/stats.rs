//! Производные показатели: избегнутые сигареты, сэкономленные деньги,
//! возвращённое время, длина стрика.

use super::log::CigaretteLog;
use super::profile::{QuitMode, UserProfile};

/// Сколько «жизни» условно возвращает каждая невыкуренная сигарета (секунды).
/// Распространённая оценка — ~11 минут.
pub const SECONDS_PER_CIG_LIFE: f64 = 11.0 * 60.0;

/// Сводка производных показателей на момент `now`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivedStats {
    /// Избегнуто сигарет (может быть дробным).
    pub cigs_avoided: f64,
    /// Сэкономлено денег (в валюте настроек).
    pub money_saved: f64,
    /// Возвращено времени (секунды).
    pub time_regained_secs: f64,
    /// Текущий стрик без курения (секунды).
    pub streak_secs: u64,
    /// Сколько сигарет залогировано всего.
    pub logged_count: u64,
}

/// Момент, от которого считается текущий стрик:
/// максимум из старта программы и последнего срыва.
pub fn streak_start(profile: &UserProfile, log: &CigaretteLog) -> u64 {
    let base = match profile.mode {
        QuitMode::Immediate => profile.quit_started_at,
        // Для тейпера «чистый стрик» — со времени последней сигареты вообще.
        QuitMode::Gradual => profile
            .quit_started_at
            .max(log.last_at().unwrap_or(profile.quit_started_at)),
    };
    match profile.mode {
        QuitMode::Immediate => base.max(log.last_slip_at().unwrap_or(base)),
        QuitMode::Gradual => base,
    }
}

/// Посчитать все показатели.
pub fn compute(profile: &UserProfile, log: &CigaretteLog, now: u64) -> DerivedStats {
    let elapsed_secs = now.saturating_sub(profile.quit_started_at) as f64 / 1000.0;
    let baseline_rate_per_sec = profile.cigs_per_day as f64 / 86_400.0;
    let logged = log.count_since(profile.quit_started_at) as f64;

    let cigs_avoided = (baseline_rate_per_sec * elapsed_secs - logged).max(0.0);

    let per_cig_price = if profile.cigs_per_pack > 0 {
        profile.price_per_pack / profile.cigs_per_pack as f64
    } else {
        0.0
    };
    let money_saved = cigs_avoided * per_cig_price;
    let time_regained_secs = cigs_avoided * SECONDS_PER_CIG_LIFE;

    let streak_secs = now.saturating_sub(streak_start(profile, log)) / 1000;

    DerivedStats {
        cigs_avoided,
        money_saved,
        time_regained_secs,
        streak_secs,
        logged_count: logged as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> UserProfile {
        UserProfile {
            cigs_per_day: 20,
            cigs_per_pack: 20,
            price_per_pack: 200.0,
            years_smoking: 5.0,
            quit_started_at: 0,
            mode: QuitMode::Immediate,
            waking_hours: 16.0,
            taper_days: 30,
        }
    }

    #[test]
    fn one_day_avoids_a_pack() {
        let p = profile();
        let log = CigaretteLog::default();
        let s = compute(&p, &log, 86_400_000);
        assert!((s.cigs_avoided - 20.0).abs() < 0.01, "{}", s.cigs_avoided);
        // 20 сигарет = 1 пачка = 200.
        assert!((s.money_saved - 200.0).abs() < 0.01, "{}", s.money_saved);
    }

    #[test]
    fn logged_slip_reduces_avoided_and_resets_streak() {
        let p = profile();
        let mut log = CigaretteLog::default();
        log.push(86_400_000, false); // срыв ровно через сутки
        let s = compute(&p, &log, 2 * 86_400_000);
        // Два дня базово = 40, минус 1 залогированная = 39.
        assert!((s.cigs_avoided - 39.0).abs() < 0.01, "{}", s.cigs_avoided);
        // Стрик отсчитывается от срыва: ровно 1 сутки.
        assert_eq!(s.streak_secs, 86_400);
    }

    #[test]
    fn never_negative() {
        let p = profile();
        let mut log = CigaretteLog::default();
        for i in 0..100 {
            log.push(i * 1000, false);
        }
        let s = compute(&p, &log, 60_000);
        assert!(s.cigs_avoided >= 0.0);
        assert!(s.money_saved >= 0.0);
    }
}

// ── Vape-вариант ────────────────────────────────────────────────────────────

use super::profile::VapeProfile;

/// Сколько «жизни» условно возвращает каждая невэйпнутая сессия.
/// Вейп за сессию менее вреден, чем сигарета — берём ~5 минут.
pub const SECONDS_PER_VAPE_LIFE: f64 = 5.0 * 60.0;

/// Момент, от которого считается текущий vape-стрик.
pub fn vape_streak_start(profile: &VapeProfile, log: &CigaretteLog) -> u64 {
    let base = match profile.mode {
        QuitMode::Immediate => profile.quit_started_at,
        QuitMode::Gradual => profile
            .quit_started_at
            .max(log.last_at().unwrap_or(profile.quit_started_at)),
    };
    match profile.mode {
        QuitMode::Immediate => base.max(log.last_slip_at().unwrap_or(base)),
        QuitMode::Gradual => base,
    }
}

/// Посчитать vape-показатели (переиспользует [`DerivedStats`]).
pub fn compute_vape(profile: &VapeProfile, log: &CigaretteLog, now: u64) -> DerivedStats {
    let elapsed_secs = now.saturating_sub(profile.quit_started_at) as f64 / 1000.0;
    let baseline_rate_per_sec = profile.vapes_per_day as f64 / 86_400.0;
    let logged = log.count_since(profile.quit_started_at) as f64;

    let avoided = (baseline_rate_per_sec * elapsed_secs - logged).max(0.0);
    let money_saved = avoided * profile.unit_price();
    let time_regained_secs = avoided * SECONDS_PER_VAPE_LIFE;

    let streak_secs = now.saturating_sub(vape_streak_start(profile, log)) / 1000;

    DerivedStats {
        cigs_avoided: avoided,
        money_saved,
        time_regained_secs,
        streak_secs,
        logged_count: logged as u64,
    }
}

#[cfg(test)]
mod vape_tests {
    use super::*;

    fn vprofile() -> VapeProfile {
        VapeProfile {
            vapes_per_day: 10,
            price_per_pod: 500.0,
            puffs_per_pod: 300,
            years_vaping: 2.0,
            quit_started_at: 0,
            mode: QuitMode::Immediate,
            waking_hours: 16.0,
            taper_days: 30,
        }
    }

    #[test]
    fn vape_one_day_avoids_baseline() {
        let p = vprofile();
        let log = CigaretteLog::default();
        let s = compute_vape(&p, &log, 86_400_000);
        assert!((s.cigs_avoided - 10.0).abs() < 0.01, "{}", s.cigs_avoided);
        assert!((s.money_saved - 500.0).abs() < 0.01, "{}", s.money_saved);
    }

    #[test]
    fn vape_slip_resets_streak() {
        let p = vprofile();
        let mut log = CigaretteLog::default();
        log.push(86_400_000, false);
        let s = compute_vape(&p, &log, 2 * 86_400_000);
        assert_eq!(s.streak_secs, 86_400);
    }

    #[test]
    fn vape_never_negative() {
        let p = vprofile();
        let mut log = CigaretteLog::default();
        for i in 0..50 {
            log.push(i * 1000, false);
        }
        let s = compute_vape(&p, &log, 60_000);
        assert!(s.cigs_avoided >= 0.0);
        assert!(s.money_saved >= 0.0);
    }
}
