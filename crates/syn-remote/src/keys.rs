//! Кнопки пульта: имена из базы («Vol_up», «VOLUME+», «ch_next»…) сводятся к общим ключам, по которым
//! пульт раскладывается на экране и выбирается кнопка для проверки при подборе.

use crate::icons;
use crate::ir::Signal;

/// Общий ключ кнопки по её имени в базе; `None` — своя, без места в раскладке.
pub fn key_of(name: &str) -> Option<&'static str> {
    let n: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '+' { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let k = match n.as_str() {
        "power" | "pwr" | "power_toggle" | "on_off" | "onoff" | "standby" | "power_onoff" | "power_on_off" | "pow" => "power",
        "on" | "power_on" | "poweron" | "pwr_on" => "power_on",
        "off" | "power_off" | "poweroff" | "pwr_off" => "power_off",
        "vol_up" | "volup" | "vol+" | "volume_up" | "vol_plus" | "volumeup" | "volume+" | "v+" | "vol_u" => "vol_up",
        "vol_dn" | "vol_down" | "voldown" | "voldn" | "vol" | "volume_down" | "vol_minus" | "volumedown" | "v" | "vol_d" => "vol_dn",
        "mute" | "muting" | "vol_mute" | "sound_off" => "mute",
        "ch_next" | "ch_up" | "chup" | "ch+" | "channel_up" | "ch_plus" | "prog+" | "p+" | "program_up" | "chan_up" | "channel+" | "pr+" => "ch_up",
        "ch_prev" | "ch_down" | "ch_dn" | "chdown" | "ch" | "channel_down" | "ch_minus" | "p" | "prog" | "program_down" | "chan_down" | "pr" => "ch_dn",
        "up" | "arrow_up" | "cursor_up" | "nav_up" | "dpad_up" | "menu_up" => "up",
        "down" | "arrow_down" | "cursor_down" | "nav_down" | "dpad_down" | "menu_down" | "dn" => "down",
        "left" | "arrow_left" | "cursor_left" | "nav_left" | "dpad_left" | "menu_left" => "left",
        "right" | "arrow_right" | "cursor_right" | "nav_right" | "dpad_right" | "menu_right" => "right",
        "ok" | "enter" | "select" | "confirm" | "center" | "dpad_center" | "ok_enter" => "ok",
        "back" | "return" | "ret" | "previous_menu" => "back",
        "exit" | "quit" => "exit",
        "home" | "smart" | "smart_hub" | "smarthub" | "smart_tv" | "my_apps" => "home",
        "menu" | "setup" | "main_menu" => "menu",
        "settings" | "setting" | "quick_settings" | "quick" | "tools" | "options" | "option" => "settings",
        "source" | "input" | "src" | "av" | "tv_av" | "input_source" | "ext" | "inputs" | "av_tv" | "tv_video" => "source",
        "info" | "display" | "disp" | "information" => "info",
        "guide" | "epg" | "tv_guide" => "guide",
        "play" => "play",
        "pause" => "pause",
        "play_pause" | "playpause" | "play_stop" => "play_pause",
        "stop" => "stop",
        "rewind" | "rew" | "fast_backward" | "fr" | "rwd" | "fast_rewind" => "rewind",
        "ffwd" | "ff" | "ffd" | "fast_forward" | "forward" | "fwd" | "fast_fwd" => "ffwd",
        "next" | "skip_next" | "skip_fwd" | "skip" | "next_track" => "next",
        "prev" | "previous" | "skip_prev" | "skip_back" | "prev_track" => "prev",
        "rec" | "record" => "record",
        "red" | "red_1" => "red",
        "green" | "green_2" => "green",
        "yellow" | "yellow_3" => "yellow",
        "blue" | "blue_4" => "blue",
        "netflix" => "netflix",
        "youtube" => "youtube",
        "prime" | "prime_video" | "amazon" => "prime",
        "subtitle" | "subtitles" | "cc" | "sub" | "subt" => "subtitle",
        "list" | "ch_list" | "channel_list" => "list",
        "sleep" | "timer" => "sleep",
        "0" | "num_0" | "digit_0" | "key_0" | "btn_0" => "0",
        "1" | "num_1" | "digit_1" | "key_1" | "btn_1" => "1",
        "2" | "num_2" | "digit_2" | "key_2" | "btn_2" => "2",
        "3" | "num_3" | "digit_3" | "key_3" | "btn_3" => "3",
        "4" | "num_4" | "digit_4" | "key_4" | "btn_4" => "4",
        "5" | "num_5" | "digit_5" | "key_5" | "btn_5" => "5",
        "6" | "num_6" | "digit_6" | "key_6" | "btn_6" => "6",
        "7" | "num_7" | "digit_7" | "key_7" | "btn_7" => "7",
        "8" | "num_8" | "digit_8" | "key_8" | "btn_8" => "8",
        "9" | "num_9" | "digit_9" | "key_9" | "btn_9" => "9",
        "temp_up" | "temp+" | "temperature_up" | "tempup" => "temp_up",
        "temp_dn" | "temp_down" | "temp" | "temperature_down" | "tempdown" => "temp_dn",
        "mode" => "mode",
        "fan" | "fan_speed" | "speed" => "fan",
        "swing" | "swing_on" => "swing",
        _ => return None,
    };
    Some(k)
}

