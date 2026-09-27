//! Корзина по спецификации freedesktop.org (Trash 1.0).
//!
//! Домашняя корзина — `$XDG_DATA_HOME/Trash/{files,info}`. Файлы с других
//! разделов уходят в корзину своего раздела: `$top/.Trash/$uid` (если
//! администратор создал `.Trash` с битом sticky) или `$top/.Trash-$uid`, —
//! иначе удаление в корзину было бы копированием на другой диск.

use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use syndesktop_common::paths;

use crate::model::{self, Entry, TrashInfo};

/// Удалённое в корзину — для отмены.
#[derive(Debug, Clone)]
pub struct Trashed {
    /// Откуда удалено.
    #[allow(dead_code)]
    pub original: PathBuf,
    pub file: PathBuf,
    #[allow(dead_code)]
    pub info: PathBuf,
}

pub fn home_trash() -> PathBuf {
    paths::data_home().join("Trash")
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Все корзины: домашняя и найденные на смонтированных разделах.
pub fn all_trashes() -> Vec<PathBuf> {
    let mut v = vec![home_trash()];
    for top in mount_points() {
        for t in [top.join(".Trash").join(uid().to_string()), top.join(format!(".Trash-{}", uid()))] {
            if t.join("files").is_dir() && !v.contains(&t) {
                v.push(t);
            }
        }
    }
    v
}

fn mount_points() -> Vec<PathBuf> {
    let Ok(t) = std::fs::read_to_string("/proc/self/mounts") else { return Vec::new() };
    t.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let dev = it.next()?;
            let mp = it.next()?;
            dev.starts_with('/').then(|| PathBuf::from(unescape_mount(mp)))
        })
        .collect()
}

fn unescape_mount(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
}

/// Точка монтирования раздела, на котором лежит путь.
fn topdir(path: &Path) -> Option<PathBuf> {
    let dev = std::fs::symlink_metadata(path).ok()?.dev();
    let mut cur = path.parent()?.to_path_buf();
    loop {
        let parent = cur.parent().map(Path::to_path_buf);
        match parent {
            Some(p) if std::fs::metadata(&p).map(|m| m.dev() == dev).unwrap_or(false) => cur = p,
            _ => return Some(cur),
        }
    }
}

/// Корзина для пути.
fn trash_for(path: &Path) -> std::io::Result<PathBuf> {
    let home = home_trash();
    let _ = std::fs::create_dir_all(home.join("files"));
    let _ = std::fs::create_dir_all(home.join("info"));
    let home_dev = std::fs::metadata(&home).map(|m| m.dev()).ok();
    let dev = std::fs::symlink_metadata(path)?.dev();
    if home_dev == Some(dev) {
        return Ok(home);
    }
    let top = topdir(path).ok_or_else(|| std::io::Error::other("не найден раздел"))?;
    let admin = top.join(".Trash");
    if let Ok(m) = std::fs::symlink_metadata(&admin) {
        // Sticky-бит и не ссылка — иначе небезопасно.
        if m.is_dir() && !m.file_type().is_symlink() && m.mode() & 0o1000 != 0 {
            let t = admin.join(uid().to_string());
            if std::fs::create_dir_all(t.join("files")).is_ok() && std::fs::create_dir_all(t.join("info")).is_ok() {
                return Ok(t);
            }
        }
    }
    let t = top.join(format!(".Trash-{}", uid()));
    std::fs::create_dir_all(t.join("files"))?;
    std::fs::create_dir_all(t.join("info"))?;
    Ok(t)
}

