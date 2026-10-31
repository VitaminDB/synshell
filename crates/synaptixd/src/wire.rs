use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub const MAX_MSG: usize = 4 << 20;
/// Файлов в одном сообщении (входы графа — до стольких за запрос).
pub const MAX_FDS: usize = 64;

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

fn sockaddr(path: &Path) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "длинный путь сокета"));
    }
    for (d, s) in addr.sun_path.iter_mut().zip(bytes) {
        *d = *s as libc::c_char;
    }
    let len = std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1;
    Ok((addr, len as libc::socklen_t))
}

fn seqpacket() -> io::Result<OwnedFd> {
    let fd = cvt(unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) })?;
    let sock = unsafe { OwnedFd::from_raw_fd(fd) };
    let size: libc::c_int = (MAX_MSG + 4096) as libc::c_int;
    for opt in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                &size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
    Ok(sock)
}

pub fn connect(path: &Path) -> io::Result<OwnedFd> {
    let sock = seqpacket()?;
    let (addr, len) = sockaddr(path)?;
    cvt(unsafe { libc::connect(sock.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, len) })?;
    Ok(sock)
}

pub fn listen(path: &Path) -> io::Result<OwnedFd> {
    let sock = seqpacket()?;
    let _ = std::fs::remove_file(path);
    let (addr, len) = sockaddr(path)?;
    cvt(unsafe { libc::bind(sock.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, len) })?;
    cvt(unsafe { libc::listen(sock.as_raw_fd(), 64) })?;
    Ok(sock)
}

pub fn accept(listener: BorrowedFd) -> io::Result<OwnedFd> {
    loop {
        let fd = unsafe { libc::accept4(listener.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        if fd >= 0 {
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

pub fn peer(sock: BorrowedFd) -> io::Result<Peer> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    cvt(unsafe {
        libc::getsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    })?;
    Ok(Peer { pid: cred.pid, uid: cred.uid, gid: cred.gid })
}

const fn cmsg_space(n: usize) -> usize {
    let a = std::mem::size_of::<usize>();
    let hdr = (std::mem::size_of::<libc::cmsghdr>() + a - 1) & !(a - 1);
    let data = (n * std::mem::size_of::<RawFd>() + a - 1) & !(a - 1);
    hdr + data
}

#[repr(C, align(8))]
struct CmsgBuf([u8; cmsg_space(MAX_FDS)]);

/// Сообщения больше этого — через memfd: большой SEQPACKET ядро кладёт в один
/// непрерывный буфер, и при фрагментированной памяти sendmsg падает с ENOBUFS
/// (запрос чата с длинной историей инструментов — сотни КБ).
const BIG_MSG: usize = 64 << 10;
/// Само сообщение — маркер, его тело — последний переданный файл (memfd).
const MEMFD_MARK: &[u8] = b"\0synaptix-memfd";
/// Предел тела в memfd.
const MAX_MEMFD_MSG: usize = 256 << 20;

pub fn send(sock: BorrowedFd, data: &[u8], fds: &[BorrowedFd]) -> io::Result<()> {
    if data.len() > BIG_MSG && data.len() <= MAX_MEMFD_MSG && fds.len() < MAX_FDS {
        let body = memfd("msg", data)?;
        let mut all: Vec<BorrowedFd> = fds.to_vec();
        all.push(std::os::fd::AsFd::as_fd(&body));
        return send_raw(sock, MEMFD_MARK, &all);
    }
    send_raw(sock, data, fds)
}

fn send_raw(sock: BorrowedFd, data: &[u8], fds: &[BorrowedFd]) -> io::Result<()> {
    if data.len() > MAX_MSG {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "сообщение больше 4 МБ"));
    }
    if fds.len() > MAX_FDS {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "много файлов в сообщении"));
    }
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    let mut cbuf = CmsgBuf([0; cmsg_space(MAX_FDS)]);
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        msg.msg_control = cbuf.0.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = unsafe { libc::CMSG_SPACE((fds.len() * std::mem::size_of::<RawFd>()) as u32) } as _;
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN((fds.len() * std::mem::size_of::<RawFd>()) as u32) as _;
            let p = libc::CMSG_DATA(c) as *mut RawFd;
            for (i, f) in fds.iter().enumerate() {
                p.add(i).write_unaligned(f.as_raw_fd());
            }
        }
    }
    loop {
        let r = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
        if r >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn pending_len(sock: BorrowedFd) -> io::Result<usize> {
    loop {
        let r = unsafe { libc::recv(sock.as_raw_fd(), std::ptr::null_mut(), 0, libc::MSG_PEEK | libc::MSG_TRUNC) };
        if r >= 0 {
            return Ok(r as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub fn recv(sock: BorrowedFd) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
    let size = pending_len(sock)?.clamp(1, MAX_MSG);
    let mut buf = vec![0u8; size];
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    let mut cbuf = CmsgBuf([0; cmsg_space(MAX_FDS)]);
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.0.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cbuf.0.len() as _;
    let n = loop {
        let r = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if r >= 0 {
            break r as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut fds = Vec::new();
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let p = libc::CMSG_DATA(c) as *const RawFd;
                for i in 0..bytes / std::mem::size_of::<RawFd>() {
                    fds.push(OwnedFd::from_raw_fd(p.add(i).read_unaligned()));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "соединение закрыто"));
    }
    if msg.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "сообщение обрезано"));
    }
    buf.truncate(n);
    if buf == MEMFD_MARK {
        if let Some(body) = fds.pop() {
            let mut f = std::fs::File::from(body);
            let mut data = Vec::new();
            std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(0))?;
            std::io::Read::read_to_end(&mut std::io::Read::take(&mut f, MAX_MEMFD_MSG as u64 + 1), &mut data)?;
            if data.len() > MAX_MEMFD_MSG {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "сообщение в memfd больше предела"));
            }
            return Ok((data, fds));
        }
    }
    Ok((buf, fds))
}

