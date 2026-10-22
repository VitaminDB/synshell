//! Реестр токенов оформления — MSS-переменных, общих для оболочки и программ.
//!
//! Один источник для:
//! - `:root`-переменных палитры ([`crate::config::Appearance::mss_variables`]);
//! - редактора тем (группы, подписи, тип значения, откуда берётся);
//! - языкового сервиса MSS (автодополнение `var(--…)`, подсказки) —
//!   [`mss_context_json`].
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
    ($name:literal, $group:ident, $kind:ident, $label:literal, $doc:literal, $src:expr) => {
        TokenSpec { name: $name, group: Group::$group, kind: Kind::$kind, label: n_!($label), doc: n_!($doc), source: $src }
    };
}

/// Все токены. Порядок токенов палитры и производных — порядок в `:root`.
pub const TOKENS: &[TokenSpec] = &[
    tok!("bg", Surfaces, Color, "Фон", "Самый нижний фон: окна, рабочий стол без обоев, панели.", Source::Palette("bg")),
    tok!("surface", Surfaces, Color, "Поверхность", "Карточки, меню, всплывающие окна.", Source::Palette("surface")),
    tok!("surface-alt", Surfaces, Color, "Поверхность 2", "Поля ввода, вложенные карточки, полосы.", Source::Palette("surface_alt")),
    tok!("panel-bg", Surfaces, Color, "Фон панели", "Фон панели и дока: «Фон» с непрозрачностью панелей.", Source::Derived(|c| c.p.bg.with_alpha(c.opacity).hex())),
    tok!("menu-bg", Surfaces, Color, "Фон меню", "Фон меню и всплывающих окон: «Поверхность», почти непрозрачная.", Source::Derived(|c| c.p.surface.with_alpha(c.opacity.max(0.96)).hex())),
    tok!("fg", Text, Color, "Текст", "Основной цвет текста и значков.", Source::Palette("fg")),
    tok!("muted", Text, Color, "Приглушённый текст", "Подписи, подсказки, второстепенный текст.", Source::Palette("muted")),
    tok!("border", Text, Color, "Рамка", "Рамки и разделители.", Source::Palette("border")),
    tok!("accent", Accent, Color, "Акцент", "Выделение, активные элементы, основные кнопки.", Source::Palette("accent")),
    tok!("accent-fg", Accent, Color, "Текст на акценте", "Текст и значки поверх акцента (по умолчанию — контрастный).", Source::Palette("accent_fg")),
    tok!("accent-soft", Accent, Color, "Мягкий акцент", "Подложка выбранного пункта: акцент с прозрачностью 22%.", Source::Derived(|c| c.p.accent.with_alpha(0.22).hex())),
    tok!("hover", States, Color, "Наведение", "Подсветка под указателем: текст с прозрачностью 8%.", Source::Derived(|c| c.p.fg.with_alpha(0.08).hex())),
    tok!("pressed", States, Color, "Нажатие", "Подсветка нажатого: текст с прозрачностью 14%.", Source::Derived(|c| c.p.fg.with_alpha(0.14).hex())),
    tok!("danger", Status, Color, "Ошибка", "Ошибки, удаление, опасные действия.", Source::Palette("danger")),
    tok!("danger-soft", Status, Color, "Мягкая ошибка", "Подложка ошибки: прозрачность 18%.", Source::Derived(|c| c.p.danger.with_alpha(0.18).hex())),
    tok!("success", Status, Color, "Успех", "Готово, подключено, заряд.", Source::Palette("success")),
    tok!("warning", Status, Color, "Внимание", "Предупреждения.", Source::Palette("warning")),
    tok!("radius", Shape, Length, "Скругление", "Скругление панелей, меню и карточек («Скругление углов»).", Source::Derived(|c| format!("{}px", c.radius))),
    tok!("radius-sm", Shape, Length, "Малое скругление", "Скругление кнопок и полей: 60% от «Скругления».", Source::Derived(|c| format!("{}px", (c.radius * 0.6).round()))),
    tok!("font-size", Shape, Length, "Размер шрифта", "Основной размер текста интерфейса.", Source::Derived(|c| format!("{}px", c.a.font_size))),
    // Поля варианта темы.
    tok!("shadow", Windows, Color, "Тень", "Тень меню, уведомлений и окон.", Source::Variant("shadow")),
    tok!("scrim", Windows, Color, "Затемнение", "Затемнение под модальными окнами и шторкой.", Source::Variant("scrim")),
    // Токены программ: каждая задаёт свои значения, тема может перекрыть.
    tok!("titlebar", Apps, Color, "Заголовок окна программы", "Полоса заголовка в программах с собственным заголовком.", Source::App),
    tok!("sidebar", Apps, Color, "Боковая панель", "Боковая панель программ (Ассистент, Пульт, Пакеты…).", Source::App),
    tok!("sidebar-bg", Apps, Color, "Боковая панель настроек", "Боковая панель «Параметров системы».", Source::App),
    tok!("content", Apps, Color, "Область содержимого", "Фон основной области программ.", Source::App),
    tok!("card", Apps, Color, "Карточка", "Фон карточек в программах.", Source::App),
    tok!("card-bg", Apps, Color, "Фон карточки настроек", "Карточки «Параметров системы» и Проводника.", Source::App),
    tok!("input-bg", Apps, Color, "Поле ввода", "Фон полей ввода.", Source::App),
    tok!("field", Apps, Color, "Поле (Ассистент)", "Фон поля ввода сообщения.", Source::App),
    tok!("divider", Apps, Color, "Разделитель", "Тонкие разделители списков.", Source::App),
    tok!("accent-2", Apps, Color, "Второй акцент", "Второй цвет градиентов в программах.", Source::App),
    tok!("accent-hover", Apps, Color, "Акцент под указателем", "Основная кнопка под указателем.", Source::App),
    tok!("accent-glow", Apps, Color, "Свечение акцента", "Свечение вокруг активных элементов.", Source::App),
    tok!("accent-dim", Apps, Color, "Тусклый акцент", "Слабая подложка акцента.", Source::App),
    tok!("window-radius", Apps, Length, "Скругление окна", "Скругление углов окна программы (из «Декораций»).", Source::App),
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

/// Контекст для языкового сервиса MSS (`.mss-context.json` для `mss-lsp`):
/// переменные с описанием и текущим значением.
pub fn mss_context_json(a: &Appearance) -> serde_json::Value {
    let c = Ctx::new(a);
    let vars: Vec<serde_json::Value> = TOKENS
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": format!("--{}", t.name),
                "value": t.value(&c),
                "doc": format!("{} — {}", synshell_tr::t(t.label), synshell_tr::t(t.doc)),
            })
        })
        .collect();
    serde_json::json!({ "variables": vars })
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
