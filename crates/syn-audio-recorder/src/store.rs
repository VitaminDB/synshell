//! Настройки диктофона (`~/.config/syn-audio-recorder/settings.toml`), папка записей, сведения о записях
//! (`<папка>/.meta/<файл>.json`: название, длительность, огибающая для миниатюры волны) и форматирование
//! времени, размеров, дат.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use syngui::{n_, t};
use synshell_common::paths;

/// Точек в огибающей записи (миниатюра волны и плеер).
pub const PEAKS: usize = 200;
/// Расширения, которые показывает список записей.
pub const EXTENSIONS: &[&str] = &["ogg", "opus", "oga", "flac", "wav", "m4a", "mp3", "webm"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Opus,
    Flac,
    Wav,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Opus, Format::Flac, Format::Wav];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Opus => "ogg",
            Format::Flac => "flac",
            Format::Wav => "wav",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Opus => "Opus",
            Format::Flac => "FLAC",
            Format::Wav => "WAV",
        }
    }
}

/// Битрейты Opus (кбит/с) на выбор.
pub const OPUS_BITRATES: [u32; 5] = [24, 32, 64, 96, 128];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub format: Format,
    /// Битрейт Opus, кбит/с.
    pub opus_kbps: u32,
    /// Папка записей; пусто — `$XDG_MUSIC_DIR/Диктофон`.
    pub folder: String,
    /// Узел PipeWire микрофона (`node.name`); пусто — по умолчанию.
    pub mic: String,
    /// Визуализация «Всегда»: волна и уровень живые и до записи (микрофон открыт, пока окно активно).
    /// `false` — «Авто»: волна идёт только во время записи, после «Стоп» замирает, микрофон закрыт.
    pub vis_always: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { format: Format::Opus, opus_kbps: 64, folder: String::new(), mic: String::new(), vis_always: false }
    }
}

pub fn settings_file() -> PathBuf {
    paths::xdg_config_home().join("syn-audio-recorder").join("settings.toml")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(settings_file()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let f = settings_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&f, s) {
                tracing::warn!("{}: {e}", f.display());
            }
        }
    }

    /// Папка записей (без создания).
    pub fn dir(&self) -> PathBuf {
        let f = self.folder.trim();
        if f.is_empty() {
            default_dir()
        } else {
            paths::expand_tilde(f)
        }
    }

    pub fn mic_node(&self) -> Option<String> {
        let m = self.mic.trim();
        (!m.is_empty()).then(|| m.to_string())
    }
}

pub fn default_dir() -> PathBuf {
    paths::user_dir_or_default("MUSIC").join("Диктофон")
}

// ─── записи ────────────────────────────────────────────────────────────────

/// Сведения о записи рядом с файлом.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Meta {
    /// Название (может содержать «:», которого нет в имени файла).
    pub title: String,
    pub duration: f64,
    /// Огибающая 0..255, [`PEAKS`] точек.
    pub peaks: Vec<u8>,
    /// Время начала записи, секунды Unix.
    pub created: i64,
    /// Микрофон (подпись).
    pub mic: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub path: PathBuf,
    pub title: String,
    pub duration: f64,
    pub size: u64,
    pub created: i64,
    pub peaks: Vec<u8>,
    /// Сведений нет — длительность и волна ещё считаются.
    pub pending: bool,
}

pub fn meta_path(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(".meta").join(format!("{name}.json"))
}

pub fn read_meta(file: &Path) -> Option<Meta> {
    serde_json::from_str(&std::fs::read_to_string(meta_path(file)).ok()?).ok()
}

pub fn write_meta(file: &Path, m: &Meta) {
    let p = meta_path(file);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(s) = serde_json::to_string(m) {
        let _ = std::fs::write(p, s);
    }
}

fn mtime(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn is_recording_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Записи папки, новые сверху.
pub fn scan(dir: &Path) -> Vec<Recording> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() || !is_recording_file(&p) || p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = read_meta(&p);
        let r = match meta {
            Some(m) => Recording {
                title: if m.title.trim().is_empty() { stem } else { m.title },
                duration: m.duration,
                size: md.len(),
                created: if m.created > 0 { m.created } else { mtime(&md) },
                peaks: m.peaks,
                pending: false,
                path: p,
            },
            None => Recording { title: stem, duration: 0.0, size: md.len(), created: mtime(&md), peaks: Vec::new(), pending: true, path: p },
        };
        out.push(r);
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.path.cmp(&a.path)));
    out
}

