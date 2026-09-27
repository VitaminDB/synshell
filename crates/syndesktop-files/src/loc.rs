//! Расположение, которое показывает панель: папка, корзина, результаты поиска.

use std::path::{Path, PathBuf};

use syndesktop_common::paths;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    Dir(PathBuf),
    Trash,
    /// Поиск по имени внутри `root`.
    Search { root: PathBuf, query: String },
}

impl Location {
    /// Разбор строки адреса: путь, `~`, `trash:`, `file://`.
    pub fn parse(s: &str) -> Option<Location> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s == "trash:" || s == "trash:/" || s.eq_ignore_ascii_case("корзина") {
            return Some(Location::Trash);
        }
        if s.starts_with("file://") {
            return syndesktop_common::xdg::path_from_uri(s).map(Location::Dir);
        }
        let p = paths::expand_tilde(s);
        let p = if p.is_relative() { paths::home().join(p) } else { p };
        Some(Location::Dir(normalize(&p)))
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Location::Dir(p) => Some(p),
            Location::Search { root, .. } => Some(root),
            Location::Trash => None,
        }
    }

    /// Папка, куда вставлять и создавать (для поиска — его корень).
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Location::Dir(p) => Some(p),
            _ => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Location::Trash => "Корзина".into(),
            Location::Search { query, .. } => format!("Поиск «{query}»"),
            Location::Dir(p) => {
                if let Some((_, t)) = crate::places::known(p) {
                    return t;
                }
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
            }
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Location::Trash => crate::ui::icons::TRASH,
            Location::Search { .. } => crate::ui::icons::SEARCH,
            Location::Dir(p) => crate::places::known(p).map(|(i, _)| i).unwrap_or(crate::ui::icons::FOLDER),
        }
    }

    /// Строка для поля адреса.
    pub fn address(&self) -> String {
        match self {
            Location::Trash => "trash:".into(),
            Location::Dir(p) | Location::Search { root: p, .. } => p.display().to_string(),
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Dir(p) => p.parent().map(|p| Location::Dir(p.to_path_buf())),
            Location::Search { root, .. } => Some(Location::Dir(root.clone())),
            Location::Trash => None,
        }
    }

    /// Сегменты хлебных крошек: (подпись, расположение). Домашняя папка —
    /// одним сегментом, а не `/ › home › user`.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        match self {
            Location::Trash => vec![("Корзина".into(), Location::Trash)],
            Location::Search { root, query } => {
                let mut v = Location::Dir(root.clone()).crumbs();
                v.push((format!("Поиск «{query}»"), self.clone()));
                v
            }
            Location::Dir(p) => {
                let home = paths::home();
                let (mut out, rest, mut acc) = if p.starts_with(&home) {
                    (vec![("Домашняя папка".to_string(), Location::Dir(home.clone()))], p.strip_prefix(&home).unwrap(), home)
                } else {
                    (vec![("/".to_string(), Location::Dir(PathBuf::from("/")))], p.strip_prefix("/").unwrap_or(p), PathBuf::from("/"))
                };
                for c in rest.components() {
                    acc.push(c);
                    out.push((c.as_os_str().to_string_lossy().to_string(), Location::Dir(acc.clone())));
                }
                out
            }
        }
    }
}

/// Убрать `.` и `..` без обращения к диску.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_crumbs() {
        assert_eq!(Location::parse("trash:"), Some(Location::Trash));
        assert_eq!(Location::parse("/usr/../etc/./x"), Some(Location::Dir(PathBuf::from("/etc/x"))));
        let c = Location::Dir(PathBuf::from("/usr/share")).crumbs();
        assert_eq!(c.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["/", "usr", "share"]);
        let home = paths::home();
        let c = Location::Dir(home.join("a/b")).crumbs();
        assert_eq!(c.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["Домашняя папка", "a", "b"]);
    }
}
