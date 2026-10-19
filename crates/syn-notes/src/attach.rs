//! Вложения чата. Ассистент synshell передаёт файлы путями; ссылки `blob:`
//! на страницах проектов synthos указывают в его общий CAS — читаем оттуда.

use std::path::{Path, PathBuf};

pub mod blobs {
    use super::*;

    /// Каталог блобов чатов synthos.
    pub fn blobs_dir() -> PathBuf {
        synshell_common::paths::home().join(".config/synthos/blobs")
    }

    pub fn blob_path(sha: &str, ext: &str) -> PathBuf {
        if ext.is_empty() {
            blobs_dir().join(sha)
        } else {
            blobs_dir().join(format!("{sha}.{ext}"))
        }
    }
}

/// Вложение, найденное по ссылке агента.
pub struct Attachment {
    pub sha256: String,
    pub ext: String,
    pub original_name: String,
}

static FINDER: std::sync::OnceLock<fn(&str) -> Option<Attachment>> = std::sync::OnceLock::new();

/// Поиск вложений чата программы-хозяина (synthos: имя, sha, `last`).
pub fn set_finder(f: fn(&str) -> Option<Attachment>) {
    let _ = FINDER.set(f);
}

/// `r` — путь к файлу или префикс sha256 блоба synthos.
pub fn find_attachment(r: &str) -> Option<Attachment> {
    if let Some(f) = FINDER.get() {
        return f(r);
    }
    let p = synshell_common::paths::expand_tilde(r);
    if p.is_file() {
        return None;
    }
    let r = r.trim().to_ascii_lowercase();
    if r.len() < 6 || !r.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let dir = std::fs::read_dir(blobs::blobs_dir()).ok()?;
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(&r) && e.path().is_file() {
            let path = e.path();
            let sha = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = path.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            return Some(Attachment { sha256: sha.clone(), ext: ext.clone(), original_name: name });
        }
    }
    let _ = Path::new("");
    None
}
