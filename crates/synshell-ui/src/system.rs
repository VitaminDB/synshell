//! Состояние системы для апплетов: громкость (PipeWire/PulseAudio),
//! батарея, сеть, загрузка процессора и памяти, яркость, медиаплееры.
//! Всё опрашивается в фоновых потоках; сигналы ставятся потокобезопасно.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::actions::{spawn, which};
use crate::ctx::ShellCtx;
use syngui::t;

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
    // сигналы читаются только в главном потоке — предел берём до запуска фонового
    let max = max_volume(ctx);
    std::thread::spawn(move || {
        synsystem::volume::change(delta, set, max);
        if let Some(v) = refresh_volume(ctx) {
            if set.is_none() {
                let icon = if v.muted || v.percent == 0 { crate::ui::mi::VOLUME_OFF } else { crate::ui::mi::VOLUME_UP };
                crate::osd::show(ctx, icon, Some(v.percent), String::new());
                // Щелчок новой громкостью — слышно, насколько громко.
                if !v.muted {
                    syngui::async_runtime::run_on_main_thread(|| crate::sounds::play(crate::sounds::Ev::Volume));
                }
            }
        }
    });
}

/// Выставить громкость микрофона, %.
pub fn set_mic(ctx: ShellCtx, percent: u32) {
    std::thread::spawn(move || {
        synsystem::volume::set_mic(percent);
        refresh_volume(ctx);
    });
}

pub fn toggle_mute(ctx: ShellCtx, mic: bool) {
    std::thread::spawn(move || {
        synsystem::volume::toggle_mute(mic);
        if let Some(v) = refresh_volume(ctx) {
            let (icon, label) = if mic {
                if v.mic_muted {
                    (crate::ui::mi::MIC_OFF, t!("Микрофон выключен"))
                } else {
                    (crate::ui::mi::MIC, t!("Микрофон включён"))
                }
            } else if v.muted {
                (crate::ui::mi::VOLUME_OFF, t!("Звук выключен"))
            } else {
                (crate::ui::mi::VOLUME_UP, String::new())
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
        // synsystem сам уходит в brightnessctl, если sysfs не пускает; пара «белая + тёплая»
        // (E Ink) — общей яркостью, а brightnessctl напрямую сдвигал бы один канал.
        // Запись требует прав (udev-правило или группа video); без них — только показ.
        let sys = synsystem::Sys::host();
        let pct = match synsystem::backlight::change(&sys, delta as f32) {
            Ok(b) => {
                crate::backlight::remember_level(b.percent());
                Some(b.percent())
            }
            Err(e) => {
                log::warn!("яркость: {e}");
                synsystem::backlight::primary(&sys).map(|b| b.percent())
            }
        };
        if let Some(p) = pct {
            crate::osd::show(ctx, crate::ui::mi::BRIGHTNESS, Some(p.round() as u32), String::new());
        }
    });
}

// ─── Батарея, сеть, процессор ────────────────────────────────────────────────

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new().name("shell-volume".into()).spawn(move || watch_volume(ctx)).ok();
    // Кабель питания: сразу по uevent, плюс опрос раз в 2 с (на случай,
    // если драйвер не шлёт событий).
    update_power(ctx);
    if let Err(e) = synsystem::battery::watch(move || update_power(ctx)) {
        log::warn!("питание: нет uevent ({e})");
    }
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
                update_power(ctx);
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

/// Перечитать внешнее питание; сменилось — обновить и батарею (сразу и
/// через пару секунд, когда драйвер сменит её статус).
fn update_power(ctx: ShellCtx) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<Option<bool>>> = Mutex::new(None);
    let sys = synsystem::Sys::host();
    let now = synsystem::battery::external_power(&sys);
    {
        let mut last = LAST.lock().unwrap();
        if *last == Some(now) {
            return;
        }
        *last = Some(now);
    }
    log::info!("питание: внешнее {now:?}");
    ctx.battery.set(synsystem::battery::read(&sys));
    ctx.external_power.set(now);
    std::thread::spawn(move || {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(1500));
            ctx.battery.set(synsystem::battery::read(&synsystem::Sys::host()));
        }
    });
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
