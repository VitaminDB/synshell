//! Поиск по имени: обход в ширину (сначала ближние результаты), по ссылкам
//! на каталоги не ходит, псевдо-ФС пропускает.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::model::Entry;

/// Запрос: слова (все должны встретиться) или шаблон `*.rs`, `отчёт?.pdf`.
pub struct Query {
    words: Vec<String>,
    glob: Option<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let q = q.trim().to_lowercase();
        if q.contains(['*', '?']) {
            Query { words: Vec::new(), glob: Some(q) }
        } else {
            Query { words: q.split_whitespace().map(String::from).collect(), glob: None }
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        let n = name.to_lowercase();
        match &self.glob {
            Some(g) => synshell_common::mime::glob_match(g, &n),
            None => !self.words.is_empty() && self.words.iter().all(|w| n.contains(w.as_str())),
        }
    }
}

const SKIP: [&str; 4] = ["/proc", "/sys", "/dev", "/run/user"];

/// Обойти `root`; `found` вызывается на каждое совпадение и возвращает,
/// продолжать ли.
pub fn walk(root: &Path, query: &str, hidden: bool, found: &mut dyn FnMut(Entry) -> bool) {
    let q = Query::new(query);
    let mut queue: VecDeque<PathBuf> = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if q.matches(&name) {
                if let Some(entry) = Entry::from_path(&path) {
                    if !found(entry) {
                        return;
                    }
                }
            }
            if ft.is_dir() && !SKIP.iter().any(|s| path == Path::new(s)) {
                queue.push_back(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query() {
        assert!(Query::new("отчёт 2024").matches("Годовой ОТЧЁТ за 2024.pdf"));
        assert!(!Query::new("отчёт 2023").matches("Годовой отчёт за 2024.pdf"));
        assert!(Query::new("*.RS").matches("main.rs"));
        assert!(!Query::new("").matches("x"));
    }

    #[test]
    fn walks_breadth_first() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/b")).unwrap();
        std::fs::write(d.path().join("a/b/note.txt"), "").unwrap();
        std::fs::write(d.path().join("note.md"), "").unwrap();
        let mut got = Vec::new();
        walk(d.path(), "note", false, &mut |e| {
            got.push(e.name);
            true
        });
        assert_eq!(got, vec!["note.md", "note.txt"]);
    }
}
