//! Глобальное состояние приложения на сигналах syngui + мутации.

use syngui::prelude::{try_use_context, use_signal, RwSignal};

use crate::model::achievements::{self, AchievementState};
use crate::model::log::CigaretteLog;
use crate::model::profile::{QuitMode, UserProfile, VapeProfile};
use crate::model::settings::{Locale, Settings};
use crate::model::stats::{self, DerivedStats};
use crate::model::achievements::VAPE_ACHIEVEMENTS;
use crate::model::taper::TaperSchedule;
use crate::persist::{AppStateData, Persistence, CURRENT_VERSION};
use crate::util::now_ms;

/// Текущий экран (маршрут).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Onboarding,
    Home,
    Stats,
    Health,
    Achievements,
    Settings,
    Craving,
}

/// Набор сигналов состояния. `Copy` — как `CalcCtx` в референс-приложении.
#[derive(Clone, Copy)]
pub struct AppState {
    pub profile: RwSignal<UserProfile>,
    pub log: RwSignal<CigaretteLog>,
    pub settings: RwSignal<Settings>,
    pub achievements: RwSignal<Vec<AchievementState>>,
    pub screen: RwSignal<Screen>,
    pub onboarded: RwSignal<bool>,
    /// Пульс времени (epoch ms), обновляется раз в секунду из Heartbeat.
    pub now_tick: RwSignal<u64>,
    /// Токен празднования — инкремент запускает конфетти.
    pub celebrate: RwSignal<u32>,
    /// Тёмная тема (отдельный сигнал для `App::with_theme_styles`).
    pub theme_dark: RwSignal<bool>,

    // ── Vape (режим «бросить вейп») ──────────────────────────────────────
    /// Профиль вейпера (независимый от сигаретного).
    pub profile_vape: RwSignal<VapeProfile>,
    /// Журнал вейпа (переиспользует [`CigaretteLog`]).
    pub log_vape: RwSignal<CigaretteLog>,
    /// Vape-достижения (id с префиксом `v_`).
    pub vape_achievements: RwSignal<Vec<AchievementState>>,
    /// Включён ли режим «бросить вейп» (показывать ли второй счётчик).
    pub vape_enabled: RwSignal<bool>,

    /// Состояние окна (фокус, развёрнуто) — для своих кнопок окна на компьютере.
    pub window: RwSignal<syngui::window::WindowState>,
}

impl AppState {
    /// Собрать из снимка (вызывать после `provide_context(Persistence)`).
    pub fn from_snapshot(data: &AppStateData) -> Self {
        let onboarded = data.onboarded;
        Self {
            profile: use_signal(data.profile.clone().unwrap_or_default()),
            log: use_signal(data.log.clone()),
            settings: use_signal(data.settings.clone()),
            achievements: use_signal(data.achievements.clone()),
            screen: use_signal(if onboarded {
                Screen::Home
            } else {
                Screen::Onboarding
            }),
            onboarded: use_signal(onboarded),
            now_tick: use_signal(now_ms()),
            celebrate: use_signal(0u32),
            theme_dark: use_signal(data.settings.dark_mode),
            // Vape
            profile_vape: use_signal(data.profile_vape.clone().unwrap_or_default()),
            log_vape: use_signal(data.log_vape.clone()),
            vape_achievements: use_signal(data.vape_achievements.clone()),
            vape_enabled: use_signal(data.vape_enabled),
            window: use_signal(syngui::window::WindowState::default()),
        }
    }

    /// Снимок для сохранения (untracked-чтения).
    pub fn snapshot(&self) -> AppStateData {
        AppStateData {
            version: CURRENT_VERSION,
            profile: Some(self.profile.get_untracked()),
            log: self.log.get_untracked(),
            settings: self.settings.get_untracked(),
            achievements: self.achievements.get_untracked(),
            onboarded: self.onboarded.get_untracked(),
            // Vape
            profile_vape: Some(self.profile_vape.get_untracked()),
            log_vape: self.log_vape.get_untracked(),
            vape_achievements: self.vape_achievements.get_untracked(),
            vape_enabled: self.vape_enabled.get_untracked(),
        }
    }

    /// Сохранить через Persistence из контекста.
    pub fn save(&self) {
        if let Some(p) = try_use_context::<Persistence>() {
            p.save(&self.snapshot());
        }
    }

    // ── Реактивные геттеры (для использования внутри render-замыканий) ──

    pub fn locale(&self) -> Locale {
        self.settings.get().locale
    }

