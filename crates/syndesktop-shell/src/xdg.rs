//! Приложения (.desktop) и значки по спецификациям freedesktop.org.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use syndesktop_common::paths;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DesktopEntry {
    /// Идентификатор: имя файла без `.desktop` (`org.kde.dolphin`).
    pub id: String,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub keywords: Vec<String>,
    pub exec: String,
    pub icon: String,
    pub terminal: bool,
    pub categories: Vec<String>,
    pub wm_class: String,
    /// Не показывать в меню (но значок для окон брать можно).
    pub no_display: bool,
    pub path: PathBuf,
    /// Рабочий каталог (`Path=`).
    pub workdir: String,
}

impl DesktopEntry {
    /// Команда запуска без кодов полей (`%u`, `%F`…).
    pub fn command(&self) -> String {
        let mut out = String::new();
        let mut chars = self.exec.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('%') => out.push('%'),
                    Some('i') if !self.icon.is_empty() => {
                        out.push_str(&format!("--icon '{}'", self.icon.replace('\'', "")));
                    }
                    Some('c') => out.push_str(&format!("'{}'", self.name.replace('\'', ""))),
                    Some('k') => out.push_str(&format!("'{}'", self.path.display())),
                    _ => {}
                }
            } else {
                out.push(c);
            }
        }
        let cmd = out.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.workdir.is_empty() {
            cmd
        } else {
            format!("cd '{}' && {cmd}", self.workdir.replace('\'', ""))
        }
    }

    /// Главная категория для меню.
    pub fn main_category(&self) -> &'static str {
        for c in &self.categories {
            let m = match c.as_str() {
                "AudioVideo" | "Audio" | "Video" => "multimedia",
                "Development" => "development",
                "Education" | "Science" => "education",
                "Game" => "games",
                "Graphics" => "graphics",
                "Network" => "internet",
                "Office" => "office",
                "Settings" => "settings",
                "System" => "system",
                "Utility" => "utilities",
                _ => continue,
            };
            return m;
        }
        "other"
    }
}

/// Категории меню в порядке показа: (ключ, подпись, значок Material).
pub const CATEGORIES: &[(&str, &str, &str)] = &[
    ("internet", "Интернет", "\u{E80B}"),
    ("multimedia", "Мультимедиа", "\u{E405}"),
    ("graphics", "Графика", "\u{E3F4}"),
    ("office", "Офис", "\u{E873}"),
    ("development", "Разработка", "\u{E86F}"),
    ("education", "Наука и образование", "\u{E80C}"),
    ("games", "Игры", "\u{E338}"),
    ("utilities", "Служебные", "\u{E869}"),
    ("settings", "Настройки", "\u{E8B8}"),
    ("system", "Система", "\u{E30A}"),
    ("other", "Прочее", "\u{E5D3}"),
];

fn locale_keys() -> Vec<String> {
    let lang = std::env::var("LC_ALL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default();
    let base = lang.split('.').next().unwrap_or("").split('@').next().unwrap_or("").to_string();
    let mut keys = Vec::new();
    if !base.is_empty() && base != "C" && base != "POSIX" {
        keys.push(base.clone());
        if let Some((l, _)) = base.split_once('_') {
            keys.push(l.to_string());
        }
    }
    keys
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_else(|_| "syndesktop".into())
        .split(':')
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

pub fn parse_desktop_file(path: &Path, id: String, locales: &[String]) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_group = false;
    let mut kv: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let get = |k: &str| -> Option<String> {
        for l in locales {
            if let Some(v) = kv.get(&format!("{k}[{l}]")) {
                return Some(unescape(v));
            }
        }
        kv.get(k).map(|v| unescape(v))
    };
    if kv.get("Type").map(String::as_str) != Some("Application") {
        return None;
    }
    if kv.get("Hidden").map(String::as_str) == Some("true") {
        return None;
    }
    let desktops = current_desktops();
    if let Some(only) = kv.get("OnlyShowIn") {
        if !only.split(';').any(|d| !d.is_empty() && desktops.contains(&d.to_ascii_lowercase())) {
            return None;
        }
    }
    let mut no_display = kv.get("NoDisplay").map(String::as_str) == Some("true");
    if let Some(not) = kv.get("NotShowIn") {
        if not.split(';').any(|d| desktops.contains(&d.to_ascii_lowercase())) {
            no_display = true;
        }
    }
    if let Some(try_exec) = kv.get("TryExec") {
        let p = Path::new(try_exec);
        let ok = if p.is_absolute() { p.exists() } else { crate::actions::which(try_exec) };
        if !ok {
            return None;
        }
    }
    let list = |s: Option<String>| -> Vec<String> {
        s.map(|s| s.split(';').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default()
    };
    Some(DesktopEntry {
        id,
        name: get("Name")?,
        generic_name: get("GenericName").unwrap_or_default(),
        comment: get("Comment").unwrap_or_default(),
        keywords: list(get("Keywords")),
        exec: kv.get("Exec").cloned().unwrap_or_default(),
        icon: kv.get("Icon").cloned().unwrap_or_default(),
        terminal: kv.get("Terminal").map(String::as_str) == Some("true"),
        categories: list(kv.get("Categories").cloned()),
        wm_class: kv.get("StartupWMClass").cloned().unwrap_or_default(),
        no_display,
        path: path.to_path_buf(),
        workdir: kv.get("Path").cloned().unwrap_or_default(),
    })
}

/// Все приложения из `$XDG_DATA_DIRS/applications`.
pub fn load_apps() -> Vec<DesktopEntry> {
    let locales = locale_keys();
    let mut by_id: HashMap<String, DesktopEntry> = HashMap::new();
    for dir in paths::data_dirs() {
        let root = dir.join("applications");
        walk(&root, &root, &mut |path, id| {
            if by_id.contains_key(&id) {
                return; // более ранний каталог главнее
            }
            if let Some(e) = parse_desktop_file(path, id.clone(), &locales) {
                by_id.insert(id, e);
            } else {
                // Скрытая запись перекрывает системную с тем же id.
                by_id.insert(id.clone(), DesktopEntry { id, no_display: true, ..Default::default() });
            }
        });
    }
    let mut v: Vec<DesktopEntry> = by_id.into_values().filter(|e| !e.name.is_empty()).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

fn walk(root: &Path, dir: &Path, f: &mut dyn FnMut(&Path, String)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, f);
        } else if p.extension().is_some_and(|x| x == "desktop") {
            let rel = p.strip_prefix(root).unwrap_or(&p).with_extension("");
            let id = rel.to_string_lossy().replace('/', "-");
            f(&p, id);
        }
    }
}

