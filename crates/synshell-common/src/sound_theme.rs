//! Звуковые темы по спецификации freedesktop (XDG Sound Theme): каталоги
//! `$XDG_DATA_HOME/sounds` и `$XDG_DATA_DIRS/sounds`, в теме — `index.theme`
//! (`[Sound Theme]`: `Name`, `Comment`, `Inherits`, `Directories`) и файлы
//! событий `stereo/<событие>.{oga,ogg,wav,flac}`. Имя события
//! укорачивается по дефисам (`message-new-instant` → `message-new` →
//! `message`); не нашлось в теме — ищется в наследуемых, затем в `synshell`
//! и `freedesktop`.

use std::path::PathBuf;

/// Тема по умолчанию (своя; в ней есть и события игрового режима).
pub const DEFAULT_THEME: &str = "synshell";

const EXTS: [&str; 4] = ["oga", "ogg", "wav", "flac"];

/// Каталоги, где лежат темы (по убыванию приоритета).
pub fn theme_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::expand_tilde("~/.local/share"));
    v.push(data_home.join("sounds"));
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for d in dirs.split(':').filter(|d| !d.is_empty()) {
        let p = PathBuf::from(d).join("sounds");
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

/// Сведения о теме из `index.theme`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeInfo {
    /// Имя каталога (то, что пишется в настройки).
    pub id: String,
    pub name: String,
    pub comment: String,
    pub inherits: Vec<String>,
    pub directories: Vec<String>,
    pub path: PathBuf,
}

fn parse_index(id: &str, path: PathBuf) -> Option<ThemeInfo> {
    let text = std::fs::read_to_string(path.join("index.theme")).ok()?;
    let lang = std::env::var("LANG").unwrap_or_default();
    let short = lang.split(['_', '.']).next().unwrap_or("").to_string();
    let mut info = ThemeInfo { id: id.to_string(), path, ..Default::default() };
    let mut in_section = false;
    let mut name_loc = None;
    let mut comment_loc = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[Sound Theme]";
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim().to_string());
        match k {
            "Name" => info.name = v,
            "Comment" => info.comment = v,
            "Inherits" => info.inherits = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "Directories" => info.directories = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            _ if !short.is_empty() && (k == format!("Name[{short}]")) => name_loc = Some(v),
            _ if !short.is_empty() && (k == format!("Comment[{short}]")) => comment_loc = Some(v),
            _ => {}
        }
    }
    if let Some(n) = name_loc {
        info.name = n;
    }
    if let Some(c) = comment_loc {
        info.comment = c;
    }
    if info.name.is_empty() {
        info.name = id.to_string();
    }
    if info.directories.is_empty() {
        info.directories = vec!["stereo".into()];
    }
    Some(info)
}

/// Тема по имени (первая найденная в каталогах).
pub fn theme(id: &str) -> Option<ThemeInfo> {
    theme_in(&theme_dirs(), id)
}

fn theme_in(dirs: &[PathBuf], id: &str) -> Option<ThemeInfo> {
    if id.is_empty() || id.contains('/') {
        return None;
    }
    dirs.iter().map(|d| d.join(id)).find(|p| p.join("index.theme").is_file()).and_then(|p| parse_index(id, p))
}

/// Все установленные темы (без повторов), по имени.
pub fn list() -> Vec<ThemeInfo> {
    list_in(&theme_dirs())
}

fn list_in(dirs: &[PathBuf]) -> Vec<ThemeInfo> {
    let mut out: Vec<ThemeInfo> = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if out.iter().any(|t| t.id == id) {
                continue;
            }
            if let Some(t) = parse_index(&id, e.path()) {
                // Служебные «темы» без звуков (только наследование) тоже годятся.
                out.push(t);
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Варианты имени события: полное, затем укороченные по дефисам.
fn event_names(event: &str) -> Vec<&str> {
    let mut v = vec![event];
    let mut s = event;
    while let Some(i) = s.rfind('-') {
        s = &s[..i];
        v.push(s);
    }
    v
}

/// Порядок обхода тем: выбранная, её предки (в ширину), затем
/// `synshell` и `freedesktop`.
fn chain(dirs: &[PathBuf], theme_id: &str) -> Vec<ThemeInfo> {
    let mut out: Vec<ThemeInfo> = Vec::new();
    let mut queue: Vec<String> = vec![theme_id.to_string()];
    let mut i = 0;
    while i < queue.len() && i < 16 {
        let id = queue[i].clone();
        i += 1;
        if out.iter().any(|t| t.id == id) {
            continue;
        }
        if let Some(t) = theme_in(dirs, &id) {
            queue.extend(t.inherits.iter().cloned());
            out.push(t);
        }
        if i == queue.len() {
            for f in [DEFAULT_THEME, "freedesktop"] {
                if !queue.iter().any(|q| q == f) {
                    queue.push(f.to_string());
                }
            }
        }
    }
    out
}

/// Файл звука события в теме (с наследованием и укорачиванием имени).
/// Полное имя ищется по всей цепочке тем раньше укороченного.
pub fn lookup(theme_id: &str, event: &str) -> Option<PathBuf> {
    lookup_in(&theme_dirs(), theme_id, event)
}

fn lookup_in(dirs: &[PathBuf], theme_id: &str, event: &str) -> Option<PathBuf> {
    let themes = chain(dirs, theme_id);
    for name in event_names(event) {
        for t in &themes {
            for d in &t.directories {
                for ext in EXTS {
                    let p = t.path.join(d).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_shorten() {
        assert_eq!(event_names("message-new-instant"), vec!["message-new-instant", "message-new", "message"]);
        assert_eq!(event_names("bell"), vec!["bell"]);
    }

    #[test]
    fn lookup_with_inheritance() {
        let root = std::env::temp_dir().join(format!("synshell-sounds-{}", std::process::id()));
        let mk = |id: &str, inherits: &str, files: &[&str]| {
            let d = root.join("sounds").join(id);
            std::fs::create_dir_all(d.join("stereo")).unwrap();
            std::fs::write(d.join("index.theme"), format!("[Sound Theme]\nName={id}\nInherits={inherits}\nDirectories=stereo\n")).unwrap();
            for f in files {
                std::fs::write(d.join("stereo").join(f), b"x").unwrap();
            }
        };
        mk("base", "", &["bell.wav", "message.oga"]);
        mk("child", "base", &["message-new-instant.ogg"]);
        let dirs = vec![root.join("sounds")];
        assert!(lookup_in(&dirs, "child", "message-new-instant").unwrap().ends_with("child/stereo/message-new-instant.ogg"));
        assert!(lookup_in(&dirs, "child", "bell").unwrap().ends_with("base/stereo/bell.wav"));
        assert!(lookup_in(&dirs, "child", "message-new-email").unwrap().ends_with("base/stereo/message.oga"));
        assert!(lookup_in(&dirs, "child", "nothing").is_none());
        assert_eq!(list_in(&dirs).iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["base", "child"]);
        let _ = std::fs::remove_dir_all(root);
    }
}
