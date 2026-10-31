//! Последняя применённая ориентация экрана (по датчику или ручная) — между запусками композитора.
//!
//! Это состояние, а не настройка: пишет synwm при каждом повороте, конфиг не трогается. Файл — строки
//! `<вывод> <поворот>` (`DSI-1 90`): у вложенного synwm свой вывод, и его поворот не попадёт на настоящий экран.
//!
//! Где лежит:
//! - [`system_file`] — `/var/lib/synshell/rotation`, общий для экрана входа (synwm от root) и сеансов:
//!   ориентация — свойство устройства. Каталог и файл (группа `video`, `0664`) создаёт `synlogin daemon`
//!   ([`ensure_system_file`]); писать в него может любой пользователь из `video`;
//! - [`user_file`] — `$XDG_STATE_HOME/synshell/rotation`, если общий недоступен для записи.
//!
//! При чтении для вывода берётся запись из более свежего файла.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::action::Rotation;

/// Подмена общего файла (проверка): `SYNSHELL_ROTATION_SYSTEM_FILE=/tmp/x`, пустое — без общего файла.
pub const SYSTEM_FILE_ENV: &str = "SYNSHELL_ROTATION_SYSTEM_FILE";

/// Общий файл состояния (`None` — отключён подменой).
pub fn system_file() -> Option<PathBuf> {
    match std::env::var_os(SYSTEM_FILE_ENV) {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(PathBuf::from(v)),
        None => Some(PathBuf::from("/var/lib/synshell/rotation")),
    }
}

/// Файл состояния пользователя: `$XDG_STATE_HOME/synshell/rotation`.
pub fn user_file() -> PathBuf {
    crate::paths::state_home().join("synshell").join("rotation")
}

/// Разбор файла: `вывод поворот` по строке; кривые строки пропускаются.
pub fn parse(text: &str) -> BTreeMap<String, Rotation> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (name, r) = l.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            if name.is_empty() || name.starts_with('#') {
                return None;
            }
            Some((name.to_string(), r.parse().ok()?))
        })
        .collect()
}

pub fn format(map: &BTreeMap<String, Rotation>) -> String {
    map.iter().map(|(n, r)| format!("{n} {r}\n")).collect()
}

fn read(path: &Path) -> Option<(std::time::SystemTime, BTreeMap<String, Rotation>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    Some((mtime, parse(&text)))
}

/// Запомненные ориентации всех выводов: из обоих файлов, у каждого вывода — из более свежего.
pub fn load() -> BTreeMap<String, Rotation> {
    load_from(system_file().as_deref(), &user_file())
}

pub fn load_from(system: Option<&Path>, user: &Path) -> BTreeMap<String, Rotation> {
    let mut files: Vec<_> = system.into_iter().chain([user]).filter_map(read).collect();
    // старые первыми: записи свежего файла перекрывают их
    files.sort_by_key(|(t, _)| *t);
    files.into_iter().flat_map(|(_, m)| m).collect()
}

/// Запомнить ориентацию вывода: в общий файл, если он есть и доступен для записи, иначе — в файл пользователя.
pub fn save(output: &str, r: Rotation) -> std::io::Result<PathBuf> {
    save_to(system_file().as_deref(), &user_file(), output, r)
}

pub fn save_to(system: Option<&Path>, user: &Path, output: &str, r: Rotation) -> std::io::Result<PathBuf> {
    if let Some(sys) = system {
        // На месте, без переименования: владелец и права (root:video 0664) остаются.
        if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open(sys) {
            use std::io::{Read, Seek, Write};
            let mut text = String::new();
            let _ = f.read_to_string(&mut text);
            let mut map = parse(&text);
            map.insert(output.to_string(), r);
            let out = format(&map);
            f.seek(std::io::SeekFrom::Start(0))?;
            f.set_len(0)?;
            f.write_all(out.as_bytes())?;
            return Ok(sys.to_path_buf());
        }
        // Нет файла, но есть право создать (synwm экрана входа — root).
        if sys.parent().is_some_and(|d| d.is_dir()) && std::fs::write(sys, format(&BTreeMap::from([(output.to_string(), r)]))).is_ok() {
            return Ok(sys.to_path_buf());
        }
    }
    let mut map = std::fs::read_to_string(user).map(|t| parse(&t)).unwrap_or_default();
    map.insert(output.to_string(), r);
    if let Some(d) = user.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = user.with_extension("tmp");
    std::fs::write(&tmp, format(&map))?;
    std::fs::rename(&tmp, user)?;
    Ok(user.to_path_buf())
}

/// Общий файл с правами для сеансов (от root, `synlogin daemon`): каталог `2775 root:video`,
/// файл `0664 root:video`. Без группы `video` — только root (`0755`/`0644`).
pub fn ensure_system_file() -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let Some(file) = system_file() else { return Ok(()) };
    let Some(dir) = file.parent() else { return Ok(()) };
    std::fs::create_dir_all(dir)?;
    if !file.exists() {
        std::fs::write(&file, "")?;
    }
    let gid = group_id("video");
    let (dmode, fmode) = if gid.is_some() { (0o2775, 0o664) } else { (0o755, 0o644) };
    if let Some(gid) = gid {
        for p in [dir, file.as_path()] {
            let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
            // SAFETY: путь — нуль-терминированная строка; uid -1 — владелец не меняется.
            unsafe { libc::chown(c.as_ptr(), u32::MAX, gid) };
        }
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dmode))?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(fmode))?;
    Ok(())
}

fn group_id(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: getgrnam возвращает указатель на статическую запись или NULL.
    let g = unsafe { libc::getgrnam(c.as_ptr()) };
    (!g.is_null()).then(|| unsafe { (*g).gr_gid })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_format_roundtrip() {
        let m = parse("DSI-1 90\n# x\nbad\nwinit normal\nHDMI-A-1 7\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m["DSI-1"], Rotation::R90);
        assert_eq!(m["winit"], Rotation::Normal);
        assert_eq!(parse(&format(&m)), m);
    }

    #[test]
    fn save_prefers_system_file_and_newest_wins() {
        let dir = std::env::temp_dir().join(format!("synshell-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sys")).unwrap();
        let sys = dir.join("sys/rotation");
        let user = dir.join("state/synshell/rotation");
        // общего файла нет, но каталог есть — создаётся он
        assert_eq!(save_to(Some(&sys), &user, "DSI-1", Rotation::R270).unwrap(), sys);
        assert_eq!(load_from(Some(&sys), &user)["DSI-1"], Rotation::R270);
        // общий недоступен — файл пользователя; он свежее
        std::thread::sleep(std::time::Duration::from_millis(20));
        let missing = dir.join("nodir/rotation");
        assert_eq!(save_to(Some(&missing), &user, "DSI-1", Rotation::R90).unwrap(), user);
        assert_eq!(load_from(Some(&sys), &user)["DSI-1"], Rotation::R90);
        // снова в общий: он свежее, другие выводы сохраняются
        std::thread::sleep(std::time::Duration::from_millis(20));
        save_to(Some(&sys), &user, "winit", Rotation::R180).unwrap();
        save_to(Some(&sys), &user, "DSI-1", Rotation::Normal).unwrap();
        let m = load_from(Some(&sys), &user);
        assert_eq!(m["DSI-1"], Rotation::Normal);
        assert_eq!(m["winit"], Rotation::R180);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
