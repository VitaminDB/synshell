//! Действия рабочего стола. Одна и та же строка работает в сочетаниях клавиш
//! (`[keybindings] "Super+Return" = "spawn konsole"`), в кнопках панели, в
//! правилах и в `synwm msg action ...`.
//!
//! Строковая форма: имя действия и аргументы через пробел. Сериализуется в
//! TOML/JSON строкой — конфиг читается человеком.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Мини-замена bitflags без внешней зависимости.
macro_rules! bitflags_lite {
    ($name:ident : $t:ty { $($flag:ident = $v:expr,)* }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub $t);
        #[allow(dead_code)]
        impl $name {
            $(pub const $flag: $name = $name($v);)*
            pub const fn empty() -> Self { $name(0) }
            pub const fn bits(self) -> $t { self.0 }
            pub const fn contains(self, o: Self) -> bool { self.0 & o.0 == o.0 }
            pub const fn is_empty(self) -> bool { self.0 == 0 }
        }
        impl std::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, o: Self) -> Self { $name(self.0 | o.0) }
        }
        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, o: Self) { self.0 |= o.0 }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

impl FromStr for Direction {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "left" | "l" => Direction::Left,
            "right" | "r" => Direction::Right,
            "up" | "u" | "top" => Direction::Up,
            "down" | "d" | "bottom" => Direction::Down,
            _ => return Err(format!("неизвестное направление «{s}»")),
        })
    }
}

/// Режим окон на телефоне.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MobileMode {
    /// Свободные окна на большом виртуальном столе.
    Free,
    /// Каждое приложение во весь экран, листание влево-вправо.
    #[default]
    Pages,
}

impl MobileMode {
    pub const ALL: [MobileMode; 2] = [MobileMode::Pages, MobileMode::Free];

    pub fn as_str(self) -> &'static str {
        match self {
            MobileMode::Free => "free",
            MobileMode::Pages => "pages",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl FromStr for MobileMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        // `tiles` (режим плиток убран) — старые конфиги открываются страницами.
        if s == "tiles" {
            return Ok(MobileMode::Pages);
        }
        Self::ALL.into_iter().find(|m| m.as_str() == s).ok_or_else(|| format!("неизвестный режим окон «{s}» (pages, free)"))
    }
}

impl Serialize for MobileMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MobileMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Поворот встроенного экрана относительно его естественной ориентации
/// (`rotate normal|90|180|270`; по часовой стрелке, как `transform` вывода).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Rotation {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "90")]
    R90,
    #[serde(rename = "180")]
    R180,
    #[serde(rename = "270")]
    R270,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::Normal, Rotation::R90, Rotation::R180, Rotation::R270];

    pub fn as_str(self) -> &'static str {
        match self {
            Rotation::Normal => "normal",
            Rotation::R90 => "90",
            Rotation::R180 => "180",
            Rotation::R270 => "270",
        }
    }

    /// Четверти оборота по часовой стрелке.
    pub fn quarters(self) -> u8 {
        self as u8
    }

    pub fn from_quarters(q: u8) -> Self {
        Self::ALL[(q % 4) as usize]
    }
}