// ─── База приложений ─────────────────────────────────────────────────────────

static APPS: OnceLock<RwLock<Arc<Vec<DesktopEntry>>>> = OnceLock::new();

pub fn apps() -> Arc<Vec<DesktopEntry>> {
    APPS.get_or_init(|| RwLock::new(Arc::new(load_apps())))
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn reload_apps() {
    let v = Arc::new(load_apps());
    let lock = APPS.get_or_init(|| RwLock::new(v.clone()));
    if let Ok(mut g) = lock.write() {
        *g = v;
    }
}

pub fn app_by_id(id: &str) -> Option<DesktopEntry> {
    apps().iter().find(|e| e.id == id).cloned()
}

/// Приложение окна по `app_id` (Wayland) — разными эвристиками.
pub fn app_for_window(app_id: &str) -> Option<DesktopEntry> {
    if app_id.is_empty() {
        return None;
    }
    let apps = apps();
    let lc = app_id.to_lowercase();
    let last = lc.rsplit('.').next().unwrap_or(&lc).to_string();
    let score = |e: &DesktopEntry| -> u8 {
        let id = e.id.to_lowercase();
        if e.id == app_id {
            5
        } else if id == lc {
            4
        } else if !e.wm_class.is_empty() && e.wm_class.to_lowercase() == lc {
            3
        } else if id.rsplit('.').next() == Some(last.as_str()) {
            2
        } else if e.exec.split_whitespace().next().and_then(|x| x.rsplit('/').next()).map(|s| s.to_lowercase())
            == Some(last.clone())
        {
            1
        } else {
            0
        }
    };
    apps.iter().map(|e| (score(e), e)).filter(|(s, _)| *s > 0).max_by_key(|(s, _)| *s).map(|(_, e)| e.clone())
}

// ─── Значки ──────────────────────────────────────────────────────────────────

struct ThemeIndex {
    /// имя → (путь, «качество»: меньше — лучше).
    icons: HashMap<String, (PathBuf, u32)>,
    inherits: Vec<String>,
}

static ICONS: OnceLock<RwLock<HashMap<String, Arc<ThemeIndex>>>> = OnceLock::new();
static ICON_CACHE: OnceLock<RwLock<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static ICON_THEME: RwLock<String> = RwLock::new(String::new());

fn icon_base_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = std::env::var_os("HOME") {
        v.push(PathBuf::from(&h).join(".icons"));
    }
    for d in paths::data_dirs() {
        v.push(d.join("icons"));
    }
    v
}

