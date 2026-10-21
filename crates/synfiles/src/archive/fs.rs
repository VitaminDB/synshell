//! Помощник `synfiles --archive-mount АРХИВ ПАПКА`: архив только для чтения
//! как FUSE-каталог. Оглавление читается один раз при запуске; файл
//! распаковывается в кэш (`~/.cache/synfiles/archives/<ключ>`) при первом
//! открытии и дальше читается оттуда. Помощник живёт отдельно от окна
//! (открытый из архива файл остаётся доступен и после закрытия Проводника)
//! и снимает себя, когда ни один файл и ни одна папка внутри не открыты
//! [`IDLE`].
//!
//! Зашифрованные архивы: пароль приходит строкой в stdin при запуске или
//! позже через управляющий сокет `<ключ>/ctl` (см. [`super::unlock`]).
//! Zip с паролем расшифровывает libarchive; 7z и rar с паролем (и 7z с
//! зашифрованным оглавлением) — программа `7z`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use fuser::{FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, Request};

use super::la::{self, Reader};

/// Сколько ждать без открытых файлов и обращений, прежде чем сняться.
const IDLE: Duration = Duration::from_secs(20);
/// Оглавление не меняется — ядро может помнить его долго.
const TTL: Duration = Duration::from_secs(3600);
/// Попутно сохранять в кэш небольшие файлы, мимо которых идёт распаковка
/// (сжатый tar читается только подряд: иначе миниатюры сотни картинок —
/// сотня проходов с начала архива).
const SIDE_CACHE_FILE: u64 = 4 << 20;
const SIDE_CACHE_TOTAL: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Dir,
    File,
    Link,
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    parent: u64,
    kind: Kind,
    perm: u32,
    size: u64,
    mtime: i64,
    children: Vec<u64>,
    /// Номер записи с данными (для жёстких ссылок — номер цели).
    entry: Option<usize>,
    link: Option<String>,
    encrypted: bool,
    /// Путь записи в архиве как есть (для распаковки через `7z`).
    apath: String,
}

struct Index {
    /// `ino - 1` → узел; корень — ino 1.
    nodes: Vec<Node>,
    names: HashMap<(u64, String), u64>,
    /// Номер записи → ino файла.
    by_entry: HashMap<usize, u64>,
    sequential: bool,
    /// Zip — его шифрование libarchive снимает сам; иначе зашифрованные
    /// записи распаковывает `7z`.
    zip: bool,
    /// Оглавление прочитано через `7z` (оглавление 7z зашифровано): все
    /// файлы распаковывает он.
    seven: bool,
}

impl Index {
    fn node(&self, ino: u64) -> Option<&Node> {
        self.nodes.get((ino as usize).wrapping_sub(1))
    }

    fn child(&mut self, parent: u64, name: &str, kind: Kind, mtime: i64) -> u64 {
        if let Some(&i) = self.names.get(&(parent, name.to_string())) {
            return i;
        }
        let ino = self.nodes.len() as u64 + 1;
        self.nodes.push(Node {
            name: name.to_string(),
            parent,
            kind,
            perm: if kind == Kind::Dir { 0o755 } else { 0o644 },
            size: 0,
            mtime,
            children: Vec::new(),
            entry: None,
            link: None,
            encrypted: false,
            apath: String::new(),
        });
        self.nodes[parent as usize - 1].children.push(ino);
        self.names.insert((parent, name.to_string()), ino);
        ino
    }

    /// Каталог по пути (промежуточные создаются).
    fn dirs(&mut self, parts: &[&str], mtime: i64) -> u64 {
        let mut cur = 1;
        for p in parts {
            cur = self.child(cur, p, Kind::Dir, mtime);
            if self.nodes[cur as usize - 1].kind != Kind::Dir {
                // Файл и каталог с одним именем — каталог побеждает.
                self.nodes[cur as usize - 1].kind = Kind::Dir;
            }
        }
        cur
    }

    fn new(mtime: i64) -> Index {
        Index {
            nodes: vec![Node {
                name: String::new(),
                parent: 1,
                kind: Kind::Dir,
                perm: 0o755,
                size: 0,
                mtime,
                children: Vec::new(),
                entry: None,
                link: None,
                encrypted: false,
                apath: String::new(),
            }],
            names: HashMap::new(),
            by_entry: HashMap::new(),
            sequential: false,
            zip: false,
            seven: false,
        }
    }

