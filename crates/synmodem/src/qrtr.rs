//! Сокет QRTR (`AF_QIPCRTR`): IPC между процессором приложений и сопроцессорами Qualcomm. Службы модема
//! (QMI) — адреса «узел:порт», их находит служба имён ядра по запросу `NEW_LOOKUP` на порт управления.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

const AF_QIPCRTR: libc::c_int = 42;
/// Порт управления (служба имён): ей шлют `NEW_LOOKUP`, от неё приходят `NEW_SERVER`/`DEL_SERVER`.
pub const PORT_CTRL: u32 = 0xffff_fffe;
const TYPE_NEW_SERVER: u32 = 4;
const TYPE_DEL_SERVER: u32 = 5;
const TYPE_NEW_LOOKUP: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Addr {
    family: u16,
    pub node: u32,
    pub port: u32,
}

impl Addr {
    pub fn new(node: u32, port: u32) -> Self {
        Self { family: AF_QIPCRTR as u16, node, port }
    }
}

/// Служба, объявленная в сети QRTR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Server {
    pub service: u32,
    /// Версия (младший байт) и экземпляр (`instance << 8`), как в пакете службы имён.
    pub instance: u32,
    pub addr: Addr,
}

/// Событие службы имён (ответ на `NEW_LOOKUP` и последующие изменения).
#[derive(Debug)]
pub enum Ctrl {
    NewServer(Server),
    DelServer(Server),
    /// Конец первоначального списка (пакет `NEW_SERVER` из нулей).
    EndOfList,
}

/// Датаграммный сокет QRTR со своим портом.
pub struct Socket {
    fd: OwnedFd,
}

impl Socket {
    pub fn open() -> io::Result<Self> {
        let fd = unsafe { libc::socket(AF_QIPCRTR, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    /// Свой адрес (узел этого процессора и выданный порт).
    pub fn local(&self) -> io::Result<Addr> {
        let mut a = Addr::default();
        let mut len = std::mem::size_of::<Addr>() as libc::socklen_t;
        let r = unsafe { libc::getsockname(self.fd.as_raw_fd(), (&mut a as *mut Addr).cast(), &mut len) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(a)
    }

    pub fn send_to(&self, to: Addr, buf: &[u8]) -> io::Result<()> {
        let r = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                buf.as_ptr().cast(),
                buf.len(),
                0,
                (&to as *const Addr).cast(),
                std::mem::size_of::<Addr>() as libc::socklen_t,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Следующая датаграмма и её отправитель; `None` — истёк `timeout`.
    pub fn recv(&self, buf: &mut [u8], timeout: Option<Duration>) -> io::Result<Option<(usize, Addr)>> {
        if let Some(t) = timeout {
            let mut p = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = t.as_millis().min(i32::MAX as u128) as i32;
            let r = unsafe { libc::poll(&mut p, 1, ms) };
            if r < 0 {
                return Err(io::Error::last_os_error());
            }
            if r == 0 {
                return Ok(None);
            }
        }
        let mut from = Addr::default();
        let mut len = std::mem::size_of::<Addr>() as libc::socklen_t;
        let n = unsafe {
            libc::recvfrom(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                (&mut from as *mut Addr).cast(),
                &mut len,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some((n as usize, from)))
    }

    /// Подписаться на службу `service` (все версии и экземпляры): служба имён пришлёт текущий список
    /// и дальше будет сообщать о появлении и уходе таких служб.
    pub fn lookup(&self, service: u32) -> io::Result<()> {
        let local = self.local()?;
        let mut pkt = [0u8; 20];
        pkt[0..4].copy_from_slice(&TYPE_NEW_LOOKUP.to_le_bytes());
        pkt[4..8].copy_from_slice(&service.to_le_bytes());
        self.send_to(Addr::new(local.node, PORT_CTRL), &pkt)
    }
}

/// Разобрать пакет службы имён (пришёл с порта [`PORT_CTRL`]).
pub fn parse_ctrl(buf: &[u8]) -> Option<Ctrl> {
    if buf.len() < 20 {
        return None;
    }
    let u = |i: usize| u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
    let srv = Server { service: u(4), instance: u(8), addr: Addr::new(u(12), u(16)) };
    match u(0) {
        TYPE_NEW_SERVER if srv.service == 0 && srv.instance == 0 && srv.addr.node == 0 && srv.addr.port == 0 => {
            Some(Ctrl::EndOfList)
        }
        TYPE_NEW_SERVER => Some(Ctrl::NewServer(srv)),
        TYPE_DEL_SERVER => Some(Ctrl::DelServer(srv)),
        _ => None,
    }
}

/// Найти службу: первая объявленная, ждать не дольше `timeout`.
pub fn find(service: u32, timeout: Duration) -> io::Result<Option<Server>> {
    let s = Socket::open()?;
    s.lookup(service)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 64];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let Some((n, from)) = s.recv(&mut buf, Some(left))? else { return Ok(None) };
        if from.port != PORT_CTRL {
            continue;
        }
        match parse_ctrl(&buf[..n]) {
            Some(Ctrl::NewServer(srv)) => return Ok(Some(srv)),
            // Список пуст — служба ещё не поднялась: ждём её объявления
            _ => continue,
        }
    }
}
