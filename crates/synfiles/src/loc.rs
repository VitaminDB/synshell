//! Расположение, которое показывает панель: папка, корзина, результаты поиска.

use std::path::{Path, PathBuf};

use synshell_common::paths;
use syngui::t;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    Dir(PathBuf),
    Trash,
    /// Поиск внутри `root`: по имени или (`content`) по тексту файлов.
    Search { root: PathBuf, query: String, content: bool },
}

impl Location {
    /// Разбор строки адреса: путь, `~`, `trash:`, `file://`.
    pub fn parse(s: &str) -> Option<Location> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s == "trash:" || s == "trash:/" || s.eq_ignore_ascii_case("корзина") || s.to_lowercase() == t!("Корзина").to_lowercase() {
            return Some(Location::Trash);
        }
        if s.starts_with("file://") {
            return synshell_common::xdg::path_from_uri(s).map(Location::Dir);
        }
        let p = paths::expand_tilde(s);
        let p = if p.is_relative() { paths::home().join(p) } else { p };
        // Путь через архив (`~/a.zip/папка`) и сам архив — папкой.
        Some(Location::Dir(crate::archive::resolve(&normalize(&p), true)))
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
            Location::Trash => t!("Корзина").into(),
            Location::Search { query, content: false, .. } => t!("Поиск «{query}»", query = query),
            Location::Search { query, .. } => t!("Текст «{query}» в файлах", query = query),
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
            Location::Dir(p) if crate::archive::root_of(p).as_deref() == Some(p.as_path()) => crate::ui::icons::ARCHIVE,
            Location::Dir(p) => crate::places::known(p).map(|(i, _)| i).unwrap_or(crate::ui::icons::FOLDER),
        }
    }

    /// Строка для поля адреса.
    pub fn address(&self) -> String {
        match self {
            Location::Trash => "trash:".into(),
            Location::Dir(p) | Location::Search { root: p, .. } => crate::archive::display(p).display().to_string(),
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Dir(p) => crate::archive::parent(p).map(Location::Dir),
            Location::Search { root, .. } => Some(Location::Dir(root.clone())),
            Location::Trash => None,
        }
    }

    /// Сегменты хлебных крошек: (подпись, расположение). Домашняя папка —
    /// одним сегментом, а не `/ › home › user`.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        match self {
            Location::Trash => vec![(t!("Корзина").into(), Location::Trash)],
            Location::Search { root, .. } => {
                let mut v = Location::Dir(root.clone()).crumbs();
                v.push((self.title(), self.clone()));
                v
            }
            Location::Dir(p) if crate::archive::root_of(p).is_some() => {
                // Внутри архива — крошки пути через архив.
                let v = crate::archive::display(p);
                if v == *p {
                    return vec![(self.title(), self.clone())];
                }
                Location::Dir(v)
                    .crumbs()
                    .into_iter()
                    .map(|(t, l)| match l {
                        Location::Dir(d) => (t, Location::Dir(crate::archive::resolve(&d, true))),
                        l => (t, l),
                    })
                    .collect()
            }
            Location::Dir(p) => {
                let home = paths::home();
                // Телефон в gvfs — от его имени, а не `/ › run › user › 1000 › gvfs › mtp:host=…`.
                let gadget = crate::places::gadget_root(p);
                let (mut out, rest, mut acc) = if let Some((root, title, _)) = gadget {
                    let rest = p.strip_prefix(&root).unwrap();
                    (vec![(title, Location::Dir(root.clone()))], rest, root)
                } else if p.starts_with(&home) {
                    (vec![(t!("Домашняя папка").to_string(), Location::Dir(home.clone()))], p.strip_prefix(&home).unwrap(), home)
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
