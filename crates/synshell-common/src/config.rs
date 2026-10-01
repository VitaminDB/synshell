//! Конфигурация рабочего стола: `~/.config/synshell/config.toml`.
//!
//! Все секции необязательны: отсутствующее поле берётся из значений по
//! умолчанию, поэтому пользовательский файл может содержать только то, что
//! отличается. Композитор и оболочка перечитывают файл на лету.

use crate::action::{Action, KeyCombo, LayoutKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::theme::{Theme, Variant};

/// Встроенный конфиг по умолчанию с комментариями — пишется в
/// `~/.config/synshell/config.toml` при первом запуске.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../default-config.toml");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub platform: Platform,
    pub appearance: Appearance,
    pub input: Input,
    #[serde(rename = "output")]
    pub outputs: Vec<OutputConfig>,
    pub windows: Windows,
    pub decorations: Decorations,
    pub animations: Animations,
    pub workspaces: Workspaces,
    #[serde(rename = "rule")]
    pub rules: Vec<WindowRule>,
    /// `"Super+Return" = "spawn konsole"`. Пользовательские сочетания
    /// дополняют встроенные; `"none"` снимает встроенное.
    pub keybindings: BTreeMap<String, Action>,
    pub wallpaper: Wallpaper,
    #[serde(rename = "panel")]
    pub panels: Vec<Panel>,
    pub launcher: Launcher,
    pub files: Files,
    pub notifications: Notifications,
    pub lock: Lock,
    pub idle: Idle,
    pub mobile: Mobile,
    pub gestures: Gestures,
    pub haptics: Haptics,
    pub rotation: ScreenRotation,
    pub brightness: Brightness,
    pub power_button: PowerButton,
    pub osk: Osk,
    pub sound: Sound,
    pub time: Time,
    pub link: Link,
    pub location: Location,
    pub wifi: Wifi,
    pub packages: Packages,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            platform: Platform::default(),
            appearance: Appearance::default(),
            input: Input::default(),
            outputs: Vec::new(),
            windows: Windows::default(),
            decorations: Decorations::default(),
            animations: Animations::default(),
            workspaces: Workspaces::default(),
            rules: Vec::new(),
            keybindings: BTreeMap::new(),
            wallpaper: Wallpaper::default(),
            panels: vec![Panel::default()],
            launcher: Launcher::default(),
            files: Files::default(),
            notifications: Notifications::default(),
            lock: Lock::default(),
            idle: Idle::default(),
            mobile: Mobile::default(),
            gestures: Gestures::default(),
            haptics: Haptics::default(),
            rotation: ScreenRotation::default(),
            brightness: Brightness::default(),
            power_button: PowerButton::default(),
            osk: Osk::default(),
            sound: Sound::default(),
            time: Time::default(),
            link: Link::default(),
            location: Location::default(),
            wifi: Wifi::default(),
            packages: Packages::default(),
        }
    }
}

// ─── general ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Терминал для `Super+Return` и `Terminal=true` в .desktop.
    pub terminal: String,
    /// Файловый менеджер, браузер — для кнопок по умолчанию.
    pub file_manager: String,
    pub browser: String,
    /// Команда оболочки (панели, меню, уведомления). Пустая — без оболочки
    /// (например, если используется waybar).
    pub shell: String,
    /// Команды, запускаемые при старте сеанса.
    pub autostart: Vec<String>,
    /// Запускать ли `~/.config/autostart/*.desktop` (XDG Autostart).
    pub xdg_autostart: bool,
    /// Переменные окружения для всех запускаемых программ.
    pub environment: BTreeMap<String, String>,
    /// Xwayland для X11-программ (через `xwayland-satellite`, если есть).
    pub xwayland: bool,
    /// Каталог снимков экрана.
    pub screenshot_dir: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            terminal: "konsole".into(),
            file_manager: "synfiles".into(),
            browser: "xdg-open https://".into(),
            shell: "syndesktop-shell".into(),
            autostart: Vec::new(),
            xdg_autostart: true,
            environment: BTreeMap::new(),
            xwayland: true,
            screenshot_dir: "~/Pictures/Screenshots".into(),
        }
    }
}

// ─── appearance ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    #[default]
    Dark,
    Light,
}

/// Внешний вид — общий для композитора (рамки, заголовки) и оболочки
/// (переменные MSS `--accent`, `--bg`, … подставляются из этих полей).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    /// Тема оформления (каталог в `themes/` или встроенная). Пусто — стандартная.
    pub theme: String,
    pub color_scheme: ColorScheme,
    /// Цвет акцента: `#rrggbb`. Пусто — акцент темы.
    pub accent: String,
    /// Семейство шрифта интерфейса (пусто — системный по fontconfig).
    pub font: String,
    pub font_size: f32,
    pub icon_theme: String,
    pub cursor_theme: String,
    pub cursor_size: u32,
    /// Когда прятать курсор мыши: `never`, `touch` (при касании экрана;
    /// движение мыши его возвращает), `always` (телефон, планшет без мыши).
    pub cursor_hide: String,
    /// Скругление углов панелей/меню/окон (логические px).
    pub corner_radius: f32,
    /// Непрозрачность фона панелей и меню (0..1).
    pub panel_opacity: f32,
    /// Масштаб интерфейса оболочки (дополнительно к масштабу монитора).
    pub ui_scale: f32,
    /// Переопределения цветов палитры: `bg`, `surface`, `fg`, `muted`,
    /// `border`, `danger`, `success`, `warning`.
    pub colors: BTreeMap<String, String>,
    /// Перекрашивать программы под тему: GTK 3/4 (Breeze, libadwaita),
    /// Qt/KDE (`kdeglobals`) и GIMP 3 (см. `app_theme`).
    pub app_colors: bool,
    /// Загруженная тема `theme` (заполняется в [`Config::parse`]).
    #[serde(skip)]
    pub resolved: Option<Arc<Theme>>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: String::new(),
            color_scheme: ColorScheme::Dark,
            accent: String::new(),
            font: String::new(),
            font_size: 13.0,
            icon_theme: "breeze-dark".into(),
            cursor_theme: "breeze_cursors".into(),
            cursor_size: 24,
            cursor_hide: "touch".into(),
            corner_radius: 10.0,
            panel_opacity: 0.92,
            ui_scale: 1.0,
            colors: BTreeMap::new(),
            app_colors: true,
            resolved: None,
        }
    }
}

/// Ключи цветов палитры (`[appearance.colors]` и вариантов темы).
pub const COLOR_KEYS: &[&str] =
    &["bg", "surface", "surface_alt", "fg", "muted", "border", "accent_fg", "danger", "success", "warning"];

/// Готовая палитра — вычисляется из [`Appearance`].
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub bg: Rgba,
    pub surface: Rgba,
    pub surface_alt: Rgba,
    pub fg: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub accent_fg: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
}

const DEFAULT_ACCENT: Rgba = Rgba::rgb(0x3d, 0x8b, 0xfd);

impl Appearance {
    /// Загрузить тему `theme` в `resolved`. Отсутствующая или сломанная
    /// тема — предупреждение и стандартное оформление.
    pub fn load_theme(&mut self) {
        self.resolved = None;
        if self.theme.trim().is_empty() {
            return;
        }
        match Theme::find(&self.theme) {
            Ok(t) => self.resolved = Some(Arc::new(t)),
            Err(e) => tracing::warn!("{e}"),
        }
    }

    /// Активный вариант темы и тёмный ли он.
    pub fn theme_variant(&self) -> Option<(&Variant, bool)> {
        let t = self.resolved.as_deref()?;
        Some(t.variant(self.color_scheme == ColorScheme::Dark))
    }

    /// Тёмная ли итоговая схема: у темы с одним вариантом — его схема.
    pub fn is_dark(&self) -> bool {
        match self.theme_variant() {
            Some((_, dark)) => dark,
            None => self.color_scheme == ColorScheme::Dark,
        }
    }

    pub fn palette(&self) -> Palette {
        let dark = self.is_dark();
        let variant = self.theme_variant().map(|v| v.0);
        let theme_color = |key: &str| variant.and_then(|v| v.color(key)).and_then(Rgba::parse);
        let accent = Rgba::parse(&self.accent).or_else(|| theme_color("accent")).unwrap_or(DEFAULT_ACCENT);
        let base = if dark {
            Palette {
                bg: Rgba::rgb(0x16, 0x18, 0x1d),
                surface: Rgba::rgb(0x20, 0x23, 0x2a),
                surface_alt: Rgba::rgb(0x2a, 0x2e, 0x37),
                fg: Rgba::rgb(0xe8, 0xea, 0xef),
                muted: Rgba::rgb(0x9a, 0xa0, 0xac),
                border: Rgba::rgb(0x34, 0x38, 0x42),
                accent,
                accent_fg: accent.contrast_fg(),
                danger: Rgba::rgb(0xe5, 0x48, 0x4d),
                success: Rgba::rgb(0x3f, 0xb9, 0x50),
                warning: Rgba::rgb(0xe8, 0xa3, 0x3d),
            }
        } else {
            Palette {
                bg: Rgba::rgb(0xf3, 0xf4, 0xf7),
                surface: Rgba::rgb(0xff, 0xff, 0xff),
                surface_alt: Rgba::rgb(0xe9, 0xeb, 0xf0),
                fg: Rgba::rgb(0x1d, 0x20, 0x26),
                muted: Rgba::rgb(0x62, 0x68, 0x75),
                border: Rgba::rgb(0xd3, 0xd7, 0xdf),
                accent,
                accent_fg: accent.contrast_fg(),
                danger: Rgba::rgb(0xd1, 0x34, 0x38),
                success: Rgba::rgb(0x2d, 0x9a, 0x3e),
                warning: Rgba::rgb(0xc2, 0x7c, 0x0e),
            }
        };
        let mut p = base;
        // Сначала цвета темы, поверх — пользовательские `[appearance.colors]`.
        // Текст на акценте из темы — только для её собственного акцента.
        let theme_colors = COLOR_KEYS
            .iter()
            .filter(|k| **k != "accent_fg" || Rgba::parse(&self.accent).is_none())
            .filter_map(|k| Some((*k, theme_color(k)?)));
        let user_colors = self.colors.iter().filter_map(|(k, v)| Some((k.as_str(), Rgba::parse(v)?)));
        for (k, c) in theme_colors.chain(user_colors) {
            match k {
                "bg" => p.bg = c,
                "surface" => p.surface = c,
                "surface_alt" | "surface-alt" => p.surface_alt = c,
                "fg" => p.fg = c,
                "muted" => p.muted = c,
                "border" => p.border = c,
                "accent_fg" | "accent-fg" => p.accent_fg = c,
                "danger" => p.danger = c,
                "success" => p.success = c,
                "warning" => p.warning = c,
                _ => {}
            }
        }
        p
    }

