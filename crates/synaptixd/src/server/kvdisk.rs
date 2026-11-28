//! Головы промптов на диске — третий уровень парковки KV (карта → RAM → диск).
//!
//! Голова роли (или общая голова проекта) — состояние кэша модели на конце
//! головы промпта ([`synaptix::facade::llm::KvHead`]). Вытесненная из RAM, она
//! ложится сюда и поднимается в новую сессию, когда её голова снова нужна:
//! это дешевле, чем префиллить 20–40 тыс. токенов заново.
//!
//! Раскладка: `<корень>/<отпечаток модели>/<ключ>.kvh`. Отпечаток — путь,
//! размер и время изменения файла модели: другие веса — другой каталог.
//! Потолок — общий на все модели; при записи вытесняются файлы, к которым
//! дольше всего не обращались (время изменения обновляется при чтении).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use synaptix::facade::llm::KvHead;

/// Потолок диска под головы (`kv_park_disk_mb` конфига, 0 — диск не используется).
pub static DISK_BYTES: AtomicU64 = AtomicU64::new(32 << 30);

/// Корень хранилища (`SYNAPTIXD_KV_DIR` — для проверок копией службы).
pub fn root() -> PathBuf {
    std::env::var_os("SYNAPTIXD_KV_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/cache/synaptix/kv"))
}

/// Отпечаток модели: путь, размер и время изменения её файла.
pub fn model_fingerprint(path: &Path) -> String {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.to_string_lossy().bytes().chain(size.to_le_bytes()).chain(mtime.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn file(root: &Path, model: &str, key: &str) -> PathBuf {
    // Ключ — `prefix:<хэш>:<токенов>`: в имени файла двоеточия ни к чему.
    root.join(model).join(format!("{}.kvh", key.replace(':', "_")))
}

/// Записать голову (через временный файл — читатель не увидит половину) и
/// ужать хранилище под потолок. Ошибки — в журнал: диск — лишь запас.
pub fn save(root: &Path, model: &str, key: &str, head: &KvHead) {
    let budget = DISK_BYTES.load(Relaxed);
    if budget == 0 {
        return;
    }
    let path = file(root, model, key);
    let bytes = head.to_bytes();
    if bytes.len() as u64 > budget {
        return;
    }
    let tmp = path.with_extension("kvh.tmp");
    let res = std::fs::create_dir_all(path.parent().expect("каталог модели"))
        .and_then(|_| std::fs::write(&tmp, &bytes))
        .and_then(|_| std::fs::rename(&tmp, &path));
    match res {
        Ok(()) => tracing::info!("kv: голова «{key}» ({} токенов, {} МБ) — на диск", head.tokens().len(), bytes.len() >> 20),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            tracing::warn!("kv: голова «{key}» не записана на диск ({e})");
            return;
        }
    }
    trim(root, budget, Some(&path));
}

/// Поднять голову с диска; `ids` — токены, которыми она обязана быть
/// (хэш ключа мог совпасть у разных голов). Время доступа обновляется.
pub fn load(root: &Path, model: &str, key: &str, ids: &[u32]) -> Option<KvHead> {
    let path = file(root, model, key);
    let bytes = std::fs::read(&path).ok()?;
    match KvHead::from_bytes(&bytes) {
        Ok(h) if h.tokens() == ids => {
            if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
            Some(h)
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("kv: голова {} не читается ({e}) — удалена", path.display());
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// Убрать голову (не легла в модель — дальше бесполезна).
pub fn forget(root: &Path, model: &str, key: &str) {
    let _ = std::fs::remove_file(file(root, model, key));
}

/// Самые давние файлы — вон, пока всё не уложится в `budget`; `keep` не трогается.
pub fn trim(root: &Path, budget: u64, keep: Option<&Path>) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for dir in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "kvh") {
                if let Ok(m) = f.metadata() {
                    files.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), p));
                }
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= budget {
            break;
        }
        if Some(p.as_path()) == keep {
            continue;
        }
        if std::fs::remove_file(&p).is_ok() {
            tracing::info!("kv: голова {} вытеснена с диска", p.display());
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, len: usize, age_s: u64) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; len]).unwrap();
        let f = std::fs::File::options().append(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_s)).unwrap();
    }

    #[test]
    fn trim_drops_oldest_first_and_keeps_fresh() {
        let root = std::env::temp_dir().join(format!("synaptixd-kvdisk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b, c) = (root.join("m1/a.kvh"), root.join("m2/b.kvh"), root.join("m1/c.kvh"));
        touch(&a, 100, 300);
        touch(&b, 100, 200);
        touch(&c, 100, 100);
        // Не .kvh — не трогается и не считается.
        touch(&root.join("m1/x.tmp"), 1000, 999);
        trim(&root, 250, None);
        assert!(!a.exists() && b.exists() && c.exists());
        // Только что записанный не вытесняется, даже если он самый давний.
        touch(&a, 100, 900);
        trim(&root, 150, Some(&a));
        assert!(a.exists() && !b.exists() && !c.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn key_to_file_name() {
        assert_eq!(file(Path::new("/r"), "m", "prefix:00ff:42"), PathBuf::from("/r/m/prefix_00ff_42.kvh"));
        assert_ne!(model_fingerprint(Path::new("/нет/такого")), model_fingerprint(Path::new("/нет/другого")));
    }
}
