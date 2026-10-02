//! Камера телефона: какая включена — подписка на службу камеры syncamd
//! платформы (arch-mobile-port, `camera/syncam.h`): сокет
//! `/run/syncam/syncam.sock` (SOCK_SEQPACKET, группа video), сообщение
//! WATCH → STATE сразу и при каждом включении/выключении камеры. Для значка
//! «камера используется».

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub const SOCKET: &str = "/run/syncam/syncam.sock";
const SC_WATCH: u32 = 9;
const SC_STATE: u32 = 10;

/// Состояние syncamd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    /// Включённая камера (id HAL).
    pub camera: Option<u32>,
    /// Клиентов, получающих кадры (мосты PipeWire сеансов).
    pub subscribers: u32,
}

fn connect() -> io::Result<OwnedFd> {
    // SAFETY: обычные вызовы сокетов; дескриптор сразу переходит во владение OwnedFd
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(SOCKET.bytes()) {
            *d = s as libc::c_char;
        }
        let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        if libc::connect(fd.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, len) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }
}

/// Подписаться и звать `f` на каждое состояние, пока она возвращает true.
/// Блокирующая; возвращается, когда syncamd закрыл соединение (перезапуск) —
/// повтор за вызывающим.
pub fn watch(mut f: impl FnMut(State) -> bool) -> io::Result<()> {
    let fd = connect()?;
    let req = SC_WATCH.to_ne_bytes();
    // SAFETY: буферы живут на время вызовов
    if unsafe { libc::send(fd.as_raw_fd(), req.as_ptr().cast(), req.len(), libc::MSG_NOSIGNAL) } != req.len() as isize {
        return Err(io::Error::last_os_error());
    }
    let mut buf = [0u8; 64];
    loop {
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(());
        }
        let word = |i: usize| u32::from_ne_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
        if n as usize >= 12 && word(0) == SC_STATE {
            let cam = word(1) as i32;
            let st = State { camera: (cam >= 0).then_some(cam as u32), subscribers: word(2) };
            if !f(st) {
                return Ok(());
            }
        }
    }
}
