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

    #[test]
    fn relocate_paths() {
        let mut p = Project::new("П");
        p.media.push(crate::doc::Media { id: 1, path: "/home/u/Videos/Новый проект/generated/r/Герой.png".into(), kind: crate::doc::MediaKind::Image, duration: 0.0, width: 1, height: 1, fps: 0.0, has_audio: false, transcript: None, index: None });
        p.media.push(crate::doc::Media { id: 2, path: "/home/u/Videos/Новый проект 2/x.png".into(), kind: crate::doc::MediaKind::Image, duration: 0.0, width: 1, height: 1, fps: 0.0, has_audio: false, transcript: None, index: None });
        relocate(&mut p, Path::new("/home/u/Videos/Новый проект"), Path::new("/home/u/Videos/Суп")).unwrap();
        assert_eq!(p.media[0].path, std::path::PathBuf::from("/home/u/Videos/Суп/generated/r/Герой.png"));
        // соседняя папка с тем же началом имени — не трогать
        assert_eq!(p.media[1].path, std::path::PathBuf::from("/home/u/Videos/Новый проект 2/x.png"));
        assert_eq!(project_folder(Path::new("/a/Суп/Суп.syn")), Some(std::path::PathBuf::from("/a/Суп")));
        assert_eq!(project_folder(Path::new("/a/b/Суп.syn")), None);
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

/// Каталог, где создаются папки проектов (`~/Videos`).
pub fn projects_dir() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Videos")
}

/// Прежний общий каталог проектов (`~/Videos/syn-video-editor`) — старые проекты.
pub fn legacy_projects_dir() -> std::path::PathBuf {
    projects_dir().join("syn-video-editor")
}

/// Имя нового проекта, пока его не переименуют.
pub const NEW_PROJECT: &str = "Новый проект";

/// Файл нового проекта — в своей папке: `~/Videos/Новый проект/Новый проект.syn`
/// (`Новый проект 2`, … если занято).
pub fn new_project_path() -> std::path::PathBuf {
    let dir = projects_dir();
    let mut k = 1;
    loop {
        let name = if k == 1 { NEW_PROJECT.to_string() } else { format!("{NEW_PROJECT} {k}") };
        let folder = dir.join(&name);
        if !folder.exists() {
            return folder.join(format!("{name}.{EXT}"));
        }
        k += 1;
    }
}

/// Папка проекта: файл лежит в папке своего имени (`X/X.syn`).
pub fn project_folder(project: &Path) -> Option<std::path::PathBuf> {
    // относительный путь («-p Имя.syn» из папки проекта) — от текущего каталога
    let abs;
    let project = if project.is_relative() {
        abs = std::env::current_dir().ok()?.join(project);
        abs.as_path()
    } else {
        project
    };
    let dir = project.parent()?;
    let stem = project.file_stem()?;
    (dir.file_name() == Some(stem)).then(|| dir.to_path_buf())
}

/// Создан программой и ещё не переименован («Новый проект…», старые «Проект ДАТА»).
pub fn is_auto_named(project: &Path) -> bool {
    project.file_stem().is_some_and(|n| {
        let n = n.to_string_lossy();
        n.starts_with(NEW_PROJECT) || n.starts_with("Проект ")
    })
}

/// Где лежат все файлы проекта (генерация, эталоны, озвучка, документы):
/// папка проекта, а у старых проектов без своей папки — `<имя>.media/` рядом.
pub fn media_dir(project: &Path) -> std::path::PathBuf {
    if let Some(d) = project_folder(project) {
        return d;
    }
    let stem = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "проект".into());
    project.parent().unwrap_or(Path::new(".")).join(format!("{stem}.media"))
}

/// Переписать в проекте пути, начинающиеся с `from`, на `to` (после переноса
/// или переименования папки проекта).
pub fn relocate(project: &mut Project, from: &Path, to: &Path) -> Result<(), String> {
    let mut v = serde_json::to_value(&*project).map_err(|e| e.to_string())?;
    let (f, t) = (from.display().to_string(), to.display().to_string());
    fn walk(v: &mut serde_json::Value, f: &str, t: &str) {
        match v {
            serde_json::Value::String(s) => {
                if s == f {
                    *s = t.to_string();
                } else if let Some(rest) = s.strip_prefix(f).filter(|r| r.starts_with('/')) {
                    *s = format!("{t}{rest}");
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|x| walk(x, f, t)),
            serde_json::Value::Object(o) => o.values_mut().for_each(|x| walk(x, f, t)),
            _ => {}
        }
    }
    walk(&mut v, &f, &t);
    *project = serde_json::from_value(v).map_err(|e| e.to_string())?;
    Ok(())
}
