//! Файловые операции: задания в фоне с прогрессом, паузой и отменой,
//! вопросы при совпадении имён, журнал для отмены (Ctrl+Z).

use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::trash;
use syngui::t;

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Copy { srcs: Vec<PathBuf>, dest: PathBuf },
    Move { srcs: Vec<PathBuf>, dest: PathBuf },
    Link { srcs: Vec<PathBuf>, dest: PathBuf },
    Trash { srcs: Vec<PathBuf> },
    Delete { srcs: Vec<PathBuf> },
    Restore { files: Vec<PathBuf> },
    EmptyTrash,
}

impl Op {
    /// Что операция меняет (размеры папок вокруг пересчитать).
    pub fn touched(&self) -> Vec<PathBuf> {
        match self {
            Op::Copy { srcs, dest } | Op::Move { srcs, dest } | Op::Link { srcs, dest } => {
                let mut v = srcs.clone();
                v.push(dest.clone());
                v
            }
            Op::Trash { srcs } | Op::Delete { srcs } => srcs.clone(),
            Op::Restore { files } => files.clone(),
            Op::EmptyTrash => vec![synshell_common::paths::data_home().join("Trash/files")],
        }
    }

    pub fn title(&self) -> String {
        let n = |v: &Vec<PathBuf>| {
            if v.len() == 1 {
                format!("«{}»", name_of(&v[0]))
            } else {
                crate::model::format_count(v.len() as u32)
            }
        };
        match self {
            Op::Copy { srcs, dest } => t!("Копирование {v} в «{v2}»", v = n(srcs), v2 = name_of(dest)),
            Op::Move { srcs, dest } => t!("Перемещение {v} в «{v2}»", v = n(srcs), v2 = name_of(dest)),
            Op::Link { srcs, dest } => t!("Ссылки на {v} в «{v2}»", v = n(srcs), v2 = name_of(dest)),
            Op::Trash { srcs } => t!("Удаление {v} в корзину", v = n(srcs)),
            Op::Delete { srcs } => t!("Удаление {v}", v = n(srcs)),
            Op::Restore { files } => t!("Восстановление {v}", v = n(files)),
            Op::EmptyTrash => t!("Очистка корзины").into(),
        }
    }
}

pub fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

/// Что делать с совпавшим именем.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    Skip,
    KeepBoth,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub dir: bool,
}

#[derive(Default)]
pub struct Progress {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub total_files: AtomicU64,
    pub done_files: AtomicU64,
    pub cancel: AtomicBool,
    pub paused: AtomicBool,
    pub finished: AtomicBool,
    pub current: Mutex<String>,
    pub errors: Mutex<Vec<String>>,
    pub conflict: Mutex<Option<Conflict>>,
    answer: Mutex<Option<(Resolution, bool)>>,
    cv: Condvar,
    /// Скорость: байт/с по последним секундам.
    pub rate: AtomicU64,
}

pub struct Job {
    pub id: u64,
    pub op: Op,
    pub title: String,
    pub progress: Arc<Progress>,
}

impl Job {
    /// Ответ на вопрос о совпадении (`all` — для всех следующих).
    pub fn answer(&self, r: Resolution, all: bool) {
        *self.progress.answer.lock().unwrap() = Some((r, all));
        *self.progress.conflict.lock().unwrap() = None;
        self.progress.cv.notify_all();
    }

    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::SeqCst);
        self.progress.paused.store(false, Ordering::SeqCst);
        self.answer(Resolution::Cancel, true);
    }

    pub fn toggle_pause(&self) {
        let p = !self.progress.paused.load(Ordering::SeqCst);
        self.progress.paused.store(p, Ordering::SeqCst);
    }

    pub fn fraction(&self) -> f32 {
        let p = &self.progress;
        let tb = p.total_bytes.load(Ordering::Relaxed);
        if tb > 0 {
            return (p.done_bytes.load(Ordering::Relaxed) as f32 / tb as f32).min(1.0);
        }
        let tf = p.total_files.load(Ordering::Relaxed);
        if tf > 0 {
            (p.done_files.load(Ordering::Relaxed) as f32 / tf as f32).min(1.0)
        } else {
            0.0
        }
    }
}

