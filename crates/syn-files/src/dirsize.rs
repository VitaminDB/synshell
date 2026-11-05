//! Размеры папок (`[files] dir_sizes`): обход в фоне — без перехода по
//! ссылкам, в пределах одной файловой системы, жёсткие ссылки — один раз.
//! Панели говорят, какие папки им нужны (`want`); очередь — сначала свежие
//! просьбы, обходы ненужных больше папок отменяются. Долгий обход отдаёт
//! промежуточные итоги. Посчитанное живёт в кэше: через минуту или после
//! файловой операции внутри папки пересчитывается.

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Размер папки: `done == false` — обход ещё идёт (столько уже насчитано).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub bytes: u64,
    pub done: bool,
}

/// Одновременных обходов.
const WORKERS: usize = 2;
/// Посчитанное старше — пересчитать при следующей просьбе.
const STALE: Duration = Duration::from_secs(60);
/// Промежуточный итог долгого обхода — не чаще.
const PARTIAL_EVERY: Duration = Duration::from_millis(300);

struct Cached {
    size: Size,
    at: Instant,
}

#[derive(Default)]
struct State {
    enabled: bool,
    cache: HashMap<PathBuf, Cached>,
    /// Панель → нужные ей папки.
    wants: HashMap<u64, Vec<PathBuf>>,
    queue: VecDeque<PathBuf>,
    /// Идущие обходы и их флаги отмены.
    running: Vec<(PathBuf, Arc<AtomicBool>)>,
    workers: usize,
}

static G: Mutex<Option<State>> = Mutex::new(None);
type Notify = Box<dyn Fn(bool) + Send + Sync>;
static NOTIFY: OnceLock<Notify> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = G.lock().unwrap();
    f(g.get_or_insert_with(State::default))
}

/// Кого будить, когда размеры изменились (`true` — какой-то обход закончен).
pub fn set_notify(f: impl Fn(bool) + Send + Sync + 'static) {
    let _ = NOTIFY.set(Box::new(f));
}

fn notify(finished: bool) {
    if let Some(f) = NOTIFY.get() {
        f(finished);
    }
}

pub fn enabled() -> bool {
    with(|s| s.enabled)
}

/// Включить или выключить подсчёт; выключение отменяет всё начатое.
pub fn set_enabled(on: bool) {
    with(|s| {
        s.enabled = on;
        if !on {
            s.wants.clear();
            s.queue.clear();
            for (_, c) in &s.running {
                c.store(true, Ordering::Relaxed);
            }
        }
    });
}

/// Папки, размер которых не считаем: псевдо-ФС ядра и сетевые gvfs.
fn skipped(p: &Path) -> bool {
    ["/proc", "/sys", "/dev"].iter().any(|r| p.starts_with(r)) || (p.starts_with("/run/user") && p.to_string_lossy().contains("/gvfs/"))
}

/// Что известно о папке (`None` — ещё ничего или не считаем).
pub fn get(p: &Path) -> Option<Size> {
    with(|s| if s.enabled { s.cache.get(p).map(|c| c.size) } else { None })
}

/// Посчитанный до конца размер (для сортировки).
pub fn done_bytes(p: &Path) -> Option<u64> {
    get(p).filter(|s| s.done).map(|s| s.bytes)
}

/// Ждёт ли папка подсчёта (в очереди или в обходе).
pub fn pending(p: &Path) -> bool {
    with(|s| s.enabled && (s.queue.iter().any(|q| q == p) || s.running.iter().any(|(r, _)| r == p)))
}

/// Панели `pane` нужны размеры этих папок (прежняя просьба заменяется).
pub fn want(pane: u64, dirs: Vec<PathBuf>) {
    let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| !skipped(d)).collect();
    with(|s| {
        if !s.enabled {
            return;
        }
        s.wants.insert(pane, dirs);
        requeue(s, Some(pane));
    });
    spawn_workers();
}

/// Панель закрыта — её просьбы больше не нужны.
pub fn forget(pane: u64) {
    with(|s| {
        s.wants.remove(&pane);
        requeue(s, None);
    });
}

/// В `paths` что-то поменялось: папки, в которых они лежат, пересчитать.
pub fn invalidate(paths: &[PathBuf]) {
    with(|s| {
        if !s.enabled {
            return;
        }
        let before = s.cache.len();
        s.cache.retain(|dir, _| !paths.iter().any(|p| p.starts_with(dir)));
        if s.cache.len() != before {
            requeue(s, None);
        }
    });
    spawn_workers();
}

