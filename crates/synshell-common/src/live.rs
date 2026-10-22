//! Живое обновление темы: программа перечитывает конфиг сразу после правки
//! `config.toml`, файлов активной темы, `theme.mss`/`apps/*.mss` пользователя
//! или примерки из редактора тем ([`crate::preview`]).
//!
//! Слежение — inotify по каталогам (переживает атомарную замену файла
//! переименованием); если inotify недоступен — опрос раз в секунду.
//! Пачка событий (редактор пишет несколько файлов) склеивается в одно
//! перечитывание.

use std::collections::BTreeSet;
use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{paths, Config};

/// Что отслеживать для конфига `cfg`: (каталог, имена файлов в нём; пусто —
/// любые).
fn targets(cfg: &Config, extra: &[PathBuf]) -> Vec<(PathBuf, BTreeSet<OsString>)> {
    let mut v: Vec<(PathBuf, BTreeSet<OsString>)> = Vec::new();
    let mut add = |dir: PathBuf, name: Option<OsString>| {
        let set = match v.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, s)) => s,
            None => {
                v.push((dir, BTreeSet::new()));
                &mut v.last_mut().unwrap().1
            }
        };
        match name {
            Some(n) => {
                set.insert(n);
            }
            // «Любой файл» — пустое множество; помечаем отдельно.
            None => {
                set.insert(OsString::from("*"));
            }
        }
    };
    let file = |p: &Path| (p.parent().map(Path::to_path_buf).unwrap_or_default(), p.file_name().map(OsString::from));
    for p in [paths::config_file(), paths::user_theme_file(), paths::system_config_file()].iter().chain(extra) {
        let (d, n) = file(p);
        add(d, n);
    }
    // Пользовательские слои программ `~/.config/synshell/apps/*.mss`.
    add(paths::config_dir().join("apps"), None);
    // Примерка: каталог подменяется переименованием в родительском.
    let pv = crate::preview::dir();
    let (d, n) = file(&pv);
    add(d, n);
    // Файлы активной темы с диска.
    for p in cfg.appearance.theme_files() {
        if let Some(d) = p.parent() {
            add(d.to_path_buf(), None);
        }
    }
    v
}

/// Следить за конфигом и темой в фоновом потоке; `on_change` получает
/// свежий конфиг после каждой правки (из этого же потока). `extra` — свои
/// файлы программы, при изменении которых тоже нужно перечитать.
pub fn spawn(extra: Vec<PathBuf>, mut on_change: impl FnMut(Config) + Send + 'static) {
    std::thread::Builder::new()
        .name("synshell-live".into())
        .spawn(move || {
            let _ = std::fs::create_dir_all(crate::preview::dir().parent().unwrap_or(Path::new("/tmp")));
            let mut cfg = Config::load().0;
            loop {
                let t = targets(&cfg, &extra);
                if !wait_inotify(&t) {
                    wait_poll(&t);
                }
                // Склеить пачку записей в одно перечитывание.
                std::thread::sleep(Duration::from_millis(40));
                let (c, err) = Config::load();
                if let Some(e) = err {
                    tracing::warn!("{e}");
                    continue;
                }
                cfg = c.clone();
                on_change(c);
            }
        })
        .expect("поток слежения за темой");
}

/// Ждать изменения через inotify; `false` — inotify не работает.
fn wait_inotify(targets: &[(PathBuf, BTreeSet<OsString>)]) -> bool {
    // SAFETY: обычные вызовы inotify, дескриптор закрывается в конце.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return false;
    }
    let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_MOVED_FROM | libc::IN_CREATE | libc::IN_DELETE | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF;
    let mut wds: Vec<(i32, &BTreeSet<OsString>)> = Vec::new();
    for (dir, names) in targets {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { continue };
        let wd = unsafe { libc::inotify_add_watch(fd, c.as_ptr(), mask) };
        if wd >= 0 {
            wds.push((wd, names));
        }
    }
    if wds.is_empty() {
        unsafe { libc::close(fd) };
        // Каталогов ещё нет — подождём и проверим заново.
        std::thread::sleep(Duration::from_secs(2));
        return true;
    }
    let mut buf = [0u8; 8192];
    'outer: loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        let mut off = 0usize;
        while off + std::mem::size_of::<libc::inotify_event>() <= n as usize {
            // SAFETY: ядро кладёт выровненные записи inotify_event подряд.
            let ev = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast::<libc::inotify_event>()) };
            let name_start = off + std::mem::size_of::<libc::inotify_event>();
            let name_bytes = &buf[name_start..name_start + ev.len as usize];
            let name = std::ffi::OsStr::from_bytes(name_bytes.split(|b| *b == 0).next().unwrap_or(&[]));
            off = name_start + ev.len as usize;
            if ev.mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED) != 0 {
                break 'outer;
            }
            let Some((_, names)) = wds.iter().find(|(wd, _)| *wd == ev.wd) else { continue };
            if names.contains(std::ffi::OsStr::new("*")) || names.contains(name) {
                break 'outer;
            }
        }
    }
    unsafe { libc::close(fd) };
    true
}

/// Запасной путь: опрос mtime раз в секунду.
fn wait_poll(targets: &[(PathBuf, BTreeSet<OsString>)]) {
    let files: Vec<PathBuf> = targets.iter().map(|(d, _)| d.clone()).collect();
    let mut w = crate::watch::FileWatcher::new(files);
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if w.poll() {
            return;
        }
    }
}

/// Таблица стилей окна syngui, которая пересобирается при каждой правке
/// темы: `build` вызывается с новым конфигом, результат кладётся в `theme`
/// в главном потоке, если изменился. `initial` — уже собранная таблица.
#[cfg(feature = "style")]
pub fn bind(theme: syngui::prelude::RwSignal<String>, initial: String, build: impl Fn(&Config) -> String + Send + 'static) {
    let mut last = initial;
    spawn(Vec::new(), move |cfg| {
        let mss = build(&cfg);
        if mss != last {
            last = mss.clone();
            syngui::prelude::run_on_main_thread(move || theme.set(mss));
        }
    });
}

/// Сигнал живой таблицы стилей для `App::with_live_styles`:
/// `initial` — таблица для текущего конфига, `build` — пересборка.
#[cfg(feature = "style")]
pub fn styles(initial: String, build: impl Fn(&Config) -> String + Send + 'static) -> syngui::prelude::RwSignal<String> {
    let sig = syngui::prelude::use_signal(initial.clone());
    bind(sig, initial, build);
    sig
}