    /// Файлов, папок, байт распакованного, зашифрованных файлов.
    fn stats(&self) -> (usize, usize, u64, usize) {
        let mut v = (0, 0, 0, 0);
        for n in &self.nodes[1..] {
            match n.kind {
                Kind::Dir => v.1 += 1,
                Kind::File | Kind::Link => {
                    v.0 += 1;
                    v.2 += n.size;
                    v.3 += n.encrypted as usize;
                }
            }
        }
        v
    }

    fn lookup_path(&self, parts: &[&str]) -> Option<u64> {
        let mut cur = 1;
        for p in parts {
            cur = *self.names.get(&(cur, p.to_string()))?;
        }
        Some(cur)
    }
}

/// Путь записи — части без `.`, `..` и пустых (не выходить за корень).
fn split(path: &str) -> Vec<&str> {
    path.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").collect()
}

/// Имя единственного файла сжатого `отчёт.txt.gz` — `отчёт.txt`.
fn raw_name(archive: &Path) -> String {
    let n = archive.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match n.rfind('.') {
        Some(i) if i > 0 => {
            let base = &n[..i];
            let ext = n[i + 1..].to_ascii_lowercase();
            match ext.as_str() {
                "tgz" | "tbz2" | "txz" | "tzst" => format!("{base}.tar"),
                _ => base.to_string(),
            }
        }
        _ => format!("{n}.data"),
    }
}

struct Shared {
    archive: PathBuf,
    cache: PathBuf,
    index: Index,
    /// Распакованные файлы: ino → путь в кэше.
    ready: Mutex<HashMap<u64, PathBuf>>,
    side_bytes: AtomicU64,
    /// Читатель, оставленный на месте прошлой распаковки (дальше по архиву —
    /// продолжать с него, а не с начала).
    cursor: Mutex<Option<Reader>>,
    files: Mutex<HashMap<u64, Arc<File>>>,
    next_fh: AtomicU64,
    open: AtomicUsize,
    last: Mutex<Instant>,
    /// Пароль зашифрованных записей (проверенный).
    password: Mutex<Option<String>>,
    /// Зашифрованных файлов в архиве.
    encrypted: usize,
}

impl Shared {
    fn touch(&self) {
        *self.last.lock().unwrap() = Instant::now();
    }

    /// Распаковать файл в кэш (если ещё нет) и вернуть путь.
    fn extract(&self, ino: u64) -> Result<PathBuf, i32> {
        if let Some(p) = self.ready.lock().unwrap().get(&ino) {
            return Ok(p.clone());
        }
        let node = self.index.node(ino).ok_or(libc::ENOENT)?;
        let Some(want) = node.entry else {
            // Записи без данных (пустой файл из жёсткой ссылки в никуда).
            return self.store(ino, &mut |_| Ok(0));
        };
        let target = self.index.by_entry.get(&want).copied().unwrap_or(ino);
        let encrypted = node.encrypted || self.index.node(target).is_some_and(|n| n.encrypted);
        // Без пароля зашифрованное не прочитать.
        let password = self.password.lock().unwrap().clone();
        if encrypted && password.is_none() {
            return Err(libc::EACCES);
        }
        if self.index.seven || (encrypted && !self.index.zip) {
            let p = self.extract_7z(target, password.as_deref())?;
            if target != ino {
                self.ready.lock().unwrap().insert(ino, p.clone());
            }
            return Ok(p);
        }
        // Ошибка чтения зашифрованного — неверный пароль.
        let fail = if encrypted { libc::EACCES } else { libc::EIO };
        let mut cur = self.cursor.lock().unwrap();
        // Пока ждали замок, его мог распаковать другой поток.
        if let Some(p) = self.ready.lock().unwrap().get(&ino) {
            return Ok(p.clone());
        }
        let reuse = matches!(&*cur, Some(r) if r.index != usize::MAX && r.index < want);
        if !reuse {
            *cur = Some(Reader::open(&self.archive, password.as_deref()).map_err(|e| {
                tracing::warn!("{}: {e}", self.archive.display());
                libc::EIO
            })?);
        }
        let r = cur.as_mut().unwrap();
        loop {
            let h = match r.next() {
                Ok(Some(h)) => h,
                Ok(None) => {
                    *cur = None;
                    return Err(libc::EIO);
                }
                Err(e) => {
                    tracing::warn!("{}: {e}", self.archive.display());
                    *cur = None;
                    return Err(libc::EIO);
                }
            };
            let idx = r.index;
            if idx == want {
                let res = self.store(target, &mut |b| r.read(b).map_err(|_| fail));
                if res.is_err() {
                    *cur = None;
                }
                let p = res?;
                if target != ino {
                    self.ready.lock().unwrap().insert(ino, p.clone());
                }
                return Ok(p);
            }
            // Попутно: небольшие файлы сжатого tar и т. п.
            if self.index.sequential && h.kind == la::IFREG && !h.encrypted {
                if let Some(&other) = self.index.by_entry.get(&idx) {
                    let small = h.size.is_some_and(|s| s <= SIDE_CACHE_FILE);
                    let room = self.side_bytes.load(Ordering::Relaxed) < SIDE_CACHE_TOTAL;
                    if small && room && !self.ready.lock().unwrap().contains_key(&other) {
                        if self.store(other, &mut |b| r.read(b).map_err(|_| libc::EIO)).is_err() {
                            *cur = None;
                            return Err(libc::EIO);
                        }
                        self.side_bytes.fetch_add(h.size.unwrap_or(0), Ordering::Relaxed);
                    }
                }
            }
        }
    }

