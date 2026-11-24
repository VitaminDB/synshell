//! Шрифты титров: папка «Шрифты» библиотеки ассетов. Указатель «семейство →
//! файлы» строится по таблице name каждого файла и хранится на диске
//! (`~/.cache/syn-video-editor/fonts.json`, по размеру и времени файла), так
//! что повторный запуск не читает 50 МБ шрифтов. Файлы семейства
//! регистрируются в syngui по требованию — когда титр или окно выбора его
//! показывает; системные шрифты (fontconfig) работают и без этого.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    /// путь → (размер, время изменения, семейство)
    files: HashMap<String, (u64, u64, String)>,
}

fn cache_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/fonts.json")
}

/// Папка шрифтов библиотеки.
pub fn dir() -> PathBuf {
    crate::fusion::library().join("Шрифты")
}

fn stamp(p: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(p).ok()?;
    let t = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((md.len(), t))
}

fn scan(dir: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut cache: Cache = std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut changed = false;
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut files: Vec<PathBuf> = walk(dir);
    files.sort();
    for p in files {
        let Some((len, t)) = stamp(&p) else { continue };
        let key = p.display().to_string();
        let fam = match cache.files.get(&key) {
            Some((l, m, f)) if *l == len && *m == t => f.clone(),
            _ => {
                let f = std::fs::read(&p).ok().and_then(|d| syngui::text::font_family_name(&d)).unwrap_or_default();
                cache.files.insert(key, (len, t, f.clone()));
                changed = true;
                f
            }
        };
        if !fam.is_empty() {
            out.entry(fam).or_default().push(p);
        }
    }
    if changed {
        if let Some(d) = cache_path().parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(cache_path(), serde_json::to_vec(&cache).unwrap_or_default());
    }
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return v };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            v.extend(walk(&p));
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf")) {
            v.push(p);
        }
    }
    v
}

/// Семейства библиотеки по алфавиту с их файлами.
pub fn library() -> &'static BTreeMap<String, Vec<PathBuf>> {
    static L: OnceLock<BTreeMap<String, Vec<PathBuf>>> = OnceLock::new();
    L.get_or_init(|| scan(&dir()))
}

/// Названия семейств (для списка и агента).
pub fn families() -> Vec<String> {
    library().keys().cloned().collect()
}

/// Подключить семейство библиотеки (один раз); неизвестное — системный шрифт.
pub fn ensure(family: &str) {
    static DONE: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    {
        let mut d = DONE.lock().unwrap();
        if !d.get_or_insert_with(HashSet::new).insert(family.to_string()) {
            return;
        }
    }
    if let Some(files) = library().get(family) {
        for f in files {
            if let Ok(data) = std::fs::read(f) {
                syngui::text::register_font(family, data);
            }
        }
    }
}