    /// Цвет по имени из палитры (`accent`, `surface`, `surface_alt`, `bg`,
    /// `fg`, …) или `#rrggbb`.
    pub fn named_color(&self, name: &str, p: &Palette) -> Option<Rgba> {
        Some(match name.trim() {
            "accent" => p.accent,
            "surface" => p.surface,
            "surface_alt" | "surface-alt" => p.surface_alt,
            "bg" => p.bg,
            "fg" => p.fg,
            "border" => p.border,
            other => return Rgba::parse(other),
        })
    }

    /// Цвета заголовков окон: `theme` (по умолчанию) берёт цвета темы,
    /// без темы — `surface`/`bg`.
    pub fn titlebar_colors(&self, deco: &Decorations) -> (Rgba, Rgba) {
        let p = self.palette();
        let variant = self.theme_variant().map(|v| v.0);
        let resolve = |s: &str, theme_key: Option<&String>, fallback: Rgba| -> Rgba {
            let name = if s.trim() == "theme" || s.trim().is_empty() {
                match theme_key {
                    Some(k) => k.as_str(),
                    None => return fallback,
                }
            } else {
                s
            };
            self.named_color(name, &p).unwrap_or(fallback)
        };
        (
            resolve(&deco.active_color, variant.and_then(|v| v.titlebar.as_ref()), p.surface),
            resolve(&deco.inactive_color, variant.and_then(|v| v.titlebar_inactive.as_ref()), p.bg),
        )
    }

    /// Фон рабочего стола без картинки как значение MSS `background`:
    /// цвета из `[wallpaper]`, иначе обои темы, иначе стандартный градиент.
    pub fn wallpaper_background(&self, w: &Wallpaper) -> String {
        if !w.color.trim().is_empty() {
            let c1 = w.color.trim();
            let c2 = if w.color2.trim().is_empty() { c1 } else { w.color2.trim() };
            return format!("linear-gradient(to bottom, {c1}, {c2})");
        }
        if let Some(bg) = self.theme_variant().and_then(|v| v.0.wallpaper.clone()) {
            return bg;
        }
        "linear-gradient(to bottom, #1b2233, #3a2a4a)".into()
    }

    /// Сплошной цвет под обоями (очистка кадра, превью).
    pub fn wallpaper_color(&self, w: &Wallpaper) -> Rgba {
        Rgba::parse(&w.color)
            .or_else(|| self.theme_variant().and_then(|v| v.0.wallpaper_color.as_deref()).and_then(Rgba::parse))
            .or_else(|| if self.resolved.is_some() { Some(self.palette().bg) } else { None })
            .unwrap_or(Rgba::rgb(0x1b, 0x22, 0x33))
    }

    /// Переменные темы (`--shadow`, `--scrim`, свои `vars`) — дописываются
    /// после базовых `:root`, поэтому могут их переопределять.
    pub fn theme_mss_variables(&self) -> String {
        let Some((v, _)) = self.theme_variant() else { return String::new() };
        let mut out = String::from(":root {\n");
        for (k, val) in [("shadow", &v.shadow), ("scrim", &v.scrim)] {
            if let Some(val) = val {
                out.push_str(&format!("  --{k}: {val};\n"));
            }
        }
        for (k, val) in &v.vars {
            out.push_str(&format!("  --{}: {val};\n", k.trim_start_matches('-')));
        }
        out.push_str("}\n");
        out
    }

    /// Свой MSS темы для оболочки (`shell.mss`) или настроек (`settings.mss`).
    pub fn theme_mss(&self, settings: bool) -> &str {
        match self.resolved.as_deref() {
            Some(t) if settings => &t.settings_mss,
            Some(t) => &t.shell_mss,
            None => "",
        }
    }

    /// Свой MSS темы для проводника (`files.mss`).
    pub fn theme_files_mss(&self) -> &str {
        self.resolved.as_deref().map(|t| t.files_mss.as_str()).unwrap_or("")
    }

    /// Файлы активной темы на диске (для слежения за изменениями).
    pub fn theme_files(&self) -> Vec<PathBuf> {
        self.resolved.as_deref().map(Theme::files).unwrap_or_default()
    }