    fn store(&self, ino: u64, read: &mut dyn FnMut(&mut [u8]) -> Result<usize, i32>) -> Result<PathBuf, i32> {
        let path = self.write_part(ino, read)?;
        self.ready.lock().unwrap().insert(ino, path.clone());
        Ok(path)
    }

    /// Записать данные в кэш (`ino.part` → `ino`), не отмечая готовым.
    fn write_part(&self, ino: u64, read: &mut dyn FnMut(&mut [u8]) -> Result<usize, i32>) -> Result<PathBuf, i32> {
        let path = self.cache.join(ino.to_string());
        let part = self.cache.join(format!("{ino}.part"));
        let mut f = File::create(&part).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))?;
        let mut buf = vec![0u8; 1 << 18];
        loop {
            let n = match read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e);
                }
            };
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))?;
        }
        drop(f);
        std::fs::rename(&part, &path).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))?;
        Ok(path)
    }

    /// Распаковать файл программой `7z` (пароль — в её stdin).
    fn extract_7z(&self, ino: u64, password: Option<&str>) -> Result<PathBuf, i32> {
        if let Some(p) = self.ready.lock().unwrap().get(&ino) {
            return Ok(p.clone());
        }
        let node = self.index.node(ino).ok_or(libc::ENOENT)?;
        let mut child = seven_cmd(&["x", "-so", "-y", "-spd"], &self.archive, Some(&node.apath)).spawn().map_err(|e| {
            tracing::warn!("7z: {e}");
            libc::EIO
        })?;
        feed_password(&mut child, password);
        let err = drain_stderr(&mut child);
        let mut out = child.stdout.take().ok_or(libc::EIO)?;
        let res = self.write_part(ino, &mut |b| out.read(b).map_err(|_| libc::EIO));
        drop(out);
        let ok = child.wait().is_ok_and(|s| s.success());
        let err = err.join().unwrap_or_default();
        match res {
            Ok(p) if ok => {
                self.ready.lock().unwrap().insert(ino, p.clone());
                Ok(p)
            }
            Ok(p) => {
                let _ = std::fs::remove_file(p);
                Err(seven_errno(&err))
            }
            Err(_) if !ok => Err(seven_errno(&err)),
            Err(e) => Err(e),
        }
    }

    /// Пароль подходит: самый маленький зашифрованный файл распаковывается
    /// целиком (контрольная сумма сходится).
    fn verify(&self) -> bool {
        let Some(ino) = (1..=self.index.nodes.len() as u64).filter(|&i| self.index.node(i).is_some_and(|n| n.encrypted && n.kind == Kind::File && n.entry.is_some())).min_by_key(|&i| self.index.node(i).map_or(0, |n| n.size)) else {
            return true;
        };
        self.ready.lock().unwrap().remove(&ino);
        self.extract(ino).is_ok()
    }

    /// Принять пароль, если подходит.
    fn set_password(&self, pw: &str) -> bool {
        let old = self.password.lock().unwrap().replace(pw.to_string());
        *self.cursor.lock().unwrap() = None;
        if self.verify() {
            return true;
        }
        *self.password.lock().unwrap() = old;
        *self.cursor.lock().unwrap() = None;
        false
    }

    fn info(&self) -> String {
        let (files, dirs, size, _) = self.index.stats();
        let locked = self.encrypted > 0 && self.password.lock().unwrap().is_none();
        format!("encrypted={} locked={} files={files} dirs={dirs} size={size}\n", self.encrypted, locked as u8)
    }
}

/// Почему архив не открылся.
#[derive(Debug)]
pub enum ScanError {
    /// Оглавление зашифровано — нужен пароль.
    NeedPassword,
    BadPassword,
    Other(String),
}