/// Отмена сделанного.
#[derive(Debug, Clone)]
pub enum Undo {
    /// Созданные копии — убрать в корзину.
    Created(Vec<PathBuf>),
    /// Перемещения `(откуда, куда)` — вернуть.
    Moved(Vec<(PathBuf, PathBuf)>),
    Trashed(Vec<trash::Trashed>),
    Renamed(PathBuf, PathBuf),
}

impl Undo {
    pub fn title(&self) -> String {
        match self {
            Undo::Created(v) => t!("Отменить создание ({n})", n = v.len()),
            Undo::Moved(v) => t!("Отменить перемещение ({n})", n = v.len()),
            Undo::Trashed(v) => t!("Отменить удаление ({n})", n = v.len()),
            Undo::Renamed(_, to) => t!("Отменить переименование «{v}»", v = name_of(to)),
        }
    }

    /// Сообщение после отмены.
    pub fn done_title(&self) -> String {
        match self {
            Undo::Created(v) => t!("Отменено — создание ({n})", n = v.len()),
            Undo::Moved(v) => t!("Отменено — перемещение ({n})", n = v.len()),
            Undo::Trashed(v) => t!("Отменено — удаление ({n})", n = v.len()),
            Undo::Renamed(_, to) => t!("Отменено — переименование «{v}»", v = name_of(to)),
        }
    }
}

struct Global {
    jobs: Vec<Arc<Job>>,
    undo: Vec<Undo>,
    next: u64,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

static G: Mutex<Global> = Mutex::new(Global { jobs: Vec::new(), undo: Vec::new(), next: 1, notify: None });

/// Колбэк «задания изменились» (вызывается из рабочих потоков).
pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    G.lock().unwrap().notify = Some(Arc::new(f));
}

fn notify() {
    let f = G.lock().unwrap().notify.clone();
    if let Some(f) = f {
        f();
    }
}

pub fn jobs() -> Vec<Arc<Job>> {
    G.lock().unwrap().jobs.clone()
}

pub fn clear_finished() {
    G.lock().unwrap().jobs.retain(|j| !j.progress.finished.load(Ordering::SeqCst));
    notify();
}

pub fn push_undo(u: Undo) {
    let mut g = G.lock().unwrap();
    g.undo.push(u);
    if g.undo.len() > 100 {
        g.undo.remove(0);
    }
}

pub fn undo_title() -> Option<String> {
    G.lock().unwrap().undo.last().map(Undo::title)
}

/// Отменить последнее действие.
pub fn undo() -> Option<Result<String, String>> {
    let u = G.lock().unwrap().undo.pop()?;
    let title = u.done_title();
    let res: std::io::Result<()> = (|| {
        match &u {
            Undo::Created(v) => {
                for p in v {
                    if p.exists() {
                        trash::trash(p)?;
                    }
                }
            }
            Undo::Moved(v) => {
                for (from, to) in v.iter().rev() {
                    move_one(to, from)?;
                }
            }
            Undo::Trashed(v) => {
                for t in v {
                    trash::restore(&t.file)?;
                }
            }
            Undo::Renamed(from, to) => {
                if from.exists() {
                    return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, t!("имя уже занято")));
                }
                std::fs::rename(to, from)?;
            }
        }
        Ok(())
    })();
    notify();
    Some(res.map(|_| title).map_err(|e| e.to_string()))
}

/// Запустить задание в фоне.
pub fn start(op: Op) -> Arc<Job> {
    let job = {
        let mut g = G.lock().unwrap();
        let id = g.next;
        g.next += 1;
        let job = Arc::new(Job { id, title: op.title(), op, progress: Arc::new(Progress::default()) });
        g.jobs.push(job.clone());
        job
    };
    notify();
    let j = job.clone();
    std::thread::Builder::new()
        .name(format!("files-job-{}", j.id))
        .spawn(move || {
            let undo = run(&j);
            j.progress.finished.store(true, Ordering::SeqCst);
            crate::dirsize::invalidate(&j.op.touched());
            if let Some(u) = undo {
                push_undo(u);
            }
            notify();
            // Успешные задания сами уходят из списка через пару секунд.
            let ok = j.progress.errors.lock().unwrap().is_empty() && !j.progress.cancel.load(Ordering::SeqCst);
            if ok {
                std::thread::sleep(Duration::from_secs(3));
                G.lock().unwrap().jobs.retain(|x| x.id != j.id);
                notify();
            }
        })
        .expect("поток задания");
    job
}