    /// MSS-переменные палитры для `:root { ... }` оболочки и настроек.
    pub fn mss_variables(&self) -> String {
        let p = self.palette();
        let a = self.panel_opacity.clamp(0.0, 1.0);
        let r = self.corner_radius.max(0.0);
        format!(
            ":root {{\n  --bg: {};\n  --surface: {};\n  --surface-alt: {};\n  --panel-bg: {};\n  --menu-bg: {};\n  --fg: {};\n  --muted: {};\n  --border: {};\n  --accent: {};\n  --accent-fg: {};\n  --accent-soft: {};\n  --hover: {};\n  --pressed: {};\n  --danger: {};\n  --success: {};\n  --warning: {};\n  --radius: {r}px;\n  --radius-sm: {}px;\n  --font-size: {}px;\n}}\n",
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
            p.success.hex(),
            p.warning.hex(),
            (r * 0.6).round(),
            self.font_size,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// `#rgb`, `#rrggbb`, `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().strip_prefix('#')?;
        let hex = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        match h.len() {
            3 => {
                let x = |i| hex(i, 1).map(|v| v * 17);
                Some(Self::rgb(x(0)?, x(1)?, x(2)?))
            }
            6 => Some(Self::rgb(hex(0, 2)?, hex(2, 2)?, hex(4, 2)?)),
            8 => Some(Self { r: hex(0, 2)?, g: hex(2, 2)?, b: hex(4, 2)?, a: hex(6, 2)? }),
            _ => None,
        }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a: (a.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }

    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Компоненты 0..1 (sRGB, не премультиплицированы).
    pub fn to_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    pub fn luminance(self) -> f32 {
        0.2126 * self.r as f32 / 255.0 + 0.7152 * self.g as f32 / 255.0 + 0.0722 * self.b as f32 / 255.0
    }

    /// Чёрный или белый текст поверх этого цвета.
    pub fn contrast_fg(self) -> Self {
        if self.luminance() > 0.6 {
            Self::rgb(0x10, 0x12, 0x16)
        } else {
            Self::rgb(0xff, 0xff, 0xff)
        }
    }

    /// Смешать с `other` в доле `t` (0 — self, 1 — other).
    pub fn mix(self, other: Rgba, t: f32) -> Self {
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self { r: l(self.r, other.r), g: l(self.g, other.g), b: l(self.b, other.b), a: l(self.a, other.a) }
    }
}

// ─── input ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub keyboard: Keyboard,
    pub mouse: Pointer,
    pub touchpad: Touchpad,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    /// Раскладки XKB через запятую: `"us,ru"`.
    pub layouts: String,
    pub variants: String,
    /// Опции XKB: `"grp:alt_shift_toggle,caps:escape"`.
    pub options: String,
    pub model: String,
    /// Задержка автоповтора, мс.
    pub repeat_delay: i32,
    /// Скорость автоповтора, нажатий/с.
    pub repeat_rate: i32,
    pub numlock: bool,
    /// Раскладка помнится для каждого окна отдельно (как в Plasma «окно»).
    pub per_window_layout: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            layouts: "us,ru".into(),
            variants: String::new(),
            options: "grp:alt_shift_toggle".into(),
            model: String::new(),
            repeat_delay: 400,
            repeat_rate: 30,
            numlock: true,
            per_window_layout: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AccelProfile {
    #[default]
    Adaptive,
    Flat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Pointer {
    /// −1..1.
    pub accel_speed: f64,
    pub accel_profile: AccelProfile,
    pub natural_scroll: bool,
    pub left_handed: bool,
    /// Множитель прокрутки колесом.
    pub scroll_factor: f64,
    pub middle_emulation: bool,
}

impl Default for Pointer {
    fn default() -> Self {
        Self {
            accel_speed: 0.0,
            accel_profile: AccelProfile::Adaptive,
            natural_scroll: false,
            left_handed: false,
            scroll_factor: 1.0,
            middle_emulation: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Touchpad {
    pub tap: bool,
    pub tap_drag: bool,
    pub natural_scroll: bool,
    pub disable_while_typing: bool,
    pub accel_speed: f64,
    pub accel_profile: AccelProfile,
    pub left_handed: bool,
    pub scroll_factor: f64,
    /// `two-finger` | `edge` | `none`.
    pub scroll_method: String,
    /// `button-areas` | `clickfinger`.
    pub click_method: String,
    pub middle_emulation: bool,
}

impl Default for Touchpad {
    fn default() -> Self {
        Self {
            tap: true,
            tap_drag: true,
            natural_scroll: true,
            disable_while_typing: true,
            accel_speed: 0.2,
            accel_profile: AccelProfile::Adaptive,
            left_handed: false,
            scroll_factor: 1.0,
            scroll_method: "two-finger".into(),
            click_method: "clickfinger".into(),
            middle_emulation: false,
        }
    }
}

// ─── outputs ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    /// Имя коннектора (`eDP-1`, `HDMI-A-1`) или описание («Dell U2720Q»).
    pub name: String,
    pub enabled: bool,
    /// `"2560x1440@144"`, `"1920x1080"` или пусто — предпочтительный режим.
    pub mode: String,
    /// 0 — автоматически по DPI.
    pub scale: f64,
    /// Положение в глобальном пространстве (логические px); не задано —
    /// справа от предыдущего.
    pub position: Option<[i32; 2]>,
    /// `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, …
    pub transform: String,
    pub vrr: bool,
    /// Основной монитор: на нём панель по умолчанию и новые окна.
    pub primary: bool,
    /// Яркость панели на максимуме подсветки, нит — для яркости в нитах
    /// (ядро их не сообщает). Не задано — только проценты.
    pub max_nits: Option<f32>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            mode: String::new(),
            scale: 0.0,
            position: None,
            transform: "normal".into(),
            vrr: false,
            primary: false,
            max_nits: None,
        }
    }
}

// ─── windows ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FocusMode {
    /// Фокус по клику.
    #[default]
    Click,
    /// Фокус за курсором, окно не теряет фокус над рабочим столом.
    Sloppy,
    /// Фокус строго под курсором.
    FollowMouse,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DecorationMode {
    /// Рамки рисует композитор (как KWin) — для всех, кто согласен.
    #[default]
    Server,
    /// Каждое приложение рисует рамки само.
    Client,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    /// По центру рабочей области.
    #[default]
    Center,
    /// Каскадом.
    Cascade,
    /// Под курсором.
    UnderMouse,
    /// Туда, где меньше перекрытие.
    Smart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Windows {
    pub focus_mode: FocusMode,
    /// Поднимать окно при получении фокуса.
    pub raise_on_focus: bool,
    /// Задержка поднятия при фокусе за курсором, мс.
    pub autoraise_delay: u32,
    pub decorations: DecorationMode,
    pub placement: Placement,
    /// Модификатор для перетаскивания (ЛКМ) и изменения размера (ПКМ) окон.
    pub mod_key: String,
    /// Толщина рамки (px); у плиточных окон рисуется всегда, у плавающих —
    /// когда нет серверных заголовков.
    pub border_width: i32,
    /// Зазоры между плиточными окнами и от краёв экрана.
    pub gaps_inner: i32,
    pub gaps_outer: i32,
    /// Раскладка новых столов.
    pub default_layout: LayoutKind,
    /// Доля мастер-области.
    pub master_ratio: f32,
    pub master_count: u32,
    /// Новые окна в плиточной раскладке встают мастером.
    pub new_is_master: bool,
    /// Прилипание к краям экрана и другим окнам при перетаскивании.
    pub snap: bool,
    pub snap_distance: i32,
    /// Перетащить окно к краю экрана — прилепить к половине (как Aero Snap).
    pub edge_tiling: bool,
    /// Двойной клик по заголовку: `toggle-maximize`, `minimize`, `none`…
    pub titlebar_double_click: Action,
    pub titlebar_middle_click: Action,
    /// Колесо над заголовком: `none` или `opacity`.
    pub titlebar_wheel: String,
    /// Скрывать курсор после N мс без движения (0 — никогда).
    pub hide_cursor_after: u32,
    /// Затемнять неактивные окна (0 — нет, 0.1 — слегка).
    pub dim_inactive: f32,
    /// Разрешить окнам активировать себя по xdg-activation без токена
    /// пользователя (иначе — только пометка «требует внимания»).
    pub focus_stealing: bool,
    /// Развёрнутые окна без серверного заголовка (как в KWin): заголовок,
    /// кнопки и меню показывает панель (апплеты `window-title`,
    /// `window-buttons`, `appmenu`).
    pub borderless_maximized: bool,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            focus_mode: FocusMode::Click,
            raise_on_focus: true,
            autoraise_delay: 250,
            decorations: DecorationMode::Server,
            placement: Placement::Center,
            mod_key: "Super".into(),
            border_width: 2,
            gaps_inner: 8,
            gaps_outer: 8,
            default_layout: LayoutKind::Floating,
            master_ratio: 0.55,
            master_count: 1,
            new_is_master: false,
            snap: true,
            snap_distance: 12,
            edge_tiling: true,
            titlebar_double_click: Action::ToggleMaximize,
            titlebar_middle_click: Action::Minimize,
            titlebar_wheel: "none".into(),
            hide_cursor_after: 0,
            dim_inactive: 0.0,
            focus_stealing: false,
            borderless_maximized: false,
        }
    }
}

// ─── decorations ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Decorations {
    /// Высота заголовка (логические px).
    pub title_height: i32,
    pub font_size: f32,
    /// Выравнивание заголовка: `left`, `center`.
    pub title_align: String,
    /// Кнопки: слева`:`справа. Имена: `icon`, `sticky`, `above`,
    /// `minimize`, `maximize`, `close`.
    pub buttons: String,
    /// Скругление верхних углов окна.
    pub corner_radius: f32,
    /// Ширина невидимой зоны изменения размера вокруг окна.
    pub resize_border: i32,
    /// Тень под окнами.
    pub shadow: bool,
    pub shadow_size: i32,
    /// Непрозрачность тени 0..1.
    pub shadow_opacity: f32,
    /// Цвет заголовка активного окна: `theme` (из темы), `accent`,
    /// `surface`, `surface_alt`, `bg` или `#rrggbb`.
    pub active_color: String,
    pub inactive_color: String,
}

impl Default for Decorations {
    fn default() -> Self {
        Self {
            title_height: 32,
            font_size: 13.0,
            title_align: "center".into(),
            buttons: "icon:minimize,maximize,close".into(),
            corner_radius: 10.0,
            resize_border: 8,
            shadow: true,
            shadow_size: 22,
            shadow_opacity: 0.45,
            active_color: "theme".into(),
            inactive_color: "theme".into(),
        }
    }
}

// ─── animations ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Animations {
    pub enabled: bool,
    /// Множитель длительности: 0.5 — вдвое быстрее, 2 — вдвое медленнее.
    pub speed: f32,
    /// Открытие/закрытие окон: `zoom`, `fade`, `slide`, `none`.
    pub window_open: String,
    pub window_close: String,
    /// Переключение столов: `slide`, `slide-vertical`, `fade`, `none`.
    pub workspace_switch: String,
    /// Сворачивание: `zoom` (к панели), `fade`, `none`.
    pub minimize: String,
    /// Плавное перемещение окон при смене плиточной раскладки.
    pub layout_changes: bool,
    /// Анимации оболочки: всплывающие окна вырастают из панели, меню
    /// запуска перетекает между разделами, уведомления въезжают и
    /// сдвигаются, OSD растворяется.
    pub shell: bool,
    /// Плавная смена темы и обоев: цвета перетекают, картинка растворяется.
    pub theme_change: bool,
    /// Меньше движения: переходы сокращаются до растворения, без частиц и
    /// пружин (как «Уменьшить движение» в системах).
    pub reduce_motion: bool,
    /// Группы анимаций оболочки (каждую можно выключить отдельно).
    /// Домашний экран: листание страниц, появление и уход.
    pub home: bool,
    /// Меню запуска: появление, перетекание в «Все приложения».
    pub menu: bool,
    /// Шторка и быстрые настройки.
    pub shade: bool,
    /// Док и панели: увеличение значков, прыжки при запуске.
    pub dock: bool,
    /// Частицы (док, запуск приложений).
    pub particles: bool,
    /// Размытие под всплывающими окнами и меню (дорого на CPU-композиторе).
    pub blur: bool,
    /// Волна от точки нажатия.
    pub ripple: bool,
    /// Переключение страниц приложений и режимов окон (композитор).
    pub pages: bool,
}

impl Default for Animations {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
            window_open: "zoom".into(),
            window_close: "zoom".into(),
            workspace_switch: "slide".into(),
            minimize: "zoom".into(),
            layout_changes: true,
            shell: true,
            theme_change: true,
            reduce_motion: false,
            home: true,
            menu: true,
            shade: true,
            dock: true,
            particles: true,
            blur: true,
            ripple: true,
            pages: true,
        }
    }
}

impl Animations {
    /// Длительность анимации группы оболочки (`home`, `menu`, `shade`,
    /// `dock`, `pages`); 0 — выключена. «Меньше движения» укорачивает вдвое.
    pub fn group_ms(&self, group: &str, base: u32) -> u32 {
        let on = match group {
            "home" => self.home,
            "menu" => self.menu,
            "shade" => self.shade,
            "dock" => self.dock,
            "pages" => self.pages,
            _ => true,
        };
        if !on {
            return 0;
        }
        let ms = self.shell_ms(base);
        if self.reduce_motion { ms / 2 } else { ms }
    }

    /// Включена ли декоративная группа (`particles`, `blur`, `ripple`).
    pub fn effect(&self, name: &str) -> bool {
        if !self.enabled || (self.reduce_motion && name != "blur") {
            return false;
        }
        match name {
            "particles" => self.particles,
            "blur" => self.blur,
            "ripple" => self.ripple,
            _ => true,
        }
    }

    /// Длительность с учётом `enabled` и `speed`; 0 — без анимации.
    pub fn ms(&self, base: u32) -> u32 {
        if !self.enabled {
            return 0;
        }
        (base as f32 * self.speed.clamp(0.1, 5.0)).round() as u32
    }

    /// Длительность анимаций оболочки (0 — выключены).
    pub fn shell_ms(&self, base: u32) -> u32 {
        if self.shell {
            self.ms(base)
        } else {
            0
        }
    }

    /// Длительность перетекания темы (0 — скачком).
    pub fn theme_ms(&self) -> u32 {
        if self.theme_change {
            self.ms(450)
        } else {
            0
        }
    }
}

// ─── workspaces ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Workspaces {
    pub count: u32,
    pub names: Vec<String>,
    /// Переход с последнего на первый.
    pub wrap: bool,
    /// Раскладка для отдельных столов: `{ "3" = "tile" }`.
    pub layouts: BTreeMap<String, LayoutKind>,
    /// Повторное нажатие «стол N» на активном столе возвращает на прошлый.
    pub back_and_forth: bool,
}

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            count: 4,
            names: Vec::new(),
            wrap: true,
            layouts: BTreeMap::new(),
            back_and_forth: false,
        }
    }
}

