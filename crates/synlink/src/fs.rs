//! Файловые операции для другого устройства (сервер `Rpc::Fs`): путь —
//! абсолютный на этой машине, права — пользователя демона.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use crate::proto::{Attr, FileKind, FsReq, FsResp};

fn errno(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(libc::EIO)
}

pub fn attr_of(m: &std::fs::Metadata) -> Attr {
    let ft = m.file_type();
    let kind = if ft.is_symlink() {
        FileKind::Symlink
    } else if ft.is_dir() {
        FileKind::Dir
    } else if ft.is_file() {
        FileKind::File
    } else {
        FileKind::Other
    };
    Attr {
        kind,
        size: m.size(),
        mode: m.mode(),
        nlink: m.nlink() as u32,
        uid: m.uid(),
        gid: m.gid(),
        atime: (m.atime(), m.atime_nsec() as u32),
        mtime: (m.mtime(), m.mtime_nsec() as u32),
        ctime: (m.ctime(), m.ctime_nsec() as u32),
        blocks: m.blocks(),
    }
}

fn check(p: &str) -> Result<&Path, i32> {
    let path = Path::new(p);
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(libc::EINVAL);
    }
    Ok(path)
}

fn set_times(path: &Path, atime: Option<(i64, u32)>, mtime: Option<(i64, u32)>) -> std::io::Result<()> {
    let ts = |t: Option<(i64, u32)>| match t {
        Some((s, n)) => libc::timespec { tv_sec: s, tv_nsec: n as _ },
        None => libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_OMIT },
    };
    let times = [ts(atime), ts(mtime)];
    let c = CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn handle(req: FsReq) -> FsResp {
    match run(req) {
        Ok(r) => r,
        Err(e) => FsResp::Err(e),
    }
}

fn run(req: FsReq) -> Result<FsResp, i32> {
    let io = |e: std::io::Error| errno(&e);
    Ok(match req {
        FsReq::Stat(p) => FsResp::Attr(attr_of(&std::fs::symlink_metadata(check(&p)?).map_err(io)?)),
        FsReq::ReadDir(p) => {
            let dir = check(&p)?;
            let mut out = Vec::new();
            for e in std::fs::read_dir(dir).map_err(io)?.flatten() {
                if let Ok(m) = std::fs::symlink_metadata(e.path()) {
                    out.push((e.file_name().to_string_lossy().into_owned(), attr_of(&m)));
                }
            }
            FsResp::Entries(out)
        }
        FsReq::Read { path, offset, len } => {
            let f = std::fs::File::open(check(&path)?).map_err(io)?;
            let mut buf = vec![0u8; len.min(8 << 20) as usize];
            let mut n = 0;
            while n < buf.len() {
                let k = f.read_at(&mut buf[n..], offset + n as u64).map_err(io)?;
                if k == 0 {
                    break;
                }
                n += k;
            }
            buf.truncate(n);
            FsResp::Data(buf)
        }
        FsReq::Write { path, offset, data } => {
            let f = std::fs::OpenOptions::new().write(true).open(check(&path)?).map_err(io)?;
            f.write_all_at(&data, offset).map_err(io)?;
            FsResp::Written(data.len() as u32)
        }
        FsReq::Create { path, mode, exclusive } => {
            let p = check(&path)?;
            let mut o = std::fs::OpenOptions::new();
            o.write(true).mode(mode & 0o7777);
            if exclusive {
                o.create_new(true);
            } else {
                o.create(true);
            }
            o.open(p).map_err(io)?;
            FsResp::Attr(attr_of(&std::fs::symlink_metadata(p).map_err(io)?))
        }
        FsReq::Mkdir { path, mode } => {
            let p = check(&path)?;
            std::fs::create_dir(p).map_err(io)?;
            let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode & 0o7777));
            FsResp::Attr(attr_of(&std::fs::symlink_metadata(p).map_err(io)?))
        }
        FsReq::Unlink(p) => {
            std::fs::remove_file(check(&p)?).map_err(io)?;
            FsResp::Ok
        }
        FsReq::Rmdir(p) => {
            std::fs::remove_dir(check(&p)?).map_err(io)?;
            FsResp::Ok
        }
        FsReq::Rename { from, to, noreplace } => {
            let (a, b) = (check(&from)?, check(&to)?);
            if noreplace && std::fs::symlink_metadata(b).is_ok() {
                return Err(libc::EEXIST);
            }
            std::fs::rename(a, b).map_err(io)?;
            FsResp::Ok
        }
        FsReq::SetAttr { path, size, mode, atime, mtime } => {
            let p = check(&path)?;
            if let Some(sz) = size {
                std::fs::OpenOptions::new().write(true).open(p).and_then(|f| f.set_len(sz)).map_err(io)?;
            }
            if let Some(m) = mode {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(m & 0o7777)).map_err(io)?;
            }
            if atime.is_some() || mtime.is_some() {
                set_times(p, atime, mtime).map_err(io)?;
            }
            FsResp::Attr(attr_of(&std::fs::symlink_metadata(p).map_err(io)?))
        }
        FsReq::ReadLink(p) => FsResp::Target(std::fs::read_link(check(&p)?).map_err(io)?.to_string_lossy().into_owned()),
        FsReq::Symlink { target, path } => {
            let p = check(&path)?;
            std::os::unix::fs::symlink(&target, p).map_err(io)?;
            FsResp::Attr(attr_of(&std::fs::symlink_metadata(p).map_err(io)?))
        }
        FsReq::StatFs(p) => {
            let c = CString::new(check(&p)?.as_os_str().as_bytes()).map_err(|_| libc::EINVAL)?;
            let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
                return Err(errno(&std::io::Error::last_os_error()));
            }
            FsResp::StatFs {
                blocks: s.f_blocks,
                bfree: s.f_bfree,
                bavail: s.f_bavail,
                files: s.f_files,
                ffree: s.f_ffree,
                bsize: s.f_bsize as u32,
                namelen: s.f_namemax as u32,
            }
        }
    })
}