impl From<String> for ScanError {
    fn from(e: String) -> Self {
        ScanError::Other(e)
    }
}

/// Прочитать оглавление. Файлы без размера в заголовке (одиночный .gz)
/// распаковываются сразу — иначе размер не узнать.
fn scan(archive: &Path, cache: &Path, password: Option<&str>) -> Result<(Index, HashMap<u64, PathBuf>), ScanError> {
    match scan_la(archive, cache, password) {
        // Зашифрованное оглавление (7z -mhe) libarchive не читает.
        Err(e) if e.contains("encrypted") || e.contains("assphrase") => scan_7z(archive, password),
        r => r.map_err(ScanError::Other),
    }
}

fn scan_la(archive: &Path, cache: &Path, password: Option<&str>) -> Result<(Index, HashMap<u64, PathBuf>), String> {
    let mut r = Reader::open(archive, password)?;
    let mtime0 = std::fs::metadata(archive).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let mut ix = Index::new(mtime0);
    let mut ready = HashMap::new();
    let mut hardlinks: Vec<(u64, String)> = Vec::new();
    let mut first = true;
    while let Some(h) = r.next()? {
        if first {
            first = false;
            ix.sequential = !super::seekable_name(archive);
            ix.zip = r.is_zip();
        }
        let raw = r.is_raw();
        let path = if raw { raw_name(archive) } else { h.path.clone() };
        let parts = split(&path);
        let Some((name, dirs)) = parts.split_last() else { continue };
        if h.kind == la::IFDIR {
            let d = ix.dirs(&parts, h.mtime);
            let n = &mut ix.nodes[d as usize - 1];
            n.perm = h.perm | 0o700;
            n.mtime = h.mtime;
            continue;
        }
        let parent = ix.dirs(dirs, h.mtime);
        let kind = if h.kind == la::IFLNK && h.symlink.is_some() { Kind::Link } else { Kind::File };
        let ino = ix.child(parent, name, kind, h.mtime);
        let idx = r.index;
        {
            let n = &mut ix.nodes[ino as usize - 1];
            if n.kind == Kind::Dir {
                continue;
            }
            n.kind = kind;
            n.perm = h.perm;
            n.mtime = h.mtime;
            n.encrypted = h.encrypted;
            n.apath = h.path.clone();
            match kind {
                Kind::Link => {
                    let t = h.symlink.clone().unwrap_or_default();
                    n.size = t.len() as u64;
                    n.link = Some(t);
                }
                _ => {
                    n.size = h.size.unwrap_or(0);
                    n.entry = Some(idx);
                }
            }
        }
        if kind == Kind::File {
            if let Some(t) = &h.hardlink {
                hardlinks.push((ino, t.clone()));
                continue;
            }
            ix.by_entry.insert(idx, ino);
            if h.size.is_none() && !h.encrypted {
                std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
                let p = cache.join(ino.to_string());
                let mut f = File::create(&p).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 1 << 18];
                let mut total = 0u64;
                loop {
                    let n = r.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    total += n as u64;
                    f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                }
                ix.nodes[ino as usize - 1].size = total;
                ready.insert(ino, p);
            }
        }
    }
    // Жёсткие ссылки tar: данные — у цели.
    for (ino, target) in hardlinks {
        let t = ix.lookup_path(&split(&target)).and_then(|t| ix.node(t).cloned());
        let n = &mut ix.nodes[ino as usize - 1];
        match t {
            Some(t) if t.kind == Kind::File => {
                n.entry = t.entry;
                n.size = t.size;
            }
            _ => n.entry = None,
        }
    }
    Ok((ix, ready))
}