impl Workspaces {
    pub fn name(&self, index: u32) -> String {
        self.names
            .get(index as usize)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| (index + 1).to_string())
    }
}

// ─── window rules ───────────────────────────────────────────────────────────

/// Правило окна: все заданные условия должны совпасть (регулярные выражения).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WindowRule {
    pub app_id: Option<String>,
    pub title: Option<String>,
    // ── что сделать ──
    pub floating: Option<bool>,
    /// Номер стола (с 1).
    pub workspace: Option<u32>,
    pub output: Option<String>,
    pub size: Option<[i32; 2]>,
    pub position: Option<[i32; 2]>,
    pub center: Option<bool>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub sticky: Option<bool>,
    pub always_on_top: Option<bool>,
    /// Непрозрачность 0..1.
    pub opacity: Option<f32>,
    /// Рисовать ли серверные рамки.
    pub decorations: Option<bool>,
    /// Не давать фокус при открытии.
    pub no_focus: Option<bool>,
    /// Не показывать на панели задач.
    pub skip_taskbar: Option<bool>,
    pub min_size: Option<[i32; 2]>,
    pub max_size: Option<[i32; 2]>,
}

// ─── wallpaper ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Wallpaper {
    /// Путь к картинке или каталогу (слайд-шоу). Пусто — градиент/цвет.
    pub path: String,
    /// `fill`, `fit`, `stretch`, `center`, `tile`.
    pub mode: String,
    /// Цвет фона без картинки. Пусто — обои темы.
    pub color: String,
    /// Второй цвет — вертикальный градиент.
    pub color2: String,
    /// Смена картинки из каталога раз в N минут.
    pub slideshow_minutes: u32,
    /// Свои обои для мониторов: `{ "HDMI-A-1" = "~/Pictures/b.jpg" }`.
    pub per_output: BTreeMap<String, String>,
    /// Показывать значки с `~/Desktop` на рабочем столе.
    pub desktop_icons: bool,
    /// Обои и рабочие столы: `same` — одни на всех столах, `workspace` —
    /// свои у каждого стола (`[wallpaper.workspace.N]`, у кого нет — общие),
    /// `panorama` — одна картинка на все столы, при переключении столов она
    /// сдвигается вбок (как домашний экран Android).
    pub layout: String,
    /// Кадр (при `mode = "fill"` и в панораме): масштаб относительно
    /// «заполнить экран», 1 — без увеличения.
    pub zoom: f32,
    /// Точка картинки в центре кадра, доли ширины и высоты `[x, y]`.
    pub center: [f32; 2],
    /// Панорама: на сколько сдвигается картинка при переходе на соседний
    /// стол, доля ширины экрана (0.05…1).
    pub panorama_shift: f32,
    /// Свои обои столов (`layout = "workspace"`), ключ — номер стола с 1.
    pub workspace: BTreeMap<String, WallpaperFrame>,
    /// Панорама: свой участок картинки для стола (`[wallpaper.panorama_desk.N]`,
    /// номер с 1) — кадр под экран (`zoom`, `center`) на той же картинке; у
    /// кого нет — своя доля общей полосы. При листании столов видимая
    /// область плавно переходит от участка к участку.
    pub panorama_desk: BTreeMap<String, PanoramaDesk>,
}

/// Участок панорамы для стола.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PanoramaDesk {
    pub zoom: f32,
    pub center: [f32; 2],
}

impl Default for PanoramaDesk {
    fn default() -> Self {
        Self { zoom: 1.0, center: [0.5, 0.5] }
    }
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            path: String::new(),
            mode: "fill".into(),
            color: String::new(),
            color2: String::new(),
            slideshow_minutes: 0,
            per_output: BTreeMap::new(),
            desktop_icons: false,
            layout: "same".into(),
            zoom: 1.0,
            center: [0.5, 0.5],
            panorama_shift: 0.5,
            workspace: BTreeMap::new(),
            panorama_desk: BTreeMap::new(),
        }
    }
}

/// Картинка с кадром: общие обои, обои стола.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WallpaperFrame {
    /// Картинка или каталог (слайд-шоу).
    pub path: String,
    pub zoom: f32,
    pub center: [f32; 2],
}

impl Default for WallpaperFrame {
    fn default() -> Self {
        Self { path: String::new(), zoom: 1.0, center: [0.5, 0.5] }
    }
}

impl Wallpaper {
    pub fn panorama(&self) -> bool {
        self.layout == "panorama"
    }

    pub fn per_workspace(&self) -> bool {
        self.layout == "workspace"
    }

    /// Участки панорамы столов `0..n`: `(zoom, center)` или `None` — доля
    /// общей полосы.
    pub fn panorama_desks(&self, n: u32) -> Vec<Option<(f32, [f32; 2])>> {
        (0..n).map(|i| self.panorama_desk.get(&(i + 1).to_string()).map(|d| (d.zoom, d.center))).collect()
    }

    /// Общие обои с кадром.
    pub fn base_frame(&self) -> WallpaperFrame {
        WallpaperFrame { path: self.path.clone(), zoom: self.zoom, center: self.center }
    }

    /// Свои обои стола `ws` (с 0), если заданы и включены.
    pub fn workspace_frame(&self, ws: u32) -> Option<&WallpaperFrame> {
        if !self.per_workspace() {
            return None;
        }
        self.workspace.get(&(ws + 1).to_string()).filter(|f| !f.path.trim().is_empty())
    }

    /// Что показать на выводе `output` на столе `ws` (с 0): свои обои
    /// стола, иначе монитора, иначе общие.
    pub fn frame_for(&self, output: &str, ws: u32) -> WallpaperFrame {
        if let Some(f) = self.workspace_frame(ws) {
            return f.clone();
        }
        match self.per_output.get(output).filter(|p| !p.trim().is_empty()) {
            Some(p) => WallpaperFrame { path: p.clone(), ..Default::default() },
            None => self.base_frame(),
        }
    }
}

// ─── panels ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn is_vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Panel {
    /// `*` — на каждом мониторе, `primary` — на основном, или имя вывода.
    pub output: String,
    pub edge: Edge,
    /// Толщина панели (логические px).
    pub size: u32,
    /// «Плавающая» панель с отступом от края и скруглениями (Plasma 6).
    pub floating: bool,
    /// Когда плавающая панель прилипает к краю (во всю длину, без отступа
    /// и скруглений), как адаптивная панель Plasma 6: `never` — никогда,
    /// `maximized` — на выводе есть развёрнутое окно, `touch` — окно
    /// касается панели или её отступа.
    pub defloat: String,
    /// Толщина прилипшей панели (`defloat`), логические px; 0 — как `size`.
    pub defloated_size: u32,
    /// Длина: доля края 0..1 (1 — во всю ширину).
    pub length: f32,
    /// Выравнивание при `length < 1`: `start`, `center`, `end`.
    pub align: String,
    /// Автоскрытие: панель выезжает при подводе курсора к краю (на
    /// телефоне — по свайпу от её края или касанию полоски у края).
    pub autohide: bool,
    /// Сколько скрываемая панель (док) остаётся на экране после ухода
    /// указателя или отрыва пальца, мс.
    pub autohide_delay: u32,
    /// Резервировать место (окна не перекрывают панель).
    pub exclusive: bool,
    /// Своя непрозрачность (иначе из appearance).
    pub opacity: Option<f32>,
    /// `panel` — обычная панель, `dock` — док: значки приложений с
    /// увеличением под курсором, индикаторами окон и анимациями (как в
    /// macOS, Latte Dock, Cairo-Dock).
    pub mode: String,
    /// Параметры дока (`mode = "dock"`).
    pub dock: Dock,
    /// Апплеты слева направо (сверху вниз).
    pub applets: Vec<Applet>,
    /// Где показывать: `desktop` (по умолчанию — так старые конфиги не
    /// выводят десктопную панель на телефон), `phone`, `any`.
    pub form_factor: String,
}

impl Panel {
    pub fn is_dock(&self) -> bool {
        self.mode == "dock"
    }

    /// Показывать ли панель на этом форм-факторе.
    pub fn shows_on(&self, ff: FormFactor) -> bool {
        match self.form_factor.trim() {
            "any" | "all" => true,
            "phone" | "mobile" => ff == FormFactor::Phone,
            _ => ff == FormFactor::Desktop,
        }
    }

    /// Док по умолчанию: меню, закреплённые приложения, окна, разделы,
    /// папка «Загрузки» и корзина.
    pub fn dock_default() -> Self {
        let app = |id: &str| {
            let mut a = Applet::new("app");
            a.options.insert("app".into(), toml::Value::String(id.into()));
            a
        };
        let mut folder = Applet::new("folder");
        folder.options.insert("path".into(), toml::Value::String("xdg:DOWNLOAD".into()));
        folder.options.insert("name".into(), toml::Value::String("Загрузки".into()));
        let mut trash = Applet::new("folder");
        trash.options.insert("path".into(), toml::Value::String("trash:".into()));
        trash.options.insert("name".into(), toml::Value::String("Корзина".into()));
        Self {
            edge: Edge::Bottom,
            size: 64,
            floating: true,
            length: 1.0,
            exclusive: false,
            mode: "dock".into(),
            applets: vec![
                Applet::new("launcher"),
                Applet::new("separator"),
                app("org.kde.dolphin"),
                app("firefox"),
                app("org.kde.konsole"),
                app("synsettings"),
                Applet::new("taskbar"),
                Applet::new("separator"),
                folder,
                trash,
            ],
            ..Default::default()
        }
    }
}

