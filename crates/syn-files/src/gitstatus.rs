//! Состояние git у файлов: открытая папка внутри репозитория — в фоне
//! `git status`, у файлов метка (изменён, добавлен, новый, конфликт), у
//! папок — самая важная метка из вложенных. Сетевые папки (gvfs) не трогаем.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Untracked,
    Added,
    Modified,
    Conflict,
}

impl Mark {
    /// Буква у имени файла.
    pub fn letter(self) -> &'static str {
        match self {
            Mark::Untracked => "?",
            Mark::Added => "A",
            Mark::Modified => "M",
            Mark::Conflict => "!",
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            Mark::Untracked => "git-mark git-new",
            Mark::Added => "git-mark git-added",
            Mark::Modified => "git-mark git-modified",
            Mark::Conflict => "git-mark git-conflict",
        }
    }

    pub fn title(self) -> String {
        use syngui::t;
        match self {
            Mark::Untracked => t!("Не отслеживается git").into(),
            Mark::Added => t!("Добавлен в git").into(),
            Mark::Modified => t!("Изменён").into(),
            Mark::Conflict => t!("Конфликт слияния").into(),
        }
    }
}

type Marks = HashMap<PathBuf, Mark>;

#[derive(Default)]
struct State {
    repos: HashMap<PathBuf, Arc<Marks>>,
    running: Vec<PathBuf>,
    /// Пока шёл подсчёт, попросили ещё раз — пересчитать по окончании.
    again: Vec<PathBuf>,
}

static G: Mutex<Option<State>> = Mutex::new(None);
static NOTIFY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = G.lock().unwrap();
    f(g.get_or_insert_with(State::default))
}

pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    let _ = NOTIFY.set(Box::new(f));
}

/// Корень репозитория, внутри которого `dir`.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let gvfs = PathBuf::from(format!("/run/user/{}/gvfs", unsafe { libc::getuid() }));
    if dir.starts_with(&gvfs) {
        return None;
    }
    dir.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

/// Разбор `git status --porcelain=v1 -z`: метки файлов и (поднятые вверх) папок.
pub fn parse(root: &Path, out: &[u8]) -> Marks {
    let mut marks = Marks::new();
    let mut it = out.split(|b| *b == 0).filter(|s| s.len() > 3);
    while let Some(rec) = it.next() {
        let (xy, path) = (&rec[..2], String::from_utf8_lossy(&rec[3..]).to_string());
        let (x, y) = (xy[0], xy[1]);
        // Переименование: следом — старое имя.
        if x == b'R' || x == b'C' {
            it.next();
        }
        let mark = match (x, y) {
            (b'?', b'?') => Mark::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Mark::Conflict,
            (b'A', _) | (b'R', _) | (b'C', _) => Mark::Added,
            _ => Mark::Modified,
        };
        let full = root.join(path.trim_end_matches('/'));
        // Папки вверх до корня — по самой важной метке внутри.
        for p in full.ancestors() {
            if p == root {
                break;
            }
            let e = marks.entry(p.to_path_buf()).or_insert(mark);
            if *e < mark {
                *e = mark;
            }
        }
    }
    marks
}

fn compute(root: &Path) -> Option<Marks> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=normal"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse(root, &out.stdout))
}

/// Обновить метки репозитория, в котором `dir` (в фоне).
pub fn request(dir: &Path) {
    let Some(root) = repo_root(dir) else { return };
    let start = with(|s| {
        if s.running.contains(&root) {
            if !s.again.contains(&root) {
                s.again.push(root.clone());
            }
            false
        } else {
            s.running.push(root.clone());
            true
        }
    });
    if !start {
        return;
    }
    std::thread::spawn(move || loop {
        let marks = compute(&root).unwrap_or_default();
        let changed = with(|s| {
            let changed = s.repos.get(&root).map(|m| **m != marks).unwrap_or(!marks.is_empty());
            s.repos.insert(root.clone(), Arc::new(marks));
            changed
        });
        if changed {
            if let Some(n) = NOTIFY.get() {
                n();
            }
        }
        let again = with(|s| {
            if let Some(i) = s.again.iter().position(|r| *r == root) {
                s.again.remove(i);
                true
            } else {
                s.running.retain(|r| *r != root);
                false
            }
        });
        if !again {
            return;
        }
    });
}

/// Метка файла или папки (по последнему подсчёту).
pub fn get(path: &Path) -> Option<Mark> {
    with(|s| s.repos.iter().find(|(root, _)| path.starts_with(root)).and_then(|(_, m)| m.get(path).copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let root = Path::new("/r");
        let out = b" M src/a.rs\0?? new/\0A  b.txt\0R  c2.txt\0c.txt\0UU src/deep/x.rs\0";
        let m = parse(root, out);
        assert_eq!(m.get(Path::new("/r/src/a.rs")), Some(&Mark::Modified));
        assert_eq!(m.get(Path::new("/r/new")), Some(&Mark::Untracked));
        assert_eq!(m.get(Path::new("/r/b.txt")), Some(&Mark::Added));
        assert_eq!(m.get(Path::new("/r/c2.txt")), Some(&Mark::Added));
        assert_eq!(m.get(Path::new("/r/c.txt")), None);
        // Папка — по самому важному внутри.
        assert_eq!(m.get(Path::new("/r/src")), Some(&Mark::Conflict));
        assert_eq!(m.get(Path::new("/r")), None);
    }

    #[test]
    fn real_repo() {
        let d = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| Command::new("git").arg("-C").arg(d.path()).args(args).output().unwrap();
        if !git(&["init", "-q"]).status.success() {
            return;
        }
        std::fs::write(d.path().join("a.txt"), "1").unwrap();
        git(&["add", "a.txt"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"]);
        std::fs::write(d.path().join("a.txt"), "2").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/b.txt"), "").unwrap();
        let m = compute(d.path()).unwrap();
        assert_eq!(m.get(&d.path().join("a.txt")), Some(&Mark::Modified));
        assert_eq!(m.get(&d.path().join("sub")), Some(&Mark::Untracked));
        assert_eq!(repo_root(&d.path().join("sub")).as_deref(), Some(d.path()));
    }
}
