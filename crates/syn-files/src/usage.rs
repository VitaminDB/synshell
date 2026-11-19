//! Анализ занятого места (панель «Занятое место», как Baobab/Filelight):
//! дерево папок с размерами — обход в фоне, без перехода по ссылкам, в
//! пределах одной файловой системы, жёсткие ссылки — один раз, размер файла —
//! занятое на диске (разреженные файлы — по выделенным блокам).
//!
//! Дерево одно на окно: панель следует за открытой папкой; папка внутри уже
//! посчитанной — показывается из дерева, иначе — новый обход. Обход папки,
//! в которой лежит прежнее досчитанное дерево, берёт его готовым. Пока обход
//! идёт, итоги растут: файлы прибавляются к папке и всем её предкам сразу.
//! В каждой папке помнятся самые большие файлы (`TOP_FILES`), остальные —
//! одной строкой «другие файлы».

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Самых больших файлов в папке — помнить по имени.
pub const TOP_FILES: usize = 24;
/// Перерисовка во время обхода — не чаще.
const NOTIFY_EVERY: Duration = Duration::from_millis(250);

pub const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub parent: u32,
    /// Всего в папке со всеми вложенными, байт.
    pub bytes: u64,
    /// Файлов со всеми вложенными.
    pub files: u64,
    /// Вложенные папки (индексы).
    pub dirs: Vec<u32>,
    /// Самые большие файлы самой папки: имя, байт (по убыванию).
    pub top: Vec<(String, u64)>,
    /// Прочие файлы самой папки: сколько и байт.
    pub rest: (u64, u64),
    /// Папка прочитана (её файлы и список вложенных известны).
    pub read: bool,
    /// Удалена из дерева.
    pub gone: bool,
    /// Не открылась (нет прав).
    pub denied: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    /// Обход закончен.
    pub done: bool,
    pub dirs: u64,
    /// Папок, которые не открылись.
    pub denied: u64,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
}

impl Tree {
    /// Узел папки `p` (лежащей в корне дерева).
    pub fn find(&self, p: &Path) -> Option<u32> {
        let rel = p.strip_prefix(&self.root).ok()?;
        let mut i = 0u32;
        for c in rel.components() {
            let name = c.as_os_str().to_string_lossy();
            i = *self.nodes[i as usize].dirs.iter().find(|&&d| self.nodes[d as usize].name == name)?;
        }
        (!self.nodes.is_empty()).then_some(i)
    }

    pub fn path(&self, mut i: u32) -> PathBuf {
        let mut parts = Vec::new();
        while i != 0 && i != NONE {
            let n = &self.nodes[i as usize];
            parts.push(n.name.as_str());
            i = n.parent;
        }
        let mut p = self.root.clone();
        for s in parts.iter().rev() {
            p.push(s);
        }
        p
    }

    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    /// Прибавить к папке и её предкам.
    fn add_up(&mut self, mut i: u32, bytes: i64, files: i64) {
        while i != NONE {
            let n = &mut self.nodes[i as usize];
            n.bytes = (n.bytes as i64 + bytes).max(0) as u64;
            n.files = (n.files as i64 + files).max(0) as u64;
            i = n.parent;
        }
    }

    /// Убрать папку из дерева (вместе с вложенными).
    fn remove_dir(&mut self, i: u32) {
        if i == 0 {
            return;
        }
        let (bytes, files, parent) = {
            let n = &self.nodes[i as usize];
            (n.bytes, n.files, n.parent)
        };
        self.add_up(parent, -(bytes as i64), -(files as i64));
        self.nodes[parent as usize].dirs.retain(|&d| d != i);
        self.nodes[i as usize].gone = true;
    }

    /// Перенести готовое поддерево `src` (узел `si`) на место узла `di`.
    fn graft(&mut self, di: u32, src: &Tree, si: u32) {
        let s = &src.nodes[si as usize];
        {
            let d = &mut self.nodes[di as usize];
            d.top = s.top.clone();
            d.rest = s.rest;
            d.read = s.read;
            d.denied = s.denied;
        }
        let own = s.top.iter().map(|f| f.1).sum::<u64>() + s.rest.1;
        let own_files = s.top.len() as u64 + s.rest.0;
        self.add_up(di, own as i64, own_files as i64);
        for &c in &s.dirs {
            let cn = &src.nodes[c as usize];
            let ni = self.nodes.len() as u32;
            self.nodes.push(Node { name: cn.name.clone(), parent: di, ..Default::default() });
            self.nodes[di as usize].dirs.push(ni);
            self.dirs += 1;
            self.graft(ni, src, c);
        }
    }

