//! Таймлайн восстановления здоровья.
//!
//! Сетка намеренно плотная там, где тяжелее всего: каждый час первые сутки,
//! каждые 2–3 часа до трёх суток, полусутки до конца первой недели, дальше —
//! по дням, неделям и месяцам. Потолок — 1 год: дальше вести человека
//! приложением уже нет смысла.
//!
//! Тексты берутся из i18n по ключам `health.<key>.title` / `health.<key>.body`.

/// Одна веха восстановления.
#[derive(Clone, Copy, Debug)]
pub struct HealthMilestone {
    /// Смещение от момента отказа (секунды).
    pub offset_secs: u64,
    /// Ключ i18n (`health.<key>.title` / `.body`).
    pub key: &'static str,
    /// Иконка (эмодзи-глиф).
    pub icon: &'static str,
}

const MIN: u64 = 60;
const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;

use crate::icons;

/// Таймлайн восстановления. Порядок строго по возрастанию `offset_secs` —
/// на этом держатся `next_index` и `milestone_progress`.
pub const HEALTH_MILESTONES: [HealthMilestone; 80] = [
    // ── Первый час ────────────────────────────────────────────────────────
    HealthMilestone { offset_secs: 20 * MIN, key: "m20min", icon: icons::HEART },
    HealthMilestone { offset_secs: 40 * MIN, key: "m40min", icon: icons::TIME },
    // ── Первые сутки: каждый час ──────────────────────────────────────────
    HealthMilestone { offset_secs: 1 * HOUR, key: "m1h", icon: icons::TIME },
    HealthMilestone { offset_secs: 2 * HOUR, key: "m2h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 3 * HOUR, key: "m3h", icon: icons::SPA },
    HealthMilestone { offset_secs: 4 * HOUR, key: "m4h", icon: icons::AIR },
    HealthMilestone { offset_secs: 5 * HOUR, key: "m5h", icon: icons::HEART },
    HealthMilestone { offset_secs: 6 * HOUR, key: "m6h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 7 * HOUR, key: "m7h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 8 * HOUR, key: "m8h", icon: icons::AIR },
    HealthMilestone { offset_secs: 9 * HOUR, key: "m9h", icon: icons::SPA },
    HealthMilestone { offset_secs: 10 * HOUR, key: "m10h", icon: icons::STAR },
    HealthMilestone { offset_secs: 11 * HOUR, key: "m11h", icon: icons::HEART },
    HealthMilestone { offset_secs: 12 * HOUR, key: "m12h", icon: icons::AIR },
    HealthMilestone { offset_secs: 13 * HOUR, key: "m13h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 14 * HOUR, key: "m14h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 15 * HOUR, key: "m15h", icon: icons::SPA },
    HealthMilestone { offset_secs: 16 * HOUR, key: "m16h", icon: icons::WALK },
    HealthMilestone { offset_secs: 17 * HOUR, key: "m17h", icon: icons::HEART },
    HealthMilestone { offset_secs: 18 * HOUR, key: "m18h", icon: icons::SPA },
    HealthMilestone { offset_secs: 19 * HOUR, key: "m19h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 20 * HOUR, key: "m20h", icon: icons::AIR },
    HealthMilestone { offset_secs: 21 * HOUR, key: "m21h", icon: icons::STAR },
    HealthMilestone { offset_secs: 22 * HOUR, key: "m22h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 23 * HOUR, key: "m23h", icon: icons::HEART },
    HealthMilestone { offset_secs: 24 * HOUR, key: "m24h", icon: icons::TROPHY },
    // ── Вторые сутки: каждые 2 часа ───────────────────────────────────────
    HealthMilestone { offset_secs: 26 * HOUR, key: "m26h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 28 * HOUR, key: "m28h", icon: icons::AIR },
    HealthMilestone { offset_secs: 30 * HOUR, key: "m30h", icon: icons::SPA },
    HealthMilestone { offset_secs: 32 * HOUR, key: "m32h", icon: icons::HEART },
    HealthMilestone { offset_secs: 34 * HOUR, key: "m34h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 36 * HOUR, key: "m36h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 38 * HOUR, key: "m38h", icon: icons::WALK },
    HealthMilestone { offset_secs: 40 * HOUR, key: "m40h", icon: icons::AIR },
    HealthMilestone { offset_secs: 42 * HOUR, key: "m42h", icon: icons::SPA },
    HealthMilestone { offset_secs: 44 * HOUR, key: "m44h", icon: icons::STAR },
    HealthMilestone { offset_secs: 46 * HOUR, key: "m46h", icon: icons::HEART },
    HealthMilestone { offset_secs: 48 * HOUR, key: "m48h", icon: icons::TROPHY },
    // ── Третьи сутки: каждые 3 часа ───────────────────────────────────────
    HealthMilestone { offset_secs: 51 * HOUR, key: "m51h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 54 * HOUR, key: "m54h", icon: icons::AIR },
    HealthMilestone { offset_secs: 57 * HOUR, key: "m57h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 60 * HOUR, key: "m60h", icon: icons::SPA },
    HealthMilestone { offset_secs: 63 * HOUR, key: "m63h", icon: icons::HEART },
    HealthMilestone { offset_secs: 66 * HOUR, key: "m66h", icon: icons::WALK },
    HealthMilestone { offset_secs: 69 * HOUR, key: "m69h", icon: icons::STAR },
    HealthMilestone { offset_secs: 72 * HOUR, key: "m72h", icon: icons::TROPHY },
    // ── До конца первой недели: каждые 12 часов ───────────────────────────
    HealthMilestone { offset_secs: 84 * HOUR, key: "m84h", icon: icons::AIR },
    HealthMilestone { offset_secs: 96 * HOUR, key: "m96h", icon: icons::LEAF },
    HealthMilestone { offset_secs: 108 * HOUR, key: "m108h", icon: icons::SPA },
    HealthMilestone { offset_secs: 120 * HOUR, key: "m120h", icon: icons::STAR },
    HealthMilestone { offset_secs: 132 * HOUR, key: "m132h", icon: icons::BOLT },
    HealthMilestone { offset_secs: 144 * HOUR, key: "m144h", icon: icons::HEART },
    HealthMilestone { offset_secs: 156 * HOUR, key: "m156h", icon: icons::WALK },
    HealthMilestone { offset_secs: 168 * HOUR, key: "m168h", icon: icons::TROPHY },
    // ── Вторая неделя: каждый день ────────────────────────────────────────
    HealthMilestone { offset_secs: 8 * DAY, key: "m8d", icon: icons::LEAF },
    HealthMilestone { offset_secs: 9 * DAY, key: "m9d", icon: icons::AIR },
    HealthMilestone { offset_secs: 10 * DAY, key: "m10d", icon: icons::WALK },
    HealthMilestone { offset_secs: 11 * DAY, key: "m11d", icon: icons::SPA },
    HealthMilestone { offset_secs: 12 * DAY, key: "m12d", icon: icons::BOLT },
    HealthMilestone { offset_secs: 13 * DAY, key: "m13d", icon: icons::HEART },
    HealthMilestone { offset_secs: 14 * DAY, key: "m14d", icon: icons::TROPHY },
    // ── Первый месяц: каждые 2–3 дня ──────────────────────────────────────
    HealthMilestone { offset_secs: 16 * DAY, key: "m16d", icon: icons::LEAF },
    HealthMilestone { offset_secs: 18 * DAY, key: "m18d", icon: icons::AIR },
    HealthMilestone { offset_secs: 21 * DAY, key: "m21d", icon: icons::HEART },
    HealthMilestone { offset_secs: 24 * DAY, key: "m24d", icon: icons::WALK },
    HealthMilestone { offset_secs: 27 * DAY, key: "m27d", icon: icons::SPA },
    HealthMilestone { offset_secs: 30 * DAY, key: "m30d", icon: icons::TROPHY },
    // ── Квартал: каждые полмесяца ─────────────────────────────────────────
    HealthMilestone { offset_secs: 45 * DAY, key: "m45d", icon: icons::SPA },
    HealthMilestone { offset_secs: 60 * DAY, key: "m60d", icon: icons::BOLT },
    HealthMilestone { offset_secs: 75 * DAY, key: "m75d", icon: icons::LEAF },
    HealthMilestone { offset_secs: 90 * DAY, key: "m90d", icon: icons::TROPHY },
    // ── До года: каждый месяц ─────────────────────────────────────────────
    HealthMilestone { offset_secs: 120 * DAY, key: "m120d", icon: icons::AIR },
    HealthMilestone { offset_secs: 150 * DAY, key: "m150d", icon: icons::WALK },
    HealthMilestone { offset_secs: 180 * DAY, key: "m180d", icon: icons::HEART },
    HealthMilestone { offset_secs: 210 * DAY, key: "m210d", icon: icons::SPA },
    HealthMilestone { offset_secs: 240 * DAY, key: "m240d", icon: icons::LEAF },
    HealthMilestone { offset_secs: 270 * DAY, key: "m270d", icon: icons::BOLT },
    HealthMilestone { offset_secs: 300 * DAY, key: "m300d", icon: icons::STAR },
    HealthMilestone { offset_secs: 330 * DAY, key: "m330d", icon: icons::BRAIN },
    HealthMilestone { offset_secs: 365 * DAY, key: "m365d", icon: icons::TROPHY },
];

/// Ключ последней (финальной) вехи — для экранов, где все вехи уже достигнуты.
pub const FINAL_KEY: &str = "m365d";

/// Индекс следующей ещё не достигнутой вехи (`None`, если все достигнуты).
pub fn next_index(streak_secs: u64) -> Option<usize> {
    HEALTH_MILESTONES
        .iter()
        .position(|m| m.offset_secs > streak_secs)
}

/// Сколько вех уже достигнуто.
pub fn reached_count(streak_secs: u64) -> usize {
    HEALTH_MILESTONES
        .iter()
        .filter(|m| m.offset_secs <= streak_secs)
        .count()
}

/// Прогресс до конкретной вехи в [0, 1].
pub fn milestone_progress(streak_secs: u64, index: usize) -> f64 {
    if index >= HEALTH_MILESTONES.len() {
        return 1.0;
    }
    let target = HEALTH_MILESTONES[index].offset_secs;
    let prev = if index == 0 {
        0
    } else {
        HEALTH_MILESTONES[index - 1].offset_secs
    };
    if streak_secs >= target {
        return 1.0;
    }
    if streak_secs <= prev {
        return 0.0;
    }
    (streak_secs - prev) as f64 / (target - prev) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_are_sorted_and_capped_at_one_year() {
        for w in HEALTH_MILESTONES.windows(2) {
            assert!(w[0].offset_secs < w[1].offset_secs, "{} !< {}", w[0].key, w[1].key);
        }
        let last = HEALTH_MILESTONES[HEALTH_MILESTONES.len() - 1];
        assert_eq!(last.offset_secs, 365 * DAY);
        assert_eq!(last.key, FINAL_KEY);
    }

    #[test]
    fn first_day_is_hourly() {
        // Между 1 и 24 часами шаг ровно час — 3 и 6 часов не должны сливаться.
        for h in 1..=24u64 {
            assert!(
                HEALTH_MILESTONES.iter().any(|m| m.offset_secs == h * HOUR),
                "нет вехи на {h} ч"
            );
        }
    }

    #[test]
    fn every_milestone_has_texts_in_both_locales() {
        use crate::i18n::t;
        use crate::model::settings::Locale;
        for m in HEALTH_MILESTONES.iter() {
            for loc in [Locale::Ru, Locale::En] {
                assert!(
                    !t(&format!("health.{}.title", m.key), loc).is_empty(),
                    "нет заголовка для {} ({loc:?})",
                    m.key
                );
                assert!(
                    !t(&format!("health.{}.body", m.key), loc).is_empty(),
                    "нет текста для {} ({loc:?})",
                    m.key
                );
            }
        }
    }

    #[test]
    fn next_index_walks_forward() {
        assert_eq!(next_index(0), Some(0));
        let i = next_index(3 * HOUR).unwrap();
        assert_eq!(HEALTH_MILESTONES[i].offset_secs, 4 * HOUR);
        assert_eq!(next_index(400 * DAY), None);
    }
}
