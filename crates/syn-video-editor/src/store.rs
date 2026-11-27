//! Файл проекта — `.syn` (synaptix-bundle) с назначением `video-project`:
//! внутри `project.json` (документ текстом). Прокси, миниатюры и кэш
//! индексации ложатся рядом в тот же контейнер (позже).

use std::path::Path;

use synaptix_bundle::{Bundle, BundleBuilder, FileTag};

use crate::doc::{Project, FORMAT_VERSION};

pub const PURPOSE: &str = "video-project";
pub const EXT: &str = "syn";
const DOC: &str = "project.json";

pub fn save(project: &Project, path: &Path) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    BundleBuilder::new(project.name.clone(), FORMAT_VERSION.to_string())
        .purpose(PURPOSE)
        .add_file_bytes(DOC, json, FileTag::Doc)
        .map_err(|e| e.to_string())?
        .write(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Project, String> {
    let b = Bundle::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.meta().purpose != PURPOSE {
        return Err(format!("{}: это не проект видеоредактора (назначение «{}»)", path.display(), b.meta().purpose));
    }
    let bytes = b.read_file(DOC).map_err(|e| format!("{}: {e}", path.display()))?;
    let p: Project = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if p.version > FORMAT_VERSION {
        return Err(format!("{}: проект из более новой версии редактора ({})", path.display(), p.version));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.syn");
        let mut p = Project::new("Проба");
        p.settings.fps = 60.0;
        save(&p, &path).unwrap();
        assert_eq!(load(&path).unwrap(), p);
        p.name = "Второй".into();
        save(&p, &path).unwrap();
        assert_eq!(load(&path).unwrap().name, "Второй");
    }
}

/// Каталог данных программы (`~/.local/share/syn-video-editor`).
pub fn data_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor")
}

/// Автосохранение проекта, у которого ещё нет файла.
pub fn autosave_path() -> std::path::PathBuf {
    data_dir().join("autosave.syn")
}

/// Последний открытый файл проекта (`None` — несохранённый, из автосохранения).
pub fn last_project() -> Option<std::path::PathBuf> {
    let s = std::fs::read_to_string(data_dir().join("last")).ok()?;
    let p = std::path::PathBuf::from(s.trim());
    p.is_file().then_some(p)
}

pub fn remember(path: Option<&Path>) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(data_dir().join("last"), path.map(|p| p.display().to_string()).unwrap_or_default());
    if let Some(p) = path {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut v = recent();
        v.retain(|(q, _)| q != p);
        v.insert(0, (p.to_path_buf(), now));
        v.truncate(RECENT_MAX);
        write_recent(&v);
    }
}

const RECENT_MAX: usize = 60;

/// Недавние проекты: (файл, когда открыт, с от эпохи), новые первыми.
pub fn recent() -> Vec<(std::path::PathBuf, u64)> {
    let Ok(s) = std::fs::read_to_string(data_dir().join("recent.json")) else { return Vec::new() };
    let v: Vec<(String, u64)> = serde_json::from_str(&s).unwrap_or_default();
    v.into_iter().map(|(p, t)| (std::path::PathBuf::from(p), t)).collect()
}

fn write_recent(v: &[(std::path::PathBuf, u64)]) {
    let list: Vec<(String, u64)> = v.iter().map(|(p, t)| (p.display().to_string(), *t)).collect();
    let _ = std::fs::write(data_dir().join("recent.json"), serde_json::to_string_pretty(&list).unwrap_or_default());
}

/// Убрать проект из списка недавних (файл не трогается).
pub fn forget(path: &Path) {
    let mut v = recent();
    v.retain(|(q, _)| q != path);
    write_recent(&v);
    // проект из папки проектов показывается и без записи — помним, что убран
    let mut hidden = hidden();
    if !hidden.iter().any(|h| h == path) {
        hidden.push(path.to_path_buf());
        let list: Vec<String> = hidden.iter().map(|p| p.display().to_string()).collect();
        let _ = std::fs::write(data_dir().join("recent-hidden.json"), serde_json::to_string(&list).unwrap_or_default());
    }
}

/// Проекты, убранные из списка вручную (пока их снова не откроют).
pub fn hidden() -> Vec<std::path::PathBuf> {
    let Ok(s) = std::fs::read_to_string(data_dir().join("recent-hidden.json")) else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(&s).unwrap_or_default().into_iter().map(std::path::PathBuf::from).collect()
}

/// Каталог проектов по умолчанию (`~/Videos/syn-video-editor`).
pub fn projects_dir() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Videos/syn-video-editor")
}

/// Файл нового проекта: «Проект ГГГГ-ММ-ДД ЧЧ-ММ.syn» (с номером, если занят).
pub fn new_project_path() -> std::path::PathBuf {
    let stamp = std::process::Command::new("date").arg("+%Y-%m-%d %H-%M").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "новый".into());
    let dir = projects_dir();
    let mut p = dir.join(format!("Проект {stamp}.syn"));
    let mut k = 2;
    while p.exists() {
        p = dir.join(format!("Проект {stamp} ({k}).syn"));
        k += 1;
    }
    p
}

/// Папка файлов проекта рядом с ним (`<имя>.media/`): всё, что программа
/// создаёт для проекта (генерация, эталоны, отчёты прогонов), — в одном месте.
pub fn media_dir(project: &Path) -> std::path::PathBuf {
    let stem = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "проект".into());
    project.parent().unwrap_or(Path::new(".")).join(format!("{stem}.media"))
}