/// Оглавление через `7z l -slt` (зашифрованное оглавление 7z).
fn scan_7z(archive: &Path, password: Option<&str>) -> Result<(Index, HashMap<u64, PathBuf>), ScanError> {
    let Some(pw) = password else { return Err(ScanError::NeedPassword) };
    let mut child = seven_cmd(&["l", "-slt"], archive, None)
        .spawn()
        .map_err(|_| ScanError::Other(syngui::t!("Оглавление архива зашифровано — для него нужен 7-Zip (пакет 7zip)").into()))?;
    feed_password(&mut child, Some(pw));
    let err = drain_stderr(&mut child);
    let mut text = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut text);
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    let err = err.join().unwrap_or_default();
    if !ok {
        return Err(if seven_errno(&err) == libc::EACCES { ScanError::BadPassword } else { ScanError::Other(err.trim().to_string()) });
    }
    let mtime0 = std::fs::metadata(archive).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let mut ix = Index::new(mtime0);
    ix.seven = true;
    // Записи — после строки «----------», через пустую строку.
    let body = text.split_once("\n----------\n").map_or("", |(_, b)| b);
    for (idx, block) in body.split("\n\n").enumerate() {
        let get = |k: &str| block.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix(" = ")));
        let Some(path) = get("Path") else { continue };
        let parts = split(path);
        let Some((name, dirs)) = parts.split_last() else { continue };
        let mtime = get("Modified").and_then(parse_7z_time).unwrap_or(mtime0);
        let is_dir = get("Folder") == Some("+") || get("Attributes").is_some_and(|a| a.starts_with('D'));
        if is_dir {
            ix.dirs(&parts, mtime);
            continue;
        }
        let parent = ix.dirs(dirs, mtime);
        let ino = ix.child(parent, name, Kind::File, mtime);
        let n = &mut ix.nodes[ino as usize - 1];
        if n.kind == Kind::Dir {
            continue;
        }
        n.size = get("Size").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        n.encrypted = get("Encrypted") == Some("+");
        n.entry = Some(idx);
        n.apath = path.to_string();
        ix.by_entry.insert(idx, ino);
    }
    Ok((ix, HashMap::new()))
}

/// «2026-10-21 15:05:38.4106500» (местное время) → секунды Unix.
fn parse_7z_time(s: &str) -> Option<i64> {
    let (d, t) = s.trim().split_once(' ')?;
    let mut d = d.split('-').map(|x| x.parse::<i32>().ok());
    let mut t = t.split(['.', ':']).map(|x| x.parse::<i32>().ok());
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = d.next()?? - 1900;
    tm.tm_mon = d.next()?? - 1;
    tm.tm_mday = d.next()??;
    tm.tm_hour = t.next()??;
    tm.tm_min = t.next()??;
    tm.tm_sec = t.next()??;
    tm.tm_isdst = -1;
    let v = unsafe { libc::mktime(&mut tm) };
    (v >= 0).then_some(v as i64)
}

/// `7z` с общими ключами: без индикатора, имена в UTF-8, пароль из stdin.
fn seven_cmd(args: &[&str], archive: &Path, entry: Option<&str>) -> std::process::Command {
    let mut c = std::process::Command::new("7z");
    c.args(args).arg("-bd").arg("-sccUTF-8").arg("--").arg(archive);
    if let Some(e) = entry {
        c.arg(e);
    }
    c.env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    c
}

fn feed_password(child: &mut std::process::Child, password: Option<&str>) {
    if let Some(mut i) = child.stdin.take() {
        let _ = writeln!(i, "{}", password.unwrap_or(""));
    }
}

/// Читать stderr в отдельном потоке (не упереться в полный канал).
fn drain_stderr(child: &mut std::process::Child) -> std::thread::JoinHandle<String> {
    let e = child.stderr.take();
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = e {
            let _ = e.read_to_string(&mut s);
        }
        s
    })
}

fn seven_errno(err: &str) -> i32 {
    if err.contains("password") || err.contains("encrypted") {
        libc::EACCES
    } else {
        tracing::warn!("7z: {}", err.trim());
        libc::EIO
    }
}

struct ArchiveFs {
    s: Arc<Shared>,
    uid: u32,
    gid: u32,
    blocks: u64,
}

impl ArchiveFs {
    fn attr(&self, ino: u64, n: &Node) -> FileAttr {
        let t = if n.mtime > 0 { UNIX_EPOCH + Duration::from_secs(n.mtime as u64) } else { UNIX_EPOCH };
        let (kind, perm) = match n.kind {
            Kind::Dir => (FileType::Directory, (n.perm | 0o555) & 0o755),
            Kind::File => (FileType::RegularFile, (n.perm | 0o444) & 0o755),
            Kind::Link => (FileType::Symlink, 0o777),
        };
        FileAttr {
            ino,
            size: n.size,
            blocks: n.size.div_ceil(512),
            atime: t,
            mtime: t,
            ctime: t,
            crtime: t,
            kind,
            perm: perm as u16,
            nlink: if n.kind == Kind::Dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 65536,
            flags: 0,
        }
    }
}