/// Док (`[panel.dock]`). Вид — классы `.dock*` в MSS (см. docs/DOCK.md).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Dock {
    /// Размер значка без увеличения, px.
    pub icon_size: u32,
    /// Во сколько раз растёт значок под курсором (1 — без увеличения).
    /// MSS `magnification` в `.dock-items` важнее.
    pub zoom: f32,
    /// Радиус увеличения — в значках.
    pub zoom_range: f32,
    /// Оформление: `glass` (стекло), `shelf` (3D-полка с отражениями, как
    /// в Mac OS X Leopard), `flat`, `neon`, `none` (только значки) —
    /// класс `.dock-style-<стиль>`.
    pub style: String,
    /// Подпись с именем над значком при наведении.
    pub labels: bool,
    /// Индикатор открытых окон: `dot`, `dots` (по точке на окно), `line`,
    /// `glow`, `none`.
    pub indicator: String,
    /// Анимация запуска: `bounce`, `pulse`, `spin` (3D-вращение), `none`.
    pub launch_animation: String,
    /// Эффект значка при наведении: `lift`, `tilt` (3D-наклон), `spin`,
    /// `glow`, `none`.
    pub hover_effect: String,
    /// Частицы при наведении: `sparkle`, `magic`, `embers`, `bubbles`,
    /// `hearts`, `snow`, `none`.
    pub hover_particles: String,
    /// Частицы при запуске (клике): `stars`, `sparkle`, `confetti`,
    /// `fireworks`, `magic`, `none`.
    pub launch_particles: String,
    /// Прятать док, когда его перекрывает окно (или всегда при
    /// `autohide`), и показывать при подводе курсора к краю.
    pub intellihide: bool,
    /// Значков больше, чем помещается по длине края: `scroll` — ряд
    /// листается пальцем (колесом), `clip` — лишнее обрезается.
    pub overflow: String,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            icon_size: 48,
            zoom: 1.7,
            zoom_range: 2.5,
            style: "glass".into(),
            labels: true,
            indicator: "dot".into(),
            launch_animation: "bounce".into(),
            hover_effect: "lift".into(),
            hover_particles: "none".into(),
            launch_particles: "stars".into(),
            intellihide: false,
            overflow: "scroll".into(),
        }
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            output: "*".into(),
            edge: Edge::Bottom,
            size: 46,
            floating: true,
            defloat: "never".into(),
            defloated_size: 0,
            length: 1.0,
            align: "center".into(),
            autohide: false,
            autohide_delay: 1500,
            exclusive: true,
            opacity: None,
            mode: "panel".into(),
            dock: Dock::default(),
            form_factor: "desktop".into(),
            applets: vec![
                Applet::new("launcher"),
                Applet::new("workspaces"),
                Applet::new("taskbar"),
                Applet::new("spacer"),
                Applet::new("tray"),
                Applet::new("keyboard"),
                Applet::new("volume"),
                Applet::new("link"),
                Applet::new("network"),
                Applet::new("battery"),
                Applet::new("notifications"),
                Applet::new("clock"),
                Applet::new("power"),
            ],
        }
    }
}

/// Апплет панели: тип + произвольные настройки типа.
///
/// Типы: `launcher`, `taskbar`, `workspaces`, `spacer`, `separator`,
/// `clock`, `keyboard`, `volume`, `battery`, `network`, `tray`,
/// `notifications`, `power`, `button` (своя кнопка: `icon`, `label`,
/// `action`), `command` (вывод команды: `command`, `interval`, `action`),
/// `cpu`, `memory`, `layout` (раскладка окон стола), `show-desktop`,
/// `app` (значок запуска: `app` — id .desktop, `command`, `icon`, `name`),
/// `group` (раздел со всплывающим окном: `name`, `icon`, `items` — id
/// приложений или пути, `open` — `click`/`hover`, `view` — `grid`/`list`/`fan`),
/// `folder` (содержимое каталога во всплывающем окне: `path` — путь или
/// `trash:`, `name`, `icon`, `open`, `view`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Applet {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub options: BTreeMap<String, toml::Value>,
}

impl Applet {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.into(), options: BTreeMap::new() }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.options.get(key).and_then(|v| v.as_str())
    }

    pub fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.str(key).unwrap_or(default)
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.options.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn int_or(&self, key: &str, default: i64) -> i64 {
        self.options.get(key).and_then(|v| v.as_integer()).unwrap_or(default)
    }

    pub fn float_or(&self, key: &str, default: f64) -> f64 {
        self.options
            .get(key)
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .unwrap_or(default)
    }

    pub fn strings(&self, key: &str) -> Vec<String> {
        self.options
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }
}

// ─── files ──────────────────────────────────────────────────────────────────

/// Проводник `synfiles`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Files {
    /// Вид по умолчанию: `details`, `list`, `tiles`, `icons`.
    pub view: String,
    /// Размер значков в виде `icons` (px).
    pub icon_size: u32,
    /// Сортировка: `name`, `modified`, `type`, `size`.
    pub sort_by: String,
    pub sort_descending: bool,
    pub folders_first: bool,
    pub show_hidden: bool,
    /// Открывать одним щелчком (как в KDE), иначе — двойным.
    pub single_click: bool,
    /// Спрашивать перед удалением в корзину (безвозвратное — всегда).
    pub confirm_trash: bool,
    /// Панель просмотра справа.
    pub preview_pane: bool,
    /// Миниатюры картинок и видео.
    pub thumbnails: bool,
    /// Не делать миниатюры файлов больше этого размера (МБ).
    pub thumbnail_max_mb: u64,
    /// Закреплённые папки на боковой панели (`~/Projects`, `/mnt/data`).
    pub pinned: Vec<String>,
    /// Открывать вкладки прошлого сеанса.
    pub restore_tabs: bool,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            view: "details".into(),
            icon_size: 64,
            sort_by: "name".into(),
            sort_descending: false,
            folders_first: true,
            show_hidden: false,
            single_click: false,
            confirm_trash: false,
            preview_pane: false,
            thumbnails: true,
            thumbnail_max_mb: 64,
            pinned: Vec::new(),
            restore_tabs: true,
        }
    }
}

// ─── launcher ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Launcher {
    /// `win11` — «Пуск» как в Windows 11 (закреплённые, «Все приложения» с
    /// алфавитным указателем, рекомендуемые), `menu` — меню с разделами
    /// (Kickoff), `fullscreen` — сетка на весь экран.
    pub style: String,
    /// Избранное: имена .desktop без расширения (`org.kde.dolphin`).
    pub favorites: Vec<String>,
    pub show_categories: bool,
    pub show_recent: bool,
    /// Колонок в сетке.
    pub columns: u32,
    /// Ширина/высота меню (логические px).
    pub width: u32,
    pub height: u32,
    /// Искать также по команде `Exec` и ключевым словам.
    pub search_keywords: bool,
    /// Вычислять выражения в строке поиска (`2+2*3`).
    pub calculator: bool,
    /// Запуск команды по `Enter`, если ничего не найдено.
    pub run_commands: bool,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            style: "win11".into(),
            favorites: vec![
                "org.kde.konsole".into(),
                "org.kde.dolphin".into(),
                "firefox".into(),
                "synsettings".into(),
            ],
            show_categories: true,
            show_recent: true,
            columns: 5,
            width: 640,
            height: 560,
            search_keywords: true,
            calculator: true,
            run_commands: true,
        }
    }
}

// ─── notifications ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Notifications {
    /// Быть сервером org.freedesktop.Notifications.
    pub enabled: bool,
    /// `top-right`, `top-left`, `bottom-right`, `bottom-left`, `top`, `bottom`.
    pub position: String,
    /// Время показа по умолчанию, мс.
    pub timeout: u32,
    /// Для критичных уведомлений (0 — пока не закроют).
    pub critical_timeout: u32,
    pub max_visible: u32,
    pub width: u32,
    pub do_not_disturb: bool,
    /// Хранить историю в центре уведомлений.
    pub history: bool,
    pub history_size: u32,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            position: "top-right".into(),
            timeout: 6000,
            critical_timeout: 0,
            max_visible: 5,
            width: 380,
            do_not_disturb: false,
            history: true,
            history_size: 100,
        }
    }
}

// ─── lock / idle ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Lock {
    /// Внешний экран блокировки (`swaylock -f`, `hyprlock`). Пусто —
    /// встроенный экран блокировки оболочки.
    pub command: String,
    /// Блокировать перед сном.
    pub before_sleep: bool,
    /// Как снимать встроенную блокировку: `password` — пароль пользователя
    /// (PAM), `swipe` — свайпом, без проверки (телефон по умолчанию: пароль
    /// может быть не задан или неизвестен владельцу).
    pub method: String,
}

impl Default for Lock {
    fn default() -> Self {
        Self { command: String::new(), before_sleep: true, method: "password".into() }
    }
}

/// Платформа: на чём и как работает композитор (десктоп / телефон).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Platform {
    /// Рендерер сеанса DRM: `auto`, `gpu` (GLES через GBM/EGL) или `cpu`
    /// (pixman + dumb-буферы — телефоны с downstream-ядром без GBM/EGL, где
    /// GPU доступен только клиентам через Vulkan/KGSL).
    pub renderer: String,
    /// Форм-фактор: `auto`, `desktop` или `phone`. `auto` — телефон, если
    /// единственный подключённый монитор — встроенная DSI-панель.
    pub form_factor: String,
    /// Выходы, которые никогда не включаются (шаблоны с `*`): writeback-коннекторы
    /// downstream-драйверов вроде `Virtual-1` иначе получают CRTC панели.
    pub ignore_outputs: Vec<String>,
}

impl Default for Platform {
    fn default() -> Self {
        Self { renderer: "auto".into(), form_factor: "auto".into(), ignore_outputs: vec!["Virtual-*".into()] }
    }
}

/// Форм-фактор, к которому подстраиваются композитор и оболочка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormFactor {
    #[default]
    Desktop,
    Phone,
}

impl FormFactor {
    pub fn as_str(self) -> &'static str {
        match self {
            FormFactor::Desktop => "desktop",
            FormFactor::Phone => "phone",
        }
    }
}