impl FromStr for Rotation {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s.trim() {
            "normal" | "0" => Rotation::Normal,
            "90" => Rotation::R90,
            "180" => Rotation::R180,
            "270" => Rotation::R270,
            o => return Err(format!("rotate: ожидалось normal/90/180/270, а не «{o}»")),
        })
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Куда листать страницы приложений (`page home|next|prev|N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageTarget {
    /// Домашний экран (страница 0).
    Home,
    Next,
    Prev,
    Index(u32),
}

impl FromStr for PageTarget {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "home" | "0" => PageTarget::Home,
            "next" => PageTarget::Next,
            "prev" | "previous" => PageTarget::Prev,
            n => PageTarget::Index(n.parse().map_err(|_| format!("page: ожидалось home/next/prev/номер, а не «{n}»"))?),
        })
    }
}

impl fmt::Display for PageTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageTarget::Home => f.write_str("home"),
            PageTarget::Next => f.write_str("next"),
            PageTarget::Prev => f.write_str("prev"),
            PageTarget::Index(i) => write!(f, "{i}"),
        }
    }
}

/// Раскладка окон на рабочем столе.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutKind {
    /// Свободные окна (как в Plasma/Windows).
    #[default]
    Floating,
    /// Мастер слева + стопка справа (dwm/xmonad).
    Tile,
    /// Равные колонки.
    Columns,
    /// Сетка.
    Grid,
    /// Одно окно на весь экран, остальные за ним.
    Monocle,
    /// Полосы друг под другом: первая — мастер.
    Rows,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 6] = [
        LayoutKind::Floating,
        LayoutKind::Tile,
        LayoutKind::Columns,
        LayoutKind::Grid,
        LayoutKind::Monocle,
        LayoutKind::Rows,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LayoutKind::Floating => "floating",
            LayoutKind::Tile => "tile",
            LayoutKind::Columns => "columns",
            LayoutKind::Grid => "grid",
            LayoutKind::Monocle => "monocle",
            LayoutKind::Rows => "rows",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl FromStr for LayoutKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|l| l.as_str() == s)
            .ok_or_else(|| format!("неизвестная раскладка «{s}»"))
    }
}

impl Serialize for LayoutKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LayoutKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Цель «рабочий стол»: номер (с 1), соседний или предыдущий активный.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceTarget {
    Index(u32),
    Next,
    Prev,
    /// Последний активный до текущего (как Alt+Tab для столов).
    Last,
}

impl FromStr for WorkspaceTarget {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "next" => WorkspaceTarget::Next,
            "prev" | "previous" => WorkspaceTarget::Prev,
            "last" => WorkspaceTarget::Last,
            n => WorkspaceTarget::Index(
                n.parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| format!("ожидался номер стола (с 1) или next/prev/last, а не «{n}»"))?,
            ),
        })
    }
}

impl fmt::Display for WorkspaceTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceTarget::Index(n) => write!(f, "{n}"),
            WorkspaceTarget::Next => f.write_str("next"),
            WorkspaceTarget::Prev => f.write_str("prev"),
            WorkspaceTarget::Last => f.write_str("last"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Запустить команду через `sh -c`.
    Spawn(String),
    /// Закрыть окно в фокусе (вежливо, xdg close).
    Close,
    /// Убить процесс окна в фокусе.
    Kill,
    ToggleFloating,
    ToggleFullscreen,
    ToggleMaximize,
    Minimize,
    /// Свернуть все окна — рабочий стол (жест «домой» как в Android).
    MinimizeAll,
    /// Прикрепить окно ко всем столам.
    ToggleSticky,
    /// Держать окно поверх остальных.
    ToggleAlwaysOnTop,
    /// Прилепить окно к половине/четверти экрана (Super+стрелки в Plasma).
    Snap(Direction),
    /// Отцентрировать плавающее окно.
    Center,
    Focus(Direction),
    Move(Direction),
    /// Следующее/предыдущее окно в порядке фокуса (Alt+Tab).
    FocusNext,
    FocusPrev,
    Workspace(WorkspaceTarget),
    MoveToWorkspace(WorkspaceTarget),
    /// Перенести окно и уйти за ним.
    MoveToWorkspaceFollow(WorkspaceTarget),
    FocusOutput(Direction),
    MoveToOutput(Direction),
    Layout(LayoutKind),
    CycleLayout,
    /// Изменить долю мастер-области (±0.05 и т.п.).
    MasterRatio(f32),
    /// Изменить число окон в мастер-области.
    MasterCount(i32),
    /// Следующая раскладка клавиатуры.
    KeyboardLayoutNext,
    /// Раскладка клавиатуры по номеру (с 0).
    KeyboardLayout(u32),
    /// Снимок экрана (весь вывод под курсором) в `~/Pictures/Screenshots`.
    Screenshot,
    /// Снимок окна в фокусе.
    ScreenshotWindow,
    /// Снимок с выбором: экран застывает, открывается `synshot`
    /// (выделить область или окно, сохранить, скопировать).
    ScreenshotInteractive,
    /// Обзор всех окон (как «Обзор» в Plasma).
    Overview,
    ReloadConfig,
    /// Перезапустить оболочку (панели, меню) — окна не трогаются.
    RestartShell,
    /// Перезапустить композитор новым бинарником, не выходя из сеанса.
    /// Окна программ при этом закрываются.
    Restart,
    /// Выйти из сеанса.
    Quit,
    /// Заблокировать экран.
    Lock,
    Suspend,
    Reboot,
    PowerOff,
    /// Выключить/включить мониторы (DPMS).
    PowerOffMonitors,
    /// Команда оболочке (панели): `launcher`, `power-menu`, `run`,
    /// `notifications`, `volume +5`, `brightness -5`, `mute`… Композитор её
    /// не разбирает, а пересылает событием `ShellCommand`.
    Shell(String),
    /// «Назад» (жест от края): закрыть оверлей оболочки, иначе окну —
    /// клавиша `[mobile] back_key`.
    Back,
    /// Нажать клавишу в окне с фокусом: `key XF86Back`, `key Alt+Left`.
    Key(KeyCombo),
    /// Режим окон на телефоне.
    MobileMode(MobileMode),
    MobileModeCycle,
    /// Страница приложений (режим `pages`).
    Page(PageTarget),
    /// Вернуть виртуальный стол (режим `free`) к началу.
    CameraHome,
    /// Повернуть встроенный экран (поверх `transform` из `[[output]]`);
    /// автоповорот по датчику — у оболочки (`[rotation]`).
    Rotate(Rotation),
    /// Кнопка питания телефона: экран включён — заблокировать и погасить,
    /// погашен — включить (на экран блокировки).
    ScreenToggle,
    /// Явно погасить экран с блокировкой (как кнопка питания при горящем экране);
    /// уже погашен — ничего. Для скриптов и удалённого управления, где
    /// переключатель опасен: не знаешь, в каком состоянии экран.
    ScreenOff,
    /// Явно включить экран (на экран блокировки, если заблокирован); горит — ничего.
    ScreenOn,
    /// Ничего не делать (снять сочетание по умолчанию).
    None,
}

impl Action {
    /// Имена всех действий — для автодополнения в настройках.
    pub const NAMES: &'static [&'static str] = &[
        "spawn", "close", "kill", "toggle-floating", "toggle-fullscreen", "toggle-maximize",
        "minimize", "minimize-all", "toggle-sticky", "toggle-always-on-top", "snap", "center", "focus", "move",
        "focus-next", "focus-prev", "workspace", "move-to-workspace", "move-to-workspace-follow",
        "focus-output", "move-to-output", "layout", "cycle-layout", "master-ratio",
        "master-count", "keyboard-layout-next", "keyboard-layout", "screenshot",
        "screenshot-window", "screenshot-interactive", "overview", "reload-config", "quit", "lock", "suspend", "reboot",
        "poweroff", "monitors-off", "shell", "back", "key", "mobile-mode", "mobile-mode-cycle", "page",
        "camera-home", "rotate", "screen-toggle", "screen-off", "screen-on", "none",
    ];
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (name, rest) = match s.split_once(char::is_whitespace) {
            Some((n, r)) => (n, r.trim()),
            None => (s, ""),
        };
        let need = |what: &str| -> Result<&str, String> {
            if rest.is_empty() {
                Err(format!("действию «{name}» нужен аргумент: {what}"))
            } else {
                Ok(rest)
            }
        };
        Ok(match name {
            "spawn" | "exec" => Action::Spawn(need("команда")?.to_string()),
            "close" => Action::Close,
            "kill" => Action::Kill,
            "toggle-floating" => Action::ToggleFloating,
            "toggle-fullscreen" | "fullscreen" => Action::ToggleFullscreen,
            "toggle-maximize" | "maximize" => Action::ToggleMaximize,
            "minimize" => Action::Minimize,
            "minimize-all" | "show-desktop" => Action::MinimizeAll,
            "toggle-sticky" | "sticky" => Action::ToggleSticky,
            "toggle-always-on-top" | "always-on-top" => Action::ToggleAlwaysOnTop,
            "snap" => Action::Snap(need("left/right/up/down")?.parse()?),
            "center" => Action::Center,
            "focus" => Action::Focus(need("left/right/up/down")?.parse()?),
            "move" => Action::Move(need("left/right/up/down")?.parse()?),
            "focus-next" => Action::FocusNext,
            "focus-prev" => Action::FocusPrev,
            "workspace" => Action::Workspace(need("номер стола")?.parse()?),
            "move-to-workspace" => Action::MoveToWorkspace(need("номер стола")?.parse()?),
            "move-to-workspace-follow" => {
                Action::MoveToWorkspaceFollow(need("номер стола")?.parse()?)
            }
            "focus-output" => Action::FocusOutput(need("направление")?.parse()?),
            "move-to-output" => Action::MoveToOutput(need("направление")?.parse()?),
            "layout" => Action::Layout(need("floating/tile/columns/grid/monocle")?.parse()?),
            "cycle-layout" => Action::CycleLayout,
            "master-ratio" => Action::MasterRatio(
                need("шаг, например +0.05")?
                    .parse()
                    .map_err(|_| format!("master-ratio: ожидалось число, а не «{rest}»"))?,
            ),
            "master-count" => Action::MasterCount(
                need("шаг, например +1")?
                    .parse()
                    .map_err(|_| format!("master-count: ожидалось целое, а не «{rest}»"))?,
            ),
            "keyboard-layout-next" => Action::KeyboardLayoutNext,
            "keyboard-layout" => Action::KeyboardLayout(
                need("номер раскладки")?
                    .parse()
                    .map_err(|_| format!("keyboard-layout: ожидался номер, а не «{rest}»"))?,
            ),
            "screenshot" => Action::Screenshot,
            "screenshot-window" => Action::ScreenshotWindow,
            "screenshot-interactive" => Action::ScreenshotInteractive,
            "overview" => Action::Overview,
            "reload-config" => Action::ReloadConfig,
            "restart-shell" => Action::RestartShell,
            "restart" => Action::Restart,
            "quit" | "logout" | "exit" => Action::Quit,
            "lock" => Action::Lock,
            "suspend" => Action::Suspend,
            "reboot" => Action::Reboot,
            "poweroff" | "shutdown" => Action::PowerOff,
            "monitors-off" => Action::PowerOffMonitors,
            "screen-toggle" => Action::ScreenToggle,
            "screen-off" => Action::ScreenOff,
            "screen-on" => Action::ScreenOn,
            "shell" => Action::Shell(need("команда оболочки")?.to_string()),
            "back" => Action::Back,
            "key" => Action::Key(need("клавиша, например XF86Back или Alt+Left")?.parse()?),
            "mobile-mode" => Action::MobileMode(need("pages/free")?.parse()?),
            "mobile-mode-cycle" => Action::MobileModeCycle,
            "page" => Action::Page(need("home/next/prev/номер")?.parse()?),
            "camera-home" => Action::CameraHome,
            "rotate" => Action::Rotate(need("normal/90/180/270")?.parse()?),
            "none" | "" => Action::None,
            other => return Err(format!("неизвестное действие «{other}»")),
        })
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Spawn(c) => write!(f, "spawn {c}"),
            Action::Close => f.write_str("close"),
            Action::Kill => f.write_str("kill"),
            Action::ToggleFloating => f.write_str("toggle-floating"),
            Action::ToggleFullscreen => f.write_str("toggle-fullscreen"),
            Action::ToggleMaximize => f.write_str("toggle-maximize"),
            Action::Minimize => f.write_str("minimize"),
            Action::MinimizeAll => f.write_str("minimize-all"),
            Action::ToggleSticky => f.write_str("toggle-sticky"),
            Action::ToggleAlwaysOnTop => f.write_str("toggle-always-on-top"),
            Action::Snap(d) => write!(f, "snap {}", d.as_str()),
            Action::Center => f.write_str("center"),
            Action::Focus(d) => write!(f, "focus {}", d.as_str()),
            Action::Move(d) => write!(f, "move {}", d.as_str()),
            Action::FocusNext => f.write_str("focus-next"),
            Action::FocusPrev => f.write_str("focus-prev"),
            Action::Workspace(t) => write!(f, "workspace {t}"),
            Action::MoveToWorkspace(t) => write!(f, "move-to-workspace {t}"),
            Action::MoveToWorkspaceFollow(t) => write!(f, "move-to-workspace-follow {t}"),
            Action::FocusOutput(d) => write!(f, "focus-output {}", d.as_str()),
            Action::MoveToOutput(d) => write!(f, "move-to-output {}", d.as_str()),
            Action::Layout(l) => write!(f, "layout {}", l.as_str()),
            Action::CycleLayout => f.write_str("cycle-layout"),
            Action::MasterRatio(r) => write!(f, "master-ratio {r:+}"),
            Action::MasterCount(c) => write!(f, "master-count {c:+}"),
            Action::KeyboardLayoutNext => f.write_str("keyboard-layout-next"),
            Action::KeyboardLayout(i) => write!(f, "keyboard-layout {i}"),
            Action::Screenshot => f.write_str("screenshot"),
            Action::ScreenshotWindow => f.write_str("screenshot-window"),
            Action::ScreenshotInteractive => f.write_str("screenshot-interactive"),
            Action::Overview => f.write_str("overview"),
            Action::ReloadConfig => f.write_str("reload-config"),
            Action::RestartShell => f.write_str("restart-shell"),
            Action::Restart => f.write_str("restart"),
            Action::Quit => f.write_str("quit"),
            Action::Lock => f.write_str("lock"),
            Action::Suspend => f.write_str("suspend"),
            Action::Reboot => f.write_str("reboot"),
            Action::PowerOff => f.write_str("poweroff"),
            Action::PowerOffMonitors => f.write_str("monitors-off"),
            Action::ScreenToggle => f.write_str("screen-toggle"),
            Action::ScreenOff => f.write_str("screen-off"),
            Action::ScreenOn => f.write_str("screen-on"),
            Action::Shell(c) => write!(f, "shell {c}"),
            Action::Back => f.write_str("back"),
            Action::Key(k) => write!(f, "key {k}"),
            Action::MobileMode(m) => write!(f, "mobile-mode {}", m.as_str()),
            Action::MobileModeCycle => f.write_str("mobile-mode-cycle"),
            Action::Page(p) => write!(f, "page {p}"),
            Action::CameraHome => f.write_str("camera-home"),
            Action::Rotate(r) => write!(f, "rotate {r}"),
            Action::None => f.write_str("none"),
        }
    }
}

impl Serialize for Action {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Сочетание клавиш: модификаторы + имя клавиши (keysym по xkb, без учёта
/// регистра: `Return`, `a`, `F4`, `XF86AudioRaiseVolume`, `Print`) или кнопка
/// мыши (`BTN_LEFT`/`BTN_RIGHT`/`BTN_MIDDLE`) или колесо (`WheelUp`/`WheelDown`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub mods: Mods,
    /// Имя клавиши как в конфиге (регистр сохранён для сообщений).
    pub key: String,
}

bitflags_lite! {
    Mods: u8 {
        SUPER = 1,
        CTRL = 2,
        ALT = 4,
        SHIFT = 8,
    }
}

impl FromStr for KeyCombo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let mut mods = Mods::empty();
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let Some((key, mod_parts)) = parts.split_last() else {
            return Err("пустое сочетание".into());
        };
        // «Ctrl++» — плюс как клавиша.
        let key = if key.is_empty() && s.ends_with("++") { "plus" } else { key };
        for m in mod_parts {
            if m.is_empty() {
                continue;
            }
            mods |= Mods::parse(m).ok_or_else(|| format!("неизвестный модификатор «{m}» в «{s}»"))?;
        }
        if key.is_empty() {
            return Err(format!("в сочетании «{s}» нет клавиши"));
        }
        Ok(KeyCombo { mods, key: key.to_string() })
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (Mods::SUPER, "Super"),
            (Mods::CTRL, "Ctrl"),
            (Mods::ALT, "Alt"),
            (Mods::SHIFT, "Shift"),
        ] {
            if self.mods.contains(flag) {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

impl Mods {
    pub fn parse(s: &str) -> Option<Mods> {
        Some(match s.to_ascii_lowercase().as_str() {
            "super" | "mod4" | "logo" | "win" | "meta" => Mods::SUPER,
            "ctrl" | "control" => Mods::CTRL,
            "alt" | "mod1" => Mods::ALT,
            "shift" => Mods::SHIFT,
            _ => return None,
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for s in [
            "spawn konsole --new-tab",
            "workspace 3",
            "move-to-workspace next",
            "snap left",
            "layout tile",
            "master-ratio +0.05",
            "shell volume +5",
            "close",
            "back",
            "key Alt+Left",
            "key XF86Back",
            "mobile-mode free",
            "mobile-mode-cycle",
            "page next",
            "page 3",
            "layout rows",
            "camera-home",
        ] {
            let a: Action = s.parse().unwrap();
            assert_eq!(a.to_string(), s);
        }
    }

    #[test]
    fn combos() {
        let c: KeyCombo = "Super+Shift+Return".parse().unwrap();
        assert!(c.mods.contains(Mods::SUPER) && c.mods.contains(Mods::SHIFT));
        assert_eq!(c.key, "Return");
        assert_eq!(c.to_string(), "Super+Shift+Return");
        let c: KeyCombo = "Ctrl++".parse().unwrap();
        assert_eq!(c.key, "plus");
        assert!("Hyper+a".parse::<KeyCombo>().is_err());
    }
}