impl Filesystem for ArchiveFs {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        self.s.touch();
        let key = (parent, name.to_string_lossy().to_string());
        match self.s.index.names.get(&key).and_then(|&i| self.s.index.node(i).map(|n| (i, n))) {
            Some((i, n)) => reply.entry(&TTL, &self.attr(i, n), 0),
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        self.s.touch();
        match self.s.index.node(ino) {
            Some(n) => reply.attr(&TTL, &self.attr(ino, n)),
            None => reply.error(libc::ENOENT),
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        match self.s.index.node(ino).and_then(|n| n.link.as_ref()) {
            Some(l) => reply.data(l.as_bytes()),
            None => reply.error(libc::EINVAL),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        self.s.touch();
        if flags & libc::O_ACCMODE != libc::O_RDONLY {
            return reply.error(libc::EROFS);
        }
        match self.s.index.node(ino) {
            Some(n) if n.kind == Kind::File => {}
            Some(_) => return reply.error(libc::EISDIR),
            None => return reply.error(libc::ENOENT),
        }
        self.s.open.fetch_add(1, Ordering::SeqCst);
        // Распаковка может быть долгой — не держать остальные запросы.
        let s = self.s.clone();
        std::thread::spawn(move || {
            let res = s.extract(ino).and_then(|p| File::open(p).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO)));
            match res {
                Ok(f) => {
                    let fh = s.next_fh.fetch_add(1, Ordering::Relaxed);
                    s.files.lock().unwrap().insert(fh, Arc::new(f));
                    s.touch();
                    reply.opened(fh, fuser::consts::FOPEN_KEEP_CACHE);
                }
                Err(e) => {
                    s.open.fetch_sub(1, Ordering::SeqCst);
                    s.touch();
                    reply.error(e);
                }
            }
        });
    }

    fn read(&mut self, _req: &Request<'_>, _ino: u64, fh: u64, offset: i64, size: u32, _flags: i32, _lock: Option<u64>, reply: ReplyData) {
        let Some(f) = self.s.files.lock().unwrap().get(&fh).cloned() else { return reply.error(libc::EBADF) };
        let mut buf = vec![0u8; size as usize];
        let mut got = 0usize;
        while got < buf.len() {
            match f.read_at(&mut buf[got..], offset as u64 + got as u64) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => return reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
            }
        }
        reply.data(&buf[..got]);
    }

    fn release(&mut self, _req: &Request<'_>, _ino: u64, fh: u64, _flags: i32, _lock: Option<u64>, _flush: bool, reply: ReplyEmpty) {
        if self.s.files.lock().unwrap().remove(&fh).is_some() {
            self.s.open.fetch_sub(1, Ordering::SeqCst);
        }
        self.s.touch();
        reply.ok();
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        self.s.touch();
        match self.s.index.node(ino) {
            Some(n) if n.kind == Kind::Dir => {
                self.s.open.fetch_add(1, Ordering::SeqCst);
                reply.opened(0, 0)
            }
            Some(_) => reply.error(libc::ENOTDIR),
            None => reply.error(libc::ENOENT),
        }
    }

    fn releasedir(&mut self, _req: &Request<'_>, _ino: u64, _fh: u64, _flags: i32, reply: ReplyEmpty) {
        self.s.open.fetch_sub(1, Ordering::SeqCst);
        self.s.touch();
        reply.ok();
    }

    fn readdir(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        self.s.touch();
        let Some(n) = self.s.index.node(ino) else { return reply.error(libc::ENOENT) };
        let mut all: Vec<(u64, FileType, &str)> = vec![(ino, FileType::Directory, "."), (n.parent, FileType::Directory, "..")];
        for &c in &n.children {
            if let Some(cn) = self.s.index.node(c) {
                let t = match cn.kind {
                    Kind::Dir => FileType::Directory,
                    Kind::File => FileType::RegularFile,
                    Kind::Link => FileType::Symlink,
                };
                all.push((c, t, &cn.name));
            }
        }
        for (i, (ino, t, name)) in all.into_iter().enumerate().skip(offset.max(0) as usize) {
            if reply.add(ino, (i + 1) as i64, t, name) {
                break;
            }
        }
        reply.ok();
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        reply.statfs(self.blocks, 0, 0, self.s.index.nodes.len() as u64, 0, 512, 255, 512);
    }

    fn access(&mut self, _req: &Request<'_>, _ino: u64, mask: i32, reply: ReplyEmpty) {
        if mask & libc::W_OK != 0 {
            reply.error(libc::EROFS)
        } else {
            reply.ok()
        }
    }
}

struct Mounted {
    session: fuser::Session<ArchiveFs>,
    shared: Arc<Shared>,
    cache: PathBuf,
    /// Архив открыт всё время: если он сам лежит в другом архиве, тот
    /// помощник видит открытый файл и не снимается.
    _keep: File,
}

