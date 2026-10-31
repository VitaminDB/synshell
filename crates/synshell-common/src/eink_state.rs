//! Последние применённые режимы экрана E Ink (`[eink]`: цвет, режим обновления, расширенные) — для
//! экрана входа и сеансов без своего `[eink]`.
//!
//! Драйвер после загрузки стартует со своими умолчаниями (монохром, `balanced`), а у экрана входа (root)
//! своего `[eink]` нет. synwm сеанса, применив своё `[eink]`, запоминает его (поверх прежнего, [`merge`])
//! в общий файл [`system_file`] (`/var/lib/synshell/eink`, `root:video 0664`), недоступен для записи — в
//! `$XDG_STATE_HOME/synshell/eink`; только файл, в sysfs synwm пишет лишь своё `[eink]`. Применяет
//! запомненное `synlogin daemon` от root — ДО запуска synwm экрана входа, пока DRM-устройство никто не
//! держит (запись в sysfs драйвера при работающем выводе ломала состояние CRTC: «active without enabled»).
//! Формат — TOML полей [`EinkSettings`].

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
pub fn merge(cfg: &EinkSettings, saved: &EinkSettings) -> EinkSettings {
    EinkSettings {
        color: cfg.color.or(saved.color),
        update_mode: cfg.update_mode.clone().or_else(|| saved.update_mode.clone()),
        contrast: cfg.contrast.or(saved.contrast),
        saturation: cfg.saturation.or(saved.saturation),
        a2_frames: cfg.a2_frames.or(saved.a2_frames),
        a2_white: cfg.a2_white.or(saved.a2_white),
        a2_black: cfg.a2_black.or(saved.a2_black),
        full_every: cfg.full_every.or(saved.full_every),
        full_pct: cfg.full_pct.or(saved.full_pct),
    }
}

fn read(path: &Path) -> Option<(std::time::SystemTime, EinkSettings)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    Some((mtime, toml::from_str(&text).ok()?))
}

/// Ничего не задано (пустое `[eink]` или файл).
pub fn is_empty(s: &EinkSettings) -> bool {
    *s == EinkSettings::default()
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
    if let Some(sys) = system {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).truncate(true).open(sys) {
            use std::io::Write;
            f.write_all(text.as_bytes())?;
            return Ok(sys.to_path_buf());
        }
        if sys.parent().is_some_and(|d| d.is_dir()) && std::fs::write(sys, &text).is_ok() {
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
        let saved = EinkSettings { color: Some(true), update_mode: UpdateMode::from_name("fast"), contrast: Some(10), ..Default::default() };
        let own = EinkSettings { update_mode: UpdateMode::from_name("balanced"), ..Default::default() };
        let m = merge(&own, &saved);
        assert_eq!(m.color, Some(true));
        assert_eq!(m.update_mode, UpdateMode::from_name("balanced"));
        assert_eq!(m.contrast, Some(10));
        assert_eq!(merge(&EinkSettings::default(), &saved), saved);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("synshell-eink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sys")).unwrap();
        let sys = dir.join("sys/eink");
        let user = dir.join("state/synshell/eink");
        let s = EinkSettings { color: Some(true), update_mode: UpdateMode::from_name("fast"), saturation: Some(200), ..Default::default() };
        assert_eq!(save_to(Some(&sys), &user, &s).unwrap(), sys);
        assert_eq!(load_from(Some(&sys), &user), s);
        // общий недоступен — файл пользователя, он свежее
        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = EinkSettings { color: Some(false), ..Default::default() };
        assert_eq!(save_to(Some(&dir.join("nodir/eink")), &user, &s2).unwrap(), user);
        assert_eq!(load_from(Some(&sys), &user), s2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