fn err(p: &Progress, msg: String) {
    p.errors.lock().unwrap().push(msg);
}

struct Runner<'a> {
    job: &'a Job,
    p: &'a Progress,
    /// Решение «для всех».
    sticky: Option<Resolution>,
    last_notify: Instant,
    rate_mark: (Instant, u64),
}

impl Runner<'_> {
    fn cancelled(&mut self) -> bool {
        while self.p.paused.load(Ordering::SeqCst) && !self.p.cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(80));
        }
        self.p.cancel.load(Ordering::SeqCst)
    }

    fn tick(&mut self, current: &Path) {
        if self.last_notify.elapsed() > Duration::from_millis(120) {
            *self.p.current.lock().unwrap() = name_of(current);
            let done = self.p.done_bytes.load(Ordering::Relaxed);
            let dt = self.rate_mark.0.elapsed().as_secs_f64();
            if dt > 0.8 {
                let r = (done.saturating_sub(self.rate_mark.1)) as f64 / dt;
                self.p.rate.store(r as u64, Ordering::Relaxed);
                self.rate_mark = (Instant::now(), done);
            }
            self.last_notify = Instant::now();
            notify();
        }
    }

    fn ask(&mut self, c: Conflict) -> Resolution {
        if let Some(r) = self.sticky {
            return r;
        }
        *self.p.answer.lock().unwrap() = None;
        *self.p.conflict.lock().unwrap() = Some(c);
        notify();
        let mut g = self.p.answer.lock().unwrap();
        while g.is_none() {
            g = self.p.cv.wait(g).unwrap();
        }
        let (r, all) = g.take().unwrap();
        if all {
            self.sticky = Some(r);
        }
        r
    }
}

