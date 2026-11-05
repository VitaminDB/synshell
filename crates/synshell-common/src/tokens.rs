//! Реестр токенов оформления — MSS-переменных, общих для оболочки и программ.
//!
//! Один источник для:
//! - `:root`-переменных палитры ([`crate::config::Appearance::mss_variables`]);
//! - редактора тем (группы, подписи, тип значения, откуда берётся);
//! - языкового сервиса MSS (автодополнение `var(--…)`, подсказки) —
//!   контекст собирает редактор тем (`syn-themeeditor --mss-context`).
//!
//! Подписи и описания помечены `n_!` и переводятся при показе (`t(label)`).

use crate::config::{Appearance, Palette};
use crate::n_;

/// Тип значения токена.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Color,
    /// Длина в px.
    Length,
    /// Любое значение MSS `background` (цвет или градиент).
    Background,
}

/// Группа в редакторе тем.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Surfaces,
    Text,
    Accent,
    Status,
    States,
    Shape,
    Windows,
    Apps,
}

impl Group {
    pub const ALL: [Group; 8] =
        [Group::Surfaces, Group::Text, Group::Accent, Group::Status, Group::States, Group::Shape, Group::Windows, Group::Apps];

    pub fn label(self) -> &'static str {
        match self {
            Group::Surfaces => n_!("Фоны и поверхности"),
            Group::Text => n_!("Текст и рамки"),
            Group::Accent => n_!("Акцент"),
            Group::Status => n_!("Состояния: ошибка, успех, внимание"),
            Group::States => n_!("Наведение и нажатие"),
            Group::Shape => n_!("Форма и шрифт"),
            Group::Windows => n_!("Окна, тени, обои"),
            Group::Apps => n_!("Программы"),
        }
    }
}

