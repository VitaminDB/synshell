//! Достижения: определения и вычисление разблокировки.
//!
//! Тексты — из i18n по ключам `ach.<id>.title` / `ach.<id>.desc`.

use serde::{Deserialize, Serialize};

use super::stats::DerivedStats;

/// Условие разблокировки достижения.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AchKind {
    /// Первая запись/действие (например, завершён онбординг).
    FirstStep,
    /// Стрик не меньше N секунд.
    StreakSecs(u64),
    /// Избегнуто не меньше N сигарет.
    CigsAvoided(u32),
    /// Сэкономлено не меньше суммы.
    MoneySaved(f64),
}

/// Определение достижения.
#[derive(Clone, Copy, Debug)]
pub struct AchievementDef {
    pub id: &'static str,
    pub icon: &'static str,
    pub kind: AchKind,
}

/// Состояние достижения (сериализуемое).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AchievementState {
    pub id: String,
    /// Момент разблокировки (epoch ms) или `None`.
    pub unlocked_at: Option<u64>,
}

const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;

use crate::icons;

/// Каталог достижений.
pub const ACHIEVEMENTS: [AchievementDef; 12] = [
    AchievementDef { id: "start", icon: icons::STAR, kind: AchKind::FirstStep },
    AchievementDef { id: "hour1", icon: icons::TIME, kind: AchKind::StreakSecs(HOUR) },
    AchievementDef { id: "day1", icon: icons::SUN, kind: AchKind::StreakSecs(DAY) },
    AchievementDef { id: "day3", icon: icons::FIRE, kind: AchKind::StreakSecs(3 * DAY) },
    AchievementDef { id: "week1", icon: icons::CALENDAR, kind: AchKind::StreakSecs(7 * DAY) },
    AchievementDef { id: "week2", icon: icons::LEAF, kind: AchKind::StreakSecs(14 * DAY) },
    AchievementDef { id: "month1", icon: icons::AWARDS, kind: AchKind::StreakSecs(30 * DAY) },
    AchievementDef { id: "avoid10", icon: icons::SMOKE_FREE, kind: AchKind::CigsAvoided(10) },
    AchievementDef { id: "avoid100", icon: icons::STAR, kind: AchKind::CigsAvoided(100) },
    AchievementDef { id: "save500", icon: icons::MONEY, kind: AchKind::MoneySaved(500.0) },
    AchievementDef { id: "save2000", icon: icons::DIAMOND, kind: AchKind::MoneySaved(2000.0) },
    AchievementDef { id: "month3", icon: icons::TROPHY, kind: AchKind::StreakSecs(90 * DAY) },
];

impl AchKind {
    /// Выполнено ли условие при данных показателях.
    pub fn satisfied(&self, stats: &DerivedStats, onboarded: bool) -> bool {
        match *self {
            AchKind::FirstStep => onboarded,
            AchKind::StreakSecs(s) => stats.streak_secs >= s,
            AchKind::CigsAvoided(n) => stats.cigs_avoided >= n as f64,
            AchKind::MoneySaved(m) => stats.money_saved >= m,
        }
    }
}

/// Обновить список состояний: разблокировать вновь достигнутые (сохраняя ранее
/// открытые), проставив `now` как момент разблокировки. Возвращает id только что
/// открытых достижений (для празднования).
pub fn refresh(
    states: &mut Vec<AchievementState>,
    stats: &DerivedStats,
    onboarded: bool,
    now: u64,
) -> Vec<String> {
    let mut newly = Vec::new();
    for def in ACHIEVEMENTS.iter() {
        let entry = match states.iter_mut().find(|s| s.id == def.id) {
            Some(e) => e,
            None => {
                states.push(AchievementState {
                    id: def.id.to_string(),
                    unlocked_at: None,
                });
                states.last_mut().unwrap()
            }
        };
        if entry.unlocked_at.is_none() && def.kind.satisfied(stats, onboarded) {
            entry.unlocked_at = Some(now);
            newly.push(def.id.to_string());
        }
    }
    newly
}

/// Достижение открыто?
pub fn is_unlocked(states: &[AchievementState], id: &str) -> bool {
    states
        .iter()
        .any(|s| s.id == id && s.unlocked_at.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(streak: u64, avoided: f64, money: f64) -> DerivedStats {
        DerivedStats {
            cigs_avoided: avoided,
            money_saved: money,
            time_regained_secs: 0.0,
            streak_secs: streak,
            logged_count: 0,
        }
    }

    #[test]
    fn unlocks_progressively_and_is_sticky() {
        let mut states = Vec::new();
        let new1 = refresh(&mut states, &stats(0, 0.0, 0.0), true, 1000);
        assert!(new1.contains(&"start".to_string()));

        let new2 = refresh(&mut states, &stats(HOUR, 12.0, 0.0), true, 2000);
        assert!(new2.contains(&"hour1".to_string()));
        assert!(new2.contains(&"avoid10".to_string()));
        // Повторно «start» не выдаётся.
        assert!(!new2.contains(&"start".to_string()));

        // Даже если стрик обнулился — ранее открытое остаётся.
        let new3 = refresh(&mut states, &stats(0, 0.0, 0.0), true, 3000);
        assert!(new3.is_empty());
        assert!(is_unlocked(&states, "hour1"));
    }
}

// ── Vape-достижения ─────────────────────────────────────────────────────────

/// Каталог vape-достижений. id с префиксом `v_`, независимый от сигаретного.
pub const VAPE_ACHIEVEMENTS: [AchievementDef; 10] = [
    AchievementDef { id: "v_start", icon: icons::VAPE, kind: AchKind::FirstStep },
    AchievementDef { id: "v_hour1", icon: icons::TIME, kind: AchKind::StreakSecs(HOUR) },
    AchievementDef { id: "v_day1", icon: icons::SUN, kind: AchKind::StreakSecs(DAY) },
    AchievementDef { id: "v_day3", icon: icons::FIRE, kind: AchKind::StreakSecs(3 * DAY) },
    AchievementDef { id: "v_week1", icon: icons::CALENDAR, kind: AchKind::StreakSecs(7 * DAY) },
    AchievementDef { id: "v_week2", icon: icons::LEAF, kind: AchKind::StreakSecs(14 * DAY) },
    AchievementDef { id: "v_month1", icon: icons::AWARDS, kind: AchKind::StreakSecs(30 * DAY) },
    AchievementDef { id: "v_avoid50", icon: icons::VAPE, kind: AchKind::CigsAvoided(50) },
    AchievementDef { id: "v_save1000", icon: icons::MONEY, kind: AchKind::MoneySaved(1000.0) },
    AchievementDef { id: "v_month3", icon: icons::TROPHY, kind: AchKind::StreakSecs(90 * DAY) },
];

/// Обновить vape-состояния; возвращает id только что открытых.
pub fn refresh_vape(
    states: &mut Vec<AchievementState>,
    stats: &DerivedStats,
    onboarded: bool,
    now: u64,
) -> Vec<String> {
    let mut newly = Vec::new();
    for def in VAPE_ACHIEVEMENTS.iter() {
        let entry = match states.iter_mut().find(|s| s.id == def.id) {
            Some(e) => e,
            None => {
                states.push(AchievementState {
                    id: def.id.to_string(),
                    unlocked_at: None,
                });
                states.last_mut().unwrap()
            }
        };
        if entry.unlocked_at.is_none() && def.kind.satisfied(stats, onboarded) {
            entry.unlocked_at = Some(now);
            newly.push(def.id.to_string());
        }
    }
    newly
}