fn load_theme(name: &str) -> Option<ThemeIndex> {
    let bases: Vec<PathBuf> = icon_base_dirs().into_iter().map(|b| b.join(name)).filter(|p| p.is_dir()).collect();
    if bases.is_empty() {
        return None;
    }
    let mut inherits = Vec::new();
    let mut dirs: Vec<(String, u32, bool)> = Vec::new(); // (подкаталог, размер, scalable)
    for b in &bases {
        let Ok(text) = std::fs::read_to_string(b.join("index.theme")) else { continue };
        let mut group = String::new();
        let mut cur_size = 0u32;
        let mut cur_scalable = false;
        let mut cur_scale = 1u32;
        let mut sections: HashMap<String, (u32, bool)> = HashMap::new();
        let mut flush = |g: &str, size: u32, sc: bool, scale: u32, sections: &mut HashMap<String, (u32, bool)>| {
            if !g.is_empty() && g != "Icon Theme" && scale == 1 {
                sections.insert(g.to_string(), (size, sc));
            }
        };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                flush(&group, cur_size, cur_scalable, cur_scale, &mut sections);
                group = line[1..line.len() - 1].to_string();
                cur_size = 0;
                cur_scalable = false;
                cur_scale = 1;
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if group == "Icon Theme" && k == "Inherits" && inherits.is_empty() {
                inherits = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            match k {
                "Size" => cur_size = v.parse().unwrap_or(0),
                "Type" => cur_scalable = v == "Scalable",
                "Scale" => cur_scale = v.parse().unwrap_or(1),
                _ => {}
            }
        }
        flush(&group, cur_size, cur_scalable, cur_scale, &mut sections);
        for (g, (s, sc)) in sections {
            dirs.push((g, s, sc));
        }
    }
    let mut icons: HashMap<String, (PathBuf, u32)> = HashMap::new();
    for b in &bases {
        for (sub, size, scalable) in &dirs {
            let d = b.join(sub);
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            // Качество: масштабируемые и 48–64 px — лучшие для панели и меню.
            let q = if *scalable { 0 } else { (*size as i32 - 48).unsigned_abs() + 1 };
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                if ext != "svg" && ext != "png" {
                    continue;
                }
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
                let q = if ext == "png" { q } else { q.min(1) };
                match icons.get(stem) {
                    Some((_, old)) if *old <= q => {}
                    _ => {
                        icons.insert(stem.to_string(), (p.clone(), q));
                    }
                }
            }
        }
    }
    Some(ThemeIndex { icons, inherits })
}

fn theme(name: &str) -> Option<Arc<ThemeIndex>> {
    let map = ICONS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(t) = map.read().ok()?.get(name) {
        return Some(t.clone());
    }
    let t = Arc::new(load_theme(name)?);
    map.write().ok()?.insert(name.to_string(), t.clone());
    Some(t)
}

/// Задать тему значков (из конфига) и сбросить кэш поиска.
pub fn set_icon_theme(name: &str) {
    if let Ok(mut g) = ICON_THEME.write() {
        if *g == name {
            return;
        }
        *g = name.to_string();
    }
    if let Some(c) = ICON_CACHE.get() {
        if let Ok(mut c) = c.write() {
            c.clear();
        }
    }
}

/// Прогреть индекс тем в фоне (первый поиск иначе займёт десятки мс).
pub fn warm_up() {
    std::thread::spawn(|| {
        let _ = lookup_icon("application-x-executable");
        let _ = apps();
    });
}

/// Путь к значку по имени (или абсолютному пути).
pub fn lookup_icon(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let p = Path::new(name);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let cache = ICON_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(v) = cache.read().ok().and_then(|c| c.get(name).cloned()) {
        return v;
    }
    let found = lookup_uncached(name);
    if let Ok(mut c) = cache.write() {
        c.insert(name.to_string(), found.clone());
    }
    found
}

fn lookup_uncached(name: &str) -> Option<PathBuf> {
    let first = ICON_THEME.read().map(|g| g.clone()).unwrap_or_default();
    let mut queue: Vec<String> = Vec::new();
    if !first.is_empty() {
        queue.push(first);
    }
    queue.extend(["breeze".to_string(), "Adwaita".to_string(), "hicolor".to_string()]);
    let mut seen = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        let t = queue[i].clone();
        i += 1;
        if seen.contains(&t) {
            continue;
        }
        seen.push(t.clone());
        if let Some(idx) = theme(&t) {
            if let Some((p, _)) = idx.icons.get(name) {
                return Some(p.clone());
            }
            // Наследники — сразу за темой, перед запасными.
            for (k, inh) in idx.inherits.iter().enumerate() {
                queue.insert(i + k, inh.clone());
            }
        }
    }
    for ext in ["svg", "png", "xpm"] {
        let p = PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}"));
        if p.exists() && ext != "xpm" {
            return Some(p);
        }
    }
    None
}

/// Значок приложения окна: из .desktop, иначе по app_id, иначе общий.
pub fn window_icon(app_id: &str) -> Option<PathBuf> {
    app_for_window(app_id)
        .and_then(|e| lookup_icon(&e.icon))
        .or_else(|| lookup_icon(app_id))
        .or_else(|| lookup_icon(&app_id.to_lowercase()))
        .or_else(|| lookup_icon("application-x-executable"))
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "зависит от установленных тем значков"]
    fn lookup_real_icons() {
        super::set_icon_theme("breeze-dark");
        for n in ["utilities-terminal", "org.kde.konsole", "firefox", "system-file-manager"] {
            eprintln!("{n} -> {:?}", super::lookup_icon(n));
        }
        let apps = super::apps();
        eprintln!("apps: {}", apps.len());
        for a in apps.iter().filter(|a| a.id.contains("konsole") || a.id.contains("firefox")) {
            eprintln!("{} {:?} icon={} -> {:?}", a.id, a.name, a.icon, super::lookup_icon(&a.icon));
        }
    }
}
