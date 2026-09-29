//! Записи папок: чтение каталога в фоне, типы, сортировка, форматирование.

use std::cmp::Ordering;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use synshell_common::mime;

/// Запись каталога.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// Каталог (для ссылок — по цели).
    pub is_dir: bool,
    pub is_link: bool,
    /// Ссылка ведёт в никуда.
    pub broken: bool,
    pub hidden: bool,
    pub size: u64,
    /// Секунды Unix.
    pub mtime: i64,
    pub mime: String,
    pub mode: u32,
    pub uid: u32,
    /// Элементов в каталоге (`None` — не удалось прочитать).
    pub children: Option<u32>,
    /// Корзина: откуда удалено и когда.
    pub trash: Option<TrashInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrashInfo {
    pub original: PathBuf,
    pub deleted: String,
}

impl Entry {
    pub fn from_path(path: &Path) -> Option<Entry> {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
        let lmeta = std::fs::symlink_metadata(path).ok()?;
        let is_link = lmeta.file_type().is_symlink();
        let meta = if is_link { std::fs::metadata(path).ok() } else { Some(lmeta.clone()) };
        let broken = is_link && meta.is_none();
        let m = meta.as_ref().unwrap_or(&lmeta);
        let is_dir = m.is_dir();
        let mime = if broken {
            mime::SYMLINK.to_string()
        } else if is_dir {
            mime::DIRECTORY.to_string()
        } else if m.file_type().is_file() {
            mime::detect_file(path)
        } else {
            mime::detect(path)
        };
        let children = if is_dir { count_children(path) } else { None };
        Some(Entry {
            hidden: name.starts_with('.'),
            name,
            path: path.to_path_buf(),
            is_dir,
            is_link,
            broken,
            size: if is_dir { 0 } else { m.len() },
            mtime: m.mtime(),
            mime,
            mode: m.permissions().mode(),
            uid: m.uid(),
            children,
            trash: None,
        })
    }

    /// Имя без расширения (для выделения при переименовании и сортировки).
    pub fn stem_len(&self) -> usize {
        if self.is_dir {
            return self.name.len();
        }
        match self.name.rfind('.') {
            Some(0) | None => self.name.len(),
            Some(i) => {
                // `.tar.gz` — расширение из двух частей.
                let base = &self.name[..i];
                if base.ends_with(".tar") {
                    i - 4
                } else {
                    i
                }
            }
        }
    }

    pub fn description(&self) -> String {
        if self.is_dir {
            return "Папка".into();
        }
        mime::description(&self.mime)
    }
}

fn count_children(path: &Path) -> Option<u32> {
    // Сетевые и медленные ФС (gvfs, sshfs) не пересчитываем.
    if path.starts_with("/run/user") && path.to_string_lossy().contains("/gvfs/") {
        return None;
    }
    std::fs::read_dir(path).ok().map(|d| d.count().min(u32::MAX as usize) as u32)
}

/// Содержимое каталога (без `.` и `..`), без сортировки.
pub fn read_dir(path: &Path) -> std::io::Result<Vec<Entry>> {
    let rd = std::fs::read_dir(path)?;
    let mut out = Vec::new();
    for e in rd.flatten() {
        if let Some(entry) = Entry::from_path(&e.path()) {
            out.push(entry);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- сортировка

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Type,
    Size,
}

impl SortKey {
    pub fn parse(s: &str) -> Self {
        match s {
            "modified" | "date" => Self::Modified,
            "type" => Self::Type,
            "size" => Self::Size,
            _ => Self::Name,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Modified => "modified",
            Self::Type => "type",
            Self::Size => "size",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Name => "Имя",
            Self::Modified => "Дата изменения",
            Self::Type => "Тип",
            Self::Size => "Размер",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
    pub folders_first: bool,
}

/// Естественное сравнение: `файл2` < `файл10`, без учёта регистра.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn sort_entries(v: &mut [Entry], s: Sort) {
    v.sort_by(|a, b| {
        if s.folders_first && a.is_dir != b.is_dir {
            return if a.is_dir { Ordering::Less } else { Ordering::Greater };
        }
        let by_name = || natural_cmp(a.name.trim_start_matches('.'), b.name.trim_start_matches('.'));
        let o = match s.key {
            SortKey::Name => by_name(),
            SortKey::Modified => a.mtime.cmp(&b.mtime).then_with(by_name),
            SortKey::Type => {
                let ta = if a.is_dir { String::new() } else { a.description() };
                let tb = if b.is_dir { String::new() } else { b.description() };
                natural_cmp(&ta, &tb).then_with(by_name)
            }
            SortKey::Size => {
                let sa = if a.is_dir { a.children.unwrap_or(0) as u64 } else { a.size };
                let sb = if b.is_dir { b.children.unwrap_or(0) as u64 } else { b.size };
                sa.cmp(&sb).then_with(by_name)
            }
        };
        if s.descending {
            o.reverse()
        } else {
            o
        }
    });
}

/// Отобранные и отсортированные записи для показа.
pub fn visible(raw: &[Entry], sort: Sort, show_hidden: bool, filter: &str) -> Arc<Vec<Entry>> {
    let f = filter.trim().to_lowercase();
    let mut v: Vec<Entry> = raw
        .iter()
        .filter(|e| show_hidden || !e.hidden)
        .filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f))
        .cloned()
        .collect();
    sort_entries(&mut v, sort);
    Arc::new(v)
}

// ---------------------------------------------------------------- формат

/// «1,4 МБ» (двоичные единицы, как в Проводнике).
pub fn format_size(n: u64) -> String {
    const U: [&str; 6] = ["Б", "КБ", "МБ", "ГБ", "ТБ", "ПБ"];
    if n < 1024 {
        return format!("{n} Б");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let s = if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    format!("{} {}", s.replace('.', ","), U[i])
}

pub fn format_count(n: u32) -> String {
    let word = match (n % 10, n % 100) {
        (1, x) if x != 11 => "элемент",
        (2..=4, x) if !(12..=14).contains(&x) => "элемента",
        _ => "элементов",
    };
    format!("{n} {word}")
}

/// Локальное время: (год, месяц, день, час, минута).
pub fn local_time(secs: i64) -> (i64, u32, u32, u32, u32) {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    (tm.tm_year as i64 + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32)
}

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// «Сегодня, 14:05», «Вчера, 09:12», «12.03.2025 18:40».
pub fn format_time(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let (y, mo, d, h, mi) = local_time(secs);
    let (ty, tmo, td, _, _) = local_time(now_secs());
    let (yy, ymo, yd, _, _) = local_time(now_secs() - 86_400);
    if (y, mo, d) == (ty, tmo, td) {
        format!("Сегодня, {h:02}:{mi:02}")
    } else if (y, mo, d) == (yy, ymo, yd) {
        format!("Вчера, {h:02}:{mi:02}")
    } else {
        format!("{d:02}.{mo:02}.{y} {h:02}:{mi:02}")
    }
}

/// `rwxr-xr-x`.
pub fn format_mode(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural() {
        let mut v = vec!["file10", "File2", "file1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "file1", "File2", "file10"]);
    }

    #[test]
    fn sizes_and_counts() {
        assert_eq!(format_size(512), "512 Б");
        assert_eq!(format_size(1536), "1,5 КБ");
        assert_eq!(format_size(50 * 1024 * 1024), "50 МБ");
        assert_eq!(format_count(1), "1 элемент");
        assert_eq!(format_count(3), "3 элемента");
        assert_eq!(format_count(11), "11 элементов");
        assert_eq!(format_count(22), "22 элемента");
        assert_eq!(format_mode(0o755), "rwxr-xr-x");
    }
}
