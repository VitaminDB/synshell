//! Фонарик: светодиоды вспышки камеры в режиме постоянного света.
//!
//! Описание даёт платформа — `/etc/syn-torch.conf` (arch-mobile-port: device.conf `TORCH_*`):
//!
//! ```text
//! leds = led:torch_0          # каналы без оттенка (или)
//! warm = led:torch_0          # двухтоновая вспышка: тёплый и холодный каналы
//! cold = led:torch_1
//! switch = led:switch_2       # «выключатель» каналов (Qualcomm qti-flash): ток задают каналы, свет — он
//! ```
//!
//! Ток канала — `brightness` (мА, до `max_brightness`), свет включает запись 1 в `brightness` выключателя.
//! Выключатель с триггером камеры (switch0_trigger) брать нельзя: запись 0 снимает триггер, и вспышка камеры
//! перестаёт работать. Без файла — первый светодиод `*torch*` без выключателя.

use std::io;
use std::path::{Path, PathBuf};

pub const CONF: &str = "/etc/syn-torch.conf";
const LEDS: &str = "/sys/class/leds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Warm,
    Cold,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub path: PathBuf,
    pub tone: Option<Tone>,
    /// Наибольший ток, мА (`max_brightness`).
    pub max: u32,
}

#[derive(Debug, Clone)]
pub struct Torch {
    pub channels: Vec<Channel>,
    pub switch: Option<PathBuf>,
}

/// Состояние: включён, яркость 0…1 (доля наибольшего тока), включённые оттенки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub on: bool,
    pub level: f32,
    pub warm: bool,
    pub cold: bool,
}

fn read_u32(p: &Path) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn channel(name: &str, tone: Option<Tone>) -> Option<Channel> {
    let path = Path::new(LEDS).join(name);
    let max = read_u32(&path.join("max_brightness"))?;
    Some(Channel { path, tone, max: max.max(1) })
}

impl Torch {
    /// Фонарик устройства или `None`.
    pub fn find() -> Option<Torch> {
        if let Ok(text) = std::fs::read_to_string(CONF) {
            let mut t = Torch { channels: Vec::new(), switch: None };
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                let Some((k, v)) = line.split_once('=') else { continue };
                let tone = match k.trim() {
                    "warm" => Some(Some(Tone::Warm)),
                    "cold" => Some(Some(Tone::Cold)),
                    "leds" => Some(None),
                    "switch" => {
                        t.switch = Some(Path::new(LEDS).join(v.trim()));
                        None
                    }
                    _ => None,
                };
                if let Some(tone) = tone {
                    t.channels.extend(v.split_whitespace().filter_map(|n| channel(n, tone)));
                }
            }
            return (!t.channels.is_empty()).then_some(t);
        }
        let name = std::fs::read_dir(LEDS)
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .find(|n| n.contains("torch"))?;
        Some(Torch { channels: vec![channel(&name, None)?], switch: None })
    }

    /// Двухтоновая вспышка (есть и тёплый, и холодный каналы).
    pub fn two_tone(&self) -> bool {
        self.channels.iter().any(|c| c.tone == Some(Tone::Warm)) && self.channels.iter().any(|c| c.tone == Some(Tone::Cold))
    }

    pub fn state(&self) -> State {
        let cur = |c: &Channel| read_u32(&c.path.join("brightness")).unwrap_or(0);
        let lit = |tone| self.channels.iter().any(|c| c.tone == Some(tone) && cur(c) > 0);
        let on = match &self.switch {
            Some(s) => read_u32(&s.join("brightness")).unwrap_or(0) > 0,
            None => self.channels.iter().any(|c| cur(c) > 0),
        };
        let level = self.channels.iter().map(|c| cur(c) as f32 / c.max as f32).fold(0.0, f32::max);
        State { on, level, warm: lit(Tone::Warm), cold: lit(Tone::Cold) }
    }

    /// Включить (или поменять яркость и оттенки) / выключить. Оттенки — только у двухтоновой вспышки; оба
    /// выключены — горят оба.
    pub fn set(&self, st: State) -> io::Result<()> {
        let both = !st.warm && !st.cold;
        let level = st.level.clamp(0.0, 1.0);
        if let Some(s) = &self.switch {
            // ток меняется только при выключенном выключателе
            std::fs::write(s.join("brightness"), "0")?;
        }
        if !st.on {
            for c in &self.channels {
                std::fs::write(c.path.join("brightness"), "0")?;
            }
            return Ok(());
        }
        for c in &self.channels {
            let lit = match c.tone {
                None => true,
                Some(Tone::Warm) => both || st.warm,
                Some(Tone::Cold) => both || st.cold,
            };
            let ma = if lit { ((c.max as f32 * level).round() as u32).max(1) } else { 0 };
            std::fs::write(c.path.join("brightness"), ma.to_string())?;
        }
        if let Some(s) = &self.switch {
            std::fs::write(s.join("brightness"), "1")?;
        }
        Ok(())
    }
}