    /// Вложенные папки по убыванию размера.
    pub fn sorted_dirs(&self, i: u32) -> Vec<u32> {
        let mut v = self.nodes[i as usize].dirs.clone();
        v.sort_by(|a, b| self.nodes[*b as usize].bytes.cmp(&self.nodes[*a as usize].bytes));
        v
    }
}

pub struct Scan {
    pub tree: Mutex<Tree>,
    cancel: AtomicBool,
}

static CUR: Mutex<Option<Arc<Scan>>> = Mutex::new(None);
type Notify = Box<dyn Fn() + Send + Sync>;
static NOTIFY: OnceLock<Notify> = OnceLock::new();

/// Кого будить, когда дерево изменилось.
pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    let _ = NOTIFY.set(Box::new(f));
}

fn notify() {
    if let Some(f) = NOTIFY.get() {
        f();
    }
}

/// Текущее дерево.
pub fn current() -> Option<Arc<Scan>> {
    CUR.lock().unwrap().clone()
}

/// Папки, которые не считаем: псевдо-ФС ядра и сетевые gvfs.
pub fn skipped(p: &Path) -> bool {
    ["/proc", "/sys", "/dev"].iter().any(|r| p.starts_with(r)) || (p.starts_with("/run/user") && p.to_string_lossy().contains("/gvfs/"))
}

/// Показать папку `dir`: внутри текущего дерева — ничего не делать, иначе — новый обход.
pub fn ensure(dir: &Path) {
    if let Some(s) = current() {
        let t = s.tree.lock().unwrap();
        if dir.starts_with(&t.root) && (t.find(dir).is_some() || !t.done) {
            return;
        }
    }
    start(dir);
}

/// Обойти `dir` заново (кнопка «Обновить» — без переноса прежнего).
pub fn rescan(dir: &Path) {
    let prev = CUR.lock().unwrap().take();
    if let Some(s) = prev {
        s.cancel.store(true, Ordering::Relaxed);
    }
    start(dir);
}

/// Остановить обход (панель закрыта); посчитанное остаётся.
pub fn stop() {
    if let Some(s) = current() {
        s.cancel.store(true, Ordering::Relaxed);
        let mut t = s.tree.lock().unwrap();
        if !t.done {
            // Недосчитанное дерево не годится для показа потом — забыть.
            drop(t);
            *CUR.lock().unwrap() = None;
        } else {
            t.started = t.started.take();
        }
    }
}

fn start(dir: &Path) {
    let prev = CUR.lock().unwrap().take();
    if let Some(p) = &prev {
        p.cancel.store(true, Ordering::Relaxed);
    }
    // Прежнее досчитанное дерево внутри новой папки — взять готовым.
    let reuse = prev.and_then(|p| {
        let t = p.tree.lock().unwrap();
        (t.done && t.root.starts_with(dir) && t.root != dir).then(|| t.clone())
    });
    let scan = Arc::new(Scan {
        tree: Mutex::new(Tree {
            root: dir.to_path_buf(),
            nodes: vec![Node { name: String::new(), parent: NONE, ..Default::default() }],
            started: Some(Instant::now()),
            ..Default::default()
        }),
        cancel: AtomicBool::new(false),
    });
    *CUR.lock().unwrap() = Some(scan.clone());
    let dir = dir.to_path_buf();
    std::thread::Builder::new()
        .name("files-usage".into())
        .spawn(move || {
            walk(&scan, &dir, 0, reuse.as_ref());
            let mut t = scan.tree.lock().unwrap();
            if !scan.cancel.load(Ordering::Relaxed) {
                t.done = true;
                t.took = t.started.map(|s| s.elapsed());
            }
            drop(t);
            notify();
        })
        .expect("поток анализа места");
}

