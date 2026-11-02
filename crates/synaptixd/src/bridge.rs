//! Мост службы через сеть (федерация synaptixd по synlink).
//!
//! Сообщения службы — JSON плюс файлы (memfd через SCM_RIGHTS), по сети так
//! не передать. Кадр моста несёт JSON и содержимое файлов по порядку:
//! на отправителе файлы читаются в байты, на получателе байты снова
//! становятся memfd, поэтому номера `FdRef` в JSON остаются верными.
//!
//! Формат кадра: `u64` длина тела (big-endian), тело — `u32` длина JSON,
//! JSON, `u32` число файлов, у каждого `u64` длина и байты.
//!
//! Кто на том конце моста: synlink открывает соединение со своей службой и
//! первым шлёт `Hello { client: "@remote:<устройство>" }` ([`REMOTE_PREFIX`]) —
//! такое соединение видит только свои модели (без пересылки дальше) и не
//! меняет настройки.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};

use crate::wire;

/// Предел кадра (видео с референсами — сотни МБ).
pub const MAX_FRAME: u64 = 1 << 30;
/// Метка соединения, пришедшего с другого устройства (`Hello.client`).
pub const REMOTE_PREFIX: &str = "@remote:";
/// Путь входного файла, приехавшего в кадре: `@blob:<номер файла>/<имя>`.
/// Служба на той стороне кладёт файл во временный каталог с этим именем.
pub const BLOB_PATH: &str = "@blob:";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Frame {
    pub json: Vec<u8>,
    pub blobs: Vec<Vec<u8>>,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

impl Frame {
    pub fn new(json: Vec<u8>) -> Self {
        Self { json, blobs: Vec::new() }
    }

    /// Тело кадра (без длины впереди).
    pub fn encode_body(&self) -> Vec<u8> {
        let size = 8 + self.json.len() + self.blobs.iter().map(|b| b.len() + 8).sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(self.json.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.json);
        out.extend_from_slice(&(self.blobs.len() as u32).to_be_bytes());
        for b in &self.blobs {
            out.extend_from_slice(&(b.len() as u64).to_be_bytes());
            out.extend_from_slice(b);
        }
        out
    }

    /// Кадр целиком: длина и тело.
    pub fn encode(&self) -> Vec<u8> {
        let body = self.encode_body();
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(&(body.len() as u64).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    pub fn decode_body(mut b: &[u8]) -> io::Result<Self> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> io::Result<&'a [u8]> {
            if b.len() < n {
                return Err(bad("кадр моста обрезан"));
            }
            let (h, t) = b.split_at(n);
            *b = t;
            Ok(h)
        }
        let jl = u32::from_be_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
        let json = take(&mut b, jl)?.to_vec();
        let n = u32::from_be_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
        if n > wire::MAX_FDS {
            return Err(bad("много файлов в кадре моста"));
        }
        let mut blobs = Vec::with_capacity(n);
        for _ in 0..n {
            let l = u64::from_be_bytes(take(&mut b, 8)?.try_into().unwrap());
            blobs.push(take(&mut b, usize::try_from(l).map_err(|_| bad("длина файла"))?)?.to_vec());
        }
        Ok(Self { json, blobs })
    }

    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.encode())?;
        w.flush()
    }

    /// Прочитать кадр; `None` — поток закрыт.
    pub fn read_from(r: &mut impl Read) -> io::Result<Option<Self>> {
        let mut len = [0u8; 8];
        match r.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let n = u64::from_be_bytes(len);
        if n > MAX_FRAME {
            return Err(bad("кадр моста больше предела"));
        }
        let mut body = vec![0u8; n as usize];
        r.read_exact(&mut body)?;
        Self::decode_body(&body).map(Some)
    }

    /// Из сообщения службы: файлы — в байты.
    pub fn from_message(json: Vec<u8>, fds: &[OwnedFd]) -> io::Result<Self> {
        let blobs = fds.iter().map(|f| fd_bytes(f.as_fd())).collect::<io::Result<_>>()?;
        Ok(Self { json, blobs })
    }

    /// Файлы — снова memfd (в том же порядке).
    pub fn memfds(&self) -> io::Result<Vec<OwnedFd>> {
        self.blobs.iter().enumerate().map(|(i, b)| wire::memfd(&format!("bridge{i}"), b)).collect()
    }

    /// Отправить в сокет службы (SEQPACKET).
    pub fn send_to(&self, sock: BorrowedFd) -> io::Result<()> {
        let fds = self.memfds()?;
        let borrowed: Vec<_> = fds.iter().map(|f| f.as_fd()).collect();
        wire::send(sock, &self.json, &borrowed)
    }

    /// Принять сообщение из сокета службы.
    pub fn recv_from(sock: BorrowedFd) -> io::Result<Self> {
        let (json, fds) = wire::recv(sock)?;
        Self::from_message(json, &fds)
    }
}

/// Всё содержимое файла (с начала, не трогая общую позицию).
pub fn fd_bytes(fd: BorrowedFd) -> io::Result<Vec<u8>> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let size = st.st_size.max(0) as u64;
    if size > MAX_FRAME {
        return Err(bad("файл больше предела моста"));
    }
    let mut out = vec![0u8; size as usize];
    let mut off = 0usize;
    while off < out.len() {
        let r = unsafe { libc::pread(fd.as_raw_fd(), out[off..].as_mut_ptr() as *mut libc::c_void, out.len() - off, off as libc::off_t) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if r == 0 {
            out.truncate(off);
            break;
        }
        off += r as usize;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    #[test]
    fn roundtrip() {
        let f = Frame { json: b"{\"id\":1}".to_vec(), blobs: vec![vec![1, 2, 3], Vec::new(), vec![7; 100_000]] };
        let enc = f.encode();
        let back = Frame::read_from(&mut &enc[..]).unwrap().unwrap();
        assert_eq!(back, f);
        assert!(Frame::read_from(&mut &[][..]).unwrap().is_none());
        assert!(Frame::decode_body(&enc[8..enc.len() - 1]).is_err());
    }

    #[test]
    fn through_socket() {
        let mut sv = [0; 2];
        assert_eq!(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, sv.as_mut_ptr()) }, 0);
        let (a, b) = unsafe { (OwnedFd::from_raw_fd(sv[0]), OwnedFd::from_raw_fd(sv[1])) };
        let pcm: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        let f = Frame { json: b"{\"op\":\"ctc\"}".to_vec(), blobs: vec![pcm.clone()] };
        let t = std::thread::spawn(move || f.send_to(a.as_fd()).unwrap());
        let got = Frame::recv_from(b.as_fd()).unwrap();
        t.join().unwrap();
        assert_eq!(got.json, b"{\"op\":\"ctc\"}");
        assert_eq!(got.blobs, vec![pcm]);
    }
}
