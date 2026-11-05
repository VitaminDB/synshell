//! Профиль пользователя и режим отказа.

use serde::{Deserialize, Serialize};

/// Режим отказа от курения.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuitMode {
    /// Постепенно — растущий интервал между разрешёнными сигаретами (тейпер).
    Gradual,
    /// Сразу — cold turkey: стрик без курения, вехи здоровья.
    Immediate,
}

impl Default for QuitMode {
    fn default() -> Self {
        QuitMode::Immediate
    }
}

/// Профиль курильщика, собираемый в онбординге.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    /// Базовое число сигарет в день (N0).
    pub cigs_per_day: u32,
    /// Сигарет в пачке (обычно 20).
    pub cigs_per_pack: u32,
    /// Цена пачки в валюте настроек.
    pub price_per_pack: f64,
    /// Стаж курения в годах.
    pub years_smoking: f32,
    /// Момент старта программы (epoch ms). Для «сразу» — момент отказа;
    /// для «постепенно» — старт тейпера.
    pub quit_started_at: u64,
    /// Выбранный режим.
    pub mode: QuitMode,
    /// Часы бодрствования в сутки (для расчёта интервала тейпера).
    pub waking_hours: f32,
    /// Длительность программы тейпера в днях (D), только для режима «постепенно».
    pub taper_days: u32,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
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
}

impl UserProfile {
    /// Дата, с которой отсчитываются вехи здоровья.
    /// Для «сразу» — момент отказа; для «постепенно» — момент завершения программы
    /// (start + taper_days), т.к. до этого пользователь ещё курит.
    pub fn health_baseline(&self) -> u64 {
        match self.mode {
            QuitMode::Immediate => self.quit_started_at,
            QuitMode::Gradual => {
                self.quit_started_at + (self.taper_days as u64) * 86_400_000
            }
        }
    }
}

/// Профиль вейпера для отдельного режима «бросить вейп».
///
/// Держится отдельно от [`UserProfile`]: у пользователя могут быть независимые
/// стрики отказа от сигарет и от вейпа, которые идут одновременно.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VapeProfile {
    /// Вейпов (затяжек-сессий) в день (N0).
    pub vapes_per_day: u32,
    /// Цена одного pod/картриджа.
    pub price_per_pod: f64,
    /// Затяжек в одном pod (для пересчёта «pod → затяжка»).
    pub puffs_per_pod: u32,
    /// Стаж вейпинга в годах.
    pub years_vaping: f32,
    /// Момент старта отказа от вейпа (epoch ms).
    pub quit_started_at: u64,
    /// Режим отказа (тот же [`QuitMode`], что и для сигарет).
    pub mode: QuitMode,
    /// Часы бодрствования (для расчёта интервала тейпера вейпа).
    pub waking_hours: f32,
    /// Длительность программы тейпера в днях.
    pub taper_days: u32,
}

impl Default for VapeProfile {
    fn default() -> Self {
        Self {
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
}

impl VapeProfile {
    /// Дата/момент для отсчёта вех здоровья вейпа — аналогично сигаретам.
    pub fn health_baseline(&self) -> u64 {
        match self.mode {
            QuitMode::Immediate => self.quit_started_at,
            QuitMode::Gradual => {
                self.quit_started_at + (self.taper_days as u64) * 86_400_000
            }
        }
    }

    /// Цена одной «сессии» (одного вейпа) для расчёта экономии.
    ///
    /// Стоимость дня делится на число сессий: если puffs_per_pod задан, считаем
    /// pod'ов в день как ceil(vapes / puffs), иначе — vapes * price_per_pod.
    pub fn unit_price(&self) -> f64 {
        let n = self.vapes_per_day.max(1) as f64;
        if self.puffs_per_pod > 0 {
            let pods_per_day = (n / self.puffs_per_pod as f64).ceil().max(1.0);
            pods_per_day * self.price_per_pod / n
        } else {
            self.price_per_pod
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vape_default_is_sane() {
        let p = VapeProfile::default();
        assert!(p.vapes_per_day > 0);
        assert_eq!(p.mode, QuitMode::Immediate);
    }

    #[test]
    fn unit_price_positive() {
        let p = VapeProfile { vapes_per_day: 10, price_per_pod: 500.0, puffs_per_pod: 300, ..Default::default() };
        assert!(p.unit_price() > 0.0);
    }

    #[test]
    fn gradual_baseline_shifts_by_taper_days() {
        let p = VapeProfile { quit_started_at: 1_000_000, mode: QuitMode::Gradual, taper_days: 3, ..Default::default() };
        assert_eq!(p.health_baseline(), 1_000_000 + 3 * 86_400_000);
    }
}
