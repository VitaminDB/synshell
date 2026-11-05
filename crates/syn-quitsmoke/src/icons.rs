//! Коды глифов Material Icons (шрифт бандлится фреймворком, грузится через
//! `App::with_icon_font`). Проверены по cmap `MaterialIcons-Regular.ttf`.

pub const HOME: &str = "\u{e88a}";
pub const STATS: &str = "\u{e26b}"; // bar_chart
pub const HEALTH: &str = "\u{e87d}"; // favorite
pub const AWARDS: &str = "\u{ea3f}"; // military_tech
pub const SETTINGS: &str = "\u{e8b8}";

pub const MONEY: &str = "\u{e2eb}"; // savings
pub const SMOKE_FREE: &str = "\u{eb4a}"; // smoke_free
pub const TIME: &str = "\u{e8b5}"; // schedule
pub const FIRE: &str = "\u{e80e}"; // whatshot

pub const LEAF: &str = "\u{ea35}"; // eco
pub const BELL: &str = "\u{e7f4}"; // notifications
pub const PARTY: &str = "\u{ea65}"; // celebration
pub const LOCK: &str = "\u{e897}";
pub const CHECK: &str = "\u{e5ca}";

pub const AIR: &str = "\u{efd8}";
pub const WALK: &str = "\u{e536}"; // directions_walk
pub const BOLT: &str = "\u{ea0b}";
pub const SUN: &str = "\u{e430}"; // wb_sunny
pub const CALENDAR: &str = "\u{e935}";
pub const DIAMOND: &str = "\u{ead5}"; // diamond
pub const STAR: &str = "\u{e838}";
pub const BRAIN: &str = "\u{ea4a}"; // psychology
pub const TROPHY: &str = "\u{ea23}"; // emoji_events
pub const SPA: &str = "\u{eb4c}"; // spa
pub const HEART: &str = "\u{e87d}"; // favorite
pub const HOURGLASS: &str = "\u{e88b}";

// Настройки
pub const MOON: &str = "\u{e51c}"; // dark_mode
pub const TRANSLATE: &str = "\u{e8e2}"; // translate
pub const DOLLAR: &str = "\u{e227}"; // attach_money
pub const TUNE: &str = "\u{e429}"; // tune
pub const TRASH: &str = "\u{e16c}"; // delete_sweep
pub const RESTART: &str = "\u{f053}"; // restart_alt
pub const ARROW_BACK: &str = "\u{e5c4}"; // arrow_back
pub const CALENDAR_TODAY: &str = "\u{e935}"; // calendar_today
pub const CLOCK: &str = "\u{e8b5}"; // schedule

// Vape
// TODO: проверить глиф по cmap MaterialIcons-Regular.ttf. Если недоступен —
// заменить на эмодзи 💨 или другой валидный глиф.
pub const VAPE: &str = "\u{e8db}"; // local_drink (приближённая иконка для вейпа)
