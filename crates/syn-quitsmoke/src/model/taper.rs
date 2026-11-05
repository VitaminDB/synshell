//! Алгоритм постепенного отказа (тейпер): растущий интервал между сигаретами.

use serde::{Deserialize, Serialize};

use super::profile::{UserProfile, VapeProfile};

const MS_PER_DAY: f64 = 86_400_000.0;
const MS_PER_MIN: f64 = 60_000.0;

/// Кривая тейпера.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaperCurve {
    /// Гиперболический рост интервала: `interval = base / (1 - d/D)` → +∞ при d→D.
    Hyperbolic,
    /// Линейное снижение дневной квоты: `quota = ceil(N0 * (1 - d/D))`.
    LinearCount,
}

impl Default for TaperCurve {
    fn default() -> Self {
        TaperCurve::Hyperbolic
    }
}

/// Расписание тейпера, вычисляемое из профиля.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaperSchedule {
    /// Базовое число сигарет в день (N0), минимум 1.
    pub baseline_per_day: u32,
    /// Длительность программы в днях (D), минимум 1.
    pub taper_days: u32,
    /// Минут бодрствования в сутки.
    pub waking_minutes: f64,
    /// Кривая.
    pub curve: TaperCurve,
    /// Старт программы (epoch ms).
    pub started_at: u64,
}

impl TaperSchedule {
    /// Построить из профиля.
    pub fn from_profile(p: &UserProfile) -> Self {
        Self {
            baseline_per_day: p.cigs_per_day.max(1),
            taper_days: p.taper_days.max(1),
            waking_minutes: (p.waking_hours as f64) * 60.0,
            curve: TaperCurve::Hyperbolic,
            started_at: p.quit_started_at,
        }
    }

    /// Построить из vape-профиля (аналог [`from_profile`], но для вейпа).
    pub fn from_vape_profile(p: &VapeProfile) -> Self {
        Self {
            baseline_per_day: p.vapes_per_day.max(1),
            taper_days: p.taper_days.max(1),
            waking_minutes: (p.waking_hours as f64) * 60.0,
            curve: TaperCurve::Hyperbolic,
            started_at: p.quit_started_at,
        }
    }

    /// Дробное число дней с момента старта.
    pub fn day_fraction(&self, now: u64) -> f64 {
        (now.saturating_sub(self.started_at) as f64) / MS_PER_DAY
    }

    /// Интервал дня 0 (минуты).
    fn base_interval_min(&self) -> f64 {
        self.waking_minutes / (self.baseline_per_day.max(1) as f64)
    }

    /// Программа завершена (сигареты больше не разрешены).
    pub fn is_complete(&self, now: u64) -> bool {
        self.day_fraction(now) >= self.taper_days as f64
    }

    /// Текущий интервал между сигаретами в минутах.
    /// `None` — программа завершена (интервал бесконечен).
    pub fn interval_minutes(&self, now: u64) -> Option<f64> {
        let d = self.day_fraction(now);
        let big_d = self.taper_days as f64;
        if d >= big_d {
            return None;
        }
        // Небольшой clamp снизу, чтобы избежать деления на ~0 у самого конца.
        let remaining = (1.0 - d / big_d).max(1e-4);
        match self.curve {
            TaperCurve::Hyperbolic => Some(self.base_interval_min() / remaining),
            TaperCurve::LinearCount => {
                let quota = (self.baseline_per_day as f64 * remaining).ceil().max(1.0);
                Some(self.waking_minutes / quota)
            }
        }
    }

    /// Сколько сигарет разрешено сегодня (для отображения квоты).
    pub fn allowed_per_day(&self, now: u64) -> u32 {
        let d = self.day_fraction(now);
        let big_d = self.taper_days as f64;
        if d >= big_d {
            return 0;
        }
        (self.baseline_per_day as f64 * (1.0 - d / big_d)).ceil().max(0.0) as u32
    }

