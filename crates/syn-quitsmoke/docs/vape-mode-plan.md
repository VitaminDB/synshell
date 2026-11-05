# План реализации: режим «Бросить вейп» (Vape) — конкретные правки

## Решения (утверждены)
1. **VapeProfile** — отдельный struct (уже есть в model/profile.rs). ✅
2. **VapeLog = CigaretteLog** — переиспользуется как есть (entries at/allowed/note). ✅
3. **Два счётчика одновременно** на dashboard: сигареты + вейп, независимые стрики/кольца.
4. **Схема v2** с миграцией v1→v2 (vape выключен по умолчанию).
5. **Настройки**: секция «Вейп» — toggle vape-режима + параметры + дата отказа.
6. **Vape achievements** — отдельный список (prefix "v_"), независимый от сигаретного.

## Ключевые точки интеграции (что уже выяснено)
- AppState — Copy, все поля RwSignal. from_snapshot/snapshot — точки (де)сериализации.
- Heartbeat.tick() → state.tick() раз в сек. Расширить на vape.
- notify::sync — читает profile/log; для vape расширять аккуратно (desktop no-op).
- i18n table!() — макрос; ключи dash.*, settings.*, onb.*.
- styles/base.mss — классы .ring-*, .tint-*, .stat-card. Нужен vape-акцент (cyan/teal).
- theme_light.mss / theme_dark.mss — токены. Добавить --vape / --vape-soft.

## Файлы и правки (порядок)
### 1. model/stats.rs
- SECONDS_PER_VAPE_LIFE константа (≈ 0.5 мин? вейп менее вреден за сессию — возьмём 5 мин).
- compute_vape(profile: &VapeProfile, log, now) -> DerivedStats (переиспользует DerivedStats).
- vape_streak_start(profile, log).

### 2. model/achievements.rs
- VAPE_ACHIEVEMENTS список (id "v_*").
- refresh_vape(states, stats, onboarded, now).

### 3. app_state.rs
- Поля: profile_vape: RwSignal<VapeProfile>, log_vape: RwSignal<CigaretteLog>,
  vape_achievements: RwSignal<Vec<AchievementState>>, vape_enabled: RwSignal<bool>.
- from_snapshot/snapshot — добавить vape.
- vape_stats_now(), vape_taper().
- Мутации: log_vape_hit(), set_vape_enabled(bool), start_vape_quit(profile), reset_vape().
- tick() — обновлять vape achievements если включён.

### 4. persist/schema.rs
- CURRENT_VERSION = 2.
- AppStateData: + profile_vape: Option<VapeProfile>, + log_vape: CigaretteLog,
  + vape_achievements: Vec<AchievementState>, + vape_enabled: bool.
- migrate(): v1 → defaults для vape.

### 5. screens/dashboard.rs
- Если vape_enabled — показать второй hero-блок (кольцо вейпа) + stats.
- Рефактор: вынести render_cigarette_block() и render_vape_block().

### 6. screens/settings.rs
- Секция «Вейп»: toggle vape_enabled, параметры (vapes/day, price/pod, puffs/pod, years),
  дата/время отказа, режим.

### 7. i18n.rs
- vape.* ключи RU/EN.

### 8. styles
- base.mss: .ring-vape, .tint-vape, .vape-block.
- theme_light/dark: --vape, --vape-soft.

### 9. icons.rs
- VAPE глиф (проверить cmap! если нет — эмодзи 💨).

## Статус выполнения
- [x] Анализ (vape-mode-analysis.md)
- [ ] stats.rs compute_vape
- [ ] achievements vape
- [ ] app_state vape сигналы
- [ ] schema v2
- [ ] dashboard 2-й блок
- [ ] settings секция
- [ ] i18n
- [ ] styles
- [ ] icons
- [ ] cargo build + test
