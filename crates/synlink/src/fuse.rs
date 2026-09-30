//! Файлы устройства как каталог: FUSE в `$XDG_RUNTIME_DIR/synlink/<имя>`
//! (Проводник показывает его в «Устройствах»). Корень — `/` устройства;
//! оболочка открывает домашний каталог внутри.
//!
//! Каждая операция — вызов `Rpc::Fs` по текущему соединению (после смены
//! Wi-Fi на USB монтирование живёт дальше). Атрибуты из `readdir`
//! кэшируются на секунду — листание больших каталогов не дёргает сеть на
//! каждый файл.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};

use crate::daemon::D;
use crate::proto::{Attr, FileKind, FsReq, FsResp, Reply, Rpc};

const TTL: Duration = Duration::from_secs(1);

static SESSIONS: Mutex<Option<HashMap<String, fuser::BackgroundSession>>> = Mutex::new(None);

struct RemoteFs {
    d: D,
    id: String,
    rt: tokio::runtime::Handle,
    inodes: HashMap<u64, String>,
    paths: HashMap<String, u64>,
    next: u64,
    cache: HashMap<String, (Attr, Instant)>,
    local_uid: u32,
    local_gid: u32,
    remote_uid: u32,
}

impl RemoteFs {
    fn call(&self, req: FsReq) -> Result<FsResp, i32> {
        let Some((conn, _)) = self.d.conn(&self.id) else { return Err(libc::ENOTCONN) };
        let res = self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(30), crate::rpc::call(&conn, &Rpc::Fs(req))).await
        });
        match res {
            Ok(Ok(Reply::Fs(FsResp::Err(e)))) => Err(e),
            Ok(Ok(Reply::Fs(r))) => Ok(r),
            Ok(Ok(_)) => Err(libc::EIO),
            Ok(Err(_)) => Err(libc::EIO),
            Err(_) => Err(libc::ETIMEDOUT),
        }
    }

    fn path(&self, ino: u64) -> Option<String> {
        self.inodes.get(&ino).cloned()
    }

    fn child(&self, parent: u64, name: &OsStr) -> Option<String> {
        let p = self.path(parent)?;
        let name = name.to_string_lossy();
        Some(if p == "/" { format!("/{name}") } else { format!("{p}/{name}") })
    }

    fn ino(&mut self, path: &str) -> u64 {
        if let Some(i) = self.paths.get(path) {
            return *i;
        }
        let i = self.next;
        self.next += 1;
        self.inodes.insert(i, path.to_string());
        self.paths.insert(path.to_string(), i);
        i
    }

    fn forget_path(&mut self, path: &str) {
        self.cache.remove(path);
    }

    fn rename_paths(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        let moved: Vec<(String, u64)> =
            self.paths.iter().filter(|(p, _)| *p == from || p.starts_with(&prefix)).map(|(p, i)| (p.clone(), *i)).collect();
        for (p, i) in moved {
            let np = format!("{to}{}", &p[from.len()..]);
            self.paths.remove(&p);
            self.paths.insert(np.clone(), i);
            self.inodes.insert(i, np);
            self.cache.remove(&p);
        }
        self.cache.remove(to);
    }

    fn stat(&mut self, path: &str) -> Result<Attr, i32> {
        if let Some((a, t)) = self.cache.get(path) {
            if t.elapsed() < TTL {
                return Ok(*a);
            }
        }
        match self.call(FsReq::Stat(path.to_string()))? {
            FsResp::Attr(a) => {
                self.cache.insert(path.to_string(), (a, Instant::now()));
                Ok(a)
            }
            _ => Err(libc::EIO),
        }
    }

    fn fattr(&mut self, path: &str, a: &Attr) -> FileAttr {
        let ino = self.ino(path);
        let t = |(s, n): (i64, u32)| {
            if s >= 0 {
                UNIX_EPOCH + Duration::new(s as u64, n)
            } else {
                UNIX_EPOCH
            }
        };
        let own = a.uid == self.remote_uid;
        FileAttr {
            ino,
            size: a.size,
            blocks: a.blocks,
            atime: t(a.atime),
            mtime: t(a.mtime),
            ctime: t(a.ctime),
            crtime: t(a.ctime),
            kind: match a.kind {
                FileKind::Dir => FileType::Directory,
                FileKind::Symlink => FileType::Symlink,
                FileKind::File => FileType::RegularFile,
                FileKind::Other => FileType::NamedPipe,
            },
            perm: (a.mode & 0o7777) as u16,
            nlink: a.nlink,
            uid: if own { self.local_uid } else { a.uid },
            gid: if own { self.local_gid } else { a.gid },
            rdev: 0,
            blksize: 65536,
            flags: 0,
        }
    }

    fn entry(&mut self, path: &str, a: Attr, reply: ReplyEntry) {
        self.cache.insert(path.to_string(), (a, Instant::now()));
        let fa = self.fattr(path, &a);
        reply.entry(&TTL, &fa, 0);
    }
}

