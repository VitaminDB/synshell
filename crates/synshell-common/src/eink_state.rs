//! Последний выбор экрана E Ink (`[eink]`: цвет и активный профиль, см. [`crate::eink_profiles`]) — для
//! экрана входа и сеансов без своего `[eink]`.
//!
//! Драйвер после загрузки стартует со своими умолчаниями (монохром, `balanced`), а у экрана входа (root)
//! своего `[eink]` нет. synwm сеанса, применив своё `[eink]`, запоминает его (поверх прежнего, [`merge`])
//! в общий файл [`system_file`] (`/var/lib/synshell/eink`, `root:video 0664`), недоступен для записи — в
//! `$XDG_STATE_HOME/synshell/eink`; только файл, в sysfs synwm пишет лишь своё `[eink]`. Применяет
//! запомненное `synlogin daemon` от root — ДО запуска synwm экрана входа, пока DRM-устройство никто не
//! держит (запись в sysfs драйвера при работающем выводе ломала состояние CRTC: «active without enabled»).
//! Формат — TOML полей [`EinkSettings`] (`color`, `profile`; устаревшие ключи режимов читаются, но не
//! применяются и при следующей записи пропадают).

use std::path::{Path, PathBuf};

use crate::config::EinkSettings;

/// Подмена общего файла (проверка): `SYNSHELL_EINK_SYSTEM_FILE=/tmp/x`, пустое — без общего файла.
pub const SYSTEM_FILE_ENV: &str = "SYNSHELL_EINK_SYSTEM_FILE";

pub fn system_file() -> Option<PathBuf> {
    match std::env::var_os(SYSTEM_FILE_ENV) {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(PathBuf::from(v)),
        None => Some(PathBuf::from("/var/lib/synshell/eink")),
    }
}

pub fn user_file() -> PathBuf {
    crate::paths::state_home().join("synshell").join("eink")
}

/// Общий файл с правами для сеансов (от root, `synlogin daemon`).
pub fn ensure_system_file() -> std::io::Result<()> {
    match system_file() {
        Some(f) => crate::rotation_state::ensure_shared_file(&f),
        None => Ok(()),
    }
}

/// Поля `cfg`, а не заданные в нём — из `saved`.
/// Только `color` и `profile`: устаревшие ключи режимов не переносятся.
pub fn merge(cfg: &EinkSettings, saved: &EinkSettings) -> EinkSettings {
    EinkSettings {
        color: cfg.color.or(saved.color),
        profile: cfg.profile.clone().or_else(|| saved.profile.clone()),
        ..Default::default()
    }
}

fn read(path: &Path) -> Option<(std::time::SystemTime, EinkSettings)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    Some((mtime, toml::from_str(&text).ok()?))
}

/// Нечего запоминать или применять: не заданы ни цвет, ни профиль (устаревшие ключи не в счёт).
pub fn is_empty(s: &EinkSettings) -> bool {
    s.color.is_none() && s.profile.is_none()
}

/// Запомненные режимы: из более свежего файла (нет — пустые).
pub fn load() -> EinkSettings {
    load_from(system_file().as_deref(), &user_file())
}

pub fn load_from(system: Option<&Path>, user: &Path) -> EinkSettings {
    system.into_iter().chain([user]).filter_map(read).max_by_key(|(t, _)| *t).map(|(_, s)| s).unwrap_or_default()
}

/// Запомнить применённые режимы: в общий файл (на месте — права остаются; нет его — создать, если можно),
/// иначе в файл пользователя.
pub fn save(s: &EinkSettings) -> std::io::Result<PathBuf> {
    save_to(system_file().as_deref(), &user_file(), s)
}

pub fn save_to(system: Option<&Path>, user: &Path, s: &EinkSettings) -> std::io::Result<PathBuf> {
    let text = toml::to_string(s).map_err(std::io::Error::other)?;
    write_shared(system, user, &text)
}

/// Записать общий файл `system` (`root:video 0664` в каталоге `2775`): атомарно — новым файлом
/// рядом с правами `0664` (группа — от каталога с setgid); каталог не даёт — на месте (права
/// остаются); файла нет — создать, если можно; иначе — в файл пользователя `user`.
pub(crate) fn write_shared(system: Option<&Path>, user: &Path, text: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    if let Some(sys) = system {
        if sys.is_file() {
            let tmp = sys.with_extension(format!("tmp{}", std::process::id()));
            let atomic = std::fs::write(&tmp, text)
                .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o664)))
                .and_then(|_| std::fs::rename(&tmp, sys));
            if atomic.is_ok() {
                return Ok(sys.to_path_buf());
            }
            let _ = std::fs::remove_file(&tmp);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).truncate(true).open(sys) {
            f.write_all(text.as_bytes())?;
            return Ok(sys.to_path_buf());
        }
        if sys.parent().is_some_and(|d| d.is_dir()) && std::fs::write(sys, text).is_ok() {
            return Ok(sys.to_path_buf());
        }
    }
    if let Some(d) = user.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = user.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, user)?;
    Ok(user.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::UpdateMode;

    #[test]
    fn own_settings_win_saved_fill_gaps() {
        let saved = EinkSettings { color: Some(true), profile: Some("Чтение".into()), ..Default::default() };
        let own = EinkSettings { profile: Some("Видео".into()), ..Default::default() };
        let m = merge(&own, &saved);
        assert_eq!(m.color, Some(true));
        assert_eq!(m.profile.as_deref(), Some("Видео"));
        assert_eq!(merge(&EinkSettings::default(), &saved), saved);
        // устаревшие ключи не переносятся и не считаются выбором
        let legacy = EinkSettings { update_mode: UpdateMode::from_name("fast"), contrast: Some(3), ..Default::default() };
        assert!(is_empty(&legacy));
        assert_eq!(merge(&legacy, &saved), saved);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("synshell-eink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sys")).unwrap();
        let sys = dir.join("sys/eink");
        let user = dir.join("state/synshell/eink");
        let s = EinkSettings { color: Some(true), profile: Some("Чтение".into()), ..Default::default() };
        assert_eq!(save_to(Some(&sys), &user, &s).unwrap(), sys);
        assert_eq!(load_from(Some(&sys), &user), s);
        // второй раз — атомарно поверх, права 0664
        assert_eq!(save_to(Some(&sys), &user, &s).unwrap(), sys);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&sys).unwrap().permissions().mode() & 0o777, 0o664);
        // старый файл с режимами — читается
        std::fs::write(&sys, "update_mode = \"fast\"\ncontrast = 2\n").unwrap();
        assert_eq!(load_from(Some(&sys), &user).update_mode, UpdateMode::from_name("fast"));
        // общий недоступен — файл пользователя, он свежее
        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = EinkSettings { color: Some(false), ..Default::default() };
        assert_eq!(save_to(Some(&dir.join("nodir/eink")), &user, &s2).unwrap(), user);
        assert_eq!(load_from(Some(&sys), &user), s2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
