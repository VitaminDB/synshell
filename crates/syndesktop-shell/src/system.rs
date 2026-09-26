//! Состояние системы для апплетов: громкость (PipeWire/PulseAudio),
//! батарея, сеть, загрузка процессора и памяти, яркость, медиаплееры.
//! Всё опрашивается в фоновых потоках; сигналы ставятся потокобезопасно.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::actions::{output, spawn, which};
use crate::ctx::ShellCtx;

#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// Проценты (может быть > 100).
    pub percent: u32,
    pub muted: bool,
    pub mic_muted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
    pub full: bool,
    /// Оценка до разряда/заряда, минуты.
    pub minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    /// `wifi`, `ethernet`, `none`.
    pub kind: String,
    pub connection: String,
    /// Сила сигнала Wi-Fi, 0..100.
    pub signal: Option<u32>,
    pub online: bool,
}

// ─── Громкость ───────────────────────────────────────────────────────────────

fn read_volume() -> Option<Volume> {
    if which("wpctl") {
        let s = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
        // «Volume: 0.45 [MUTED]»
        let v: f32 = s.split_whitespace().nth(1)?.parse().ok()?;
        let mic = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).unwrap_or_default();
        return Some(Volume {
            percent: (v * 100.0).round() as u32,
            muted: s.contains("MUTED"),
            mic_muted: mic.contains("MUTED"),
        });
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

pub fn refresh_volume(ctx: ShellCtx) -> Option<Volume> {
    let v = read_volume();
    ctx.volume.set(v.clone());
    v
}

/// Изменить громкость на `delta` процентов (или выставить при `set`).
pub fn change_volume(ctx: ShellCtx, delta: i32, set: Option<u32>) {
    std::thread::spawn(move || {
        if which("wpctl") {
            let arg = match set {
                Some(v) => format!("{v}%"),
                None if delta >= 0 => format!("{delta}%+"),
                None => format!("{}%-", -delta),
            };
            let _ = Command::new("wpctl")
                .args(["set-volume", "-l", "1.5", "@DEFAULT_AUDIO_SINK@", &arg])
                .status();
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
        if let Some(v) = refresh_volume(ctx) {
            if set.is_none() {
                let icon = if v.muted || v.percent == 0 { crate::ui::mi::VOLUME_OFF } else { crate::ui::mi::VOLUME_UP };
                crate::osd::show(ctx, icon, Some(v.percent), String::new());
            }
        }
    });
}

pub fn toggle_mute(ctx: ShellCtx, mic: bool) {
    std::thread::spawn(move || {
        if which("wpctl") {
            let dev = if mic { "@DEFAULT_AUDIO_SOURCE@" } else { "@DEFAULT_AUDIO_SINK@" };
            let _ = Command::new("wpctl").args(["set-mute", dev, "toggle"]).status();
        } else if which("pactl") {
            let (cmd, dev) = if mic { ("set-source-mute", "@DEFAULT_SOURCE@") } else { ("set-sink-mute", "@DEFAULT_SINK@") };
            let _ = Command::new("pactl").args([cmd, dev, "toggle"]).status();
        }
        if let Some(v) = refresh_volume(ctx) {
            let (icon, label) = if mic {
                if v.mic_muted {
                    (crate::ui::mi::MIC_OFF, "Микрофон выключен")
                } else {
                    (crate::ui::mi::MIC, "Микрофон включён")
                }
            } else if v.muted {
                (crate::ui::mi::VOLUME_OFF, "Звук выключен")
            } else {
                (crate::ui::mi::VOLUME_UP, "")
            };
            crate::osd::show(ctx, icon, if mic { None } else { Some(v.percent) }, label.into());
        }
    });
}

/// Следить за звуком: `pactl subscribe` сообщает об изменениях, иначе опрос.
fn watch_volume(ctx: ShellCtx) {
    refresh_volume(ctx);
    if which("pactl") {
        loop {
            let child = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            let Ok(mut child) = child else { break };
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if line.contains("sink") || line.contains("source") || line.contains("server") {
                        refresh_volume(ctx);
                    }
                }
            }
            let _ = child.wait();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(3));
        refresh_volume(ctx);
    }
}

// ─── Яркость ─────────────────────────────────────────────────────────────────

pub fn change_brightness(ctx: ShellCtx, delta: i32) {
    std::thread::spawn(move || {
        let pct = if which("brightnessctl") {
            let arg = if delta >= 0 { format!("{delta}%+") } else { format!("{}%-", -delta) };
            let _ = Command::new("brightnessctl").args(["-q", "set", &arg]).status();
            let cur: f32 = output("brightnessctl", &["get"]).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0);
            let max: f32 = output("brightnessctl", &["max"]).and_then(|s| s.trim().parse().ok()).unwrap_or(1.0);
            Some((cur / max.max(1.0) * 100.0).round() as u32)
        } else {
            sysfs_brightness(delta)
        };
        if let Some(p) = pct {
            crate::osd::show(ctx, crate::ui::mi::BRIGHTNESS, Some(p), String::new());
        }
    });
}

fn sysfs_brightness(delta: i32) -> Option<u32> {
    let dir = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
    let read = |f: &str| -> Option<i64> { std::fs::read_to_string(dir.join(f)).ok()?.trim().parse().ok() };
    let max = read("max_brightness")?;
    let cur = read("brightness")?;
    let new = (cur + max * delta as i64 / 100).clamp(max / 100, max);
    // Запись требует прав (udev-правило или группа video); без них — только показ.
    let _ = std::fs::write(dir.join("brightness"), new.to_string());
    let now = read("brightness").unwrap_or(cur);
    Some((now * 100 / max.max(1)) as u32)
}

