//! Примерка темы: черновик из редактора тем сразу видно во всех программах,
//! без записи в `config.toml`.
//!
//! Редактор кладёт черновик в `$XDG_RUNTIME_DIR/synshell/theme-preview/`:
//! - `preview.toml` — `pid` редактора, `theme` (имя темы) и необязательная
//!   таблица `[appearance]` с переопределениями (`accent`, `color_scheme`,
//!   `corner_radius`, `panel_opacity`, `colors`, …);
//! - файлы темы (`theme.toml`, `shell.mss`, `apps/*.mss`), если тема меняется.
//!
//! [`Config::parse`](crate::Config::parse) накладывает примерку поверх
//! настроек, поэтому её видят и композитор, и оболочка, и программы, которые
//! перечитывают конфиг через [`crate::live`]. Если процесса редактора уже нет
//! (упал), примерка не действует. Убрать примерку — [`clear`].

use std::path::{Path, PathBuf};

use crate::config::Appearance;
use crate::theme::Theme;

/// Каталог примерки (`$XDG_RUNTIME_DIR/synshell/theme-preview`).
pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNSHELL_THEME_PREVIEW_DIR") {
        return PathBuf::from(p);
    }
    crate::paths::runtime_dir().join("synshell/theme-preview")
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct PreviewFile {
    pid: u32,
    theme: String,
    appearance: toml::Table,
}

fn pid_alive(pid: u32) -> bool {
    pid != 0 && Path::new(&format!("/proc/{pid}")).exists()
}

/// Идёт ли сейчас примерка (файл есть, редактор жив).
pub fn active() -> bool {
    read().is_some()
}

fn read() -> Option<PreviewFile> {
    let text = std::fs::read_to_string(dir().join("preview.toml")).ok()?;
    let f: PreviewFile = toml::from_str(&text).ok()?;
    pid_alive(f.pid).then_some(f)
}

/// Наложить примерку на `[appearance]` (вызывается из `Config::parse`).
pub fn apply(a: &mut Appearance) {
    if std::env::var_os("SYNSHELL_NO_THEME_PREVIEW").is_some() {
        return;
    }
    let Some(f) = read() else { return };
    apply_file(a, &f, &dir());
}

fn apply_file(a: &mut Appearance, f: &PreviewFile, dir: &Path) {
    if !f.appearance.is_empty() {
        if let Ok(toml::Value::Table(mut cur)) = toml::Value::try_from(&*a) {
            for (k, v) in &f.appearance {
                cur.insert(k.clone(), v.clone());
            }
            match toml::Value::Table(cur).try_into::<Appearance>() {
                Ok(mut merged) => {
                    merged.resolved = a.resolved.take();
                    *a = merged;
                }
                Err(e) => tracing::warn!("примерка темы: {e}"),
            }
        }
    }
    if dir.join("theme.toml").is_file() {
        let id = if f.theme.trim().is_empty() { "preview" } else { f.theme.trim() };
        match Theme::load_dir(id, dir) {
            Ok(t) => {
                a.theme = id.to_string();
                a.resolved = Some(std::sync::Arc::new(t));
            }
            Err(e) => tracing::warn!("примерка темы: {e}"),
        }
    } else if f.appearance.contains_key("theme") {
        a.load_theme();
    }
}

/// Записать примерку: `theme` — имя темы, `files` — её файлы
/// (`theme.toml`, `shell.mss`, `apps/x.mss`; пусто — тема не меняется),
/// `appearance` — переопределения `[appearance]`. Запись атомарная: каталог
/// собирается рядом и подменяется переименованием.
pub fn write(theme: &str, files: &[(String, String)], appearance: &toml::Table) -> std::io::Result<()> {
    let target = dir();
    let parent = target.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&parent)?;
    let tmp = parent.join(format!(".theme-preview.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    for (name, text) in files {
        if name.contains("..") || name.starts_with('/') {
            continue;
        }
        let p = tmp.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, text)?;
    }
    let f = PreviewFile { pid: std::process::id(), theme: theme.to_string(), appearance: appearance.clone() };
    std::fs::write(tmp.join("preview.toml"), toml::to_string(&f).map_err(std::io::Error::other)?)?;
    let old = parent.join(format!(".theme-preview.old.{}", std::process::id()));
    if target.exists() {
        std::fs::rename(&target, &old)?;
    }
    std::fs::rename(&tmp, &target)?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Убрать примерку — все программы вернутся к сохранённым настройкам.
pub fn clear() {
    let _ = std::fs::remove_dir_all(dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_and_theme_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("theme.toml"), "name = \"Черновик\"\n[dark]\nbg = \"#101010\"\naccent = \"#ff0000\"\n").unwrap();
        std::fs::create_dir(d.path().join("apps")).unwrap();
        std::fs::write(d.path().join("apps/synpkg.mss"), ".x { color: red; }").unwrap();
        let mut over = toml::Table::new();
        over.insert("corner_radius".into(), toml::Value::Float(3.0));
        let f = PreviewFile { pid: std::process::id(), theme: "draft".into(), appearance: over };
        let mut a = Appearance::default();
        apply_file(&mut a, &f, d.path());
        assert_eq!(a.corner_radius, 3.0);
        assert_eq!(a.theme, "draft");
        assert_eq!(a.palette().bg.hex(), "#101010");
        assert!(a.theme_app_mss("synpkg").contains(".x"));
    }

    #[test]
    fn dead_editor_ignored() {
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX - 1));
    }
}