/// Откуда берётся значение.
#[derive(Clone, Copy)]
pub enum Source {
    /// Цвет палитры варианта темы (`[dark] bg = …`) — ключ в `theme.toml`.
    Palette(&'static str),
    /// Поле варианта темы, не входящее в палитру (`shadow`, `wallpaper`, …).
    Variant(&'static str),
    /// Вычисляется из палитры и настроек; тема может перекрыть в `[dark.vars]`.
    Derived(fn(&Ctx) -> String),
    /// Задаёт каждая программа по-своему; тема может перекрыть в `vars`.
    App,
}

/// Данные для вычисления производных токенов.
pub struct Ctx<'a> {
    pub a: &'a Appearance,
    pub p: Palette,
    /// Непрозрачность панелей (0..1).
    pub opacity: f32,
    /// Скругление (px).
    pub radius: f32,
}

impl<'a> Ctx<'a> {
    pub fn new(a: &'a Appearance) -> Self {
        Self { a, p: a.palette(), opacity: a.panel_opacity.clamp(0.0, 1.0), radius: a.corner_radius.max(0.0) }
    }
}

pub struct TokenSpec {
    /// Имя переменной без `--`.
    pub name: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub label: &'static str,
    pub doc: &'static str,
    pub source: Source,
}

impl TokenSpec {
    /// Значение для `:root` у токенов палитры и производных; у токенов
    /// программ и полей варианта — `None`.
    pub fn value(&self, c: &Ctx) -> Option<String> {
        match self.source {
            Source::Palette(k) => palette_color(&c.p, k).map(|x| x.hex()),
            Source::Derived(f) => Some(f(c)),
            Source::Variant(_) | Source::App => None,
        }
    }
}

fn palette_color(p: &Palette, key: &str) -> Option<crate::config::Rgba> {
    Some(match key {
        "bg" => p.bg,
        "surface" => p.surface,
        "surface_alt" => p.surface_alt,
        "fg" => p.fg,
        "muted" => p.muted,
        "border" => p.border,
        "accent" => p.accent,
        "accent_fg" => p.accent_fg,
        "danger" => p.danger,
        "success" => p.success,
        "warning" => p.warning,
        _ => return None,
    })
}

macro_rules! tok {
    ($name:literal, $group:ident, $kind:ident, $label:expr, $doc:expr, $src:expr) => {
        TokenSpec { name: $name, group: Group::$group, kind: Kind::$kind, label: $label, doc: $doc, source: $src }
    };
}

/// Все токены. Порядок токенов палитры и производных — порядок в `:root`.
pub const TOKENS: &[TokenSpec] = &[
    tok!("bg", Surfaces, Color, n_!("Фон"), n_!("Самый нижний фон: окна, рабочий стол без обоев, панели."), Source::Palette("bg")),
    tok!("surface", Surfaces, Color, n_!("Поверхность"), n_!("Карточки, меню, всплывающие окна."), Source::Palette("surface")),
    tok!("surface-alt", Surfaces, Color, n_!("Поверхность 2"), n_!("Поля ввода, вложенные карточки, полосы."), Source::Palette("surface_alt")),
    tok!("panel-bg", Surfaces, Color, n_!("Фон панели"), n_!("Фон панели и дока: «Фон» с непрозрачностью панелей."), Source::Derived(|c| c.p.bg.with_alpha(c.opacity).hex())),
    tok!("menu-bg", Surfaces, Color, n_!("Фон меню"), n_!("Фон меню и всплывающих окон: «Поверхность», почти непрозрачная."), Source::Derived(|c| c.p.surface.with_alpha(c.opacity.max(0.96)).hex())),
    tok!("fg", Text, Color, n_!("Текст"), n_!("Основной цвет текста и значков."), Source::Palette("fg")),
    tok!("muted", Text, Color, n_!("Приглушённый текст"), n_!("Подписи, подсказки, второстепенный текст."), Source::Palette("muted")),
    tok!("border", Text, Color, n_!("Рамка"), n_!("Рамки и разделители."), Source::Palette("border")),
    tok!("accent", Accent, Color, n_!("Акцент"), n_!("Выделение, активные элементы, основные кнопки."), Source::Palette("accent")),
    tok!("accent-fg", Accent, Color, n_!("Текст на акценте"), n_!("Текст и значки поверх акцента (по умолчанию — контрастный)."), Source::Palette("accent_fg")),
    tok!("accent-soft", Accent, Color, n_!("Мягкий акцент"), n_!("Подложка выбранного пункта: акцент с прозрачностью 22%."), Source::Derived(|c| c.p.accent.with_alpha(0.22).hex())),
    tok!("hover", States, Color, n_!("Наведение"), n_!("Подсветка под указателем: текст с прозрачностью 8%."), Source::Derived(|c| c.p.fg.with_alpha(0.08).hex())),
    tok!("pressed", States, Color, n_!("Нажатие"), n_!("Подсветка нажатого: текст с прозрачностью 14%."), Source::Derived(|c| c.p.fg.with_alpha(0.14).hex())),
    tok!("danger", Status, Color, n_!("Ошибка"), n_!("Ошибки, удаление, опасные действия."), Source::Palette("danger")),
    tok!("danger-soft", Status, Color, n_!("Мягкая ошибка"), n_!("Подложка ошибки: прозрачность 18%."), Source::Derived(|c| c.p.danger.with_alpha(0.18).hex())),
    tok!("success", Status, Color, n_!("Успех"), n_!("Готово, подключено, заряд."), Source::Palette("success")),
    tok!("warning", Status, Color, n_!("Внимание"), n_!("Предупреждения."), Source::Palette("warning")),
    tok!("radius", Shape, Length, n_!("Скругление"), n_!("Скругление панелей, меню и карточек («Скругление углов»)."), Source::Derived(|c| format!("{}px", c.radius))),
    tok!("radius-sm", Shape, Length, n_!("Малое скругление"), n_!("Скругление кнопок и полей: 60% от «Скругления»."), Source::Derived(|c| format!("{}px", (c.radius * 0.6).round()))),
    tok!("font-size", Shape, Length, n_!("Размер шрифта"), n_!("Основной размер текста интерфейса."), Source::Derived(|c| format!("{}px", c.a.font_size))),
    // Поля варианта темы.
    tok!("shadow", Windows, Color, n_!("Тень"), n_!("Тень меню, уведомлений и окон."), Source::Variant("shadow")),
    tok!("scrim", Windows, Color, n_!("Затемнение"), n_!("Затемнение под модальными окнами и шторкой."), Source::Variant("scrim")),
    // Токены программ: каждая задаёт свои значения, тема может перекрыть.
    tok!("titlebar", Apps, Color, n_!("Заголовок окна программы"), n_!("Полоса заголовка в программах с собственным заголовком."), Source::App),
    tok!("sidebar", Apps, Color, n_!("Боковая панель"), n_!("Боковая панель программ (Ассистент, Пульт, Пакеты…)."), Source::App),
    tok!("sidebar-bg", Apps, Color, n_!("Боковая панель настроек"), n_!("Боковая панель «Параметров системы»."), Source::App),
    tok!("content", Apps, Color, n_!("Область содержимого"), n_!("Фон основной области программ."), Source::App),
    tok!("card", Apps, Color, n_!("Карточка"), n_!("Фон карточек в программах."), Source::App),
    tok!("card-bg", Apps, Color, n_!("Фон карточки настроек"), n_!("Карточки «Параметров системы» и Проводника."), Source::App),
    tok!("input-bg", Apps, Color, n_!("Поле ввода"), n_!("Фон полей ввода."), Source::App),
    tok!("field", Apps, Color, n_!("Поле (Ассистент)"), n_!("Фон поля ввода сообщения."), Source::App),
    tok!("divider", Apps, Color, n_!("Разделитель"), n_!("Тонкие разделители списков."), Source::App),
    tok!("accent-2", Apps, Color, n_!("Второй акцент"), n_!("Второй цвет градиентов в программах."), Source::App),
    tok!("accent-hover", Apps, Color, n_!("Акцент под указателем"), n_!("Основная кнопка под указателем."), Source::App),
    tok!("accent-glow", Apps, Color, n_!("Свечение акцента"), n_!("Свечение вокруг активных элементов."), Source::App),
    tok!("accent-dim", Apps, Color, n_!("Тусклый акцент"), n_!("Слабая подложка акцента."), Source::App),
    tok!("window-radius", Apps, Length, n_!("Скругление окна"), n_!("Скругление углов окна программы (из «Декораций»)."), Source::App),
];

/// Найти токен по имени (с `--` или без).
pub fn find(name: &str) -> Option<&'static TokenSpec> {
    let n = name.trim_start_matches('-');
    TOKENS.iter().find(|t| t.name == n)
}

