# QuitSmoke — режим «Бросить вейп» (Vape). Анализ и план

## Что уже есть (архитектура)
- **Стек**: Rust + syngui (path `../syngui/syngui`), MSS-стили (`styles/base.mss`), serde JSON persistence.
- **Точка входа**: `src/lib.rs` → `nav::build_app(state)`; состояние — `AppState` из RwSignal.
- **Персистентность**: `src/persist/mod.rs`, файл `quitsmoke.json`, снимок = `AppStateData { version, profile: Option<UserProfile>, log: CigaretteLog, settings, achievements, onboarded }`.
  - ⚠️ ОДИН профиль / ОДИН лог. Для вейпа нужен второй профиль + второй лог → **миграция схемы до v2**.

## Ключевые файлы и что в них менять

| Файл | Роль | Изменения |
|------|------|-----------|
| `src/model/profile.rs` | `UserProfile { cigs_per_day, cigs_per_pack, price_per_pack, years_smoking, quit_started_at, mode: QuitMode, waking_hours, taper_days }`, `health_baseline()` | Добавить поля вейпа: `vapes_per_day`, `pods_per_pack`(?), `price_per_pod/pack`, `years_vaping`, `quit_started_at_vape`. Либо отдельный struct. |
| `src/model/log.rs` | `CigaretteLog { entries: Vec<LogEntry> }` — универсальный (at, allowed, note) | Переиспользовать как тип для вейпа (`VapeLog = CigaretteLog`) — логика идентична. |
| `src/model/stats.rs` | `compute(profile, log, now) -> DerivedStats { cigs_avoided, money_saved, time_regained_secs, streak_secs }`, `streak_start()` | Добавить vape-вариант: `compute_vape()` / параметризовать. SECONDS_PER_CIG_LIFE — для вейпа своя константа. |
| `src/model/taper.rs` | `TaperSchedule::from_profile(&UserProfile)` — берёт cigs_per_day, waking_hours, taper_days, quit_started_at | Нужен vape-тейпер из vape-полей. |
| `src/app_state.rs` | 2 счётчика сейчас: сигареты. Мутации: `log_cigarette()`, `complete_onboarding(profile)`, `refresh_achievements`, `reset_all()`. `snapshot()/from_snapshot()` | Добавить сигналы: **`profile_vape: RwSignal<VapeProfile>`, `log_vape: RwSignal<CigaretteLog>`** (+ vape-achievements). Мутации: `log_vape_hit()`, `start/stop vaping quit`. Держать оба счётчика активными одновременно. |
| `src/persist/schema.rs` | AppStateData v1 | **v2**: + `profile_vape: Option<VapeProfile>`, + `log_vape: CigaretteLog`, миграция v1→v2. |
| `src/screens/dashboard.rs` | 1 hero-ring (сигареты), stats grid, milestone card, кнопки. | 2-й блок для вейпа: ring + счётчики; оба работают одновременно. |
| `src/screens/settings.rs` | Тёмная тема/язык/валюта/уведомления/сброс | Секция «Бросить курить вейп»: toggle vape-режима, параметры (pods/day, цена), дата отказа. |
| `src/screens/onboarding.rs` | 7 шагов (`STEPS=7`), Form с mode/cigs/price/quit_date. `step_finish()` собирает UserProfile. | Расширить форму для вейпа (или отдельный мини-онбординг для вейпа из настроек). |
| `src/i18n.rs` | table! RU/EN; ключи `dash.*`, `onb.*`, `settings.*` | Добавить строки для вейпа. |
| `src/icons.rs` | Material глифы (проверены по cmap!) | ⚠️ Нужна валидная иконка для вейпа — проверить глиф или эмодзи 💨. |
| `styles/base.mss` | классы `.ring-*`, `.stat-card`, `.tint-*` | Добавить стили для второго счётчика (вейп-акцент). |

## Важные детали
- `AppState` — `Copy` (все поля RwSignal), new-type для вейпа тоже должен быть Copy.
- `stats_now()` читает реактивно; для вейпа — аналог `vape_stats_now()`.
- Heartbeat (`src/widgets/heartbeat.rs`) раз в секунду дергает `state.tick()` → обновляет now_tick, achievements, notify. Нужно расширить tick на vape.
- Achievements (`src/model/achievements.rs`) — 12 штук по DerivedStats; для вейпа — отдельный список или общие?
- `notify/android.rs` — 229 строк, привязан к сигаретам; для вейпа расширять аккуратно.

## Решения (обдуманные)
1. **VapeProfile** — отдельный struct (не захламлять UserProfile), serde-сериализуемый, поля: vapes_per_day, price_per_pod, years_vaping, quit_started_at_vape, mode (Gradual/Immediate), waking_hours (общая), taper_days.
2. **VapeLog** = `CigaretteLog` переиспользуется как есть (entries at/allowed/note).
3. **Два счётчика одновременно**: dashboard показывает оба hero-блока; каждый со своим ring + stats; независимые стрики.
4. **Схема v2** с миграцией: старые v1-файлы → vape поля = None/defaults (вейп-режим выключен).
5. **Настройки**: секция «Вейп» — включить/выключить, параметры, дата старта отказа.

## Статус
- [ ] memory/vape-mode-analysis.md создан ✅
- [ ] model/profile.rs — VapeProfile
- [ ] model/stats.rs — compute_vape + константа SECONDS_PER_VAPE_LIFE
- [ ] app_state.rs — сигналы profile_vape/log_vape + мутации
- [ ] persist/schema.rs — v2 + миграция
- [ ] dashboard.rs — второй счётчик
- [ ] settings.rs — секция вейпа
- [ ] i18n.rs — строки RU/EN