/// Кнопка с данным ключом.
pub fn find<'a>(signals: &'a [Signal], key: &str) -> Option<&'a Signal> {
    signals.iter().find(|s| key_of(&s.name) == Some(key))
}

/// Кнопка питания: «Power», иначе «On»/«Off» (у кондиционеров чаще всего «Off» и режимы).
pub fn power(signals: &[Signal]) -> Option<&Signal> {
    find(signals, "power").or_else(|| find(signals, "power_off")).or_else(|| find(signals, "power_on")).or_else(|| signals.first())
}

/// Только настоящая кнопка питания (без запасного «первая попавшаяся»).
pub fn power_strict(signals: &[Signal]) -> Option<&Signal> {
    find(signals, "power").or_else(|| find(signals, "power_off")).or_else(|| find(signals, "power_on"))
}

/// Значок кнопки для раскладки, если есть.
pub fn glyph(key: &str) -> Option<&'static str> {
    Some(match key {
        "power" | "power_on" | "power_off" => icons::POWER,
        "vol_up" => icons::VOL_UP,
        "vol_dn" => icons::VOL_DOWN,
        "mute" => icons::MUTE,
        "ch_up" | "up" => icons::UP,
        "ch_dn" | "down" => icons::DOWN,
        "left" => icons::LEFT,
        "right" => icons::RIGHT,
        "back" => icons::BACK,
        "exit" => icons::CLOSE,
        "home" => icons::HOME,
        "menu" => icons::MENU,
        "settings" => icons::SETTINGS,
        "source" => icons::INPUT,
        "info" => icons::INFO,
        "guide" => icons::GUIDE,
        "play" => icons::PLAY,
        "pause" => icons::PAUSE,
        "play_pause" => icons::PLAY_PAUSE,
        "stop" => icons::STOP,
        "rewind" => icons::REWIND,
        "ffwd" => icons::FFWD,
        "next" => icons::NEXT,
        "prev" => icons::PREV,
        "record" => icons::RECORD,
        "subtitle" => icons::SUBTITLE,
        "list" => icons::LIST,
        "sleep" => icons::SLEEP,
        "temp_up" => icons::ADD,
        "temp_dn" => icons::REMOVE,
        "fan" => icons::FAN,
        "mode" => icons::TUNE,
        "swing" => icons::SWING,
        _ => return None,
    })
}

/// Подпись кнопки по имени из базы: «Vol_up» → «Vol up», «COOL_HI» → «Cool hi».
pub fn pretty(name: &str) -> String {
    let s = name.replace(['_', '-'], " ");
    let s = s.trim();
    let lower = s.chars().any(|c| c.is_lowercase());
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if lower {
            out.push(c);
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Ключи, которые повторяются при удержании.
pub fn repeats(key: Option<&str>) -> bool {
    matches!(key, Some("vol_up" | "vol_dn" | "ch_up" | "ch_dn" | "up" | "down" | "left" | "right" | "rewind" | "ffwd" | "temp_up" | "temp_dn"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(key_of("Vol_up"), Some("vol_up"));
        assert_eq!(key_of("VOL-"), Some("vol_dn"));
        assert_eq!(key_of("Ch_next"), Some("ch_up"));
        assert_eq!(key_of("POWER"), Some("power"));
        assert_eq!(key_of("Ok"), Some("ok"));
        assert_eq!(key_of("Cool_hi"), None);
        assert_eq!(pretty("COOL_HI"), "Cool hi");
        assert_eq!(pretty("Vol_up"), "Vol up");
    }
}
