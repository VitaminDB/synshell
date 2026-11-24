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
}
