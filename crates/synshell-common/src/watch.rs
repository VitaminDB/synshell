//! Слежение за файлами конфигурации опросом mtime — без inotify-зависимостей,
//! дёшево (раз в секунду `stat` двух файлов) и переживает атомарную замену
//! файла редакторами (rename поверх).

use std::path::PathBuf;
use std::time::SystemTime;

pub struct FileWatcher {
    files: Vec<(PathBuf, Option<SystemTime>)>,
}

impl FileWatcher {
    pub fn new(files: impl IntoIterator<Item = PathBuf>) -> Self {
        let files = files
            .into_iter()
            .map(|p| {
                let m = mtime(&p);
                (p, m)
            })
            .collect();
        Self { files }
    }

    /// `true`, если хотя бы один файл изменился (появился, пропал, сменил mtime)
    /// со времени прошлого вызова.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for (path, last) in &mut self.files {
            let now = mtime(path);
            if now != *last {
                *last = now;
                changed = true;
            }
        }
        changed
    }
}

fn mtime(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}
