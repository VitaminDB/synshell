//! Настройки приложения.

use serde::{Deserialize, Serialize};

/// Язык интерфейса: тег каталога («ru», «de», «pt-BR» …) или «auto» — как в системе.
/// Хранится строкой; прежние значения настроек «Ru»/«En» читаются как «ru»/«en».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locale(&'static str);

impl Default for Locale {
    fn default() -> Self {
        Locale::AUTO
    }
}

impl Locale {
    /// Как в системе.
    pub const AUTO: Locale = Locale("auto");

    /// Язык по тегу (тегов конечное число — строка закрепляется один раз).
    pub fn new(tag: &str) -> Self {
        use std::collections::HashSet;
        use std::sync::{Mutex, OnceLock};
        let tag = match tag.trim() {
            "" | "auto" => return Locale::AUTO,
            "Ru" => "ru",
            "En" => "en",
            t => t,
        };
        static TAGS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
        let mut set = TAGS.get_or_init(Default::default).lock().unwrap();
        if let Some(t) = set.get(tag) {
            return Locale(t);
        }
        let t: &'static str = Box::leak(tag.to_string().into_boxed_str());
        set.insert(t);
        Locale(t)
    }

    pub fn tag(self) -> &'static str {
        self.0
    }

    pub fn is_auto(self) -> bool {
        self.0 == "auto"
    }

    /// Язык для каталогов: выбранный или язык системы.
    pub fn resolve(self) -> syngui::i18n::Lang {
        if self.is_auto() {
            syngui::i18n::system_language()
        } else {
            syngui::i18n::Lang::new(self.0)
        }
    }
}

impl Serialize for Locale {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Locale {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Locale::new(&String::deserialize(d)?))
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
            locale: Locale::AUTO,
        }
    }
}
