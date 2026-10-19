//! Один проект — один хозяин. Файл `.syn` пишет очередь автосохранения
//! своего процесса; два процесса с одним файлом (syn-notes и synthos,
//! два экземпляра) затирали бы коммиты друг друга. Открывая проект,
//! процесс берёт на него блокировку — `flock` на файле в
//! `$XDG_RUNTIME_DIR/notes-locks/` (имя — хэш пути). Сам `.syn` для этого
//! не годится: уплотнение подменяет его новым файлом, и блокировка осталась
//! бы на старом.
//!
//! Блокировка держится, пока проект открыт ([`release`] — при закрытии), и
//! снимается системой вместе с процессом. synthos до перехода на эту
//! библиотеку блокировок не берёт: работающий synthos, у которого проект
//! открыт в конфиге, тоже считается хозяином.

use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static HELD: Mutex<Option<HashMap<PathBuf, File>>> = Mutex::new(None);

fn lock_file(path: &Path) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("notes-locks");
    dir.join(format!("{:016x}.lock", h.finish()))
}

/// Занять проект. `Err` — кто его держит (для сообщения).
pub fn acquire(path: &Path) -> Result<(), String> {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let map = held.get_or_insert_with(HashMap::new);
    if map.contains_key(path) {
        return Ok(());
    }
    if let Some(who) = legacy_owner(path) {
        return Err(who);
    }
    let lf = lock_file(path);
    if let Some(dir) = lf.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).read(true).open(&lf).map_err(|e| e.to_string())?;
    // SAFETY: flock над своим открытым дескриптором.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let owner = std::fs::read_to_string(&lf).unwrap_or_default();
        return Err(if owner.trim().is_empty() { "другая программа".into() } else { owner.trim().to_string() });
    }
    // Кто держит — для сообщения второму процессу.
    let _ = file.set_len(0);
    let me = format!("{} (pid {})", std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default(), std::process::id());
    let _ = std::io::Write::write_all(&mut &file, me.as_bytes());
    map.insert(path.to_path_buf(), file);
    Ok(())
}

/// Отпустить проект (закрыт).
pub fn release(path: &Path) {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = held.as_mut() {
        map.remove(path);
    }
}

/// Проект переехал (переименование, «Сохранить как» с переходом).
pub fn moved(from: &Path, to: &Path) {
    release(from);
    let _ = acquire(to);
}

/// Работающий synthos без блокировок, у которого проект открыт.
fn legacy_owner(path: &Path) -> Option<String> {
    if std::env::var_os("SYN_NOTES_IGNORE_SYNTHOS").is_some() {
        return None;
    }
    let me = std::process::id();
    let pid = std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        if pid == me {
            return None;
        }
        let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
        (comm.trim() == "synthos").then_some(pid)
    })?;
    let cfg = std::fs::read_to_string(synshell_common::paths::home().join(".config/synthos/config.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&cfg).ok()?;
    let open = v.get("notes_projects")?.as_array()?.iter().filter_map(|p| p.get("path")?.as_str()).any(|p| super::project::normalize_path(Path::new(p)) == path);
    open.then(|| format!("synthos (pid {pid})"))
}