impl Config {
    /// Разобрать `platform.form_factor`; `None` — `auto` (решает бэкенд по мониторам).
    pub fn form_factor_setting(&self) -> Option<FormFactor> {
        match self.platform.form_factor.trim() {
            "phone" | "mobile" => Some(FormFactor::Phone),
            "desktop" => Some(FormFactor::Desktop),
            _ => None,
        }
    }

    /// Подстроить значения по умолчанию под телефон: окна во весь экран
    /// (monocle), без рамок и зазоров, оболочка `synmobile-shell`, без Xwayland.
    /// Явно заданные в конфиге значения не трогаются.
    /// Форм-фактор процесса: `SYNSHELL_FORM_FACTOR` (его выставляет
    /// композитор детям), иначе `[platform] form_factor`, иначе рабочий стол.
    pub fn process_form_factor(&self) -> FormFactor {
        match std::env::var("SYNSHELL_FORM_FACTOR").as_deref() {
            Ok("phone") | Ok("mobile") => FormFactor::Phone,
            Ok("desktop") => FormFactor::Desktop,
            _ => self.form_factor_setting().unwrap_or_default(),
        }
    }

    pub fn apply_form_factor(&mut self, ff: FormFactor) {
        if ff != FormFactor::Phone {
            return;
        }
        let d = Config::default();
        if self.windows.default_layout == d.windows.default_layout {
            self.windows.default_layout = crate::action::LayoutKind::Monocle;
        }
        if self.windows.border_width == d.windows.border_width {
            self.windows.border_width = 0;
        }
        if self.windows.gaps_inner == d.windows.gaps_inner {
            self.windows.gaps_inner = 0;
        }
        if self.windows.gaps_outer == d.windows.gaps_outer {
            self.windows.gaps_outer = 0;
        }
        if self.windows.decorations == d.windows.decorations {
            self.windows.decorations = DecorationMode::Client;
        }
        if self.general.shell == d.general.shell {
            self.general.shell = "synmobile-shell".into();
        }
        if self.general.xwayland == d.general.xwayland {
            self.general.xwayland = false;
        }
        if self.lock.method == d.lock.method {
            self.lock.method = "swipe".into();
        }
    }

    /// Совпадает ли имя выхода с одним из шаблонов `platform.ignore_outputs`.
    pub fn output_ignored(&self, name: &str) -> bool {
        self.platform.ignore_outputs.iter().any(|pat| glob_match(pat, name))
    }
}

/// Простое сопоставление с `*` (любая подстрока).
pub fn glob_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match s[pos..].find(part) {
            Some(idx) => {
                if i == 0 && idx != 0 {
                    return false;
                }
                pos += idx + part.len();
            }
            None => return false,
        }
    }
    pat.ends_with('*') || pos == s.len()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Idle {
    /// Погасить мониторы через N секунд простоя (0 — никогда).
    pub dpms_after: u32,
    /// Заблокировать через N секунд.
    pub lock_after: u32,
    /// Уснуть через N секунд.
    pub suspend_after: u32,
    /// Телефон: глубокий сон через N секунд после гашения экрана (кнопкой или
    /// по простою); 0 — сразу. Композитор пишет это в `/run/syn-sleep/screen-off`
    /// вместе со своим pid, усыпляет служба платформы `syn-sleepd`.
    pub sleep_delay: u32,
}

impl Default for Idle {
    fn default() -> Self {
        Self { dpms_after: 600, lock_after: 0, suspend_after: 0, sleep_delay: 0 }
    }
}

// ─── keybindings ────────────────────────────────────────────────────────────

/// Встроенные сочетания клавиш (как в Plasma, где есть аналог).
pub fn default_keybindings() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Super+Return", "spawn $TERMINAL"),
        ("Ctrl+Alt+T", "spawn $TERMINAL"),
        ("Super+E", "spawn $FILE_MANAGER"),
        ("Super+D", "shell launcher"),
        ("Alt+F1", "shell launcher"),
        ("Alt+F2", "shell run"),
        ("Alt+Space", "shell run"),
        ("Super+Q", "close"),
        ("Alt+F4", "close"),
        ("Super+Shift+Q", "kill"),
        ("Super+F", "toggle-fullscreen"),
        ("Super+Up", "toggle-maximize"),
        ("Super+Page_Up", "toggle-maximize"),
        ("Super+Down", "minimize"),
        ("Super+Page_Down", "minimize"),
        ("Super+Left", "snap left"),
        ("Super+Right", "snap right"),
        ("Super+Space", "toggle-floating"),
        ("Super+C", "center"),
        ("Super+Shift+P", "toggle-sticky"),
        ("Super+Shift+T", "toggle-always-on-top"),
        ("Alt+Tab", "focus-next"),
        ("Alt+Shift+Tab", "focus-prev"),
        ("Super+Tab", "overview"),
        ("Super+W", "overview"),
        ("Super+H", "focus left"),
        ("Super+L", "focus right"),
        ("Super+K", "focus up"),
        ("Super+J", "focus down"),
        ("Super+Shift+H", "move left"),
        ("Super+Shift+L", "move right"),
        ("Super+Shift+K", "move up"),
        ("Super+Shift+J", "move down"),
        ("Super+1", "workspace 1"),
        ("Super+2", "workspace 2"),
        ("Super+3", "workspace 3"),
        ("Super+4", "workspace 4"),
        ("Super+5", "workspace 5"),
        ("Super+6", "workspace 6"),
        ("Super+7", "workspace 7"),
        ("Super+8", "workspace 8"),
        ("Super+9", "workspace 9"),
        ("Super+Shift+1", "move-to-workspace 1"),
        ("Super+Shift+2", "move-to-workspace 2"),
        ("Super+Shift+3", "move-to-workspace 3"),
        ("Super+Shift+4", "move-to-workspace 4"),
        ("Super+Shift+5", "move-to-workspace 5"),
        ("Super+Shift+6", "move-to-workspace 6"),
        ("Super+Shift+7", "move-to-workspace 7"),
        ("Super+Shift+8", "move-to-workspace 8"),
        ("Super+Shift+9", "move-to-workspace 9"),
        ("Ctrl+Super+Left", "workspace prev"),
        ("Ctrl+Super+Right", "workspace next"),
        ("Ctrl+Alt+Left", "workspace prev"),
        ("Ctrl+Alt+Right", "workspace next"),
        ("Ctrl+Super+Shift+Left", "move-to-workspace-follow prev"),
        ("Ctrl+Super+Shift+Right", "move-to-workspace-follow next"),
        ("Super+WheelDown", "workspace next"),
        ("Super+WheelUp", "workspace prev"),
        ("Super+Comma", "focus-output left"),
        ("Super+Period", "focus-output right"),
        ("Super+Shift+Comma", "move-to-output left"),
        ("Super+Shift+Period", "move-to-output right"),
        ("Super+T", "cycle-layout"),
        ("Super+Shift+F", "layout floating"),
        ("Super+Equal", "master-ratio +0.05"),
        ("Super+Minus", "master-ratio -0.05"),
        ("Super+BracketRight", "master-count +1"),
        ("Super+BracketLeft", "master-count -1"),
        ("Print", "screenshot-interactive"),
        ("Shift+Print", "screenshot"),
        ("Alt+Print", "screenshot-window"),
        ("Super+Shift+R", "reload-config"),
        ("Super+Shift+E", "shell power-menu"),
        ("Ctrl+Alt+Delete", "shell power-menu"),
        ("Super+Escape", "lock"),
        ("Super+N", "shell notifications"),
        ("Super+V", "shell clipboard"),
        ("Super+I", "spawn synsettings"),
        ("XF86AudioRaiseVolume", "shell volume +5"),
        ("XF86AudioLowerVolume", "shell volume -5"),
        ("XF86AudioMute", "shell mute"),
        ("XF86AudioMicMute", "shell mic-mute"),
        ("XF86MonBrightnessUp", "shell brightness +5"),
        ("XF86MonBrightnessDown", "shell brightness -5"),
        ("XF86AudioPlay", "shell media play-pause"),
        ("XF86AudioNext", "shell media next"),
        ("XF86AudioPrev", "shell media previous"),
        ("XF86PowerOff", "shell power-menu"),
    ]
}

impl Config {
    /// Загрузить конфиг; при отсутствии файла — значения по умолчанию.
    /// Ошибка разбора возвращается вместе с конфигом по умолчанию, чтобы
    /// сеанс не падал из-за опечатки.
    pub fn load() -> (Config, Option<String>) {
        Self::load_from(&crate::paths::config_file())
    }

    pub fn load_from(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (Config::default(), Some(format!("{}: {e}", path.display()))),
        }
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c = toml::from_str::<Config>(text).map_err(|e| e.to_string())?;
        c.appearance.load_theme();
        Ok(c)
    }

    /// Создать файл конфигурации с комментариями, если его ещё нет.
    pub fn ensure_file_exists() -> std::io::Result<()> {
        let path = crate::paths::config_file();
        if path.exists() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, DEFAULT_CONFIG_TOML)
    }

    /// Итоговые сочетания: встроенные + пользовательские (перекрывают).
    /// Подставляет `$TERMINAL` и `$FILE_MANAGER`. Ошибки разбора сочетаний
    /// возвращаются отдельным списком.
    pub fn resolved_keybindings(&self) -> (Vec<(KeyCombo, Action)>, Vec<String>) {
        let mut map: Vec<(KeyCombo, Action)> = Vec::new();
        let mut errors = Vec::new();
        let subst = |a: Action| -> Action {
            match a {
                Action::Spawn(cmd) => Action::Spawn(
                    cmd.replace("$TERMINAL", &self.general.terminal)
                        .replace("$FILE_MANAGER", &self.general.file_manager)
                        .replace("$BROWSER", &self.general.browser),
                ),
                other => other,
            }
        };
        let mut put = |combo: KeyCombo, action: Action| {
            let key_lc = combo.key.to_ascii_lowercase();
            map.retain(|(c, _)| !(c.mods == combo.mods && c.key.to_ascii_lowercase() == key_lc));
            if action != Action::None {
                map.push((combo, action));
            }
        };
        for (k, a) in default_keybindings() {
            match (k.parse::<KeyCombo>(), a.parse::<Action>()) {
                (Ok(c), Ok(a)) => put(c, subst(a)),
                (Err(e), _) | (_, Err(e)) => errors.push(format!("встроенное «{k}»: {e}")),
            }
        }
        for (k, a) in &self.keybindings {
            match k.parse::<KeyCombo>() {
                Ok(c) => put(c, subst(a.clone())),
                Err(e) => errors.push(e),
            }
        }
        (map, errors)
    }

    /// Раскладка для стола с индексом `index` (с 0).
    pub fn workspace_layout(&self, index: u32) -> LayoutKind {
        self.workspaces
            .layouts
            .get(&(index + 1).to_string())
            .copied()
            .unwrap_or(self.windows.default_layout)
    }
}