pub fn memfd(name: &str, data: &[u8]) -> io::Result<OwnedFd> {
    let cname = CString::new(name).unwrap_or_default();
    let fd = cvt(unsafe { libc::memfd_create(cname.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) })?;
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(fd);
    std::io::Write::write_all(&mut file, data)?;
    // Позиция файла общая с получателем — в начало, чтобы читать можно было
    // и обычным read, а не только mmap.
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0))?;
    let fd = OwnedFd::from(file);
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE | libc::F_SEAL_SEAL);
    }
    Ok(fd)
}

pub fn memfd_f32(name: &str, data: &[f32]) -> io::Result<OwnedFd> {
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) };
    memfd(name, bytes)
}

pub struct Mapped {
    ptr: *mut libc::c_void,
    len: usize,
    _fd: OwnedFd,
}

unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

impl Mapped {
    pub fn map(fd: OwnedFd, len: usize) -> io::Result<Self> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        cvt(unsafe { libc::fstat(fd.as_raw_fd(), &mut st) })?;
        if (st.st_size as usize) < len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "файл данных короче ожидаемого"));
        }
        if len == 0 {
            return Ok(Self { ptr: std::ptr::NonNull::<f32>::dangling().as_ptr() as *mut libc::c_void, len: 0, _fd: fd });
        }
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, fd.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { ptr, len, _fd: fd })
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }

    pub fn f32s(&self) -> &[f32] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const f32, self.len / 4) }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        if self.len > 0 {
            unsafe {
                libc::munmap(self.ptr, self.len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn fds_cross_socket() {
        let mut sv = [0; 2];
        assert_eq!(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, sv.as_mut_ptr()) }, 0);
        let (a, b) = unsafe { (OwnedFd::from_raw_fd(sv[0]), OwnedFd::from_raw_fd(sv[1])) };
        let data: Vec<f32> = (0..1000).map(|i| i as f32 * 0.5).collect();
        let m = memfd_f32("t", &data).unwrap();
        send(a.as_fd(), b"{\"x\":1}", &[m.as_fd()]).unwrap();
        send(a.as_fd(), b"second", &[]).unwrap();
        let (msg, mut fds) = recv(b.as_fd()).unwrap();
        assert_eq!(msg, b"{\"x\":1}");
        assert_eq!(fds.len(), 1);
        let mapped = Mapped::map(fds.remove(0), data.len() * 4).unwrap();
        assert_eq!(mapped.f32s(), &data[..]);
        let (msg, fds) = recv(b.as_fd()).unwrap();
        assert_eq!(msg, b"second");
        assert!(fds.is_empty());
    }

    /// Большое сообщение идёт через memfd и приходит целиком; свои файлы — на месте.
    #[test]
    fn big_message_via_memfd() {
        let mut sv = [0; 2];
        assert_eq!(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, sv.as_mut_ptr()) }, 0);
        let (a, b) = unsafe { (OwnedFd::from_raw_fd(sv[0]), OwnedFd::from_raw_fd(sv[1])) };
        let big: Vec<u8> = (0..(6usize << 20)).map(|i| (i % 251) as u8).collect();
        let m = memfd("x", b"attach").unwrap();
        send(a.as_fd(), &big, &[m.as_fd()]).unwrap();
        let (msg, fds) = recv(b.as_fd()).unwrap();
        assert_eq!(msg, big);
        assert_eq!(fds.len(), 1);
    }
}