/// Занятое на диске: разреженный файл — по выделенным блокам, иначе — длина.
fn disk_size(m: &std::fs::Metadata) -> u64 {
    m.len().min(m.blocks().saturating_mul(512))
}

struct Listing {
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
    denied: bool,
}

fn list(dir: &Path, dev: u64, links: &mut HashSet<(u64, u64)>) -> Listing {
    let mut out = Listing { dirs: Vec::new(), files: Vec::new(), denied: false };
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            out.denied = true;
            return out;
        }
    };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if m.is_dir() {
            // Подключённые внутрь разделы не считаем (как `du -x`).
            if m.dev() == dev && !skipped(&e.path()) {
                out.dirs.push(name);
            }
        } else if m.is_file() {
            if m.nlink() > 1 && !links.insert((m.dev(), m.ino())) {
                continue;
            }
            out.files.push((name, disk_size(&m)));
        }
    }
    out
}

/// Записать прочитанное в узел `i`: файлы, новые вложенные папки (их индексы).
fn apply(t: &mut Tree, i: u32, l: Listing) -> Vec<(u32, String)> {
    let mut files = l.files;
    files.sort_by(|a, b| b.1.cmp(&a.1));
    let rest: Vec<(String, u64)> = if files.len() > TOP_FILES { files.split_off(TOP_FILES) } else { Vec::new() };
    let own: u64 = files.iter().map(|f| f.1).sum::<u64>() + rest.iter().map(|f| f.1).sum::<u64>();
    let count = (files.len() + rest.len()) as i64;
    {
        let n = &mut t.nodes[i as usize];
        n.top = files;
        n.rest = (rest.len() as u64, rest.iter().map(|f| f.1).sum());
        n.read = true;
        n.denied = l.denied;
    }
    if l.denied {
        t.denied += 1;
    }
    t.add_up(i, own as i64, count);
    let mut out = Vec::new();
    for d in l.dirs {
        let ni = t.nodes.len() as u32;
        t.nodes.push(Node { name: d.clone(), parent: i, ..Default::default() });
        t.nodes[i as usize].dirs.push(ni);
        t.dirs += 1;
        out.push((ni, d));
    }
    out
}

/// Обойти папку `dir` в узел `at` (вглубь, со стеком).
fn walk(scan: &Scan, dir: &Path, at: u32, reuse: Option<&Tree>) {
    let Ok(root) = std::fs::symlink_metadata(dir) else {
        let mut t = scan.tree.lock().unwrap();
        t.nodes[at as usize].denied = true;
        t.nodes[at as usize].read = true;
        return;
    };
    let dev = root.dev();
    let mut links: HashSet<(u64, u64)> = HashSet::new();
    let mut stack: Vec<(u32, PathBuf)> = vec![(at, dir.to_path_buf())];
    let mut last = Instant::now();
    while let Some((i, p)) = stack.pop() {
        if scan.cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Some(r) = reuse.filter(|r| r.root == p) {
            scan.tree.lock().unwrap().graft(i, r, 0);
            continue;
        }
        let l = list(&p, dev, &mut links);
        let subs = {
            let mut t = scan.tree.lock().unwrap();
            apply(&mut t, i, l)
        };
        for (ni, name) in subs.into_iter().rev() {
            stack.push((ni, p.join(name)));
        }
        if last.elapsed() > NOTIFY_EVERY {
            notify();
            last = Instant::now();
        }
    }
}

/// В `paths` что-то поменялось (файловая операция): удалённые убрать из
/// дерева, появившиеся и изменённые — пересчитать (в фоне).
pub fn invalidate(paths: &[PathBuf]) {
    let Some(scan) = current() else { return };
    let paths = paths.to_vec();
    std::thread::spawn(move || {
        let mut changed = false;
        for p in &paths {
            // Папка, в которой лежит путь, — её список и файлы перечитать.
            let Some(parent) = p.parent() else { continue };
            let (root, inside) = {
                let t = scan.tree.lock().unwrap();
                (t.root.clone(), t.find(parent))
            };
            let Some(pi) = inside else { continue };
            changed |= refresh(&scan, pi, parent, &root);
        }
        if changed {
            notify();
        }
    });
}