/// Имя файла из названия: без `/` и управляющих, `:` → `.` (файл переносится и на FAT/Windows).
pub fn file_stem_of(title: &str) -> String {
    let s: String = title
        .trim()
        .chars()
        .map(|c| match c {
            ':' => '.',
            '/' | '\\' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if s.is_empty() {
        "Запись".to_string()
    } else {
        s.chars().take(120).collect()
    }
}

/// Свободный путь `dir/stem.ext`, при занятом — `stem (2).ext`, … (`except` — сам переименовываемый файл).
pub fn unique_path(dir: &Path, stem: &str, ext: &str, except: Option<&Path>) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while (p.exists() || dir.join(format!("{}.part", p.file_name().unwrap().to_string_lossy())).exists()) && Some(p.as_path()) != except {
        p = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    p
}

/// Название новой записи: «Запись 2026-11-02 19:30».
pub fn default_title(unix: i64) -> String {
    let t = local(unix);
    format!("{} {:04}-{:02}-{:02} {:02}:{:02}", t!("Запись"), t.year, t.month, t.day, t.hour, t.min)
}

/// Переименовать запись: файл (по названию) и сведения. Возвращает новый путь.
pub fn rename(file: &Path, title: &str) -> std::io::Result<PathBuf> {
    let title = title.trim();
    let dir = file.parent().unwrap_or(Path::new("."));
    let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let new = unique_path(dir, &file_stem_of(title), &ext, Some(file));
    let mut meta = read_meta(file).unwrap_or_default();
    if new != file {
        std::fs::rename(file, &new)?;
        let _ = std::fs::remove_file(meta_path(file));
    }
    meta.title = title.to_string();
    write_meta(&new, &meta);
    Ok(new)
}

/// Корзина папки: удалённое можно вернуть, пока висит подсказка «Отменить».
pub fn trash_dir(dir: &Path) -> PathBuf {
    dir.join(".trash")
}

/// Убрать запись в корзину; возвращает путь в корзине.
pub fn trash(file: &Path) -> std::io::Result<PathBuf> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let t = trash_dir(dir);
    std::fs::create_dir_all(&t)?;
    let dst = t.join(file.file_name().unwrap_or_default());
    std::fs::rename(file, &dst)?;
    let m = meta_path(file);
    if m.exists() {
        let _ = std::fs::rename(&m, meta_path(&dst));
    }
    Ok(dst)
}

/// Вернуть из корзины на место.
pub fn untrash(trashed: &Path, original: &Path) -> std::io::Result<()> {
    std::fs::rename(trashed, original)?;
    let m = meta_path(trashed);
    if m.exists() {
        let _ = std::fs::create_dir_all(meta_path(original).parent().unwrap());
        let _ = std::fs::rename(&m, meta_path(original));
    }
    Ok(())
}

/// Удалить из корзины насовсем.
pub fn purge(trashed: &Path) {
    let _ = std::fs::remove_file(trashed);
    let _ = std::fs::remove_file(meta_path(trashed));
}

/// Очистить корзину и оборванные служебные сведения (файл исчез — его `.json` тоже).
pub fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(trash_dir(dir));
    if let Ok(rd) = std::fs::read_dir(dir.join(".meta")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(f) = name.strip_suffix(".json") {
                if !dir.join(f).exists() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Огибающая из пиков по 50 мс → `n` точек 0..255 (максимум в каждом отрезке, шкала — как у волны записи).
pub fn envelope(bins: &[f32], n: usize) -> Vec<u8> {
    if bins.is_empty() || n == 0 {
        return Vec::new();
    }
    let n = n.min(bins.len()).max(1);
    (0..n)
        .map(|i| {
            let a = i * bins.len() / n;
            let b = ((i + 1) * bins.len() / n).max(a + 1).min(bins.len());
            let peak = bins[a..b].iter().copied().fold(0.0f32, f32::max);
            (loudness(peak) * 255.0).round() as u8
        })
        .collect()
}

/// Пик 0..1 → высота столбика 0..1: логарифмическая шкала (−48…0 дБ), тихая речь видна, громкая не упирается.
pub fn loudness(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db + 48.0) / 48.0).clamp(0.0, 1.0)
}

// ─── форматирование ───────────────────────────────────────────────────────

/// Таймер записи: («00:01:23», «.4»).
pub fn timer(secs: f64) -> (String, String) {
    let tenths = (secs.max(0.0) * 10.0).floor() as u64;
    let s = tenths / 10;
    (format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60), format!(".{}", tenths % 10))
}

