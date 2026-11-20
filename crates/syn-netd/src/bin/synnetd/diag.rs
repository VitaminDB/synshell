//! Сокеты системы через `NETLINK_SOCK_DIAG` (как `ss`): адреса, uid, inode
//! и cookie — тот же, что видит BPF (`bpf_get_socket_cookie`). Нужен для
//! сокетов, о которых учёт не узнал при `connect` (принятые входящие
//! соединения, открытые до запуска службы), и чтобы найти их процесс.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

const SOCK_DIAG_BY_FAMILY: u16 = 20;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_DUMP: u16 = 0x300;
const TCP_LISTEN: u8 = 10;

#[derive(Clone, Debug)]
pub struct Sock {
    pub cookie: u64,
    pub proto: u8,
    pub lport: u16,
    pub remote: IpAddr,
    pub rport: u16,
    pub state: u8,
    pub uid: u32,
    pub inode: u64,
}

fn ip(family: u8, b: &[u8]) -> IpAddr {
    if family == libc::AF_INET as u8 {
        IpAddr::from([b[0], b[1], b[2], b[3]])
    } else {
        let mut a = [0u8; 16];
        a.copy_from_slice(&b[..16]);
        let v6 = std::net::Ipv6Addr::from(a);
        v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6))
    }
}

fn dump_one(fd: i32, family: u8, proto: u8, out: &mut Vec<Sock>) -> std::io::Result<()> {
    // nlmsghdr (16) + inet_diag_req_v2 (56).
    let mut req = [0u8; 72];
    req[0..4].copy_from_slice(&72u32.to_ne_bytes());
    req[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    req[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    req[16] = family;
    req[17] = proto;
    req[20..24].copy_from_slice(&u32::MAX.to_ne_bytes()); // все состояния
    if unsafe { libc::send(fd, req.as_ptr().cast(), req.len(), 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buf = vec![0u8; 65536];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let b = &buf[..n as usize];
        let mut off = 0;
        while off + 16 <= b.len() {
            let len = u32::from_ne_bytes(b[off..off + 4].try_into().unwrap()) as usize;
            let kind = u16::from_ne_bytes(b[off + 4..off + 6].try_into().unwrap());
            if len < 16 || off + len > b.len() {
                return Ok(());
            }
            if kind == libc::NLMSG_DONE as u16 || kind == libc::NLMSG_ERROR as u16 {
                return Ok(());
            }
            let m = &b[off + 16..off + len];
            if m.len() >= 72 {
                let u32at = |o: usize| u32::from_ne_bytes(m[o..o + 4].try_into().unwrap());
                let fam = m[0];
                out.push(Sock {
                    cookie: u32at(44) as u64 | (u32at(48) as u64) << 32,
                    proto,
                    lport: u16::from_be_bytes([m[4], m[5]]),
                    rport: u16::from_be_bytes([m[6], m[7]]),
                    remote: ip(fam, &m[24..40]),
                    state: m[1],
                    uid: u32at(64),
                    inode: u32at(68) as u64,
                });
            }
            off += (len + 3) & !3;
        }
    }
}

/// Все сокеты TCP и UDP (IPv4 и IPv6).
pub fn dump() -> Vec<Sock> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_SOCK_DIAG) };
    if fd < 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for family in [libc::AF_INET as u8, libc::AF_INET6 as u8] {
        for proto in [libc::IPPROTO_TCP as u8, libc::IPPROTO_UDP as u8] {
            if let Err(e) = dump_one(fd, family, proto, &mut out) {
                tracing::debug!("sock_diag {family}/{proto}: {e}");
            }
        }
    }
    unsafe { libc::close(fd) };
    out
}

/// Порты, которые слушают службы этой машины (TCP LISTEN и UDP без адреса назначения).
pub fn listening(socks: &[Sock]) -> HashSet<(u8, u16)> {
    socks.iter().filter(|s| (s.proto == libc::IPPROTO_TCP as u8 && s.state == TCP_LISTEN) || (s.proto == libc::IPPROTO_UDP as u8 && s.rport == 0)).map(|s| (s.proto, s.lport)).collect()
}

/// Процессы, владеющие сокетами с этими inode (обход `/proc/*/fd`).
pub fn owners(inodes: &HashSet<u64>) -> HashMap<u64, i32> {
    let mut out = HashMap::new();
    if inodes.is_empty() {
        return out;
    }
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        for f in std::fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten() {
            let Ok(l) = std::fs::read_link(f.path()) else { continue };
            let l = l.to_string_lossy();
            if let Some(ino) = l.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')).and_then(|s| s.parse::<u64>().ok()) {
                if inodes.contains(&ino) {
                    out.entry(ino).or_insert(pid);
                }
            }
        }
        if out.len() == inodes.len() {
            break;
        }
    }
    out
}
