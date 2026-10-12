use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::registry::{Manifest, Variant};

pub const CATEGORIES: &[&str] = &["llm", "image", "video", "music", "tts", "asr", "embed", "pronunciation"];

pub fn statvfs(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let bs = st.f_frsize as u64;
    Some((st.f_blocks as u64 * bs, st.f_bavail as u64 * bs))
}

pub fn ensure(lib: &Path) -> anyhow::Result<()> {
    for c in CATEGORIES {
        std::fs::create_dir_all(lib.join(c))?;
    }
    Ok(())
}

fn walk(dir: &Path, depth: usize, syn: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        if p.is_dir() {
            if p.join("model.toml").is_file() {
                dirs.push(p);
            } else if depth < 4 {
                walk(&p, depth + 1, syn, dirs);
            }
        } else if name.ends_with(".syn") && p.is_file() {
            syn.push(p);
        }
    }
}

pub fn scan(lib: &Path) -> Vec<Manifest> {
    let (mut syn, mut dirs) = (Vec::new(), Vec::new());
    walk(lib, 0, &mut syn, &mut dirs);
    let mut out: Vec<Manifest> = Vec::new();
    for d in dirs {
        match Manifest::load(&d) {
            Ok(mut m) => {
                m.library = Some(lib.to_path_buf());
                out.push(m);
            }
            Err(e) => tracing::warn!("{}: {e}", d.display()),
        }
    }
    for p in synaptix::facade::catalog::plan_files(&syn).models {
        let main = p.components.iter().find(|(r, _)| r == "model").map(|(_, f)| f.clone());
        let Some(main) = main else { continue };
        let components: BTreeMap<String, String> =
            p.components.iter().filter(|(r, _)| r != "model").map(|(r, f)| (r.clone(), f.to_string_lossy().into_owned())).collect();
        let kind = crate::defaults::kind_of(p.task);
        // NPU — речь и эмбеддинги: языковые модели на нём на порядки медленнее
        // (граф на каждый токен), картинки, видео и музыка не проверены.
        let mut devices = vec![synaptixd::DeviceKind::Cuda, synaptixd::DeviceKind::Cpu];
        if matches!(kind, synaptixd::ModelKind::Tts | synaptixd::ModelKind::Asr | synaptixd::ModelKind::Ctc | synaptixd::ModelKind::Embed) {
            devices.push(synaptixd::DeviceKind::Npu);
        }
        out.push(Manifest {
            id: p.id,
            title: p.title,
            kind,
            arch: p.arch,
            languages: Vec::new(),
            license: String::new(),
            description: p.description,
            package: None,
            variants: vec![Variant { file: main.to_string_lossy().into_owned(), devices, precision: String::new() }],
            resources: Default::default(),
            defaults: Default::default(),
            components,
            dir: main.parent().map(Path::to_path_buf).unwrap_or_default(),
            library: Some(lib.to_path_buf()),
        });
    }
    out
}

pub fn files_of(m: &Manifest) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = m.variants.iter().map(|x| m.path(x)).chain(m.components.values().map(|f| m.dir.join(f))).collect();
    v.sort();
    v.dedup();
    v
}

pub fn is_own_dir(m: &Manifest) -> bool {
    m.dir.join("model.toml").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_dirs_and_skips_private() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        ensure(lib).unwrap();
        let m = lib.join("pronunciation/w2v2-kk");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("model.toml"), "id = \"w2v2-kk\"\ntitle = \"W\"\nkind = \"ctc\"\narch = \"wav2vec2\"\n[[variant]]\nfile = \"model.syn\"\ndevices = [\"cpu\"]\n").unwrap();
        std::fs::write(m.join("model.syn"), b"x").unwrap();
        std::fs::create_dir_all(lib.join("_old/other")).unwrap();
        std::fs::write(lib.join("_old/other/model.toml"), "broken").unwrap();
        let found = scan(lib);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "w2v2-kk");
        assert!(is_own_dir(&found[0]));
        assert_eq!(files_of(&found[0]), vec![m.join("model.syn")]);
    }
}