// ─── mobile ─────────────────────────────────────────────────────────────────

/// Телефон (`[mobile]`): режим окон, домашний экран.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Mobile {
    /// Режим окон: `pages` — каждое приложение во весь экран, листание
    /// влево-вправо; `free` — свободные окна на большом виртуальном столе
    /// (пан двумя пальцами).
    pub mode: crate::action::MobileMode,
    /// Виртуальный стол режима `free`: `2x2`, `3x3`, `infinite`.
    pub desk: String,
    /// Ручка окна в `free`: `server` (полоса композитора), `none`.
    pub handle: String,
    pub handle_height: u32,
    /// Клавиша «назад» для окна: `XF86Back`, `Escape`, `Alt+Left`.
    pub back_key: String,
    /// Устарело: режим плиток убран; поле читается, чтобы старые
    /// config.toml не давали ошибку.
    #[serde(skip_serializing)]
    pub tiles_max: u32,
    /// Щипок меняет масштаб виртуального стола (дорого на CPU).
    pub pinch_zoom: bool,
    /// Запоминать выбранный режим в config.toml.
    pub remember_mode: bool,
    /// Первая страница домашнего экрана — ресурсы (процессор, память,
    /// питание, запущенные приложения).
    pub resources_page: bool,
    /// Приложения на домашнем экране (id .desktop); пусто — все по алфавиту.
    pub home_apps: Vec<String>,
    /// Колонок в сетке домашнего экрана.
    pub home_columns: u32,
    /// Вид «Пуска» на телефоне: `pages` — все приложения значками по
    /// страницам (листаются пальцем), `list` — закреплённые, рекомендуемые
    /// и «Все» списком.
    pub launcher: String,
}

impl Default for Mobile {
    fn default() -> Self {
        Self {
            mode: crate::action::MobileMode::Pages,
            desk: "3x3".into(),
            handle: "server".into(),
            handle_height: 28,
            back_key: "XF86Back".into(),
            tiles_max: 0,
            pinch_zoom: false,
            remember_mode: true,
            resources_page: true,
            home_apps: Vec::new(),
            home_columns: 4,
            launcher: "pages".into(),
        }
    }
}

// ─── gestures ───────────────────────────────────────────────────────────────

/// Сенсорные жесты композитора (`[gestures]`); действия — как в
/// `[keybindings]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Gestures {
    pub enabled: bool,
    /// Ширина зоны у края экрана, логические px.
    pub edge_size: u32,
    /// Путь пальца до распознавания жеста, px.
    pub threshold: u32,
    /// До этого сдвига — тап.
    pub tap_slop: u32,
    /// Долгое нажатие, мс (и в оболочке, и в композиторе).
    pub long_press_ms: u32,
    /// Скорость броска для перелистывания, px/мс.
    pub velocity: f32,
    pub edge_left: Action,
    pub edge_right: Action,
    pub edge_bottom: Action,
    /// Свайп снизу с задержкой пальца.
    pub edge_bottom_hold: Action,
    pub edge_top: Action,
    /// Пан двумя пальцами двигает виртуальный стол (`free`).
    pub two_finger_pan: bool,
    /// Долгое нажатие по окну: `none` или `move` (перетаскивание окна).
    pub long_press_window: String,
    /// Тачпад: свайп тремя пальцами вбок / вверх (десктоп).
    pub touchpad_horizontal: Action,
    pub touchpad_up: Action,
}

impl Default for Gestures {
    fn default() -> Self {
        let a = |s: &str| s.parse::<Action>().unwrap_or(Action::None);
        Self {
            enabled: true,
            edge_size: 24,
            threshold: 48,
            tap_slop: 10,
            long_press_ms: 450,
            velocity: 0.5,
            edge_left: a("back"),
            edge_right: a("back"),
            edge_bottom: a("shell home"),
            edge_bottom_hold: a("shell recents"),
            edge_top: a("shell shade"),
            two_finger_pan: true,
            long_press_window: "none".into(),
            touchpad_horizontal: a("workspace next"),
            touchpad_up: a("overview"),
        }
    }
}

// ─── вибрация ──────────────────────────────────────────────────────────────

/// Виброотклик (`[haptics]`, телефон): общий выключатель, сила и что
/// отзывается вибрацией.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Haptics {
    pub enabled: bool,
    /// Сила, % (0–100).
    pub strength: u32,
    /// Жесты от краёв экрана: «назад», «домой», «Недавние», шторка.
    pub gestures: bool,
    /// Экранная клавиатура.
    pub keyboard: bool,
    /// Касания в приложениях: удержание (выбор, меню).
    pub touch: bool,
    /// Уведомления (кроме «Не беспокоить»).
    pub notifications: bool,
}

impl Default for Haptics {
    fn default() -> Self {
        Self { enabled: true, strength: 70, gestures: true, keyboard: true, touch: true, notifications: true }
    }
}

// ─── звук ───────────────────────────────────────────────────────────────────

/// Звук (`[sound]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Sound {
    /// Громкость выше 100 % (до 150 %, программное усиление PipeWire — как «Сверхусиление» GNOME):
    /// для тихих записей; громкие звуки при этом искажаются.
    pub overamplify: bool,
}

impl Sound {
    /// Предел громкости, %.
    pub fn max_volume(&self) -> u32 {
        if self.overamplify { 150 } else { 100 }
    }
}

// ─── поворот экрана ─────────────────────────────────────────────────────────

/// Поворот встроенного экрана по акселерометру (`[rotation]`, как в
/// Android). Датчик — iio-sensor-proxy (D-Bus `net.hadess.SensorProxy`);
/// поворачивает оболочка действием `rotate`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ScreenRotation {
    /// Автоповорот: экран поворачивается за телефоном. Выключен —
    /// ориентация зафиксирована.
    pub auto: bool,
    /// При зафиксированной ориентации, когда телефон повернули, показать
    /// кнопку «повернуть» (как в Android).
    pub suggest: bool,
    /// Поворачивать и «вверх ногами» (180°).
    pub upside_down: bool,
    /// Сколько новая ориентация должна продержаться, прежде чем экран
    /// повернётся (мс): случайный наклон на миг не поворачивает экран.
    pub delay_ms: u32,
    /// Анимация поворота (как в Android): старая картинка поворачивается
    /// и растворяется в новой. Длительность, мс; 0 — без анимации.
    pub animation_ms: u32,
    /// Угол наклона (градусы, 10–80), после которого датчик считает, что
    /// телефон повернули; меньше — чувствительнее. Нужна служба датчиков с
    /// методом `SetOrientationThreshold` (патч arch-mobile-port), иначе — 35°.
    pub threshold_deg: u32,
}

impl Default for ScreenRotation {
    fn default() -> Self {
        Self { auto: true, suggest: true, upside_down: false, delay_ms: 1000, animation_ms: 300, threshold_deg: 35 }
    }
}

/// Яркость экрана (`[brightness]`): автояркость по датчику освещённости
/// (iio-sensor-proxy, как адаптивная яркость Android). Без датчика — ничего
/// не делает.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Brightness {
    /// Автояркость: подсветка следует за освещённостью. Сдвиг ползунка при
    /// включённой автояркости запоминается как поправка к кривой.
    pub auto: bool,
    /// Нижняя и верхняя граница автояркости, %.
    pub min_pct: u32,
    pub max_pct: u32,
    /// Время перехода к новой яркости, мс (темнее — вдвое медленнее).
    pub smooth_ms: u32,
    /// Поправка к кривой, % (−50…50): её задаёт сдвиг ползунка при включённой
    /// автояркости, оболочка сохраняет сама.
    pub offset: f32,
}

impl Default for Brightness {
    fn default() -> Self {
        Self { auto: false, min_pct: 2, max_pct: 100, smooth_ms: 800, offset: 0.0 }
    }
}

/// Экранная клавиатура synkeyboard (`[osk]`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Osk {
    /// Масштаб клавиатуры поверх масштаба вывода — отдельно от масштаба
    /// оболочки (`[appearance] ui_scale`): высота клавиш, подписи, отступы.
    pub scale: f32,
}

impl Default for Osk {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

/// Кнопка питания (`[power_button]`): короткое нажатие и удержание — разные
/// действия, как в Android. Строки — действия (`screen-toggle`, `lock`,
/// `shell power-menu`, `none`…); пусто — по форм-фактору: на телефоне
/// короткое — `screen-toggle` (погасить и заблокировать / включить), удержание —
/// меню выключения; на компьютере короткое — меню выключения.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PowerButton {
    pub short: String,
    pub long: String,
    /// Сколько держать для `long`, мс; 0 — по умолчанию (3000).
    pub long_ms: u32,
}

impl PowerButton {
    /// Действия (короткое, удержание) и порог удержания с учётом форм-фактора.
    pub fn resolved(&self, ff: FormFactor) -> (String, String, u32) {
        let phone = ff == FormFactor::Phone;
        let pick = |v: &str, phone_default: &str, desktop_default: &str| {
            if v.trim().is_empty() {
                (if phone { phone_default } else { desktop_default }).to_string()
            } else {
                v.trim().to_string()
            }
        };
        (
            pick(&self.short, "screen-toggle", "shell power-menu"),
            pick(&self.long, "shell power-menu", "none"),
            if self.long_ms == 0 { 3000 } else { self.long_ms },
        )
    }
}

