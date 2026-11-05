//! Настройки приложения.

use serde::{Deserialize, Serialize};

/// Язык интерфейса.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    Ru,
    En,
}

impl Default for Locale {
    fn default() -> Self {
        Locale::Ru
    }
}

impl Locale {
    pub fn toggled(self) -> Self {
        match self {
            Locale::Ru => Locale::En,
            Locale::En => Locale::Ru,
        }
    }
}

/// Стиль напоминания в системном уведомлении (режим «постепенно»).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReminderStyle {
    /// Только текстовый хронометр обратного отсчёта.
    Chronometer,
    /// Живой прогресс-бар (foreground-сервис).
    ProgressBar,
    /// И бар, и хронометр-fallback.
    Both,
}

impl Default for ReminderStyle {
    fn default() -> Self {
        ReminderStyle::Both
    }
}

/// Пользовательские настройки.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Тёмная тема (false = светлая, флагманская).
    pub dark_mode: bool,
    /// Символ валюты для отображения сумм.
    pub currency_symbol: String,
    /// Уведомления включены.
    pub notifications_enabled: bool,
    /// Стиль напоминания.
    pub reminder_style: ReminderStyle,
    /// Язык интерфейса.
    pub locale: Locale,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark_mode: false,
            currency_symbol: "₸".to_string(), // тенге по умолчанию
            notifications_enabled: true,
            reminder_style: ReminderStyle::Both,
            locale: Locale::Ru,
        }
    }
}