fn tm(t: Option<TimeOrNow>) -> Option<(i64, u32)> {
    t.map(|t| {
        let st = match t {
            TimeOrNow::SpecificTime(s) => s,
            TimeOrNow::Now => SystemTime::now(),
        };
        let d = st.duration_since(UNIX_EPOCH).unwrap_or_default();
        (d.as_secs() as i64, d.subsec_nanos())
    })
}

impl Filesystem for RemoteFs {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(p) = self.child(parent, name) else { return reply.error(libc::ENOENT) };
        match self.stat(&p) {
            Ok(a) => self.entry(&p, a, reply),
            Err(e) => reply.error(e),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        match self.stat(&p) {
            Ok(a) => {
                let fa = self.fattr(&p, &a);
                reply.attr(&TTL, &fa)
            }
            Err(e) => reply.error(e),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::SetAttr { path: p.clone(), size, mode, atime: tm(atime), mtime: tm(mtime) }) {
            Ok(FsResp::Attr(a)) => {
                self.cache.insert(p.clone(), (a, Instant::now()));
                let fa = self.fattr(&p, &a);
                reply.attr(&TTL, &fa)
            }
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::ReadLink(p)) {
            Ok(FsResp::Target(t)) => reply.data(t.as_bytes()),
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn mkdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, mode: u32, umask: u32, reply: ReplyEntry) {
        let Some(p) = self.child(parent, name) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::Mkdir { path: p.clone(), mode: mode & !umask }) {
            Ok(FsResp::Attr(a)) => self.entry(&p, a, reply),
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let Some(p) = self.child(parent, name) else { return reply.error(libc::ENOENT) };
        self.forget_path(&p);
        match self.call(FsReq::Unlink(p)) {
            Ok(_) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let Some(p) = self.child(parent, name) else { return reply.error(libc::ENOENT) };
        self.forget_path(&p);
        match self.call(FsReq::Rmdir(p)) {
            Ok(_) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn symlink(&mut self, _req: &Request<'_>, parent: u64, link_name: &OsStr, target: &Path, reply: ReplyEntry) {
        let Some(p) = self.child(parent, link_name) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::Symlink { target: target.to_string_lossy().into_owned(), path: p.clone() }) {
            Ok(FsResp::Attr(a)) => self.entry(&p, a, reply),
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        let (Some(a), Some(b)) = (self.child(parent, name), self.child(newparent, newname)) else {
            return reply.error(libc::ENOENT);
        };
        let noreplace = flags & libc::RENAME_NOREPLACE != 0;
        match self.call(FsReq::Rename { from: a.clone(), to: b.clone(), noreplace }) {
            Ok(_) => {
                self.rename_paths(&a, &b);
                reply.ok()
            }
            Err(e) => reply.error(e),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        if flags & libc::O_TRUNC != 0 {
            if let Err(e) = self.call(FsReq::SetAttr { path: p.clone(), size: Some(0), mode: None, atime: None, mtime: None }) {
                return reply.error(e);
            }
            self.forget_path(&p);
        }
        reply.opened(0, 0)
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::Read { path: p, offset: offset.max(0) as u64, len: size }) {
            Ok(FsResp::Data(d)) => reply.data(&d),
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyWrite,
    ) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        self.forget_path(&p);
        match self.call(FsReq::Write { path: p, offset: offset.max(0) as u64, data: data.to_vec() }) {
            Ok(FsResp::Written(n)) => reply.written(n),
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }

    fn flush(&mut self, _req: &Request<'_>, _ino: u64, _fh: u64, _lock: u64, reply: ReplyEmpty) {
        reply.ok()
    }

    fn fsync(&mut self, _req: &Request<'_>, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        reply.ok()
    }

    fn readdir(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        let Some(p) = self.path(ino) else { return reply.error(libc::ENOENT) };
        let entries = match self.call(FsReq::ReadDir(p.clone())) {
            Ok(FsResp::Entries(e)) => e,
            Ok(_) => return reply.error(libc::EIO),
            Err(e) => return reply.error(e),
        };
        let parent = if p == "/" { "/".to_string() } else { Path::new(&p).parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or("/".into()) };
        let mut list: Vec<(u64, FileType, String)> = vec![(ino, FileType::Directory, ".".into())];
        let pino = self.ino(&parent);
        list.push((pino, FileType::Directory, "..".into()));
        let now = Instant::now();
        for (name, a) in entries {
            let cp = if p == "/" { format!("/{name}") } else { format!("{p}/{name}") };
            self.cache.insert(cp.clone(), (a, now));
            let ft = match a.kind {
                FileKind::Dir => FileType::Directory,
                FileKind::Symlink => FileType::Symlink,
                FileKind::File => FileType::RegularFile,
                FileKind::Other => FileType::NamedPipe,
            };
            let i = self.ino(&cp);
            list.push((i, ft, name));
        }
        for (i, (ino, ft, name)) in list.into_iter().enumerate().skip(offset.max(0) as usize) {
            if reply.add(ino, (i + 1) as i64, ft, name) {
                break;
            }
        }
        reply.ok()
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        match self.call(FsReq::StatFs("/".into())) {
            Ok(FsResp::StatFs { blocks, bfree, bavail, files, ffree, bsize, namelen }) => {
                reply.statfs(blocks, bfree, bavail, files, ffree, bsize, namelen, bsize)
            }
            _ => reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096),
        }
    }

    fn access(&mut self, _req: &Request<'_>, _ino: u64, _mask: i32, reply: ReplyEmpty) {
        reply.ok()
    }

    fn create(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, mode: u32, umask: u32, flags: i32, reply: ReplyCreate) {
        let Some(p) = self.child(parent, name) else { return reply.error(libc::ENOENT) };
        match self.call(FsReq::Create { path: p.clone(), mode: mode & !umask, exclusive: flags & libc::O_EXCL != 0 }) {
            Ok(FsResp::Attr(a)) => {
                self.cache.insert(p.clone(), (a, Instant::now()));
                let fa = self.fattr(&p, &a);
                reply.created(&TTL, &fa, 0, 0, 0)
            }
            Ok(_) => reply.error(libc::EIO),
            Err(e) => reply.error(e),
        }
    }
}

/// Имя каталога монтирования: имя устройства без `/`.
fn dir_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "device".into()
    } else {
        s
    }
}

fn force_unmount(p: &Path) {
    let _ = std::process::Command::new("fusermount3")
        .arg("-uz")
        .arg(p)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub async fn mount(d: &D, id: &str) -> Result<String> {
    if let Some(m) = d.mount_of(id) {
        return Ok(m);
    }
    let t = d.trusted(id).context("не спарено")?;
    let root = synshell_common::link::mount_root();
    std::fs::create_dir_all(&root)?;
    let mp: PathBuf = root.join(dir_name(&t.name));
    // Осталось от прошлого запуска (демон упал) — снять.
    force_unmount(&mp);
    std::fs::create_dir_all(&mp)?;
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let fs = RemoteFs {
        d: d.clone(),
        id: id.to_string(),
        rt: tokio::runtime::Handle::current(),
        inodes: HashMap::from([(1, "/".to_string())]),
        paths: HashMap::from([("/".to_string(), 1)]),
        next: 2,
        cache: HashMap::new(),
        local_uid: uid,
        local_gid: gid,
        remote_uid: t.uid,
    };
    let opts = [
        MountOption::FSName(format!("synlink:{}", t.name)),
        MountOption::Subtype("synlink".into()),
        MountOption::NoAtime,
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    let mp2 = mp.clone();
    let sess = tokio::task::spawn_blocking(move || fuser::spawn_mount2(fs, &mp2, &opts)).await?.context("fusermount3")?;
    SESSIONS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id.to_string(), sess);
    let path = mp.to_string_lossy().into_owned();
    tracing::info!(device = %t.name, %path, "файлы смонтированы");
    d.set_mount(id, Some(path.clone()));
    Ok(path)
}

pub fn unmount(d: &crate::daemon::Daemon, id: &str) {
    let sess = SESSIONS.lock().unwrap().as_mut().and_then(|m| m.remove(id));
    let path = d.mount_of(id);
    if sess.is_some() || path.is_some() {
        // Снять сразу (lazy), иначе зависшая операция держит сессию.
        if let Some(p) = &path {
            force_unmount(Path::new(p));
            let _ = std::fs::remove_dir(p);
        }
        drop(sess);
        d.set_mount(id, None);
    }
}

pub fn unmount_all() {
    let all = SESSIONS.lock().unwrap().take();
    if let Some(m) = all {
        for (_, s) in m {
            drop(s);
        }
    }
}
