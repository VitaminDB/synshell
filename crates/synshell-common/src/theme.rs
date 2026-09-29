//! Темы оформления: палитра, обои, свой MSS для оболочки и настроек.
//!
//! Тема — каталог `<id>/`:
//! - `theme.toml` — название, палитры вариантов `[dark]`/`[light]`,
//!   градиент обоев, свои MSS-переменные и рекомендуемые значения
//!   `[appearance]`/`[decorations]`/`[panel]` (их записывают «Параметры системы»
//!   при выборе темы);
//! - `shell.mss` — правила поверх встроенного стиля оболочки (необязательно);
//! - `settings.mss` — то же для «Параметров системы» (необязательно).
//!
//! Где ищутся (первая найденная побеждает): `~/.config/synshell/themes/`,
//! `$XDG_DATA_HOME|$XDG_DATA_DIRS/synshell/themes/`, затем встроенные.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::paths;

/// Вариант темы — палитра одной схемы (тёмной или светлой).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Variant {
    pub bg: Option<String>,
    pub surface: Option<String>,
    pub surface_alt: Option<String>,
    pub fg: Option<String>,
    pub muted: Option<String>,
    pub border: Option<String>,
    pub accent: Option<String>,
    pub accent_fg: Option<String>,
    pub danger: Option<String>,
    pub success: Option<String>,
    pub warning: Option<String>,
    /// Фон рабочего стола без картинки — любое значение MSS `background`
    /// (`linear-gradient(...)`, `radial-gradient(...)`, `#rrggbb`).
    pub wallpaper: Option<String>,
    /// Сплошной цвет под обоями (очистка кадра композитором, превью).
    pub wallpaper_color: Option<String>,
    /// Цвет заголовка активного/неактивного окна: имя цвета палитры
    /// (`accent`, `surface`, `surface_alt`, `bg`) или `#rrggbb`.
    pub titlebar: Option<String>,
    pub titlebar_inactive: Option<String>,
    /// Цвет теней меню и уведомлений (`--shadow`) и затемнения (`--scrim`).
    pub shadow: Option<String>,
    pub scrim: Option<String>,
    /// Свои MSS-переменные: `glow = "#88c0d066"` → `--glow: #88c0d066;`.
    /// Могут переопределять и встроенные (`--panel-bg`, `--radius`, …).
    pub vars: BTreeMap<String, String>,
}