/// `:root { … }` из токенов палитры и производных.
pub fn root_block(a: &Appearance) -> String {
    let c = Ctx::new(a);
    let mut s = String::from(":root {\n");
    for t in TOKENS {
        if let Some(v) = t.value(&c) {
            s.push_str(&format!("  --{}: {v};\n", t.name));
        }
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_unique() {
        let mut names: Vec<_> = TOKENS.iter().map(|t| t.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }

    // Прежняя сборка `:root` (до реестра) — вывод не должен измениться.
    fn old_mss_variables(ap: &Appearance) -> String {
        let p = ap.palette();
        let a = ap.panel_opacity.clamp(0.0, 1.0);
        let r = ap.corner_radius.max(0.0);
        format!(
            ":root {{\n  --bg: {};\n  --surface: {};\n  --surface-alt: {};\n  --panel-bg: {};\n  --menu-bg: {};\n  --fg: {};\n  --muted: {};\n  --border: {};\n  --accent: {};\n  --accent-fg: {};\n  --accent-soft: {};\n  --hover: {};\n  --pressed: {};\n  --danger: {};\n  --danger-soft: {};\n  --success: {};\n  --warning: {};\n  --radius: {r}px;\n  --radius-sm: {}px;\n  --font-size: {}px;\n}}\n",
            p.bg.hex(),
            p.surface.hex(),
            p.surface_alt.hex(),
            p.bg.with_alpha(a).hex(),
            p.surface.with_alpha(a.max(0.96)).hex(),
            p.fg.hex(),
            p.muted.hex(),
            p.border.hex(),
            p.accent.hex(),
            p.accent_fg.hex(),
            p.accent.with_alpha(0.22).hex(),
            p.fg.with_alpha(0.08).hex(),
            p.fg.with_alpha(0.14).hex(),
            p.danger.hex(),
            p.danger.with_alpha(0.18).hex(),
            p.success.hex(),
            p.warning.hex(),
            (r * 0.6).round(),
            ap.font_size,
        )
    }

    #[test]
    fn root_block_matches_old() {
        let mut a = Appearance::default();
        assert_eq!(root_block(&a), old_mss_variables(&a));
        a.color_scheme = crate::config::ColorScheme::Light;
        a.panel_opacity = 0.5;
        a.corner_radius = 7.0;
        a.accent = "#ff8800".into();
        assert_eq!(root_block(&a), old_mss_variables(&a));
        a.theme = "tokyo-night".into();
        a.load_theme();
        assert_eq!(root_block(&a), old_mss_variables(&a));
    }

    #[test]
    fn find_works() {
        assert_eq!(find("--accent").map(|t| t.name), Some("accent"));
        assert!(find("nope").is_none());
    }
}