fn run(job: &Job) -> Option<Undo> {
    let p = &*job.progress;
    let mut r = Runner { job, p, sticky: None, last_notify: Instant::now(), rate_mark: (Instant::now(), 0) };
    let _ = &r.job;
    match &job.op {
        Op::Copy { srcs, dest } | Op::Move { srcs, dest } => {
            let moving = matches!(job.op, Op::Move { .. });
            // Перемещение в пределах раздела — переименование, считать байты не нужно.
            let same_fs = moving && srcs.iter().all(|s| same_device(s, dest));
            if !same_fs {
                let (bytes, files) = measure(srcs);
                p.total_bytes.store(bytes, Ordering::Relaxed);
                p.total_files.store(files, Ordering::Relaxed);
            } else {
                p.total_files.store(srcs.len() as u64, Ordering::Relaxed);
            }
            let mut created = Vec::new();
            let mut moved = Vec::new();
            for src in srcs {
                if r.cancelled() {
                    break;
                }
                let Some(name) = src.file_name() else { continue };
                let mut dst = dest.join(name);
                if src.is_dir() && dest.starts_with(src) {
                    err(p, if moving { t!("Нельзя переместить папку «{name}» в саму себя", name = name_of(src)) } else { t!("Нельзя скопировать папку «{name}» в саму себя", name = name_of(src)) });
                    continue;
                }
                if dst == *src {
                    if moving {
                        continue;
                    }
                    dst = free_name(&dst);
                } else if dst.symlink_metadata().is_ok() {
                    match r.ask(Conflict { src: src.clone(), dst: dst.clone(), dir: dst.is_dir() && src.is_dir() }) {
                        Resolution::Cancel => break,
                        Resolution::Skip => continue,
                        Resolution::KeepBoth => dst = free_name(&dst),
                        Resolution::Replace => {
                            // Папка в папку — слияние; иначе заменить.
                            if !(dst.is_dir() && src.is_dir()) {
                                if let Err(e) = remove_recursive(&dst) {
                                    err(p, format!("{}: {e}", dst.display()));
                                    continue;
                                }
                            }
                        }
                    }
                }
                let res = if moving {
                    if dst.is_dir() && src.is_dir() && dst.exists() {
                        copy_tree(&mut r, src, &dst).and_then(|_| remove_recursive(src))
                    } else if std::fs::rename(src, &dst).is_ok() {
                        p.done_files.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    } else {
                        copy_tree(&mut r, src, &dst).and_then(|_| {
                            if r.cancelled() {
                                Ok(())
                            } else {
                                remove_recursive(src)
                            }
                        })
                    }
                } else {
                    copy_tree(&mut r, src, &dst)
                };
                match res {
                    Ok(()) => {
                        if moving {
                            moved.push((src.clone(), dst));
                        } else {
                            created.push(dst);
                        }
                    }
                    Err(e) => err(p, format!("{}: {e}", src.display())),
                }
                r.tick(src);
            }
            if moving {
                (!moved.is_empty()).then_some(Undo::Moved(moved))
            } else {
                (!created.is_empty()).then_some(Undo::Created(created))
            }
        }
        Op::Link { srcs, dest } => {
            let mut created = Vec::new();
            for src in srcs {
                let dst = free_name(&dest.join(src.file_name().unwrap_or_default()));
                match std::os::unix::fs::symlink(src, &dst) {
                    Ok(()) => created.push(dst),
                    Err(e) => err(p, format!("{}: {e}", src.display())),
                }
            }
            (!created.is_empty()).then_some(Undo::Created(created))
        }
        Op::Trash { srcs } => {
            p.total_files.store(srcs.len() as u64, Ordering::Relaxed);
            let mut done = Vec::new();
            for s in srcs {
                if r.cancelled() {
                    break;
                }
                match trash::trash(s) {
                    Ok(t) => done.push(t),
                    Err(e) => err(p, format!("{}: {e}", s.display())),
                }
                p.done_files.fetch_add(1, Ordering::Relaxed);
                r.tick(s);
            }
            (!done.is_empty()).then_some(Undo::Trashed(done))
        }
        Op::Delete { srcs } => {
            let (_, files) = measure(srcs);
            p.total_files.store(files, Ordering::Relaxed);
            for s in srcs {
                if r.cancelled() {
                    break;
                }
                let res = if trash::contains(s) { trash::purge(s) } else { delete_tree(&mut r, s) };
                if let Err(e) = res {
                    err(p, format!("{}: {e}", s.display()));
                }
            }
            None
        }
        Op::Restore { files } => {
            for f in files {
                if let Err(e) = trash::restore(f) {
                    err(p, format!("{}: {e}", name_of(f)));
                }
            }
            None
        }
        Op::EmptyTrash => {
            if let Err(e) = trash::empty() {
                err(p, e.to_string());
            }
            None
        }
    }
}

fn same_device(a: &Path, b: &Path) -> bool {
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// Объём и число файлов (без перехода по ссылкам).
pub fn measure(srcs: &[PathBuf]) -> (u64, u64) {
    fn walk(p: &Path, b: &mut u64, f: &mut u64) {
        let Ok(m) = std::fs::symlink_metadata(p) else { return };
        *f += 1;
        if m.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), b, f);
                }
            }
        } else {
            *b += m.len();
        }
    }
    let (mut b, mut f) = (0, 0);
    for s in srcs {
        walk(s, &mut b, &mut f);
    }
    (b, f)
}

/// Свободное имя: `a (1).txt`, `a (2).txt`…
pub fn free_name(p: &Path) -> PathBuf {
    if p.symlink_metadata().is_err() {
        return p.to_path_buf();
    }
    let parent = p.parent().unwrap_or(Path::new("/"));
    let name = name_of(p);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !p.is_dir() => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    // «a (3)» → основа «a».
    let stem = match stem.rfind(" (") {
        Some(i) if stem.ends_with(')') && stem[i + 2..stem.len() - 1].chars().all(|c| c.is_ascii_digit()) => stem[..i].to_string(),
        _ => stem,
    };
    (1..)
        .map(|n| parent.join(format!("{stem} ({n}){ext}")))
        .find(|c| c.symlink_metadata().is_err())
        .unwrap()
}

