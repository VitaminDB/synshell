//! Хранилище истории: `~/.local/share/synshell/clipboard/history.json` и
//! рядом файлы записей — картинка как есть (`<id>.img`), её уменьшенная
//! копия для окна (`<id>.png`) и HTML текста (`<id>.html`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Длинная сторона уменьшенной картинки, px.
pub const THUMB: u32 = 720;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Text,
    Image,
    Files,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Когда скопировано (последний раз), секунды Unix.
    pub time: i64,
    pub kind: Kind,
    /// Текст; у файлов — пути, по одному на строку.
    #[serde(default)]
    pub text: String,
    /// Рядом лежит HTML текста.
    #[serde(default)]
    pub html: bool,
    #[serde(default)]
    pub image: Option<ImageInfo>,
    /// Откуда: id .desktop программы в фокусе или `synlink` (другое устройство).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Сколько раз копировали то же самое.
    #[serde(default)]
    pub count: u32,
    pub hash: u64,
}

impl Entry {
    /// Уменьшенная картинка для окна.
    pub fn thumb(&self) -> PathBuf {
        dir().join(format!("{}.png", self.id))
    }
    pub fn image_file(&self) -> PathBuf {
        dir().join(format!("{}.img", self.id))
    }
    pub fn html_file(&self) -> PathBuf {
        dir().join(format!("{}.html", self.id))
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        if self.kind != Kind::Files {
            return Vec::new();
        }
        self.text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
    }
    fn remove_files(&self) {
        let _ = std::fs::remove_file(self.thumb());
        let _ = std::fs::remove_file(self.image_file());
        let _ = std::fs::remove_file(self.html_file());
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    entries: Vec<Entry>,
}

pub fn dir() -> PathBuf {
    synshell_common::paths::data_home().join("synshell/clipboard")
}

fn index() -> PathBuf {
    dir().join("history.json")
}

/// Записи, новые первыми, и следующий свободный id.
pub fn load() -> (Vec<Entry>, u64) {
    let f: File = std::fs::read(index()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let next = f.next_id.max(f.entries.iter().map(|e| e.id + 1).max().unwrap_or(1));
    (f.entries, next)
}

pub fn save(entries: &[Entry], next_id: u64) {
    let f = File { next_id, entries: entries.to_vec() };
    let Ok(data) = serde_json::to_vec(&f) else { return };
    let _ = std::fs::create_dir_all(dir());
    let tmp = dir().join("history.json.tmp");
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, index());
    }
}

pub fn hash_bytes(kind: Kind, data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    kind.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

/// Положить файлы картинки записи `id`: оригинал и уменьшенную копию.
/// Размеры — из картинки; не читается — `None` (запись не нужна).
pub fn write_image(id: u64, mime: &str, data: &[u8]) -> Option<ImageInfo> {
    let img = image::load_from_memory(data).ok()?;
    let (width, height) = (img.width(), img.height());
    let _ = std::fs::create_dir_all(dir());
    std::fs::write(dir().join(format!("{id}.img")), data).ok()?;
    let small = if width.max(height) > THUMB { img.thumbnail(THUMB, THUMB) } else { img };
    small.to_rgba8().save_with_format(dir().join(format!("{id}.png")), image::ImageFormat::Png).ok()?;
    Some(ImageInfo { mime: mime.to_string(), width, height, bytes: data.len() as u64 })
}

pub fn write_html(id: u64, html: &[u8]) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(dir().join(format!("{id}.html")), html);
}

/// Убрать лишнее сверх `max` (закреплённые не считаются и не удаляются).
pub fn trim(entries: &mut Vec<Entry>, max: usize) {
    let mut kept = 0usize;
    entries.retain(|e| {
        if e.pinned {
            return true;
        }
        kept += 1;
        if kept > max {
            e.remove_files();
            false
        } else {
            true
        }
    });
}

pub fn remove(entries: &mut Vec<Entry>, id: u64) {
    if let Some(i) = entries.iter().position(|e| e.id == id) {
        entries.remove(i).remove_files();
    }
}

/// Очистить всё, кроме закреплённых.
pub fn clear(entries: &mut Vec<Entry>) {
    entries.retain(|e| {
        if !e.pinned {
            e.remove_files();
        }
        e.pinned
    });
}