// ─── Батарея, сеть, процессор ────────────────────────────────────────────────

fn read_battery() -> Option<Battery> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut total_now = 0f64;
    let mut total_full = 0f64;
    let mut charging = false;
    let mut full = true;
    let mut power = 0f64;
    let mut found = false;
    for e in rd.flatten() {
        let p = e.path();
        let read = |f: &str| std::fs::read_to_string(p.join(f)).ok().map(|s| s.trim().to_string());
        if read("type").as_deref() != Some("Battery") || read("scope").as_deref() == Some("Device") {
            continue;
        }
        found = true;
        let num = |f: &str| read(f).and_then(|s| s.parse::<f64>().ok());
        let (now, fl) = match (num("energy_now"), num("energy_full")) {
            (Some(n), Some(f)) => (n, f),
            _ => match (num("charge_now"), num("charge_full")) {
                (Some(n), Some(f)) => (n, f),
                _ => {
                    let c = num("capacity").unwrap_or(0.0);
                    (c, 100.0)
                }
            },
        };
        total_now += now;
        total_full += fl;
        power += num("power_now").or_else(|| num("current_now")).unwrap_or(0.0);
        let st = read("status").unwrap_or_default();
        charging |= st == "Charging";
        full &= st == "Full" || st == "Not charging";
    }
    if !found || total_full <= 0.0 {
        return None;
    }
    let percent = (total_now / total_full * 100.0).round().clamp(0.0, 100.0) as u32;
    let minutes = if power > 0.0 {
        let hours = if charging { (total_full - total_now) / power } else { total_now / power };
        Some((hours * 60.0) as u32)
    } else {
        None
    };
    Some(Battery { percent, charging, full, minutes })
}

fn read_network() -> Network {
    if which("nmcli") {
        if let Some(s) = output("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) {
            for line in s.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 && parts[1] == "connected" && (parts[0] == "wifi" || parts[0] == "ethernet") {
                    let signal = if parts[0] == "wifi" {
                        output("nmcli", &["-t", "-f", "ACTIVE,SIGNAL", "device", "wifi"]).and_then(|w| {
                            w.lines().find(|l| l.starts_with("yes:")).and_then(|l| l[4..].parse().ok())
                        })
                    } else {
                        None
                    };
                    return Network { kind: parts[0].into(), connection: parts[2..].join(":"), signal, online: true };
                }
            }
            return Network { kind: "none".into(), ..Default::default() };
        }
    }
    // Без NetworkManager — по состоянию интерфейсов.
    if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue;
            }
            let up = std::fs::read_to_string(e.path().join("operstate")).map(|s| s.trim() == "up").unwrap_or(false);
            if up {
                let wifi = e.path().join("wireless").exists();
                return Network {
                    kind: if wifi { "wifi" } else { "ethernet" }.into(),
                    connection: name,
                    signal: None,
                    online: true,
                };
            }
        }
    }
    Network { kind: "none".into(), ..Default::default() }
}

fn cpu_times() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let idle = v.get(3)? + v.get(4).unwrap_or(&0);
    Some((v.iter().sum(), idle))
}

fn memory_used() -> Option<f32> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let get = |k: &str| -> Option<f32> {
        s.lines().find(|l| l.starts_with(k))?.split_whitespace().nth(1)?.parse().ok()
    };
    let total = get("MemTotal:")?;
    let avail = get("MemAvailable:")?;
    Some((1.0 - avail / total) * 100.0)
}

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new().name("shell-volume".into()).spawn(move || watch_volume(ctx)).ok();
    std::thread::Builder::new()
        .name("shell-sys".into())
        .spawn(move || {
            let mut last = cpu_times();
            let mut tick = 0u64;
            loop {
                if tick % 15 == 0 {
                    ctx.battery.set(read_battery());
                }
                if tick % 3 == 0 {
                    ctx.network.set(read_network());
                }
                let now = cpu_times();
                if let (Some((t0, i0)), Some((t1, i1))) = (last, now) {
                    let dt = (t1 - t0).max(1) as f32;
                    ctx.cpu.set(((1.0 - (i1 - i0) as f32 / dt) * 100.0).clamp(0.0, 100.0).round());
                }
                last = now;
                if let Some(m) = memory_used() {
                    ctx.memory.set(m.round());
                }
                tick += 1;
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .ok();
}

// ─── Медиаплееры (MPRIS) ─────────────────────────────────────────────────────

/// `play-pause`, `next`, `previous`, `stop` первому найденному плееру.
pub fn media(cmd: &str) {
    let method = match cmd {
        "play-pause" | "toggle" => "PlayPause",
        "next" => "Next",
        "previous" | "prev" => "Previous",
        "stop" => "Stop",
        "play" => "Play",
        "pause" => "Pause",
        _ => return,
    };
    if which("playerctl") {
        spawn(&format!("playerctl {}", cmd.replace("toggle", "play-pause")));
        return;
    }
    let method = method.to_string();
    std::thread::spawn(move || {
        let r = (|| -> zbus::Result<()> {
            let conn = zbus::blocking::Connection::session()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
            let names = dbus.list_names()?;
            let Some(player) = names.iter().find(|n| n.as_str().starts_with("org.mpris.MediaPlayer2.")) else {
                return Ok(());
            };
            conn.call_method(
                Some(player.as_str()),
                "/org/mpris/MediaPlayer2",
                Some("org.mpris.MediaPlayer2.Player"),
                method.as_str(),
                &(),
            )?;
            Ok(())
        })();
        if let Err(e) = r {
            log::debug!("MPRIS: {e}");
        }
    });
}