fn prepare(archive: &Path, mp: &Path, password: Option<&str>) -> Result<Mounted, ScanError> {
    let keep = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let cache = super::cache_dir(archive);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let (index, ready) = match scan(archive, &cache, password) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&cache);
            return Err(e);
        }
    };
    let encrypted = index.stats().3;
    let size = keep.metadata().map(|m| m.len()).unwrap_or(0);
    let s = Arc::new(Shared {
        archive: archive.to_path_buf(),
        cache: cache.clone(),
        index,
        ready: Mutex::new(ready),
        side_bytes: AtomicU64::new(0),
        cursor: Mutex::new(None),
        files: Mutex::new(HashMap::new()),
        next_fh: AtomicU64::new(1),
        open: AtomicUsize::new(0),
        last: Mutex::new(Instant::now()),
        password: Mutex::new(password.map(str::to_string)),
        encrypted,
    });
    // Пароль дан при запуске — проверить сразу (не монтировать с неверным).
    if password.is_some() && encrypted > 0 && !s.index.seven && !s.verify() {
        let _ = std::fs::remove_dir_all(&cache);
        return Err(ScanError::BadPassword);
    }
    std::fs::create_dir_all(mp).map_err(|e| format!("{}: {e}", mp.display()))?;
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let fs = ArchiveFs { s: s.clone(), uid, gid, blocks: size.div_ceil(512) };
    let name = archive.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let opts = [
        MountOption::RO,
        MountOption::FSName(format!("synfiles:{name}")),
        MountOption::Subtype("archive".into()),
        MountOption::NoAtime,
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    match fuser::Session::new(fs, mp, &opts) {
        Ok(session) => Ok(Mounted { session, shared: s, cache, _keep: keep }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&cache);
            Err(ScanError::Other(format!("fusermount3: {e}")))
        }
    }
}

/// Точка входа помощника. Пароль (если есть) — первой строкой stdin.
/// Печатает в stdout `ready`, когда смонтировано, `password` — нужен
/// пароль, `badpassword` — пароль не подошёл (или ошибку в stderr и код 1),
/// и дальше живёт сам по себе.
pub fn run(archive: &Path, mp: &Path) -> i32 {
    if super::is_mounted(mp) {
        println!("ready");
        return 0;
    }
    let mut password = None;
    if unsafe { libc::isatty(0) } == 0 {
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_ok() {
            let pw = line.strip_suffix('\n').unwrap_or(&line);
            if !pw.is_empty() {
                password = Some(pw.to_string());
            }
        }
    }
    sweep_cache();
    let Mounted { mut session, shared: s, cache, _keep } = match prepare(archive, mp, password.as_deref()) {
        Ok(m) => m,
        Err(ScanError::NeedPassword) => {
            println!("password");
            return 3;
        }
        Err(ScanError::BadPassword) => {
            println!("badpassword");
            return 3;
        }
        Err(ScanError::Other(e)) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let ctl = mp.parent().map(|k| k.join("ctl"));
    if let Some(ctl) = &ctl {
        serve_ctl(ctl, s.clone());
    }
    println!("ready");
    detach_stdio();
    let mp2 = mp.to_path_buf();
    let watch = s.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let idle = watch.open.load(Ordering::SeqCst) == 0 && watch.last.lock().unwrap().elapsed() > IDLE;
        // Архив удалили или переместили — тоже сняться.
        let gone = !watch.archive.exists();
        if idle || gone {
            // Не «ленивое» снятие: занято (терминал стоит внутри) — живём дальше.
            let ok = std::process::Command::new("fusermount3")
                .arg("-u")
                .arg(&mp2)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                break;
            }
            watch.touch();
        }
    });
    let _ = session.run();
    drop(session);
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir(mp);
    if let Some(key) = mp.parent() {
        let _ = std::fs::remove_file(key.join("source"));
        let _ = std::fs::remove_file(key.join("ctl"));
        let _ = std::fs::remove_dir(key);
    }
    0
}

/// Управляющий сокет: `info` — сведения об архиве ([`Shared::info`]),
/// `password <пароль>` — проверить и принять пароль (`ok` / `bad`).
fn serve_ctl(path: &Path, s: Arc<Shared>) {
    let _ = std::fs::remove_file(path);
    // Относительно папки ключа: длина пути сокета ограничена 108 байтами.
    let bound = match (path.parent(), path.file_name()) {
        (Some(d), Some(n)) if std::env::set_current_dir(d).is_ok() => UnixListener::bind(n),
        _ => UnixListener::bind(path),
    };
    let Ok(l) = bound else { return };
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let s = s.clone();
            std::thread::spawn(move || {
                let mut r = std::io::BufReader::new(&conn);
                let mut line = String::new();
                if r.read_line(&mut line).is_err() {
                    return;
                }
                s.touch();
                let line = line.strip_suffix('\n').unwrap_or(&line);
                let reply = if line == "info" {
                    s.info()
                } else if let Some(pw) = line.strip_prefix("password ") {
                    if s.set_password(pw) { "ok\n".into() } else { "bad\n".into() }
                } else {
                    "?\n".into()
                };
                let _ = (&conn).write_all(reply.as_bytes());
                s.touch();
            });
        }
    });
}

