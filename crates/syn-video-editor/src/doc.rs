//! Документ проекта: настройки, медиатека, дорожки и клипы. Время — секунды
//! (f64) на шкале проекта, команды выравнивают его по кадрам.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type Id = u64;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub settings: Settings,
    #[serde(default)]
    pub media: Vec<Media>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default = "first_id")]
    pub next_id: Id,
}

fn first_id() -> Id {
    1
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
}

fn default_rate() -> u32 {
    48_000
}

impl Default for Settings {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: 30.0, sample_rate: 48_000 }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Image => "image",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Media {
    pub id: Id,
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Длительность, с (у картинки — 0: длина клипа задаётся на таймлайне).
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub has_audio: bool,
}

impl Media {
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    /// Видео, картинки, титры (слои: верхняя дорожка — поверх).
    Video,
    Audio,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub id: Id,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Клипы по возрастанию `start`, без наложений.
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// Откуда берётся содержимое клипа.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    Media { media: Id },
    /// Титр: текст и стиль (позже — блок из шаблона).
    Title {
        text: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        style: Map<String, Value>,
    },
    /// Заливка цветом (`#rrggbb`).
    Color { color: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: Id,
    pub source: Source,
    /// Начало на таймлайне, с.
    pub start: f64,
    /// Длительность на таймлайне, с.
    pub duration: f64,
    /// Смещение в источнике, с (откуда в файле начинается клип).
    #[serde(default)]
    pub offset: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "one")]
    pub volume: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
}

fn one() -> f64 {
    1.0
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
    pub fn media(&self) -> Option<Id> {
        match self.source {
            Source::Media { media } => Some(media),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Effect {
    pub id: Id,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new("Без названия")
    }
}

impl Project {
    pub fn new(name: &str) -> Self {
        let mut p = Self { version: FORMAT_VERSION, name: name.to_string(), settings: Settings::default(), media: Vec::new(), tracks: Vec::new(), next_id: 1 };
        let v = p.alloc();
        p.tracks.push(Track { id: v, kind: TrackKind::Video, name: "V1".into(), muted: false, hidden: false, clips: Vec::new() });
        let a = p.alloc();
        p.tracks.push(Track { id: a, kind: TrackKind::Audio, name: "A1".into(), muted: false, hidden: false, clips: Vec::new() });
        p
    }

    pub fn alloc(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: Id) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn track(&self, id: Id) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: Id) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Дорожка и индекс клипа.
    pub fn find_clip(&self, id: Id) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| t.clips.iter().position(|c| c.id == id).map(|ci| (ti, ci)))
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.find_clip(id).map(|(t, c)| &self.tracks[t].clips[c])
    }

    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        self.find_clip(id).map(move |(t, c)| &mut self.tracks[t].clips[c])
    }

    /// Длительность всего таймлайна, с.
    pub fn duration(&self) -> f64 {
        self.tracks.iter().flat_map(|t| t.clips.iter().map(Clip::end)).fold(0.0, f64::max)
    }

    /// Время, выровненное по кадру проекта.
    pub fn snap(&self, t: f64) -> f64 {
        let fps = self.settings.fps.max(1.0);
        ((t.max(0.0) * fps).round() / fps * 1e6).round() / 1e6
    }

    /// Компактный текст таймлайна — для агента, CLI и журнала.
    pub fn dump(&self) -> String {
        use std::fmt::Write;
        let s = &self.settings;
        let mut o = String::new();
        let _ = writeln!(o, "Проект «{}»: {}x{} @ {} fps, длительность {}", self.name, s.width, s.height, fmt_fps(s.fps), tc(self.duration()));
        if self.media.is_empty() {
            let _ = writeln!(o, "Медиатека пуста");
        } else {
            let _ = writeln!(o, "Медиатека:");
            for m in &self.media {
                let mut info = vec![m.kind.as_str().to_string()];
                if m.duration > 0.0 {
                    info.push(tc(m.duration));
                }
                if m.width > 0 {
                    info.push(format!("{}x{}", m.width, m.height));
                }
                if m.fps > 0.0 {
                    info.push(format!("{} fps", fmt_fps(m.fps)));
                }
                if m.kind == MediaKind::Video && m.has_audio {
                    info.push("со звуком".into());
                }
                let _ = writeln!(o, "  m{} {} ({})", m.id, m.name(), info.join(", "));
            }
        }
        for t in &self.tracks {
            let mut flags = String::new();
            if t.muted {
                flags.push_str(" [без звука]");
            }
            if t.hidden {
                flags.push_str(" [скрыта]");
            }
            let _ = writeln!(o, "Дорожка t{} {} ({}){}: {} клип(ов)", t.id, t.name, t.kind.as_str(), flags, t.clips.len());
            for c in &t.clips {
                let what = match &c.source {
                    Source::Media { media } => format!("m{} {}", media, self.media(*media).map(|m| m.name()).unwrap_or_default()),
                    Source::Title { text, .. } => format!("титр «{text}»"),
                    Source::Color { color } => format!("цвет {color}"),
                };
                let mut extra = Vec::new();
                if c.offset > 0.0 {
                    extra.push(format!("из источника с {}", tc(c.offset)));
                }
                if (c.speed - 1.0).abs() > 1e-9 {
                    extra.push(format!("скорость {}", c.speed));
                }
                if (c.volume - 1.0).abs() > 1e-9 {
                    extra.push(format!("громкость {}", c.volume));
                }
                for e in &c.effects {
                    extra.push(format!("эффект e{} {}{}", e.id, e.kind, if e.params.is_empty() { String::new() } else { format!(" {}", Value::Object(e.params.clone())) }));
                }
                let extra = if extra.is_empty() { String::new() } else { format!(" — {}", extra.join("; ")) };
                let _ = writeln!(o, "  c{} {}–{} {}{}", c.id, tc(c.start), tc(c.end()), what, extra);
            }
        }
        o
    }
}

fn fmt_fps(f: f64) -> String {
    if (f - f.round()).abs() < 1e-6 {
        format!("{}", f.round() as i64)
    } else {
        format!("{f:.3}")
    }
}

/// Таймкод `м:сс.ммм` (часы — если есть).
pub fn tc(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    let (h, m, s, r) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{r:03}")
    } else {
        format!("{m}:{s:02}.{r:03}")
    }
}

/// Разбор времени: секунды (`12.5`), `м:сс(.ммм)`, `ч:мм:сс(.ммм)`.
pub fn parse_time(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim().trim_end_matches('s').trim_end_matches('с');
            let parts: Vec<&str> = s.split(':').collect();
            let mut t = 0.0;
            for p in &parts {
                t = t * 60.0 + p.trim().replace(',', ".").parse::<f64>().ok()?;
            }
            (parts.len() <= 3).then_some(t)
        }
        _ => None,
    }
}

/// Строка-путь в `PathBuf` с раскрытием `~`.
pub fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}
