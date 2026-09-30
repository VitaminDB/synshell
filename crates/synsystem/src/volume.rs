//! Громкость (PipeWire `wpctl` или PulseAudio `pactl`).

use crate::util::{output, which};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// Проценты (может быть > 100).
    pub percent: u32,
    pub muted: bool,
    pub mic_muted: bool,
}

pub fn read() -> Option<Volume> {
    if which("wpctl") {
        let s = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
        // «Volume: 0.45 [MUTED]»
        let v: f32 = s.split_whitespace().nth(1)?.parse().ok()?;
        let mic = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).unwrap_or_default();
        return Some(Volume { percent: (v * 100.0).round() as u32, muted: s.contains("MUTED"), mic_muted: mic.contains("MUTED") });
    }
    if which("pactl") {
        let s = output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let pct = s.split('/').nth(1)?.trim().trim_end_matches('%').trim().parse().ok()?;
        let m = output("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).unwrap_or_default();
        let mm = output("pactl", &["get-source-mute", "@DEFAULT_SOURCE@"]).unwrap_or_default();
        return Some(Volume { percent: pct, muted: m.contains("yes"), mic_muted: mm.contains("yes") });
    }
    None
}

/// Изменить на `delta` процентов или выставить `set`, не выше `max` процентов (блокирует — звать из
/// фонового потока). На телефоне предел 100: выше PipeWire усиливает s16 с клиппингом — хрип динамиков.
pub fn change(delta: i32, set: Option<u32>, max: u32) {
    let set = set.map(|v| v.min(max));
    if which("wpctl") {
        let limit = format!("{:.2}", max as f32 / 100.0);
        let arg = match set {
            Some(v) => format!("{v}%"),
            None if delta >= 0 => format!("{delta}%+"),
            None => format!("{}%-", -delta),
        };
        let _ = Command::new("wpctl").args(["set-volume", "-l", &limit, "@DEFAULT_AUDIO_SINK@", &arg]).status();
        if delta > 0 {
            let _ = Command::new("wpctl").args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]).status();
        }
    } else if which("pactl") {
        let arg = match set {
            Some(v) => format!("{v}%"),
            None => format!("{delta:+}%"),
        };
        let _ = Command::new("pactl").args(["set-sink-volume", "@DEFAULT_SINK@", &arg]).status();
    }
}

pub fn toggle_mute(mic: bool) {
    if which("wpctl") {
        let dev = if mic { "@DEFAULT_AUDIO_SOURCE@" } else { "@DEFAULT_AUDIO_SINK@" };
        let _ = Command::new("wpctl").args(["set-mute", dev, "toggle"]).status();
    } else if which("pactl") {
        let (cmd, dev) = if mic { ("set-source-mute", "@DEFAULT_SOURCE@") } else { ("set-sink-mute", "@DEFAULT_SINK@") };
        let _ = Command::new("pactl").args([cmd, dev, "toggle"]).status();
    }
}