fn copy_tree(r: &mut Runner, src: &Path, dst: &Path) -> std::io::Result<()> {
    if r.cancelled() {
        return Ok(());
    }
    let m = std::fs::symlink_metadata(src)?;
    let ft = m.file_type();
    if ft.is_symlink() {
        let target = std::fs::read_link(src)?;
        if dst.symlink_metadata().is_ok() {
            std::fs::remove_file(dst)?;
        }
        std::os::unix::fs::symlink(target, dst)?;
        r.p.done_files.fetch_add(1, Ordering::Relaxed);
    } else if ft.is_dir() {
        if !dst.exists() {
            std::fs::create_dir(dst)?;
        }
        r.p.done_files.fetch_add(1, Ordering::Relaxed);
        for e in std::fs::read_dir(src)? {
            let e = e?;
            let to = dst.join(e.file_name());
            if to.symlink_metadata().is_ok() && !(to.is_dir() && e.path().is_dir()) {
                match r.ask(Conflict { src: e.path(), dst: to.clone(), dir: false }) {
                    Resolution::Cancel => {
                        r.p.cancel.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    Resolution::Skip => {
                        r.p.done_bytes.fetch_add(e.metadata().map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
                        continue;
                    }
                    Resolution::KeepBoth => {
                        let free = free_name(&to);
                        copy_tree(r, &e.path(), &free)?;
                        continue;
                    }
                    Resolution::Replace => remove_recursive(&to)?,
                }
            }
            copy_tree(r, &e.path(), &to)?;
            if r.cancelled() {
                return Ok(());
            }
        }
        let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(m.mode() & 0o7777));
    } else if ft.is_file() {
        copy_file(r, src, dst, &m)?;
        r.p.done_files.fetch_add(1, Ordering::Relaxed);
    } else {
        // Устройства, сокеты, FIFO не копируем.
        r.p.done_files.fetch_add(1, Ordering::Relaxed);
    }
    r.tick(src);
    Ok(())
}

fn copy_file(r: &mut Runner, src: &Path, dst: &Path, m: &std::fs::Metadata) -> std::io::Result<()> {
    let mut from = std::fs::File::open(src)?;
    let mut to = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if r.cancelled() {
            drop(to);
            let _ = std::fs::remove_file(dst);
            return Ok(());
        }
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        to.write_all(&buf[..n])?;
        r.p.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
        r.tick(src);
    }
    let _ = to.set_permissions(std::fs::Permissions::from_mode(m.mode() & 0o7777));
    set_mtime(&to, m);
    Ok(())
}

fn set_mtime(f: &std::fs::File, m: &std::fs::Metadata) {
    use std::os::unix::io::AsRawFd;
    let ts = [
        libc::timespec { tv_sec: m.atime() as libc::time_t, tv_nsec: m.atime_nsec() as _ },
        libc::timespec { tv_sec: m.mtime() as libc::time_t, tv_nsec: m.mtime_nsec() as _ },
    ];
    unsafe {
        libc::futimens(f.as_raw_fd(), ts.as_ptr());
    }
}

fn delete_tree(r: &mut Runner, p: &Path) -> std::io::Result<()> {
    if r.cancelled() {
        return Ok(());
    }
    let m = std::fs::symlink_metadata(p)?;
    if m.is_dir() {
        for e in std::fs::read_dir(p)? {
            delete_tree(r, &e?.path())?;
        }
        std::fs::remove_dir(p)?;
    } else {
        std::fs::remove_file(p)?;
    }
    r.p.done_files.fetch_add(1, Ordering::Relaxed);
    r.tick(p);
    Ok(())
}

/// Удалить файл или каталог целиком (ссылки — сами ссылки).
pub fn remove_recursive(p: &Path) -> std::io::Result<()> {
    let m = std::fs::symlink_metadata(p)?;
    if m.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

/// Копия без прогресса (восстановление из корзины с другого раздела).
pub fn copy_recursive_simple(src: &Path, dst: &Path) -> std::io::Result<()> {
    let m = std::fs::symlink_metadata(src)?;
    if m.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)
    } else if m.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_recursive_simple(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn move_one(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.symlink_metadata().is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, t!("{to} уже существует", to = to.display())));
    }
    if std::fs::rename(from, to).is_err() {
        copy_recursive_simple(from, to)?;
        remove_recursive(from)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- мгновенные

/// Проверка имени файла: пустое, `/`, `.`/`..`.
pub fn check_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(t!("Имя не может быть пустым").into());
    }
    if n.contains('/') || n.contains('\0') {
        return Err(t!("Имя не может содержать «/»").into());
    }
    if n == "." || n == ".." {
        return Err(t!("Недопустимое имя").into());
    }
    Ok(())
}

