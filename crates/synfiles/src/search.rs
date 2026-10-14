//! Поиск по имени или по тексту файлов: обход в ширину (сначала ближние
//! результаты), по ссылкам на каталоги не ходит, псевдо-ФС пропускает.
//! Текст файлов читают несколько потоков; двоичные и большие файлы, папки
//! сборки и зависимостей пропускаются.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use crate::model::Entry;

/// Запрос: слова (все должны встретиться), шаблон `*.rs`, `отчёт?.pdf`
/// или метка — `метка:красная`, `tag:Red`.
pub struct Query {
    words: Vec<String>,
    glob: Option<String>,
    tag: Option<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let q = q.trim().to_lowercase();
        if let Some(t) = q.strip_prefix("метка:").or_else(|| q.strip_prefix("tag:")) {
            return Query { words: Vec::new(), glob: None, tag: Some(t.trim().to_string()) };
        }
        if q.contains(['*', '?']) {
            Query { words: Vec::new(), glob: Some(q), tag: None }
        } else {
            Query { words: q.split_whitespace().map(String::from).collect(), glob: None, tag: None }
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
/// продолжать ли. `content` — искать слова запроса в тексте файлов.
pub fn walk(root: &Path, query: &str, content: bool, hidden: bool, found: &mut dyn FnMut(Entry) -> bool) {
    if content {
        return walk_content(root, query, hidden, found);
    }
    let q = Query::new(query);
    each(root, hidden, false, &mut |path, name, _| {
        let hit = match &q.tag {
            Some(t) => crate::tags::has(path, t),
            None => q.matches(name),
        };
        if hit {
            if let Some(entry) = Entry::from_path(path) {
                return found(entry);
            }
        }
        true
    });
}

/// Обход в ширину: `f(путь, имя, файл ли)`; `false` — прекратить.
fn each(root: &Path, hidden: bool, skip_build: bool, f: &mut dyn FnMut(&Path, &str, bool) -> bool) {
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
            if !f(&path, &name, ft.is_file()) {
                return;
            }
            if ft.is_dir() && !SKIP.iter().any(|s| path == Path::new(s)) && !(skip_build && SKIP_BUILD.contains(&name.as_str())) {
                queue.push_back(path);
            }
        }
    }
}

/// Папки, где текст искать бессмысленно (сборка, зависимости, кэши).
const SKIP_BUILD: [&str; 6] = ["node_modules", "target", "__pycache__", ".git", ".cache", "build"];
/// Больше — не читаем.
const MAX_TEXT: u64 = 32 << 20;

/// Есть ли в тексте файла все слова (без учёта регистра). Двоичные
/// (нулевой байт в начале) и большие — нет.
pub fn file_contains(path: &Path, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() || meta.len() > MAX_TEXT || meta.len() == 0 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else { return false };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return false;
    }
    let text = String::from_utf8_lossy(&bytes).to_lowercase();
    words.iter().all(|w| text.contains(w.as_str()))
}

fn walk_content(root: &Path, query: &str, hidden: bool, found: &mut dyn FnMut(Entry) -> bool) {
    let words: Arc<Vec<String>> = Arc::new(query.trim().to_lowercase().split_whitespace().map(String::from).collect());
    if words.is_empty() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (work_tx, work_rx) = mpsc::sync_channel::<PathBuf>(256);
    let work_rx = Arc::new(Mutex::new(work_rx));
    let (hit_tx, hit_rx) = mpsc::channel::<PathBuf>();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (rx, tx, words, stop) = (work_rx.clone(), hit_tx.clone(), words.clone(), stop.clone());
            std::thread::spawn(move || loop {
                let next = rx.lock().unwrap().recv();
                let Ok(path) = next else { return };
                if stop.load(Ordering::Relaxed) {
                    continue;
                }
                if file_contains(&path, &words) && tx.send(path).is_err() {
                    return;
                }
            })
        })
        .collect();
    drop(hit_tx);
    let deliver = |found: &mut dyn FnMut(Entry) -> bool, p: PathBuf| -> bool {
        match Entry::from_path(&p) {
            Some(e) => found(e),
            None => true,
        }
    };
    each(root, hidden, true, &mut |path, _, is_file| {
        while let Ok(p) = hit_rx.try_recv() {
            if !deliver(found, p) {
                stop.store(true, Ordering::Relaxed);
                return false;
            }
        }
        !is_file || work_tx.send(path.to_path_buf()).is_ok()
    });
    drop(work_tx);
    // Дочитать очередь: находки приходят, пока работники не закончат.
    for p in hit_rx.iter() {
        if !stop.load(Ordering::Relaxed) && !deliver(found, p) {
            stop.store(true, Ordering::Relaxed);
        }
    }
    for w in workers {
        let _ = w.join();
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
        walk(d.path(), "note", false, false, &mut |e| {
            got.push(e.name);
            true
        });
        assert_eq!(got, vec!["note.md", "note.txt"]);
    }

    #[test]
    fn finds_text_in_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/node_modules")).unwrap();
        std::fs::write(d.path().join("a/план.md"), "Встреча в Пятницу\nобсудить бюджет").unwrap();
        std::fs::write(d.path().join("b.txt"), "только бюджет").unwrap();
        std::fs::write(d.path().join("a/node_modules/x.js"), "пятницу бюджет").unwrap();
        std::fs::write(d.path().join("bin.dat"), "\0\0 пятницу бюджет").unwrap();
        let mut got = Vec::new();
        walk(d.path(), "пятницу БЮДЖЕТ", true, false, &mut |e| {
            got.push(e.name);
            true
        });
        assert_eq!(got, vec!["план.md"]);
    }
}