/// Перечитать папку `i` (путь `dir`): её файлы заново, исчезнувшие вложенные —
/// убрать, новые — обойти целиком. Изменённые вложенные (по времени изменения не
/// узнать) не трогаем — кроме случая, когда путь операции — сама папка.
fn refresh(scan: &Scan, i: u32, dir: &Path, root: &Path) -> bool {
    let dev = std::fs::symlink_metadata(root).map(|m| m.dev()).unwrap_or(0);
    let mut links = HashSet::new();
    let l = list(dir, dev, &mut links);
    let mut t = scan.tree.lock().unwrap();
    if t.nodes.get(i as usize).is_none_or(|n| n.gone) {
        return false;
    }
    // Свои файлы — вычесть прежние, записать новые.
    let (old_bytes, old_files) = {
        let n = &t.nodes[i as usize];
        (n.top.iter().map(|f| f.1).sum::<u64>() + n.rest.1, n.top.len() as u64 + n.rest.0)
    };
    t.add_up(i, -(old_bytes as i64), -(old_files as i64));
    let names: HashSet<String> = l.dirs.iter().cloned().collect();
    for d in t.nodes[i as usize].dirs.clone() {
        if !names.contains(&t.nodes[d as usize].name) {
            t.remove_dir(d);
        }
    }
    let known: HashSet<String> = t.nodes[i as usize].dirs.iter().map(|&d| t.nodes[d as usize].name.clone()).collect();
    let fresh = Listing { dirs: l.dirs.into_iter().filter(|d| !known.contains(d)).collect(), files: l.files, denied: l.denied };
    let subs = apply(&mut t, i, fresh);
    drop(t);
    for (ni, name) in subs {
        walk(scan, &dir.join(name), ni, None);
    }
    true
}

/// Удалить узел папки по пути сразу (не дожидаясь задания) — для отклика.
pub fn forget_path(p: &Path) {
    let Some(scan) = current() else { return };
    let mut t = scan.tree.lock().unwrap();
    if let Some(i) = t.find(p) {
        t.remove_dir(i);
    } else if let (Some(parent), Some(name)) = (p.parent(), p.file_name()) {
        let name = name.to_string_lossy();
        if let Some(pi) = t.find(parent) {
            let n = &mut t.nodes[pi as usize];
            if let Some(k) = n.top.iter().position(|f| f.0 == name) {
                let (_, b) = n.top.remove(k);
                t.add_up(pi, -(b as i64), -1);
            }
        }
    }
    drop(t);
    notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_done(s: &Scan) {
        for _ in 0..200 {
            if s.tree.lock().unwrap().done {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("обход не закончился");
    }

    #[test]
    fn scans_and_reuses() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("x"), vec![1u8; 10000]).unwrap();
        std::fs::write(root.join("a/b/y"), vec![1u8; 50000]).unwrap();
        std::fs::hard_link(root.join("x"), root.join("a/x2")).unwrap();
        start(&root.join("a"));
        let s = current().unwrap();
        wait_done(&s);
        let ab = s.tree.lock().unwrap().node(0).bytes;
        assert!(ab >= 50000, "{ab}");
        // Родитель берёт готовое поддерево «a».
        ensure(&root);
        let s = current().unwrap();
        wait_done(&s);
        let tr = s.tree.lock().unwrap();
        let a = tr.find(&root.join("a")).unwrap();
        let b = tr.find(&root.join("a/b")).unwrap();
        assert_eq!(tr.node(b).top[0].0, "y");
        assert_eq!(tr.path(b), root.join("a/b"));
        assert!(tr.node(0).bytes >= tr.node(a).bytes + 10000);
        assert_eq!(tr.node(0).files, 3);
        drop(tr);
        // Удалили папку — дерево пересчитано.
        std::fs::remove_dir_all(root.join("a/b")).unwrap();
        let parent_i = s.tree.lock().unwrap().find(&root.join("a")).unwrap();
        refresh(&s, parent_i, &root.join("a"), &root);
        let tr = s.tree.lock().unwrap();
        assert!(tr.find(&root.join("a/b")).is_none());
        assert_eq!(tr.node(0).files, 2);
    }
}