pub fn rename(from: &Path, new_name: &str) -> Result<PathBuf, String> {
    check_name(new_name)?;
    let to = from.parent().unwrap_or(Path::new("/")).join(new_name.trim());
    if to == from {
        return Ok(to);
    }
    // Смена только регистра допустима (на нечувствительных к регистру ФС).
    let same_ignoring_case = name_of(from).to_lowercase() == new_name.trim().to_lowercase();
    if to.symlink_metadata().is_ok() && !same_ignoring_case {
        return Err(t!("«{trim}» уже существует", trim = new_name.trim()));
    }
    std::fs::rename(from, &to).map_err(|e| e.to_string())?;
    push_undo(Undo::Renamed(from.to_path_buf(), to.clone()));
    notify();
    Ok(to)
}

pub fn new_folder(dir: &Path, name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    let p = free_name(&dir.join(name.trim()));
    std::fs::create_dir(&p).map_err(|e| e.to_string())?;
    push_undo(Undo::Created(vec![p.clone()]));
    Ok(p)
}

pub fn new_file(dir: &Path, name: &str, content: &[u8]) -> Result<PathBuf, String> {
    check_name(name)?;
    let p = free_name(&dir.join(name.trim()));
    std::fs::write(&p, content).map_err(|e| e.to_string())?;
    push_undo(Undo::Created(vec![p.clone()]));
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(job: &Arc<Job>) {
        let t = Instant::now();
        while !job.progress.finished.load(Ordering::SeqCst) {
            assert!(t.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn free_names() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.txt");
        assert_eq!(free_name(&a), a);
        std::fs::write(&a, "").unwrap();
        assert_eq!(free_name(&a), d.path().join("a (1).txt"));
        std::fs::write(d.path().join("a (1).txt"), "").unwrap();
        assert_eq!(free_name(&d.path().join("a (1).txt")), d.path().join("a (2).txt"));
    }

    #[test]
    fn copy_move_and_undo() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/f.txt"), "hello").unwrap();
        let dst = d.path().join("dst");
        std::fs::create_dir(&dst).unwrap();
        let j = start(Op::Copy { srcs: vec![src.clone()], dest: dst.clone() });
        wait(&j);
        assert_eq!(std::fs::read_to_string(dst.join("src/sub/f.txt")).unwrap(), "hello");
        assert_eq!(j.fraction(), 1.0);
        // Вставка в ту же папку — «(1)».
        let j = start(Op::Copy { srcs: vec![src.clone()], dest: d.path().to_path_buf() });
        wait(&j);
        assert!(d.path().join("src (1)/sub/f.txt").exists());
        let j = start(Op::Move { srcs: vec![d.path().join("src (1)")], dest: dst.clone() });
        wait(&j);
        assert!(dst.join("src (1)").exists() && !d.path().join("src (1)").exists());
        undo().unwrap().unwrap();
        assert!(d.path().join("src (1)").exists());
    }

    #[test]
    fn conflict_keep_both() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.txt"), "new").unwrap();
        std::fs::write(b.join("x.txt"), "old").unwrap();
        let j = start(Op::Copy { srcs: vec![a.join("x.txt")], dest: b.clone() });
        let t = Instant::now();
        while j.progress.conflict.lock().unwrap().is_none() {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        j.answer(Resolution::KeepBoth, false);
        wait(&j);
        assert_eq!(std::fs::read_to_string(b.join("x.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(b.join("x (1).txt")).unwrap(), "new");
    }

    #[test]
    fn rename_checks() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::write(&a, "").unwrap();
        std::fs::write(d.path().join("b"), "").unwrap();
        assert!(rename(&a, "b").is_err());
        assert!(rename(&a, "x/y").is_err());
        assert_eq!(rename(&a, "c").unwrap(), d.path().join("c"));
    }
}