    /// Перевести ключ по текущей локали (реактивно — обновляется при смене языка).
    pub fn tr(&self, key: &str) -> &'static str {
        crate::i18n::t(key, self.settings.get().locale)
    }

    /// Производные показатели на реактивно-считанный момент.
    pub fn stats_now(&self) -> DerivedStats {
        let now = self.now_tick.get();
        stats::compute(&self.profile.get(), &self.log.get(), now)
    }

    pub fn taper(&self) -> TaperSchedule {
        TaperSchedule::from_profile(&self.profile.get())
    }

    /// Vape-показатели на реактивно-считанный момент.
    pub fn vape_stats_now(&self) -> DerivedStats {
        let now = self.now_tick.get();
        stats::compute_vape(&self.profile_vape.get(), &self.log_vape.get(), now)
    }

    /// Vape-тейпер (для gradual-режима вейпа).
    pub fn vape_taper(&self) -> TaperSchedule {
        TaperSchedule::from_vape_profile(&self.profile_vape.get())
    }

    // ── Untracked-версии (для мутаций и Heartbeat) ──

    fn stats_untracked(&self, now: u64) -> DerivedStats {
        stats::compute(
            &self.profile.get_untracked(),
            &self.log.get_untracked(),
            now,
        )
    }

    fn vape_stats_untracked(&self, now: u64) -> DerivedStats {
        stats::compute_vape(
            &self.profile_vape.get_untracked(),
            &self.log_vape.get_untracked(),
            now,
        )
    }

    // ── Мутации ────────────────────────────────────────────────────────

    /// Отметить выкуренную сигарету.
    pub fn log_cigarette(&self) {
        let now = now_ms();
        let profile = self.profile.get_untracked();
        let allowed = match profile.mode {
            QuitMode::Immediate => false,
            QuitMode::Gradual => {
                let sched = TaperSchedule::from_profile(&profile);
                let last = self.log.get_untracked().last_at();
                match sched.next_allowed(now, last) {
                    Some(next) => now >= next,
                    None => false,
                }
            }
        };
        self.log.update(|l| l.push(now, allowed));
        self.refresh_achievements(now);
        self.save();
        crate::notify::sync(self);
    }

    /// Завершить онбординг с готовым профилем.
    pub fn complete_onboarding(&self, profile: UserProfile) {
        self.profile.set(profile);
        self.onboarded.set(true);
        self.screen.set(Screen::Home);
        self.refresh_achievements(now_ms());
        self.save();
        crate::notify::init(self);
        crate::notify::sync(self);
        self.trigger_celebrate();
    }

    /// Пересчитать достижения; при новых — отпраздновать.
    pub fn refresh_achievements(&self, now: u64) {
        let stats = self.stats_untracked(now);
        let onboarded = self.onboarded.get_untracked();
        let mut states = self.achievements.get_untracked();
        let newly = achievements::refresh(&mut states, &stats, onboarded, now);
        if !newly.is_empty() {
            self.achievements.set(states);
            self.trigger_celebrate();
        }
    }

    pub fn trigger_celebrate(&self) {
        self.celebrate.update(|c| *c = c.wrapping_add(1));
    }

    pub fn go(&self, s: Screen) {
        self.screen.set(s);
    }

    pub fn set_dark(&self, dark: bool) {
        self.theme_dark.set(dark);
        self.settings.update(|s| s.dark_mode = dark);
        self.save();
    }

    pub fn set_locale(&self, locale: Locale) {
        // Сначала язык каталогов — перерисовка после смены настройки уже переведена.
        crate::i18n::apply(locale);
        self.settings.update(|s| s.locale = locale);
        self.sync_dialog_labels();
        self.save();
    }

    /// Локализовать подписи кнопок встроенных диалогов syngui под текущий язык.
    pub fn sync_dialog_labels(&self) {
        let loc = self.settings.get_untracked().locale;
        syngui::prelude::set_dialog_labels(
            crate::i18n::t("common.ok", loc),
            crate::i18n::t("common.cancel", loc),
        );
    }

    pub fn set_currency(&self, symbol: String) {
        self.settings.update(|s| s.currency_symbol = symbol);
        self.save();
    }

    pub fn set_notifications(&self, enabled: bool) {
        self.settings.update(|s| s.notifications_enabled = enabled);
        self.save();
        if enabled {
            crate::notify::init(self);
            crate::notify::sync(self);
        } else {
            crate::notify::clear();
        }
    }

    pub fn set_reminder_style(&self, style: crate::model::settings::ReminderStyle) {
        self.settings.update(|s| s.reminder_style = style);
        self.save();
        crate::notify::sync(self);
    }

    /// Перезапустить онбординг, сохранив накопленный прогресс.
    pub fn restart_onboarding(&self) {
        self.onboarded.set(false);
        self.screen.set(Screen::Onboarding);
        self.save();
    }

    /// Отмена мастера настройки (кнопка/жест «назад» на первом шаге):
    /// возвращаемся на главный, сохраняя текущий профиль.
    pub fn cancel_onboarding(&self) {
        self.onboarded.set(true);
        self.screen.set(Screen::Home);
        self.save();
    }

    /// Полный сброс прогресса (включая vape).
    pub fn reset_all(&self) {
        self.log.set(CigaretteLog::default());
        self.achievements.set(Vec::new());
        self.onboarded.set(false);
        self.screen.set(Screen::Onboarding);
        // Vape
        self.log_vape.set(CigaretteLog::default());
        self.vape_achievements.set(Vec::new());
        self.vape_enabled.set(false);
        self.save();
        crate::notify::clear();
    }

    /// Секундный тик из Heartbeat: обновить время, достижения, уведомления.
    pub fn tick(&self) {
        let now = now_ms();
        self.now_tick.set(now);
        self.refresh_achievements(now);
        if self.vape_enabled.get_untracked() {
            self.refresh_vape_achievements(now);
        }
        crate::notify::sync(self);
        crate::notify::poll(self);
    }

    // ── Vape-мутации ────────────────────────────────────────────────────────

    /// Отметить вейп-сессию (срыв для immediate / по расписанию для gradual).
    pub fn log_vape_hit(&self) {
        let now = now_ms();
        let profile = self.profile_vape.get_untracked();
        let allowed = match profile.mode {
            QuitMode::Immediate => false,
            QuitMode::Gradual => {
                let sched = TaperSchedule::from_vape_profile(&profile);
                let last = self.log_vape.get_untracked().last_at();
                match sched.next_allowed(now, last) {
                    Some(next) => now >= next,
                    None => false,
                }
            }
        };
        self.log_vape.update(|l| l.push(now, allowed));
        self.refresh_vape_achievements(now);
        self.save();
    }

    /// Пересчитать vape-достижения; при новых — отпраздновать.
    pub fn refresh_vape_achievements(&self, now: u64) {
        let stats = self.vape_stats_untracked(now);
        let onboarded = self.onboarded.get_untracked();
        let mut states = self.vape_achievements.get_untracked();
        let newly = VAPE_ACHIEVEMENTS.iter().filter(|def| {
            let entry = states.iter_mut().find(|s| s.id == def.id);
            match entry {
                Some(e) => {
                    if e.unlocked_at.is_none() && def.kind.satisfied(&stats, onboarded) {
                        e.unlocked_at = Some(now);
                        true
                    } else {
                        false
                    }
                }
                None => {
                    if def.kind.satisfied(&stats, onboarded) {
                        states.push(AchievementState {
                            id: def.id.to_string(),
                            unlocked_at: Some(now),
                        });
                        true
                    } else {
                        false
                    }
                }
            }
        });
        let newly_ids: Vec<String> = newly.map(|d| d.id.to_string()).collect();
        if !newly_ids.is_empty() {
            self.vape_achievements.set(states);
            self.trigger_celebrate();
        }
    }

    /// Включить/выключить режим вейпа.
    pub fn set_vape_enabled(&self, enabled: bool) {
        self.vape_enabled.set(enabled);
        self.save();
    }

    /// Запустить отказ от вейпа с готовым профилем.
    pub fn start_vape_quit(&self, profile: VapeProfile) {
        self.profile_vape.set(profile);
        self.vape_enabled.set(true);
        self.refresh_vape_achievements(now_ms());
        self.save();
        self.trigger_celebrate();
    }

    /// Сбросить vape-прогресс (лог, достижения), оставив профиль.
    pub fn reset_vape(&self) {
        self.log_vape.set(CigaretteLog::default());
        self.vape_achievements.set(Vec::new());
        self.save();
    }
}

/// Загрузить состояние из Persistence в контексте (или дефолт).
pub fn load_state() -> AppState {
    let data = try_use_context::<Persistence>()
        .map(|p| p.load())
        .unwrap_or_default();
    AppState::from_snapshot(&data)
}
