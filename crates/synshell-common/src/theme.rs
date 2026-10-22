//! Темы оформления: палитра, обои, свой MSS для оболочки и настроек.
//!
//! Тема — каталог `<id>/`:
//! - `theme.toml` — название, палитры вариантов `[dark]`/`[light]`,
//!   градиент обоев, свои MSS-переменные и рекомендуемые значения
//!   `[appearance]`/`[decorations]`/`[panel]` (их записывают «Параметры системы»
//!   при выборе темы);
//! - `shell.mss` — правила поверх встроенного стиля оболочки (необязательно);
//! - `settings.mss` — то же для «Параметров системы» (необязательно);
//! - `files.mss` — проводник и терминал (необязательно);
//! - `apps/<программа>.mss` — слой любой другой программы (`synpkg.mss`,
//!   `syn-remote.mss`, …), его подключает [`crate::style::AppStyle`].
//!
//! Где ищутся (первая найденная побеждает): `~/.config/synshell/themes/`,
//! `$XDG_DATA_HOME|$XDG_DATA_DIRS/synshell/themes/`, затем встроенные.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;
use crate::t;

/// Вариант темы — палитра одной схемы (тёмной или светлой).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Variant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_fg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub danger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Фон рабочего стола без картинки — любое значение MSS `background`
    /// (`linear-gradient(...)`, `radial-gradient(...)`, `#rrggbb`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
    /// Сплошной цвет под обоями (очистка кадра композитором, превью).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper_color: Option<String>,
    /// Цвет заголовка активного/неактивного окна: имя цвета палитры
    /// (`accent`, `surface`, `surface_alt`, `bg`) или `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titlebar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titlebar_inactive: Option<String>,
    /// Цвет теней меню и уведомлений (`--shadow`) и затемнения (`--scrim`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrim: Option<String>,
    /// Свои MSS-переменные: `glow = "#88c0d066"` → `--glow: #88c0d066;`.
    /// Могут переопределять и встроенные (`--panel-bg`, `--radius`, …).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
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
    /// Слои MSS по программам: `shell` (`shell.mss`), `settings`, `files`
    /// и `apps/<id>.mss` → `<id>`.
    pub mss: BTreeMap<String, String>,
    /// Исходный текст `theme.toml` (копия темы в редакторе сохраняет комментарии).
    pub toml_text: String,
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
    fn from_parts(id: &str, source: Source, toml_text: &str, mss: BTreeMap<String, String>) -> Result<Theme, String> {
        let f: ThemeFile = toml::from_str(toml_text).map_err(|e| t!("тема {id}: {e}", id = id, e = e))?;
        if f.dark.is_none() && f.light.is_none() {
            return Err(t!("тема {id}: нет ни [dark], ни [light]", id = id));
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
            mss: mss.into_iter().filter(|(_, v)| !v.trim().is_empty()).collect(),
            toml_text: toml_text.to_string(),
        })
    }

    /// Загрузить тему из каталога `dir` (с `theme.toml`) под именем `id`.
    pub fn load_dir(id: &str, dir: &Path) -> Result<Theme, String> {
        let toml_text = std::fs::read_to_string(dir.join("theme.toml")).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut mss = BTreeMap::new();
        for (sub, prefix) in [(dir.to_path_buf(), ""), (dir.join("apps"), "apps/")] {
            let Ok(rd) = std::fs::read_dir(&sub) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "mss") {
                    let name = format!("{prefix}{}", e.file_name().to_string_lossy());
                    mss.insert(layer_of(&name), std::fs::read_to_string(&p).unwrap_or_default());
                }
            }
        }
        Self::from_parts(id, Source::Dir(dir.to_path_buf()), &toml_text, mss)
    }

    /// Тема из файлов в памяти (`theme.toml`, `shell.mss`, `apps/x.mss`) —
    /// черновик редактора тем.
    pub fn from_files(id: &str, source: Source, files: &[(String, String)]) -> Result<Theme, String> {
        let toml_text = files.iter().find(|(n, _)| n == "theme.toml").map(|(_, t)| t.as_str()).unwrap_or("");
        let mss = files.iter().filter(|(n, _)| n.ends_with(".mss")).map(|(n, t)| (layer_of(n), t.clone())).collect();
        Self::from_parts(id, source, toml_text, mss)
    }

    fn builtin(id: &str) -> Option<Result<Theme, String>> {
        BUILTIN.iter().find(|b| b.0 == id).map(|(id, t, files)| {
            let mss = files.iter().map(|(name, text)| (layer_of(name), text.to_string())).collect();
            Self::from_parts(id, Source::Builtin, t, mss)
        })
    }

    /// Файлы темы как есть: `theme.toml`, `shell.mss`, `apps/x.mss`, … —
    /// для копии в редакторе тем.
    pub fn source_files(&self) -> Vec<(String, String)> {
        let mut v = vec![("theme.toml".to_string(), self.toml_text.clone())];
        v.extend(self.mss.iter().map(|(layer, text)| (file_of_layer(layer), text.clone())));
        v
    }

    /// Слой MSS темы для программы `app` (`shell`, `settings`, `files`,
    /// `synpkg`, …); пусто, если его нет.
    pub fn app_mss(&self, app: &str) -> &str {
        self.mss.get(app).map(String::as_str).unwrap_or("")
    }

    /// Найти тему по идентификатору (имени каталога).
    pub fn find(id: &str) -> Result<Theme, String> {
        let id = id.trim();
        if id.is_empty() || id.contains('/') || id.starts_with('.') {
            return Err(t!("неверное имя темы «{id}»", id = id));
        }
        for dir in search_dirs() {
            let d = dir.join(id);
            if d.join("theme.toml").is_file() {
                return Self::load_dir(id, &d);
            }
        }
        Self::builtin(id).unwrap_or_else(|| Err(t!("тема «{id}» не найдена", id = id)))
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
            Source::Dir(d) => {
                let mut v: Vec<PathBuf> = ["theme.toml", "shell.mss", "settings.mss", "files.mss"].iter().map(|f| d.join(f)).collect();
                v.extend(self.mss.keys().map(|l| d.join(file_of_layer(l))));
                v.sort();
                v.dedup();
                v
            }
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

/// Имя слоя по имени файла темы: `shell.mss` → `shell`, `apps/synpkg.mss` → `synpkg`.
pub fn layer_of(file: &str) -> String {
    let f = file.strip_prefix("apps/").unwrap_or(file);
    f.strip_suffix(".mss").unwrap_or(f).to_string()
}

/// Имя файла слоя: свои слои оболочки, настроек и проводника — в корне темы,
/// остальные программы — в `apps/`.
pub fn file_of_layer(layer: &str) -> String {
    match layer {
        "shell" | "settings" | "files" => format!("{layer}.mss"),
        other => format!("apps/{other}.mss"),
    }
}

/// Встроенные темы: (id, theme.toml, [(файл, текст)]) — собирает build.rs.
const BUILTIN: &[(&str, &str, &[(&str, &str)])] = include!(concat!(env!("OUT_DIR"), "/builtin_themes.rs"));

/// Идентификаторы встроенных тем в порядке галереи.
pub fn builtin_ids() -> impl Iterator<Item = &'static str> {
    BUILTIN.iter().map(|b| b.0)
}

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
    fn builtin_order_and_layers() {
        let ids: Vec<_> = builtin_ids().collect();
        assert_eq!(ids.first(), Some(&"nord"));
        assert_eq!(ids.len(), 14);
        let t = Theme::builtin("tokyo-night").unwrap().unwrap();
        assert!(t.app_mss("shell").contains(".ws-active"));
        assert!(t.app_mss("settings").contains(".nav-item"));
        assert_eq!(layer_of("apps/synpkg.mss"), "synpkg");
        assert_eq!(file_of_layer("synpkg"), "apps/synpkg.mss");
        assert!(t.source_files().iter().any(|(f, _)| f == "shell.mss"));
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
            BTreeMap::new(),
        )
        .unwrap();
        assert!(t.appearance.contains_key("corner_radius"));
        assert!(!t.appearance.contains_key("ui_scale"));
        assert!(Theme::find("../etc").is_err());
    }
}