impl Variant {
    /// Цвет палитры по имени ключа `[appearance.colors]`.
    pub fn color(&self, key: &str) -> Option<&str> {
        match key {
            "bg" => self.bg.as_deref(),
            "surface" => self.surface.as_deref(),
            "surface_alt" | "surface-alt" => self.surface_alt.as_deref(),
            "fg" => self.fg.as_deref(),
            "muted" => self.muted.as_deref(),
            "border" => self.border.as_deref(),
            "accent" => self.accent.as_deref(),
            "accent_fg" | "accent-fg" => self.accent_fg.as_deref(),
            "danger" => self.danger.as_deref(),
            "success" => self.success.as_deref(),
            "warning" => self.warning.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeFile {
    name: String,
    description: String,
    author: String,
    dark: Option<Variant>,
    light: Option<Variant>,
    appearance: toml::Table,
    decorations: toml::Table,
    panel: toml::Table,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Dir(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub source: Source,
    pub dark: Option<Variant>,
    pub light: Option<Variant>,
    /// Рекомендуемые значения `[appearance]` (только ключи из
    /// [`APPEARANCE_KEYS`]).
    pub appearance: toml::Table,
    /// Рекомендуемые значения `[decorations]` (ключи [`DECORATION_KEYS`]).
    pub decorations: toml::Table,
    /// Рекомендуемые значения панелей `[[panel]]` (ключи [`PANEL_KEYS`]),
    /// записываются во все панели, кроме доков.
    pub panel: toml::Table,
    pub shell_mss: String,
    pub settings_mss: String,
    /// Свой MSS для проводника (`files.mss`, необязательный).
    pub files_mss: String,
}

/// Какие ключи `[appearance]` тема может рекомендовать.
pub const APPEARANCE_KEYS: &[&str] = &["corner_radius", "panel_opacity", "font", "font_size", "icon_theme", "cursor_theme"];
/// Какие ключи панелей тема может рекомендовать: толщина плавающей и
/// прилипшей к краю панели.
pub const PANEL_KEYS: &[&str] = &["size", "defloated_size"];
/// Какие ключи `[decorations]` тема может рекомендовать.
pub const DECORATION_KEYS: &[&str] =
    &["title_height", "font_size", "title_align", "buttons", "corner_radius", "shadow", "shadow_size", "shadow_opacity"];

impl Theme {
    fn from_parts(id: &str, source: Source, toml_text: &str, shell_mss: String, settings_mss: String) -> Result<Theme, String> {
        let f: ThemeFile = toml::from_str(toml_text).map_err(|e| format!("тема {id}: {e}"))?;
        if f.dark.is_none() && f.light.is_none() {
            return Err(format!("тема {id}: нет ни [dark], ни [light]"));
        }
        let keep = |t: toml::Table, keys: &[&str]| -> toml::Table {
            t.into_iter().filter(|(k, _)| keys.contains(&k.as_str())).collect()
        };
        Ok(Theme {
            id: id.to_string(),
            name: if f.name.is_empty() { id.to_string() } else { f.name },
            description: f.description,
            author: f.author,
            source,
            dark: f.dark,
            light: f.light,
            appearance: keep(f.appearance, APPEARANCE_KEYS),
            decorations: keep(f.decorations, DECORATION_KEYS),
            panel: keep(f.panel, PANEL_KEYS),
            shell_mss,
            settings_mss,
            files_mss: String::new(),
        })
    }

    fn load_dir(id: &str, dir: &Path) -> Result<Theme, String> {
        let toml_text = std::fs::read_to_string(dir.join("theme.toml")).map_err(|e| format!("{}: {e}", dir.display()))?;
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let mut t = Self::from_parts(id, Source::Dir(dir.to_path_buf()), &toml_text, read("shell.mss"), read("settings.mss"))?;
        t.files_mss = read("files.mss");
        Ok(t)
    }

    fn builtin(id: &str) -> Option<Result<Theme, String>> {
        BUILTIN
            .iter()
            .find(|b| b.0 == id)
            .map(|(id, t, shell, settings)| Self::from_parts(id, Source::Builtin, t, shell.to_string(), settings.to_string()))
    }

    /// Найти тему по идентификатору (имени каталога).
    pub fn find(id: &str) -> Result<Theme, String> {
        let id = id.trim();
        if id.is_empty() || id.contains('/') || id.starts_with('.') {
            return Err(format!("неверное имя темы «{id}»"));
        }
        for dir in search_dirs() {
            let d = dir.join(id);
            if d.join("theme.toml").is_file() {
                return Self::load_dir(id, &d);
            }
        }
        Self::builtin(id).unwrap_or_else(|| Err(format!("тема «{id}» не найдена")))
    }

    /// Вариант для схемы: запрошенный, иначе единственный имеющийся.
    pub fn variant(&self, dark: bool) -> (&Variant, bool) {
        match (dark, &self.dark, &self.light) {
            (true, Some(v), _) => (v, true),
            (false, _, Some(v)) => (v, false),
            (_, Some(v), None) => (v, true),
            (_, None, Some(v)) => (v, false),
            (_, None, None) => unreachable!("тема без вариантов отсеивается при загрузке"),
        }
    }

    pub fn has_both_variants(&self) -> bool {
        self.dark.is_some() && self.light.is_some()
    }

    /// Файлы темы на диске — за ними следят композитор и оболочка.
    pub fn files(&self) -> Vec<PathBuf> {
        match &self.source {
            Source::Builtin => Vec::new(),
            Source::Dir(d) => ["theme.toml", "shell.mss", "settings.mss", "files.mss"].iter().map(|f| d.join(f)).collect(),
        }
    }
}

/// Каталоги с пользовательскими и системными темами (по приоритету).
pub fn search_dirs() -> Vec<PathBuf> {
    let mut v = vec![user_dir()];
    v.extend(paths::data_dirs().into_iter().map(|d| d.join("synshell/themes")));
    v
}

/// `~/.config/synshell/themes`.
pub fn user_dir() -> PathBuf {
    paths::config_dir().join("themes")
}

/// Все доступные темы: встроенные и из каталогов (одноимённая тема из
/// каталога заменяет встроенную). Порядок — встроенные как объявлены,
/// затем остальные по имени. Сломанные темы пропускаются с предупреждением.
pub fn list() -> Vec<Theme> {
    let mut ids: Vec<String> = BUILTIN.iter().map(|b| b.0.to_string()).collect();
    let mut extra = Vec::new();
    for dir in search_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with('.') && e.path().join("theme.toml").is_file() && !ids.contains(&name) && !extra.contains(&name) {
                extra.push(name);
            }
        }
    }
    extra.sort();
    ids.extend(extra);
    ids.iter()
        .filter_map(|id| match Theme::find(id) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        })
        .collect()
}

macro_rules! builtin {
    ($($id:literal),* $(,)?) => {
        &[$((
            $id,
            include_str!(concat!("../themes/", $id, "/theme.toml")),
            include_str!(concat!("../themes/", $id, "/shell.mss")),
            include_str!(concat!("../themes/", $id, "/settings.mss")),
        )),*]
    };
}

/// Встроенные темы: (id, theme.toml, shell.mss, settings.mss).
const BUILTIN: &[(&str, &str, &str, &str)] = builtin![
    "nord",
    "catppuccin",
    "tokyo-night",
    "gruvbox",
    "rose-pine",
    "everforest",
    "dracula",
    "kanagawa",
    "solarized",
    "synthwave",
    "aurora",
    "sakura",
    "brutal",
    "terminal",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rgba;

    #[test]
    fn builtin_themes_parse() {
        for (id, ..) in BUILTIN {
            let t = Theme::builtin(id).unwrap().unwrap_or_else(|e| panic!("{e}"));
            for v in [&t.dark, &t.light].into_iter().flatten() {
                for key in ["bg", "surface", "surface_alt", "fg", "muted", "border", "accent", "danger", "success", "warning"] {
                    let c = v.color(key).unwrap_or_else(|| panic!("{id}: нет цвета {key}"));
                    assert!(Rgba::parse(c).is_some(), "{id}: {key} = {c}");
                }
                assert!(v.wallpaper.is_some(), "{id}: нет обоев");
                for c in [&v.wallpaper_color, &v.accent_fg].into_iter().flatten() {
                    assert!(Rgba::parse(c).is_some(), "{id}: {c}");
                }
            }
            assert!(!t.description.is_empty(), "{id}: нет описания");
        }
    }

    #[test]
    fn variant_fallback() {
        let t = Theme::builtin("dracula").unwrap().unwrap();
        assert!(t.light.is_none());
        let (_, dark) = t.variant(false);
        assert!(dark, "у тёмной темы светлый вариант берётся из тёмного");
    }

    #[test]
    fn unknown_keys_dropped() {
        let t = Theme::from_parts(
            "x",
            Source::Builtin,
            "[dark]\nbg = \"#000000\"\n[appearance]\ncorner_radius = 4.0\nui_scale = 3.0\n",
            String::new(),
            String::new(),
        )
        .unwrap();
        assert!(t.appearance.contains_key("corner_radius"));
        assert!(!t.appearance.contains_key("ui_scale"));
        assert!(Theme::find("../etc").is_err());
    }
}
