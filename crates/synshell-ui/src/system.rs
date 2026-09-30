//! Состояние системы для апплетов: громкость (PipeWire/PulseAudio),
//! батарея, сеть, загрузка процессора и памяти, яркость, медиаплееры.
//! Всё опрашивается в фоновых потоках; сигналы ставятся потокобезопасно.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::actions::{output, spawn, which};
use crate::ctx::ShellCtx;

pub use synsystem::battery::Battery;
pub use synsystem::network::Network;
pub use synsystem::volume::Volume;

// ─── Громкость ───────────────────────────────────────────────────────────────

fn read_volume() -> Option<Volume> {
    synsystem::volume::read()
}

pub fn refresh_volume(ctx: ShellCtx) -> Option<Volume> {
    let v = read_volume();
    ctx.volume.set(v.clone());
    v
}

/// Предел громкости, %: 100, с `[sound] overamplify` — 150.
pub fn max_volume(ctx: ShellCtx) -> u32 {
    ctx.config.get_untracked().sound.max_volume()
}

/// Изменить громкость на `delta` процентов (или выставить при `set`).
pub fn change_volume(ctx: ShellCtx, delta: i32, set: Option<u32>) {
    std::thread::spawn(move || {
        synsystem::volume::change(delta, set, max_volume(ctx));
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
        synsystem::volume::toggle_mute(mic);
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
    // Запись требует прав (udev-правило или группа video); без них — только показ.
    let sys = synsystem::Sys::host();
    let b = synsystem::backlight::change(&sys, delta as f32).ok().or_else(|| synsystem::backlight::primary(&sys))?;
    Some(b.percent().round() as u32)
}

// ─── Батарея, сеть, процессор ────────────────────────────────────────────────

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new().name("shell-volume".into()).spawn(move || watch_volume(ctx)).ok();
    std::thread::Builder::new()
        .name("shell-sys".into())
        .spawn(move || {
            let sys = synsystem::Sys::host();
            let mut cpu = synsystem::cpu::CpuSampler::new(sys.clone());
            cpu.sample();
            let mut tick = 0u64;
            loop {
                if tick % 15 == 0 {
                    ctx.battery.set(synsystem::battery::read(&sys));
                }
                if tick % 3 == 0 {
                    ctx.network.set(synsystem::network::read(&sys));
                }
                ctx.cpu.set(cpu.sample().usage.round());
                if let Some(m) = synsystem::memory::read(&sys) {
                    ctx.memory.set(m.used_percent().round());
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