fn percent_encode(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

fn percent_decode(s: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

fn now_iso() -> String {
    let (y, mo, d, h, mi) = model::local_time(model::now_secs());
    let s = model::now_secs() % 60;
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// Переместить в корзину.
pub fn trash(path: &Path) -> std::io::Result<Trashed> {
    let path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let t = trash_for(&path)?;
    let name = path.file_name().ok_or_else(|| std::io::Error::other("нет имени"))?.to_string_lossy().to_string();
    // Уникальное имя: info создаётся с O_EXCL — это и есть «замок».
    let mut n = 0;
    let (info_path, mut info) = loop {
        let cand = if n == 0 { name.clone() } else { numbered(&name, n) };
        let ip = t.join("info").join(format!("{cand}.trashinfo"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&ip) {
            Ok(f) => break (ip, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    };
    let stem = info_path.file_name().unwrap().to_string_lossy().trim_end_matches(".trashinfo").to_string();
    let file_path = t.join("files").join(&stem);
    // В домашней корзине путь абсолютный, в корзине раздела — от его корня.
    let shown = if t == home_trash() {
        path.clone()
    } else {
        topdir(&path).and_then(|top| path.strip_prefix(top).ok().map(Path::to_path_buf)).unwrap_or(path.clone())
    };
    writeln!(info, "[Trash Info]\nPath={}\nDeletionDate={}", percent_encode(&shown), now_iso())?;
    if let Err(e) = std::fs::rename(&path, &file_path) {
        let _ = std::fs::remove_file(&info_path);
        return Err(e);
    }
    Ok(Trashed { original: path, file: file_path, info: info_path })
}

fn numbered(name: &str, n: usize) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

/// Вернуть из корзины (после отмены или «Восстановить»).
pub fn restore(file: &Path) -> std::io::Result<PathBuf> {
    let info = info_for(file).ok_or_else(|| std::io::Error::other("нет .trashinfo"))?;
    let original = read_info(&info, file).map(|i| i.original).ok_or_else(|| std::io::Error::other("нет исходного пути"))?;
    if original.exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} уже существует", original.display())));
    }
    if let Some(p) = original.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::rename(file, &original).or_else(|_| {
        crate::ops::copy_recursive_simple(file, &original)?;
        crate::ops::remove_recursive(file)
    })?;
    let _ = std::fs::remove_file(&info);
    Ok(original)
}

fn info_for(file: &Path) -> Option<PathBuf> {
    let t = file.parent()?.parent()?;
    let name = file.file_name()?.to_string_lossy();
    Some(t.join("info").join(format!("{name}.trashinfo")))
}

fn read_info(info: &Path, file: &Path) -> Option<TrashInfo> {
    let t = std::fs::read_to_string(info).ok()?;
    let mut path = None;
    let mut date = String::new();
    for l in t.lines() {
        if let Some(v) = l.strip_prefix("Path=") {
            path = Some(percent_decode(v));
        } else if let Some(v) = l.strip_prefix("DeletionDate=") {
            date = v.replace('T', " ");
        }
    }
    let mut p = path?;
    if p.is_relative() {
        // Корзина раздела: путь от корня раздела.
        let trash_dir = file.parent()?.parent()?;
        let top = if trash_dir.file_name()?.to_string_lossy().starts_with(".Trash-") {
            trash_dir.parent()?
        } else {
            trash_dir.parent()?.parent()?
        };
        p = top.join(p);
    }
    Some(TrashInfo { original: p, deleted: date })
}

/// Содержимое всех корзин (записи из `files/` с исходным путём).
pub fn list() -> Vec<Entry> {
    let mut out = Vec::new();
    for t in all_trashes() {
        let Ok(rd) = std::fs::read_dir(t.join("files")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Some(mut entry) = Entry::from_path(&p) else { continue };
            if let Some(info) = info_for(&p).and_then(|i| read_info(&i, &p)) {
                entry.name = info.original.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(entry.name);
                entry.hidden = false;
                entry.trash = Some(info);
            }
            out.push(entry);
        }
    }
    out
}

pub fn is_empty() -> bool {
    all_trashes().iter().all(|t| std::fs::read_dir(t.join("files")).map(|mut d| d.next().is_none()).unwrap_or(true))
}

/// Удалить из корзины навсегда.
pub fn purge(file: &Path) -> std::io::Result<()> {
    crate::ops::remove_recursive(file)?;
    if let Some(i) = info_for(file) {
        let _ = std::fs::remove_file(i);
    }
    Ok(())
}

pub fn empty() -> std::io::Result<()> {
    for t in all_trashes() {
        for sub in ["files", "info"] {
            if let Ok(rd) = std::fs::read_dir(t.join(sub)) {
                for e in rd.flatten() {
                    crate::ops::remove_recursive(&e.path())?;
                }
            }
        }
        let _ = std::fs::remove_file(t.join("directorysizes"));
    }
    Ok(())
}

/// Лежит ли путь внутри какой-нибудь корзины.
pub fn contains(path: &Path) -> bool {
    all_trashes().iter().any(|t| path.starts_with(t.join("files")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip() {
        let p = Path::new("/tmp/Мой файл #1.txt");
        assert_eq!(percent_decode(&percent_encode(p)), p);
        assert_eq!(numbered("a.txt", 2), "a (2).txt");
        assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
    }

    #[test]
    fn trash_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::env::set_var("XDG_DATA_HOME", &data);
        let f = dir.path().join("x.txt");
        std::fs::write(&f, "hi").unwrap();
        let t = trash(&f).unwrap();
        assert!(!f.exists());
        assert!(t.file.exists() && t.info.exists());
        let l = list();
        assert!(l.iter().any(|e| e.trash.as_ref().map(|i| i.original == f).unwrap_or(false)));
        restore(&t.file).unwrap();
        assert!(f.exists());
        assert!(!t.info.exists());
    }
}