/// Убрать кэш помощников, которых убили (выход из сеанса): ключи без
/// смонтированного архива.
fn sweep_cache() {
    let Some(base) = super::cache_dir(Path::new("/")).parent().map(Path::to_path_buf) else { return };
    let Ok(rd) = std::fs::read_dir(&base) else { return };
    for e in rd.flatten() {
        let key = e.file_name();
        let live = std::fs::read_dir(super::base().join(&key))
            .map(|d| d.flatten().any(|m| m.path().is_dir() && super::is_mounted(&m.path())))
            .unwrap_or(false);
        // Свежий — может быть кэшем помощника, который ещё монтирует.
        let fresh = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_none_or(|d| d < Duration::from_secs(120));
        if !live && !fresh {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Отпустить stdout/stderr (их читает запустивший Проводник).
fn detach_stdio() {
    let _ = std::io::stdout().flush();
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 0);
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            libc::close(null);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(split("./a//b/../c/"), vec!["a", "b", "c"]);
        assert_eq!(raw_name(Path::new("/x/отчёт.txt.gz")), "отчёт.txt");
        assert_eq!(raw_name(Path::new("/x/a.tgz")), "a.tar");
    }

    #[test]
    fn mount_and_read() {
        // Настоящий FUSE — только где он есть.
        if !Path::new("/dev/fuse").exists() || std::process::Command::new("fusermount3").arg("-V").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("папка/вложенная")).unwrap();
        std::fs::write(src.join("папка/вложенная/a.txt"), "привет").unwrap();
        std::fs::write(src.join("b.txt"), "пока").unwrap();
        std::fs::write(d.path().join("один.txt"), "сжатый").unwrap();
        let ok = std::process::Command::new("zstd").arg("-q").arg(d.path().join("один.txt")).status().is_ok_and(|s| s.success());
        for name in ["t.zip", "t.tar.gz", "t.7z", "один.txt.zst"] {
            let arc = d.path().join(name);
            if name.ends_with(".zst") {
                if !ok {
                    continue;
                }
            } else {
                let st = std::process::Command::new("bsdtar").arg("-a").arg("-cf").arg(&arc).arg("-C").arg(&src).arg(".").status().unwrap();
                assert!(st.success());
            }
            let mp = d.path().join(format!("mnt-{name}"));
            let m = prepare(&arc, &mp, None).unwrap();
            let bg = m.session.spawn().unwrap();
            if name.ends_with(".zst") {
                assert_eq!(std::fs::read_to_string(mp.join("один.txt")).unwrap(), "сжатый");
            } else {
                assert_eq!(std::fs::read_to_string(mp.join("папка/вложенная/a.txt")).unwrap(), "привет", "{name}");
                assert_eq!(std::fs::read_to_string(mp.join("b.txt")).unwrap(), "пока");
                assert!(mp.join("папка").is_dir());
                assert!(std::fs::write(mp.join("new.txt"), "x").is_err());
            }
            drop(bg);
            let _ = std::fs::remove_dir_all(&m.cache);
        }
    }

    #[test]
    fn encrypted_zip() {
        if !Path::new("/dev/fuse").exists() || std::process::Command::new("zip").arg("-v").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "секрет").unwrap();
        let arc = d.path().join("e.zip");
        let st = std::process::Command::new("zip").current_dir(d.path()).args(["-q", "-P", "pw"]).arg(&arc).arg("a.txt").status().unwrap();
        assert!(st.success());
        assert!(matches!(prepare(&arc, &d.path().join("m0"), Some("wrong")), Err(ScanError::BadPassword)));
        let mp = d.path().join("m");
        let m = prepare(&arc, &mp, None).unwrap();
        let s = m.shared.clone();
        let bg = m.session.spawn().unwrap();
        assert!(s.info().contains("locked=1"));
        assert_eq!(std::fs::read(mp.join("a.txt")).unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!s.set_password("nope"));
        assert!(s.set_password("pw"));
        assert_eq!(std::fs::read_to_string(mp.join("a.txt")).unwrap(), "секрет");
        drop(bg);
        let _ = std::fs::remove_dir_all(&m.cache);
    }

}