// ─── дата и время ───────────────────────────────────────────────────────────

/// Дата и время (`[time]`). Пояс и синхронизация по сети — настройки
/// системы (systemd-timedated), здесь — что делает оболочка.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Time {
    /// Определять часовой пояс по сети (по внешнему IP) и ставить его
    /// системе. Не задано — на телефоне да, на десктопе нет.
    pub auto_timezone: Option<bool>,
}

impl Time {
    pub fn auto_timezone(&self, ff: FormFactor) -> bool {
        self.auto_timezone.unwrap_or(ff == FormFactor::Phone)
    }
}

// ─── связь устройств ────────────────────────────────────────────────────────

/// Связь с другими оболочками synshell (`[link]`, демон `synlink`):
/// телефон ↔ компьютер по USB и Wi-Fi.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Link {
    pub enabled: bool,
    /// Имя этой машины для других устройств; пусто — имя хоста.
    pub name: String,
    /// Видна ли машина в сети (Wi-Fi) для спаривания; спаренные и USB
    /// соединяются всегда.
    pub discoverable: bool,
    /// Пересылать уведомления на спаренные устройства и показывать их.
    pub notifications: bool,
    /// Монтировать файлы соединённых устройств (Проводник → Устройства).
    pub auto_mount: bool,
    /// Порт UDP (QUIC); поиск — на порту на единицу меньше.
    pub port: u16,
    /// Трансляция экрана другого устройства: `auto` — видео аппаратным
    /// кодером той стороны для крупных изменений (HEVC, нет — H.264), мелкие
    /// и всё после остановки — без потерь; `lossless` — только без потерь
    /// (zstd); `h264`, `hevc` — видео для всех изменений (в покое всё равно
    /// досылается без потерь).
    pub screen_codec: String,
    /// Битрейт видео, Мбит/с; 0 — сам: по кабелю 80, по Wi-Fi 30.
    pub screen_bitrate: u32,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            discoverable: true,
            notifications: true,
            auto_mount: true,
            port: 47471,
            screen_codec: "auto".into(),
            screen_bitrate: 0,
        }
    }
}

// ─── местоположение ─────────────────────────────────────────────────────────

/// Местоположение для программ (`[location]`, GeoClue): оболочка — агент GeoClue
/// и решает, кому его отдавать.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Location {
    /// Выключено — местоположение не получает никто.
    pub enabled: bool,
    /// Программы (id .desktop без `.desktop`), которым разрешено и запрещено
    /// навсегда; остальных оболочка спрашивает при первом запросе.
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
}

impl Default for Location {
    fn default() -> Self {
        Self { enabled: true, allowed: Vec::new(), denied: Vec::new() }
    }
}

// ─── wifi / packages ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Wifi {
    /// `auto` (кто есть на D-Bus), `iwd`, `networkmanager`.
    pub backend: String,
}

impl Default for Wifi {
    fn default() -> Self {
        Self { backend: "auto".into() }
    }
}

/// Установка программ (`synpkg`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Packages {
    /// Искать и собирать пакеты из AUR.
    pub aur: bool,
    /// От чьего имени собирать AUR (makepkg не работает от root); пусто —
    /// текущий пользователь.
    pub build_user: String,
    /// Проверять обновления раз в столько часов (0 — не проверять).
    pub check_hours: u32,
}

impl Default for Packages {
    fn default() -> Self {
        Self { aur: true, build_user: String::new(), check_hours: 6 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_and_matches_defaults() {
        let c = Config::parse(DEFAULT_CONFIG_TOML).expect("default-config.toml must parse");
        let d = Config::default();
        assert_eq!(c.general, d.general);
        assert_eq!(c.windows, d.windows);
        assert_eq!(c.appearance, d.appearance);
        assert_eq!(c.decorations, d.decorations);
        assert_eq!(c.wallpaper, d.wallpaper);
        assert_eq!(c.panels.len(), 1);
    }

    #[test]
    fn mobile_sections_and_panel_form_factor() {
        let c = Config::parse(
            r#"
            [mobile]
            mode = "free"
            desk = "2x2"
            [gestures]
            edge_left = "key Escape"
            [[output]]
            name = "DSI-1"
            max_nits = 1000
            [[panel]]
            form_factor = "phone"
            mode = "dock"
            [[panel]]
            "#,
        )
        .unwrap();
        assert_eq!(c.mobile.mode, crate::action::MobileMode::Free);
        assert_eq!(c.gestures.edge_left.to_string(), "key Escape");
        assert_eq!(c.gestures.edge_right, Action::Back);
        assert_eq!(c.outputs[0].max_nits, Some(1000.0));
        assert!(c.panels[0].shows_on(FormFactor::Phone) && !c.panels[0].shows_on(FormFactor::Desktop));
        // Панель без form_factor — десктопная (старые конфиги не выводят её на телефон).
        assert!(!c.panels[1].shows_on(FormFactor::Phone) && c.panels[1].shows_on(FormFactor::Desktop));
        assert_eq!(Config::default().animations.group_ms("home", 300), 300);
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn keybindings_override() {
        let c = Config::parse(
            r#"
            [keybindings]
            "Super+Return" = "spawn foot"
            "Super+Q" = "none"
            "#,
        )
        .unwrap();
        let (kb, errs) = c.resolved_keybindings();
        assert!(errs.is_empty(), "{errs:?}");
        let ret = kb.iter().find(|(c, _)| c.to_string() == "Super+Return").unwrap();
        assert_eq!(ret.1, Action::Spawn("foot".into()));
        assert!(!kb.iter().any(|(c, _)| c.to_string() == "Super+Q"));
    }

    #[test]
    fn applet_options() {
        let c = Config::parse(
            r#"
            [[panel]]
            edge = "top"
            applets = [ { type = "clock", format = "%H:%M", seconds = true } ]
            "#,
        )
        .unwrap();
        let a = &c.panels[0].applets[0];
        assert_eq!(a.kind, "clock");
        assert_eq!(a.str("format"), Some("%H:%M"));
        assert!(a.bool_or("seconds", false));
    }

    #[test]
    fn palette_vars() {
        let v = Appearance::default().mss_variables();
        assert!(v.contains("--accent: #3d8bfd"));
    }

    #[test]
    fn theme_palette_and_overrides() {
        let c = Config::parse("[appearance]\ntheme = \"nord\"\n").unwrap();
        let a = &c.appearance;
        assert!(a.resolved.is_some());
        let p = a.palette();
        assert_eq!(p.bg.hex(), "#2e3440");
        assert_eq!(p.accent.hex(), "#88c0d0");
        assert_eq!(p.accent_fg.hex(), "#2e3440");
        assert!(a.wallpaper_background(&c.wallpaper).contains("#5e81ac"));
        assert!(a.theme_mss(false).contains(".panel"));
        assert!(a.theme_mss_variables().contains("--frost:"));
        // Акцент пользователя и `[appearance.colors]` — поверх темы.
        let c = Config::parse(
            "[appearance]\ntheme = \"nord\"\naccent = \"#ff0000\"\n[appearance.colors]\nbg = \"#000000\"\n",
        )
        .unwrap();
        let p = c.appearance.palette();
        assert_eq!(p.accent.hex(), "#ff0000");
        assert_eq!(p.accent_fg.hex(), "#ffffff", "текст на своём акценте — по контрасту, не из темы");
        assert_eq!(p.bg.hex(), "#000000");
        assert_eq!(p.surface.hex(), "#3b4252");
    }

    #[test]
    fn theme_variant_and_titlebar() {
        // Тема с одним вариантом навязывает свою схему.
        let c = Config::parse("[appearance]\ntheme = \"dracula\"\ncolor_scheme = \"light\"\n").unwrap();
        assert!(c.appearance.is_dark());
        let c = Config::parse("[appearance]\ntheme = \"nord\"\ncolor_scheme = \"light\"\n").unwrap();
        assert!(!c.appearance.is_dark());
        assert_eq!(c.appearance.palette().bg.hex(), "#e5e9f0");
        let (active, inactive) = c.appearance.titlebar_colors(&c.decorations);
        assert_eq!((active.hex().as_str(), inactive.hex().as_str()), ("#eceff4", "#e5e9f0"));
        // Явный цвет заголовка сильнее темы.
        let c = Config::parse("[appearance]\ntheme = \"nord\"\n[decorations]\nactive_color = \"accent\"\n").unwrap();
        assert_eq!(c.appearance.titlebar_colors(&c.decorations).0.hex(), "#88c0d0");
    }

    #[test]
    fn missing_theme_falls_back() {
        let c = Config::parse("[appearance]\ntheme = \"no-such-theme\"\n").unwrap();
        assert!(c.appearance.resolved.is_none());
        assert_eq!(c.appearance.palette(), Appearance::default().palette());
        assert!(c.appearance.wallpaper_background(&c.wallpaper).contains("#1b2233"));
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;
    #[test]
    fn glob() {
        assert!(glob_match("Virtual-*", "Virtual-1"));
        assert!(!glob_match("Virtual-*", "DSI-1"));
        assert!(glob_match("DSI-1", "DSI-1"));
        assert!(glob_match("*-1", "DSI-1"));
        assert!(!glob_match("*-2", "DSI-1"));
    }
    #[test]
    fn phone_defaults() {
        let mut c = Config::default();
        c.apply_form_factor(FormFactor::Phone);
        assert_eq!(c.general.shell, "synmobile-shell");
        assert_eq!(c.windows.border_width, 0);
        let mut c = Config::default();
        c.general.shell = "my-shell".into();
        c.apply_form_factor(FormFactor::Phone);
        assert_eq!(c.general.shell, "my-shell");
    }
}
