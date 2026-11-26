//! Кодовые точки Material Icons, которые рисует сам редактор (кнопки нод,
//! палитра, панель масштаба). Шрифт подключает программа:
//! `App::with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)`.
//! Значки своих видов нод программа задаёт в `NodeKindMeta::icon`.

#![allow(dead_code)]

pub const MI_ADD: &str                 = "\u{E145}";
pub const MI_APPS: &str                = "\u{E5C3}";
pub const MI_ARTICLE: &str             = "\u{EF42}";
pub const MI_ASPECT_RATIO: &str        = "\u{E85B}";
pub const MI_AUDIOTRACK: &str          = "\u{E3A1}"; // .mp3/.wav/.flac — audiotrack
pub const MI_AUTO_AWESOME: &str        = "\u{E65F}";
pub const MI_AUTORENEW: &str           = "\u{E863}";
pub const MI_BLUR_ON: &str              = "\u{E3A5}"; // blur_on (для Reverb)
pub const MI_CHECK: &str               = "\u{E5CA}";
pub const MI_CHEVRON_RIGHT: &str       = "\u{E5CC}";
pub const MI_CIRCLE: &str              = "\u{EF4A}";
pub const MI_CLOSE: &str               = "\u{E5CD}";
pub const MI_CONTENT_COPY: &str        = "\u{E14D}";
pub const MI_CROP_SQUARE: &str         = "\u{E3B6}";
pub const MI_EDIT_NOTE: &str           = "\u{E745}";
pub const MI_EXPAND_MORE: &str         = "\u{E5CF}";
pub const MI_FILTER_ALT: &str          = "\u{EF4F}"; // filter_alt
pub const MI_FIT_SCREEN: &str          = "\u{EA10}";
pub const MI_FOLDER_OPEN: &str         = "\u{E2C8}";
pub const MI_GRAPHIC_EQ: &str          = "\u{E1B8}";
pub const MI_GROUPS: &str              = "\u{F233}";
pub const MI_HELP_OUTLINE: &str        = "\u{E8FD}";
pub const MI_HUB: &str                 = "\u{E9F4}";
pub const MI_IMAGE_ICON: &str          = "\u{E3F4}";
pub const MI_INVENTORY_2: &str         = "\u{E1A1}";
pub const MI_LANGUAGE: &str            = "\u{E894}";
pub const MI_LAYERS: &str              = "\u{E53B}";
pub const MI_LIBRARY_MUSIC: &str       = "\u{E030}";
pub const MI_MERGE_TYPE: &str          = "\u{E252}";
pub const MI_MIC: &str                 = "\u{E029}";
pub const MI_MOVIE: &str               = "\u{E02C}"; // .mp4/.mov/.mkv — movie
pub const MI_PALETTE: &str             = "\u{E40A}";
pub const MI_PAUSE: &str               = "\u{E034}";
pub const MI_PLAY_ARROW: &str          = "\u{E037}";
pub const MI_POWER_SETTINGS: &str      = "\u{E8AC}";
pub const MI_PSYCHOLOGY: &str          = "\u{EA4A}";
pub const MI_RECORD_VOICE_OVER: &str   = "\u{E63C}";
pub const MI_REMOVE_CIRCLE_OUTLINE: &str = "\u{E15D}";
pub const MI_SAVE: &str                = "\u{E161}";
pub const MI_STOP: &str                = "\u{E047}";
pub const MI_TIMER: &str               = "\u{E425}";
pub const MI_TRANSLATE: &str           = "\u{E8E2}";
pub const MI_TUNE: &str                = "\u{E429}";
pub const MI_VOLUME_UP: &str           = "\u{E050}";
pub const MI_WB_SHADE: &str            = "\u{E43E}";
pub const MI_ZOOM_IN: &str             = "\u{E8FF}";
pub const MI_ZOOM_OUT: &str            = "\u{E900}";