    /// Момент, когда можно закурить следующую (epoch ms).
    /// `last_log` — время последней сигареты (или `None`, если ещё не курил).
    /// `None` в результате — программа завершена.
    pub fn next_allowed(&self, now: u64, last_log: Option<u64>) -> Option<u64> {
        let interval = self.interval_minutes(now)?;
        let last = last_log.unwrap_or(self.started_at);
        Some(last + (interval * MS_PER_MIN) as u64)
    }

    /// Прогресс ожидания следующей сигареты в диапазоне [0, 1].
    /// 1.0 — можно курить.
    pub fn progress(&self, now: u64, last_log: Option<u64>) -> f64 {
        let Some(interval) = self.interval_minutes(now) else {
            return 1.0; // программа завершена — «навсегда можно не курить»
        };
        let last = last_log.unwrap_or(self.started_at);
        let interval_ms = interval * MS_PER_MIN;
        if interval_ms <= 0.0 {
            return 1.0;
        }
        let elapsed = now.saturating_sub(last) as f64;
        (elapsed / interval_ms).clamp(0.0, 1.0)
    }

    /// Секунд до момента, когда можно курить (0 — уже можно).
    pub fn secs_until_allowed(&self, now: u64, last_log: Option<u64>) -> u64 {
        match self.next_allowed(now, last_log) {
            Some(next) if next > now => (next - now) / 1000,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched() -> TaperSchedule {
        TaperSchedule {
            baseline_per_day: 20,
            taper_days: 30,
            waking_minutes: 16.0 * 60.0,
            curve: TaperCurve::Hyperbolic,
            started_at: 0,
        }
    }

    #[test]
    fn base_interval_is_waking_over_baseline() {
        let s = sched();
        // 960 минут / 20 = 48 минут в день 0.
        let iv = s.interval_minutes(0).unwrap();
        assert!((iv - 48.0).abs() < 0.5, "iv={iv}");
    }

    #[test]
    fn interval_grows_monotonically() {
        let s = sched();
        let mut prev = 0.0;
        for day in 0..30 {
            let now = (day as u64) * 86_400_000;
            let iv = s.interval_minutes(now).unwrap();
            assert!(iv >= prev, "интервал должен расти: day={day} iv={iv} prev={prev}");
            prev = iv;
        }
    }

    #[test]
    fn interval_becomes_large_near_end() {
        let s = sched();
        // День 29.99 — интервал должен стать очень большим.
        let now = (29.99f64 * 86_400_000.0) as u64;
        let iv = s.interval_minutes(now).unwrap();
        assert!(iv > 48.0 * 50.0, "iv у конца должен быть огромным: {iv}");
    }

    #[test]
    fn program_completes_at_d() {
        let s = sched();
        let now = 30 * 86_400_000;
        assert!(s.is_complete(now));
        assert_eq!(s.interval_minutes(now), None);
        assert_eq!(s.next_allowed(now, None), None);
        assert_eq!(s.allowed_per_day(now), 0);
    }

    #[test]
    fn linear_count_quota_decreases() {
        let mut s = sched();
        s.curve = TaperCurve::LinearCount;
        let q0 = s.allowed_per_day(0);
        let q15 = s.allowed_per_day(15 * 86_400_000);
        let q29 = s.allowed_per_day(29 * 86_400_000);
        assert_eq!(q0, 20);
        assert!(q15 < q0 && q29 < q15, "квота должна падать: {q0} {q15} {q29}");
    }

    #[test]
    fn progress_is_clamped() {
        let s = sched();
        let now = 0;
        // Сразу после старта — прогресс ~0.
        assert!(s.progress(now, Some(0)) < 0.01);
        // Спустя два интервала — прогресс = 1 (clamp).
        let iv_ms = (s.interval_minutes(0).unwrap() * 60_000.0) as u64;
        assert!((s.progress(2 * iv_ms, Some(0)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn next_allowed_after_last_log() {
        let s = sched();
        let last = 1_000_000u64;
        let next = s.next_allowed(last, Some(last)).unwrap();
        let iv_ms = (s.interval_minutes(last).unwrap() * 60_000.0) as u64;
        assert_eq!(next, last + iv_ms);
    }
}
