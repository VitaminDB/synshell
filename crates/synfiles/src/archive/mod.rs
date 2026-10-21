//! Архивы как папки. Архив монтируется помощником ([`fs`]) в
//! `$XDG_RUNTIME_DIR/synfiles/archives/<ключ>/<имя архива>`, рядом —
//! файл `source` с путём архива. Панель ходит по настоящим путям
//! монтирования (копирование, открытие, миниатюры работают как обычно),
//! а адрес и крошки показывают путь через архив: `~/Загрузки/фото.zip/2024`.
//! Вложенные архивы — так же: путь архива сам лежит внутри другого
//! монтирования.

pub mod fs;
mod la;

use std::collections::HashMap;
use std::io::{BufRead, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use syngui::t;

use crate::model::Entry;

pub(crate) fn base() -> PathBuf {
    synshell_common::paths::runtime_dir().join("synfiles/archives")
}

fn key(archive: &Path) -> String {
    let d = md5::compute(archive.as_os_str().as_bytes());
    format!("{d:x}")[..16].to_string()
}

pub(crate) fn cache_dir(archive: &Path) -> PathBuf {
    synshell_common::paths::cache_home().join("synfiles/archives").join(key(archive))
}

/// Архив читается вразнобой без распаковки с начала (zip, iso).
pub(crate) fn seekable_name(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    [".zip", ".jar", ".apk", ".iso", ".cbz", ".xpi", ".whl"].iter().any(|e| n.ends_with(e))
}

/// Папка, куда монтируется архив.
pub fn mount_point(archive: &Path) -> PathBuf {
    let name = archive.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "archive".into());
    base().join(key(archive)).join(name)
}

/// Корень монтирования архива, внутри которого лежит `p`.
pub fn root_of(p: &Path) -> Option<PathBuf> {
    let rest = p.strip_prefix(base()).ok()?;
    let mut c = rest.components();
    let (Some(Component::Normal(k)), Some(Component::Normal(n))) = (c.next(), c.next()) else { return None };
    Some(base().join(k).join(n))
}

/// Путь архива, смонтированного в `root`.
pub fn source(root: &Path) -> Option<PathBuf> {
    let t = std::fs::read(root.parent()?.join("source")).ok()?;
    let s = t.split(|b| *b == b'\n').next()?;
    (!s.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(s)))
}

/// Путь для показа: через архив, а не через папку монтирования.
pub fn display(p: &Path) -> PathBuf {
    if let Some(root) = root_of(p) {
        if let Some(src) = source(&root) {
            let rest = p.strip_prefix(&root).unwrap_or(Path::new(""));
            let d = display(&src);
            return if rest.as_os_str().is_empty() { d } else { d.join(rest) };
        }
    }
    p.to_path_buf()
}

/// Запись этого пути в родительской папке: для корня архива — сам архив
/// (выделить его, поднявшись наверх).
pub fn entry_path(p: &Path) -> PathBuf {
    match root_of(p) {
        Some(r) if r == p => source(&r).unwrap_or_else(|| p.to_path_buf()),
        _ => p.to_path_buf(),
    }
}

/// Родительская папка: из корня архива — папка, где лежит архив.
pub fn parent(p: &Path) -> Option<PathBuf> {
    match root_of(p) {
        Some(r) if r == p => source(&r).and_then(|s| s.parent().map(Path::to_path_buf)),
        _ => p.parent().map(Path::to_path_buf),
    }
}

fn archive_name(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    crate::ops::is_archive("", &n)
}

/// Путь, набранный «через архив» (`/x/a.zip/папка`), — в путь монтирования.
/// `dir` — сам архив тоже превращается в папку (крошки, строка адреса).
pub fn resolve(p: &Path, dir: bool) -> PathBuf {
    if p.exists() && !(dir && p.is_file() && archive_name(p)) {
        return p.to_path_buf();
    }
    let mut acc = PathBuf::new();
    let comps: Vec<Component> = p.components().collect();
    for (i, c) in comps.iter().enumerate() {
        acc.push(c);
        let last = i + 1 == comps.len();
        // Внутри ещё не смонтированного архива файлов не видно — по имени.
        let inner = root_of(&acc).is_some_and(|r| r != acc) && !acc.is_dir();
        if (!last || dir) && archive_name(&acc) && (acc.is_file() || inner) {
            let mut m = mount_point(&acc);
            // Записать источник сразу: путь показывается до монтирования.
            remember(&acc, &m);
            for c in &comps[i + 1..] {
                m.push(c);
            }
            return resolve(&m, dir);
        }
    }
    p.to_path_buf()
}