/// Длительность в списке: «0:42», «12:03», «1:02:03».
pub fn duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Размер: «812 Б», «340 КБ», «1,2 МБ».
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        t!("{n} Б", n = bytes)
    } else if b < 1024.0 * 1024.0 {
        t!("{n} КБ", n = (b / 1024.0).round() as u64)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        t!("{n} МБ", n = format!("{:.1}", b / 1024.0 / 1024.0).replace('.', &t!(",")))
    } else {
        t!("{n} ГБ", n = format!("{:.1}", b / 1024.0 / 1024.0 / 1024.0).replace('.', &t!(",")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    /// День от эпохи по местному времени (сравнение «сегодня/вчера»).
    pub yday_abs: i64,
}

pub fn local(unix: i64) -> Local {
    let t = unix as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let off = tm.tm_gmtoff;
    Local {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        min: tm.tm_min as u32,
        yday_abs: (unix + off).div_euclid(86400),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

const MONTHS: [&str; 12] = [n_!("янв."), n_!("февр."), n_!("марта"), n_!("апр."), n_!("мая"), n_!("июня"), n_!("июля"), n_!("авг."), n_!("сент."), n_!("окт."), n_!("нояб."), n_!("дек.")];

/// Дата записи в списке: «Сегодня, 19:30», «Вчера, 08:05», «2 нояб., 19:30», «2 нояб. 2025».
pub fn when(unix: i64, now: i64) -> String {
    let t = local(unix);
    let n = local(now);
    let hm = format!("{:02}:{:02}", t.hour, t.min);
    if t.yday_abs == n.yday_abs {
        t!("Сегодня, {t}", t = hm)
    } else if t.yday_abs + 1 == n.yday_abs {
        t!("Вчера, {t}", t = hm)
    } else if t.year == n.year {
        t!("{d} {mon}, {time}", d = t.day, mon = t!(MONTHS[(t.month - 1) as usize]), time = hm)
    } else {
        t!("{d} {mon} {y}", d = t.day, mon = t!(MONTHS[(t.month - 1) as usize]), y = t.year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_format() {
        assert_eq!(timer(0.0), ("00:00:00".into(), ".0".into()));
        assert_eq!(timer(5.37), ("00:00:05".into(), ".3".into()));
        assert_eq!(timer(3723.99), ("01:02:03".into(), ".9".into()));
        assert_eq!(timer(-1.0), ("00:00:00".into(), ".0".into()));
    }

    #[test]
    fn duration_format() {
        assert_eq!(duration(42.4), "0:42");
        assert_eq!(duration(723.0), "12:03");
        assert_eq!(duration(3723.0), "1:02:03");
    }

    #[test]
    fn size_format() {
        assert_eq!(size(812), "812 Б");
        assert_eq!(size(340 * 1024), "340 КБ");
        assert_eq!(size(1_258_291), "1,2 МБ");
    }

    #[test]
    fn file_names() {
        assert_eq!(file_stem_of("Запись 2026-11-02 19:30"), "Запись 2026-11-02 19.30");
        assert_eq!(file_stem_of("  a/b?c  "), "a_b_c");
        assert_eq!(file_stem_of("..."), "Запись");
        assert_eq!(file_stem_of(""), "Запись");
    }

    #[test]
    fn unique_and_rename() {
        let d = std::env::temp_dir().join(format!("sar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let a = unique_path(&d, "Запись", "ogg", None);
        assert_eq!(a, d.join("Запись.ogg"));
        std::fs::write(&a, b"x").unwrap();
        let b = unique_path(&d, "Запись", "ogg", None);
        assert_eq!(b, d.join("Запись (2).ogg"));
        // переименование в то же имя — тот же файл
        assert_eq!(unique_path(&d, "Запись", "ogg", Some(&a)), a);
        write_meta(&a, &Meta { title: "Запись".into(), duration: 3.5, peaks: vec![1, 2, 3], created: 100, mic: String::new() });
        let n = rename(&a, "Лекция: часть 1").unwrap();
        assert_eq!(n, d.join("Лекция. часть 1.ogg"));
        assert!(!a.exists() && n.exists());
        let m = read_meta(&n).unwrap();
        assert_eq!(m.title, "Лекция: часть 1");
        assert_eq!(m.duration, 3.5);
        assert!(!meta_path(&a).exists());
        let list = scan(&d);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Лекция: часть 1");
        // корзина
        let t = trash(&n).unwrap();
        assert!(scan(&d).is_empty());
        untrash(&t, &n).unwrap();
        assert_eq!(scan(&d).len(), 1);
        assert_eq!(read_meta(&n).unwrap().duration, 3.5);
        let t = trash(&n).unwrap();
        purge(&t);
        cleanup(&d);
        assert!(!trash_dir(&d).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn envelope_points() {
        assert!(envelope(&[], 200).is_empty());
        let bins: Vec<f32> = (0..1000).map(|i| if i < 500 { 0.0 } else { 1.0 }).collect();
        let e = envelope(&bins, 200);
        assert_eq!(e.len(), 200);
        assert_eq!(e[0], 0);
        assert_eq!(e[199], 255);
        // короче нужного — по точке на отрезок
        assert_eq!(envelope(&[0.5, 1.0], 200).len(), 2);
        assert!(loudness(0.004) < 0.05 && loudness(0.5) > 0.85);
    }

    #[test]
    fn settings_roundtrip() {
        let s = Settings { format: Format::Flac, opus_kbps: 96, folder: "~/Rec".into(), mic: "palaudio.mic.top".into(), vis_always: true };
        let txt = toml::to_string_pretty(&s).unwrap();
        assert!(txt.contains("format = \"flac\""));
        assert_eq!(toml::from_str::<Settings>(&txt).unwrap(), s);
        // неполный файл — умолчания
        let p: Settings = toml::from_str("format = \"wav\"").unwrap();
        assert_eq!(p.format, Format::Wav);
        assert_eq!(p.opus_kbps, 64);
        assert!(!p.vis_always);
        assert_eq!(s.dir(), paths::home().join("Rec"));
        assert_eq!(s.mic_node().as_deref(), Some("palaudio.mic.top"));
    }

    #[test]
    fn dates() {
        let now = 1_793_000_000; // 2026-10-26
        assert!(when(now - 60, now).starts_with("Сегодня, ") || when(now - 60, now).starts_with("Вчера, "));
        assert!(default_title(now).starts_with("Запись 2026-"));
    }
}
