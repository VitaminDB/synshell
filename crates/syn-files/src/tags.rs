//! Метки и оценки файлов в расширенных атрибутах — как у Dolphin: метки в
//! `user.xdg.tags` (через запятую), оценка в `user.baloo.rating` (0–10,
//! звезда — 2). Цветные метки хранятся английскими именами (Red, Blue…),
//! показываются по-русски; чужие метки (из Dolphin) видны серыми.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use syngui::t;

const TAGS: &str = "user.xdg.tags";
const RATING: &str = "user.baloo.rating";

/// Цветные метки: (имя в атрибуте, цвет).
pub const COLORS: [(&str, &str); 7] = [
    ("Red", "#e5534b"),
    ("Orange", "#e8913a"),
    ("Yellow", "#e3c341"),
    ("Green", "#57ab5a"),
    ("Blue", "#4c8ff0"),
    ("Purple", "#a371f7"),
    ("Gray", "#8b949e"),
];

pub fn color_title(name: &str) -> String {
    match name {
        "Red" => t!("Красная").into(),
        "Orange" => t!("Оранжевая").into(),
        "Yellow" => t!("Жёлтая").into(),
        "Green" => t!("Зелёная").into(),
        "Blue" => t!("Синяя").into(),
        "Purple" => t!("Фиолетовая").into(),
        "Gray" => t!("Серая").into(),
        other => other.to_string(),
    }
}

/// Цвет метки (чужие — серым).
pub fn color_of(tag: &str) -> &'static str {
    COLORS.iter().find(|(n, _)| *n == tag).map(|(_, c)| *c).unwrap_or("#8b949e")
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub tags: Vec<String>,
    /// 0–10.
    pub rating: u8,
}

fn cpath(p: &Path) -> Option<CString> {
    CString::new(p.as_os_str().as_bytes()).ok()
}

fn get_attr(p: &Path, name: &str) -> Option<String> {
    let (cp, cn) = (cpath(p)?, CString::new(name).ok()?);
    let mut buf = vec![0u8; 1024];
    let n = unsafe { libc::getxattr(cp.as_ptr(), cn.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string())
}

fn set_attr(p: &Path, name: &str, value: Option<&str>) -> std::io::Result<()> {
    let cp = cpath(p).ok_or_else(|| std::io::Error::other("nul"))?;
    let cn = CString::new(name).unwrap();
    let r = match value {
        Some(v) => unsafe { libc::setxattr(cp.as_ptr(), cn.as_ptr(), v.as_ptr().cast(), v.len(), 0) },
        None => {
            let r = unsafe { libc::removexattr(cp.as_ptr(), cn.as_ptr()) };
            // Снять то, чего нет, — не ошибка.
            if r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA) {
                0
            } else {
                r
            }
        }
    };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Метки и оценка прямо с диска.
pub fn read(p: &Path) -> Info {
    let tags = get_attr(p, TAGS)
        .map(|s| s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    let rating = get_attr(p, RATING).and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0).min(10);
    Info { tags, rating }
}

static CACHE: Mutex<Option<HashMap<PathBuf, Info>>> = Mutex::new(None);

/// Метки для показа (из кэша; первый раз — с диска).
pub fn get(p: &Path) -> Info {
    let mut g = CACHE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(i) = m.get(p) {
        return i.clone();
    }
    let i = read(p);
    m.insert(p.to_path_buf(), i.clone());
    i
}

/// Забыть прочитанное в папке (её перечитывают — могли поменять снаружи).
pub fn forget_dir(dir: &Path) {
    if let Some(m) = CACHE.lock().unwrap().as_mut() {
        m.retain(|p, _| p.parent() != Some(dir));
    }
}

fn store(p: &Path, i: &Info) {
    if let Some(m) = CACHE.lock().unwrap().as_mut() {
        m.insert(p.to_path_buf(), i.clone());
    }
}

pub fn set_tags(p: &Path, tags: &[String]) -> std::io::Result<()> {
    let joined = tags.join(",");
    set_attr(p, TAGS, (!tags.is_empty()).then_some(joined.as_str()))?;
    store(p, &Info { tags: tags.to_vec(), rating: read(p).rating });
    Ok(())
}

/// Переключить цветную метку у всех: есть у всех — снять, иначе — поставить.
pub fn toggle(paths: &[PathBuf], tag: &str) -> Result<(), String> {
    let all = paths.iter().all(|p| get(p).tags.iter().any(|t| t == tag));
    for p in paths {
        let mut tags = read(p).tags;
        tags.retain(|t| t != tag);
        if !all {
            tags.push(tag.to_string());
        }
        set_tags(p, &tags).map_err(|e| t!("Не удалось поставить метку «{v}»: {e}", v = crate::ops::name_of(p), e = e))?;
    }
    Ok(())
}

pub fn clear(paths: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        set_tags(p, &[]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Оценка 0–5 звёзд (в атрибуте — вдвое больше).
pub fn set_stars(paths: &[PathBuf], stars: u8) -> Result<(), String> {
    for p in paths {
        let v = (stars.min(5) * 2).to_string();
        set_attr(p, RATING, (stars > 0).then_some(v.as_str())).map_err(|e| t!("Не удалось поставить оценку «{v}»: {e}", v = crate::ops::name_of(p), e = e))?;
        store(p, &Info { tags: read(p).tags, rating: stars.min(5) * 2 });
    }
    Ok(())
}

/// Есть ли у файла метка `q` (по имени или русскому названию цвета).
pub fn has(p: &Path, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    !q.is_empty() && read(p).tags.iter().any(|t| t.to_lowercase() == q || color_title(t).to_lowercase().starts_with(&q))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_rating_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, "").unwrap();
        let paths = vec![f.clone()];
        if toggle(&paths, "Red").is_err() {
            eprintln!("ФС без user xattr — пропуск");
            return;
        }
        toggle(&paths, "Blue").unwrap();
        assert_eq!(read(&f).tags, ["Red", "Blue"]);
        assert!(has(&f, "red") && has(&f, &color_title("Blue")));
        toggle(&paths, "Red").unwrap();
        assert_eq!(read(&f).tags, ["Blue"]);
        set_stars(&paths, 4).unwrap();
        assert_eq!(read(&f).rating, 8);
        assert_eq!(get(&f), Info { tags: vec!["Blue".into()], rating: 8 });
        clear(&paths).unwrap();
        set_stars(&paths, 0).unwrap();
        assert_eq!(read(&f), Info::default());
    }
}
