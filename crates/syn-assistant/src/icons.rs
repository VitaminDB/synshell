pub const MIC: &str = "\u{E029}";
pub const MIC_OFF: &str = "\u{E02B}";
pub const STOP: &str = "\u{E047}";
pub const VOLUME_UP: &str = "\u{E050}";
pub const VOLUME_OFF: &str = "\u{E04F}";
pub const SEND: &str = "\u{E163}";
pub const ADD: &str = "\u{E145}";
pub const DELETE: &str = "\u{E872}";
pub const MENU: &str = "\u{E5D2}";
pub const ATTACH: &str = "\u{E226}";
pub const COPY: &str = "\u{E14D}";
pub const REFRESH: &str = "\u{E5D5}";
pub const EDIT: &str = "\u{E3C9}";
pub const SEARCH: &str = "\u{E8B6}";
pub const IMAGE: &str = "\u{E3F4}";
pub const KEYBOARD: &str = "\u{E312}";
pub const SPARKLE: &str = "\u{E65F}";
pub const PSYCHOLOGY: &str = "\u{EA4A}";
pub const BOLT: &str = "\u{EA0B}";
pub const CHECK: &str = "\u{E5CA}";
pub const ERROR: &str = "\u{E000}";
pub const MORE: &str = "\u{E5D4}";
pub const FOLDER: &str = "\u{E2C7}";
pub const TERMINAL: &str = "\u{EB8E}";
pub const LANGUAGE: &str = "\u{E894}";
pub const WINDOW: &str = "\u{E069}";
pub const SETTINGS: &str = "\u{E8B8}";
pub const FILE: &str = "\u{E873}";
pub const DOWNLOAD: &str = "\u{E2C4}";
pub const SUMMARY: &str = "\u{F071}";
pub const VOICE_CHAT: &str = "\u{E62E}";
pub const CALL_END: &str = "\u{E0B1}";
pub const CHAT: &str = "\u{E0CA}";
pub const ARROW_BACK: &str = "\u{E5C4}";
pub const CLOUD: &str = "\u{E2BD}";
pub const MEMORY: &str = "\u{E322}";
pub const TUNE: &str = "\u{E429}";
pub const DONE: &str = "\u{E876}";
pub const SIDEBAR: &str = "\u{E5D2}";
pub const EXPAND_MORE: &str = "\u{E5CF}";

pub fn tool(name: &str) -> &'static str {
    match name {
        "system" => SETTINGS,
        "windows" => WINDOW,
        "files" => FOLDER,
        "bash" => TERMINAL,
        "web" => LANGUAGE,
        "media" => IMAGE,
        "subagent" => BOLT,
        _ => "\u{E869}",
    }
}
