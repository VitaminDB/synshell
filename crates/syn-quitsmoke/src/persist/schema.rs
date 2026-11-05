//! Сериализуемый снимок состояния приложения + версионирование/миграции.

use serde::{Deserialize, Serialize};

use crate::model::{
    achievements::AchievementState, log::CigaretteLog, profile::{UserProfile, VapeProfile},
    settings::Settings,
};

/// Текущая версия схемы на диске.
pub const CURRENT_VERSION: u32 = 2;

/// Полный снимок состояния для сохранения/загрузки.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppStateData {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub profile: Option<UserProfile>,
    #[serde(default)]
    pub log: CigaretteLog,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub achievements: Vec<AchievementState>,
    #[serde(default)]
    pub onboarded: bool,
    // ── Vape (v2) ──
    /// Профиль вейпера; `None` — режим вейпа не настроен.
    #[serde(default)]
    pub profile_vape: Option<VapeProfile>,
    /// Журнал вейпа (переиспользует [`CigaretteLog`]).
    #[serde(default)]
    pub log_vape: CigaretteLog,
    /// Vape-достижения.
    #[serde(default)]
    pub vape_achievements: Vec<AchievementState>,
    /// Включён ли режим «бросить вейп».
    #[serde(default)]
    pub vape_enabled: bool,
}

impl Default for AppStateData {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            profile: None,
            log: CigaretteLog::default(),
            settings: Settings::default(),
            achievements: Vec::new(),
            onboarded: false,
            profile_vape: None,
            log_vape: CigaretteLog::default(),
            vape_achievements: Vec::new(),
            vape_enabled: false,
        }
    }
}

impl AppStateData {
    /// Привести старую версию к текущей.
    ///
    /// v1 → v2: vape-поля отсутствуют в JSON → serde default (None / пустой лог /
    /// vape выключен). Миграция идемпотентна.
    pub fn migrate(mut self) -> Self {
        if self.version < 2 {
            // v1 не имел vape-полей — они уже заполнены дефолтами через #[serde(default)].
            // Явно выставляем vape_enabled = false (на случай, если JSON содержал мусор).
            self.vape_enabled = false;
            self.profile_vape = None;
            self.log_vape = CigaretteLog::default();
            self.vape_achievements = Vec::new();
        }
        self.version = CURRENT_VERSION;
        self
    }
}