fn remember(archive: &Path, mp: &Path) {
    if let Some(k) = mp.parent() {
        let _ = std::fs::create_dir_all(k);
        let f = k.join("source");
        if source(mp).as_deref() != Some(archive) {
            let _ = std::fs::write(f, archive.as_os_str().as_bytes());
        }
    }
}

/// Папка для открытия архива как папки.
pub fn folder(archive: &Path) -> PathBuf {
    let mp = mount_point(archive);
    remember(archive, &mp);
    mp
}

/// Открывать ли архив папкой (настройка или Проводник — программа для него).
pub fn browse(e: &Entry) -> bool {
    if e.is_dir || e.broken || !crate::ops::is_archive(&e.mime, &e.name) {
        return false;
    }
    if crate::state::try_ctx().map(|c| c.cfg.get_untracked().files.archives_as_folders).unwrap_or(true) {
        return true;
    }
    synshell_common::mime::default_app(&e.mime).is_some_and(|a| a.id == "synfiles")
}

fn unescape_mount(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

pub(crate) fn is_mounted(mp: &Path) -> bool {
    let Ok(t) = std::fs::read_to_string("/proc/self/mountinfo") else { return false };
    let want = mp.as_os_str().as_bytes();
    t.lines().any(|l| l.split(' ').nth(4).is_some_and(|m| unescape_mount(m) == want))
}

/// Монтирования, которые сейчас поднимаются (две панели в один архив).
static MOUNTING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Смонтировать архив, внутри которого лежит `p` (если ещё нет). Долго для
/// больших сжатых архивов — вызывать не из потока интерфейса.
pub fn ensure(p: &Path) -> Result<(), String> {
    let Some(root) = root_of(p) else { return Ok(()) };
    let Some(src) = source(&root) else { return Err(t!("Архив для «{p}» не найден", p = p.display()).into()) };
    // Архив во вложенном архиве — сначала внешний.
    ensure(&src)?;
    loop {
        let mut m = MOUNTING.lock().unwrap();
        if !m.contains(&root) {
            m.push(root.clone());
            break;
        }
        drop(m);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let r = mount(&src, &root);
    MOUNTING.lock().unwrap().retain(|x| x != &root);
    r
}

fn mount(src: &Path, root: &Path) -> Result<(), String> {
    if is_mounted(root) {
        match std::fs::metadata(root) {
            Ok(_) => return Ok(()),
            // Помощник умер — снять мёртвое монтирование.
            Err(_) => {
                let _ = Command::new("fusermount3").arg("-uz").arg(root).stderr(Stdio::null()).status();
            }
        }
    }
    if !src.is_file() {
        return Err(t!("Архив «{p}» не найден", p = display(src).display()).into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.arg("--archive-mount").arg(src).arg(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Своя сессия: помощник переживает окно Проводника.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let _ = out.read_line(&mut line);
    if line.trim() == "ready" {
        // Не ждать помощника: он живёт дальше сам (зомби подберёт поток).
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let _ = child.wait();
    let err = err.trim();
    Err(if err.is_empty() {
        t!("Не удалось открыть архив «{p}»", p = display(src).display()).into()
    } else {
        t!("Не удалось открыть архив «{p}»: {e}", p = display(src).display(), e = err).into()
    })
}

/// Открытые корни архивов по панелям: пока панель внутри архива, помощник
/// видит открытую папку и не снимается.
static HOLD: Mutex<Option<HashMap<u64, (PathBuf, std::fs::File)>>> = Mutex::new(None);

/// Панель `pid` теперь в `p` (или ушла из архивов — `None`).
pub fn hold(pid: u64, p: Option<&Path>) {
    let root = p.and_then(root_of);
    let mut g = HOLD.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.get(&pid).map(|(r, _)| r) == root.as_ref() {
        return;
    }
    m.remove(&pid);
    if let Some(r) = root {
        if let Ok(f) = std::fs::File::open(&r) {
            m.insert(pid, (r, f));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_paths() {
        let a = Path::new("/home/u/Загрузки/фото.zip");
        let mp = mount_point(a);
        assert_eq!(mp.file_name().unwrap(), "фото.zip");
        assert_eq!(root_of(&mp.join("x/y")), Some(mp.clone()));
        assert_eq!(root_of(Path::new("/home/u")), None);
        assert_eq!(unescape_mount("/a\\040b"), b"/a b");
    }
}