/// Пересобрать очередь: сначала папки панели `first`, потом остальные;
/// уже посчитанные (и не устаревшие) и идущие — пропустить. Обходы папок,
/// которые больше никому не нужны, отменить.
fn requeue(s: &mut State, first: Option<u64>) {
    let mut order: Vec<&PathBuf> = Vec::new();
    if let Some(d) = first.and_then(|f| s.wants.get(&f)) {
        order.extend(d);
    }
    for (pane, d) in &s.wants {
        if Some(*pane) != first {
            order.extend(d);
        }
    }
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    for d in order {
        if !seen.insert(d) {
            continue;
        }
        let fresh = s.cache.get(d).map(|c| c.size.done && c.at.elapsed() < STALE).unwrap_or(false);
        if !fresh && !s.running.iter().any(|(r, _)| r == d) {
            queue.push_back(d.clone());
        }
    }
    for (r, cancel) in &s.running {
        if !seen.contains(r) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    s.queue = queue;
}

fn spawn_workers() {
    loop {
        let start = with(|s| {
            if s.workers < WORKERS && !s.queue.is_empty() {
                s.workers += 1;
                true
            } else {
                false
            }
        });
        if !start {
            return;
        }
        std::thread::Builder::new().name("files-dirsize".into()).spawn(worker).expect("поток размеров папок");
    }
}

fn worker() {
    loop {
        let job = with(|s| {
            let next = s.queue.pop_front();
            match next {
                Some(p) => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    s.running.push((p.clone(), cancel.clone()));
                    Some((p, cancel))
                }
                None => {
                    s.workers -= 1;
                    None
                }
            }
        });
        let Some((dir, cancel)) = job else { return };
        let total = measure(&dir, &cancel, &mut |bytes| {
            with(|s| {
                // Старый итог (из кэша) не затираем меньшим промежуточным.
                let keep = s.cache.get(&dir).is_some_and(|c| c.size.done && c.size.bytes >= bytes);
                if !keep {
                    s.cache.insert(dir.clone(), Cached { size: Size { bytes, done: false }, at: Instant::now() });
                }
            });
            notify(false);
        });
        with(|s| {
            s.running.retain(|(r, c)| !(r == &dir && Arc::ptr_eq(c, &cancel)));
            match total {
                Some(bytes) => {
                    s.cache.insert(dir.clone(), Cached { size: Size { bytes, done: true }, at: Instant::now() });
                }
                // Отменён: недосчитанное не показываем.
                None => {
                    if s.cache.get(&dir).is_some_and(|c| !c.size.done) {
                        s.cache.remove(&dir);
                    }
                }
            }
        });
        notify(total.is_some());
    }
}

/// Объём файлов в папке, байт (`None` — отменено). `partial` получает
/// промежуточные итоги долгого обхода.
pub fn measure(dir: &Path, cancel: &AtomicBool, partial: &mut dyn FnMut(u64)) -> Option<u64> {
    let root = std::fs::symlink_metadata(dir).ok()?;
    if !root.is_dir() {
        return Some(0);
    }
    let dev = root.dev();
    let mut stack = vec![dir.to_path_buf()];
    let mut links: HashSet<(u64, u64)> = HashSet::new();
    let mut bytes = 0u64;
    let mut last = Instant::now();
    while let Some(d) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                // Подключённые внутрь разделы не считаем (как `du -x`).
                if m.dev() == dev {
                    stack.push(e.path());
                }
            } else if m.is_file() {
                if m.nlink() > 1 && !links.insert((m.dev(), m.ino())) {
                    continue;
                }
                bytes += m.len();
            }
        }
        if last.elapsed() > PARTIAL_EVERY {
            partial(bytes);
            last = Instant::now();
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_files_once() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("a/b")).unwrap();
        std::fs::write(t.path().join("x"), vec![0u8; 100]).unwrap();
        std::fs::write(t.path().join("a/b/y"), vec![0u8; 50]).unwrap();
        std::fs::hard_link(t.path().join("x"), t.path().join("a/x2")).unwrap();
        std::os::unix::fs::symlink(t.path().join("a"), t.path().join("link")).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(measure(t.path(), &cancel, &mut |_| {}), Some(150));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(measure(t.path(), &cancel, &mut |_| {}), None);
    }
}
