use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use synaptixd::ModelKind;

pub const MODELS: &str = "models";
pub const BLOBS: &str = "blobs";
pub const MARK: &str = "library.toml";

pub fn models_dir(lib: &Path) -> PathBuf {
    lib.join(MODELS)
}

pub fn blobs_dir(lib: &Path) -> PathBuf {
    lib.join(BLOBS)
}

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

pub fn ensure(lib: &Path, name: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(models_dir(lib))?;
    std::fs::create_dir_all(blobs_dir(lib))?;
    let mark = lib.join(MARK);
    if !mark.exists() {
        std::fs::write(mark, format!("name = {:?}\n", name))?;
    }
    Ok(())
}

pub fn fingerprint(path: &Path) -> anyhow::Result<String> {
    use sha2::Digest;
    const CHUNK: u64 = 4 << 20;
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut h = sha2::Sha256::new();
    h.update(len.to_le_bytes());
    let mut buf = vec![0u8; CHUNK as usize];
    let mut offsets: Vec<u64> = vec![0];
    if len > CHUNK {
        for i in 1..=6u64 {
            offsets.push(len / 7 * i);
        }
        offsets.push(len.saturating_sub(CHUNK));
    }
    for off in offsets {
        f.seek(SeekFrom::Start(off))?;
        let n = f.read(&mut buf)?;
        h.update(&buf[..n]);
    }
    let d = h.finalize();
    Ok(d.iter().take(16).map(|b| format!("{b:02x}")).collect())
}

fn same_fs(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

const FICLONE: libc::c_ulong = 0x4004_9409;

pub fn put_file(src: &Path, dst: &Path, progress: &dyn Fn(u64)) -> anyhow::Result<bool> {
    if dst.exists() {
        return Ok(false);
    }
    let tmp = dst.with_extension("part");
    let _ = std::fs::remove_file(&tmp);
    let parent = dst.parent().unwrap_or(Path::new("/"));
    let cloned = if same_fs(src, parent) {
        let s = std::fs::File::open(src)?;
        let d = std::fs::File::create(&tmp)?;
        let ok = unsafe { libc::ioctl(d.as_raw_fd(), FICLONE as _, s.as_raw_fd()) } == 0;
        if !ok {
            drop(d);
            let _ = std::fs::remove_file(&tmp);
        }
        ok
    } else {
        false
    };
    if cloned {
        progress(std::fs::metadata(src)?.len());
    } else {
        let mut s = std::fs::File::open(src)?;
        let mut d = std::fs::File::create(&tmp)?;
        let mut buf = vec![0u8; 16 << 20];
        loop {
            let n = s.read(&mut buf)?;
            if n == 0 {
                break;
            }
            d.write_all(&buf[..n])?;
            progress(n as u64);
        }
        d.sync_all()?;
    }
    std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
    std::fs::rename(&tmp, dst)?;
    Ok(true)
}

pub fn kind_id(k: ModelKind) -> &'static str {
    k.id()
}

pub struct Component {
    pub role: String,
    pub blob: String,
}

pub fn write_model(lib: &Path, id: &str, title: &str, kind: ModelKind, arch: &str, description: &str, comps: &[Component]) -> anyhow::Result<()> {
    let dir = models_dir(lib).join(id);
    let tmp = models_dir(lib).join(format!(".{id}.new"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let mut toml = String::new();
    toml.push_str(&format!("id = {:?}\ntitle = {:?}\nkind = {:?}\narch = {:?}\ndescription = {:?}\n", id, title, kind_id(kind), arch, description));
    toml.push_str("\n[[variant]]\nfile = \"model.syn\"\ndevices = [\"cuda\", \"cpu\", \"npu\"]\n");
    let others: Vec<&Component> = comps.iter().filter(|c| c.role != "model").collect();
    if !others.is_empty() {
        toml.push_str("\n[components]\n");
        for c in &others {
            toml.push_str(&format!("{} = \"{}.syn\"\n", c.role, c.role));
        }
    }
    std::fs::write(tmp.join("model.toml"), toml)?;
    for c in comps {
        let link = tmp.join(format!("{}.syn", c.role));
        std::os::unix::fs::symlink(format!("../../{BLOBS}/{}.syn", c.blob), link)?;
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&tmp, &dir)?;
    Ok(())
}

pub fn model_blobs(model_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(model_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
                if let Ok(t) = std::fs::canonicalize(&p) {
                    out.push(t);
                }
            }
        }
    }
    out
}

pub fn gc(lib: &Path) -> u64 {
    let mut used: BTreeSet<PathBuf> = BTreeSet::new();
    if let Ok(rd) = std::fs::read_dir(models_dir(lib)) {
        for e in rd.flatten() {
            used.extend(model_blobs(&e.path()));
        }
    }
    let mut freed = 0;
    if let Ok(rd) = std::fs::read_dir(blobs_dir(lib)) {
        for e in rd.flatten() {
            let p = e.path();
            let canon = std::fs::canonicalize(&p).unwrap_or(p.clone());
            if !used.contains(&canon) {
                freed += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    freed
}

pub fn used_bytes(lib: &Path) -> u64 {
    std::fs::read_dir(blobs_dir(lib))
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

pub fn model_count(lib: &Path) -> u32 {
    std::fs::read_dir(models_dir(lib))
        .map(|rd| rd.flatten().filter(|e| e.path().join("model.toml").is_file()).count() as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_dedupe_gc() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        ensure(&lib, "test").unwrap();
        let src = dir.path().join("a.syn");
        std::fs::write(&src, vec![7u8; 10 << 20]).unwrap();
        let fp = fingerprint(&src).unwrap();
        let blob = blobs_dir(&lib).join(format!("{fp}.syn"));
        assert!(put_file(&src, &blob, &|_| {}).unwrap());
        assert!(!put_file(&src, &blob, &|_| {}).unwrap());
        let comps = [Component { role: "model".into(), blob: fp.clone() }, Component { role: "vae".into(), blob: fp.clone() }];
        write_model(&lib, "m1", "M1", ModelKind::Image, "sdxl", "", &comps).unwrap();
        write_model(&lib, "m2", "M2", ModelKind::Image, "sdxl", "", &comps[..1]).unwrap();
        assert_eq!(std::fs::read(models_dir(&lib).join("m1/vae.syn")).unwrap().len(), 10 << 20);
        assert!(std::fs::read_to_string(models_dir(&lib).join("m1/model.toml")).unwrap().contains("vae = \"vae.syn\""));
        assert_eq!(model_count(&lib), 2);
        std::fs::remove_dir_all(models_dir(&lib).join("m1")).unwrap();
        assert_eq!(gc(&lib), 0);
        std::fs::remove_dir_all(models_dir(&lib).join("m2")).unwrap();
        assert_eq!(gc(&lib), 10 << 20);
    }
}
