//! BPF-программы учёта трафика, собранные вручную (как и метки в
//! [`crate::bpf`]): без libbpf и компилятора BPF. Все прикреплены к корневой
//! cgroup через BPF link — служба завершилась, и они сняты сами.
//!
//! * `cgroup_skb` (вход и выход): байты каждого сокета по его cookie
//!   (`bytes`: cookie → принято, передано, интерфейс последнего пакета).
//!   Петля (`lo`) не считается.
//! * `cgroup/connect4|6`, `cgroup/sendmsg4|6`: куда соединяется или пишет
//!   процесс (`conns`: cookie + адрес + порт + протокол → tgid, uid, время,
//!   имя процесса). Повтор того же адреса — один поиск в таблице.
//! * `cgroup/sock_release`: сокет закрыт (`closed`: cookie → время).
//!
//! Служба раз в секунду читает таблицы, считает разности и убирает
//! закрытые сокеты.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::bpf::{bpf, insn, load_typed, put32, put64};

const BPF_MAP_CREATE: libc::c_long = 0;
const BPF_MAP_LOOKUP_ELEM: libc::c_long = 1;
const BPF_MAP_DELETE_ELEM: libc::c_long = 3;
const BPF_MAP_GET_NEXT_KEY: libc::c_long = 4;
const BPF_LINK_CREATE: libc::c_long = 28;
const BPF_MAP_TYPE_LRU_HASH: u32 = 9;

const PROG_CGROUP_SKB: u32 = 8;
const PROG_CGROUP_SOCK: u32 = 9;
const PROG_CGROUP_SOCK_ADDR: u32 = 18;
const AT_INGRESS: u32 = 0;
const AT_EGRESS: u32 = 1;
const AT_CONNECT4: u32 = 10;
const AT_CONNECT6: u32 = 11;
const AT_SENDMSG4: u32 = 14;
const AT_SENDMSG6: u32 = 15;
const AT_SOCK_RELEASE: u32 = 34;

const F_MAP_LOOKUP: i32 = 1;
const F_MAP_UPDATE: i32 = 2;
const F_PID_TGID: i32 = 14;
const F_UID_GID: i32 = 15;
const F_COMM: i32 = 16;
const F_SOCKET_COOKIE: i32 = 46;
const F_KTIME_BOOT: i32 = 125;
const BPF_NOEXIST: i32 = 1;

pub const BYTES_ENTRIES: u32 = 65536;
pub const CONNS_ENTRIES: u32 = 65536;

/// Сборщик с метками переходов.
#[derive(Default)]
struct Asm {
    code: Vec<[u8; 8]>,
    labels: HashMap<&'static str, usize>,
    fix: Vec<(usize, &'static str)>,
}

impl Asm {
    fn i(&mut self, code: u8, dst: u8, src: u8, off: i16, imm: i32) -> &mut Self {
        self.code.push(insn(code, dst, src, off, imm));
        self
    }
    fn mov(&mut self, dst: u8, src: u8) -> &mut Self {
        self.i(0xbf, dst, src, 0, 0)
    }
    fn movi(&mut self, dst: u8, imm: i32) -> &mut Self {
        self.i(0xb7, dst, 0, 0, imm)
    }
    fn addi(&mut self, dst: u8, imm: i32) -> &mut Self {
        self.i(0x07, dst, 0, 0, imm)
    }
    fn ldw(&mut self, dst: u8, src: u8, off: i16) -> &mut Self {
        self.i(0x61, dst, src, off, 0)
    }
    fn stw(&mut self, dst: u8, off: i16, src: u8) -> &mut Self {
        self.i(0x63, dst, src, off, 0)
    }
    fn stdw(&mut self, dst: u8, off: i16, src: u8) -> &mut Self {
        self.i(0x7b, dst, src, off, 0)
    }
    fn stwi(&mut self, dst: u8, off: i16, imm: i32) -> &mut Self {
        self.i(0x62, dst, 0, off, imm)
    }
    fn stdwi(&mut self, dst: u8, off: i16, imm: i32) -> &mut Self {
        self.i(0x7a, dst, 0, off, imm)
    }
    fn call(&mut self, f: i32) -> &mut Self {
        self.i(0x85, 0, 0, 0, f)
    }
    /// `r_dst = map` (ld_imm64, BPF_PSEUDO_MAP_FD).
    fn map(&mut self, dst: u8, fd: &OwnedFd) -> &mut Self {
        self.i(0x18, dst, 1, 0, fd.as_raw_fd());
        self.i(0, 0, 0, 0, 0)
    }
    fn jeqi(&mut self, dst: u8, imm: i32, l: &'static str) -> &mut Self {
        self.fix.push((self.code.len(), l));
        self.i(0x15, dst, 0, 0, imm)
    }
    fn jnei(&mut self, dst: u8, imm: i32, l: &'static str) -> &mut Self {
        self.fix.push((self.code.len(), l));
        self.i(0x55, dst, 0, 0, imm)
    }
    fn ja(&mut self, l: &'static str) -> &mut Self {
        self.fix.push((self.code.len(), l));
        self.i(0x05, 0, 0, 0, 0)
    }
    fn label(&mut self, l: &'static str) -> &mut Self {
        self.labels.insert(l, self.code.len());
        self
    }
    /// `r0 = 1; exit` — пропустить пакет, разрешить вызов.
    fn allow(&mut self) -> &mut Self {
        self.movi(0, 1);
        self.i(0x95, 0, 0, 0, 0)
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, l) in std::mem::take(&mut self.fix) {
            let to = self.labels[l] as i64;
            let off = (to - at as i64 - 1) as i16;
            self.code[at][2..4].copy_from_slice(&off.to_ne_bytes());
        }
        self.code.concat()
    }
}

fn map_create(kind: u32, key: u32, value: u32, entries: u32, name: &[u8]) -> std::io::Result<OwnedFd> {
    let mut attr = [0u8; 72];
    put32(&mut attr, 0, kind);
    put32(&mut attr, 4, key);
    put32(&mut attr, 8, value);
    put32(&mut attr, 12, entries);
    let n = name.len().min(15);
    attr[28..28 + n].copy_from_slice(&name[..n]); // map_name
    let fd = bpf(BPF_MAP_CREATE, &mut attr);
    if fd < 0 {
        return Err(std::io::Error::other(format!("BPF_MAP_CREATE: {}", std::io::Error::last_os_error())));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn link(prog: &OwnedFd, cgroup: &std::fs::File, attach: u32) -> std::io::Result<OwnedFd> {
    let mut attr = [0u8; 64];
    put32(&mut attr, 0, prog.as_raw_fd() as u32);
    put32(&mut attr, 4, cgroup.as_raw_fd() as u32);
    put32(&mut attr, 8, attach);
    let fd = bpf(BPF_LINK_CREATE, &mut attr);
    if fd < 0 {
        return Err(std::io::Error::other(format!("BPF_LINK_CREATE ({attach}): {}", std::io::Error::last_os_error())));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

/// Таблица BPF: чтение, удаление, обход ключей.
pub struct Map {
    fd: OwnedFd,
    key: usize,
    value: usize,
}

impl Map {
    pub fn lookup(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut v = vec![0u8; self.value];
        let mut attr = [0u8; 32];
        put32(&mut attr, 0, self.fd.as_raw_fd() as u32);
        put64(&mut attr, 8, key.as_ptr() as u64);
        put64(&mut attr, 16, v.as_mut_ptr() as u64);
        (bpf(BPF_MAP_LOOKUP_ELEM, &mut attr) == 0).then_some(v)
    }

    pub fn delete(&self, key: &[u8]) {
        let mut attr = [0u8; 32];
        put32(&mut attr, 0, self.fd.as_raw_fd() as u32);
        put64(&mut attr, 8, key.as_ptr() as u64);
        bpf(BPF_MAP_DELETE_ELEM, &mut attr);
    }

    /// Все ключи (до `limit`).
    pub fn keys(&self, limit: usize) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        let mut next = vec![0u8; self.key];
        loop {
            let mut attr = [0u8; 32];
            put32(&mut attr, 0, self.fd.as_raw_fd() as u32);
            // Первый вызов — без ключа: первый элемент.
            put64(&mut attr, 8, out.last().map(|k| k.as_ptr() as u64).unwrap_or(0));
            put64(&mut attr, 16, next.as_mut_ptr() as u64);
            if bpf(BPF_MAP_GET_NEXT_KEY, &mut attr) != 0 || out.len() >= limit {
                break;
            }
            // Хеш-таблица, изменившаяся во время обхода, может начать сначала.
            if !seen.insert(next.clone()) {
                break;
            }
            out.push(next.clone());
        }
        out
    }

    /// Все пары ключ — значение.
    pub fn entries(&self, limit: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.keys(limit).into_iter().filter_map(|k| self.lookup(&k).map(|v| (k, v))).collect()
    }
}

/// Программы и таблицы учёта.
pub struct Accounting {
    pub bytes: Map,
    pub conns: Map,
    pub closed: Map,
    /// Имя процесса записывается (у старых ядер `get_current_comm` в cgroup-программах нет).
    pub has_comm: bool,
    _keep: Vec<OwnedFd>,
}

/// Значение `bytes`: принято, передано, интерфейс.
pub const BYTES_VALUE: u32 = 24;
/// Ключ `conns`: cookie, адрес (16), порт (сетевой порядок, u32), семейство << 16 | протокол.
pub const CONNS_KEY: u32 = 32;
/// Значение `conns`: tgid, uid, время (ktime_get_boot_ns), имя процесса (16).
pub const CONNS_VALUE: u32 = 32;

/// `cgroup_skb`: прибавить длину пакета к полю `field` (0 — принято, 8 — передано).
fn skb_prog(bytes: &OwnedFd, field: i16) -> Vec<u8> {
    let mut a = Asm::default();
    a.mov(6, 1)
        .ldw(7, 6, 40) // r7 = skb->ifindex
        .jeqi(7, 1, "out") // петля
        .mov(1, 6)
        .call(F_SOCKET_COOKIE)
        .stdw(10, -8, 0)
        .map(1, bytes)
        .mov(2, 10)
        .addi(2, -8)
        .call(F_MAP_LOOKUP)
        .jeqi(0, 0, "new")
        .ldw(1, 6, 0) // r1 = skb->len
        .i(0xdb, 0, 1, field, 0) // lock *(u64 *)(r0 + field) += r1
        .stw(0, 16, 7)
        .ja("out")
        .label("new")
        .ldw(1, 6, 0)
        .stdwi(10, -32, 0)
        .stdwi(10, -24, 0)
        .stdwi(10, -16, 0)
        .stdw(10, -32 + field, 1)
        .stw(10, -16, 7)
        .map(1, bytes)
        .mov(2, 10)
        .addi(2, -8)
        .mov(3, 10)
        .addi(3, -32)
        .movi(4, BPF_NOEXIST)
        .call(F_MAP_UPDATE)
        .label("out")
        .allow();
    a.finish()
}

/// `cgroup/connect*`, `cgroup/sendmsg*`: записать адрес назначения.
fn addr_prog(conns: &OwnedFd, v6: bool, comm: bool) -> Vec<u8> {
    let mut a = Asm::default();
    // Ключ: r10-48 … r10-16.
    a.mov(6, 1).mov(1, 6).call(F_SOCKET_COOKIE).stdw(10, -48, 0);
    if v6 {
        for (i, off) in [8i16, 12, 16, 20].into_iter().enumerate() {
            a.ldw(1, 6, off).stw(10, -40 + 4 * i as i16, 1);
        }
    } else {
        a.ldw(1, 6, 4).stw(10, -40, 1).stwi(10, -36, 0).stdwi(10, -32, 0);
    }
    a.ldw(1, 6, 24) // user_port
        .stw(10, -24, 1)
        .ldw(1, 6, 36) // protocol
        .i(0x47, 1, 0, 0, if v6 { 6 << 16 } else { 4 << 16 }) // r1 |= семейство
        .stw(10, -20, 1)
        // Уже записан — выйти.
        .map(1, conns)
        .mov(2, 10)
        .addi(2, -48)
        .call(F_MAP_LOOKUP)
        .jnei(0, 0, "out")
        // Значение: r10-80 … r10-48.
        .call(F_PID_TGID)
        .i(0x77, 0, 0, 0, 32)
        .stw(10, -80, 0)
        .call(F_UID_GID)
        .stw(10, -76, 0)
        .call(F_KTIME_BOOT)
        .stdw(10, -72, 0);
    if comm {
        a.mov(1, 10).addi(1, -64).movi(2, 16).call(F_COMM);
    } else {
        a.stdwi(10, -64, 0).stdwi(10, -56, 0);
    }
    a.map(1, conns).mov(2, 10).addi(2, -48).mov(3, 10).addi(3, -80).movi(4, BPF_NOEXIST).call(F_MAP_UPDATE).label("out").allow();
    a.finish()
}

/// `cgroup/sock_release`: сокет закрыт.
fn release_prog(closed: &OwnedFd) -> Vec<u8> {
    let mut a = Asm::default();
    a.mov(6, 1)
        .call(F_SOCKET_COOKIE)
        .stdw(10, -8, 0)
        .call(F_KTIME_BOOT)
        .stdw(10, -16, 0)
        .map(1, closed)
        .mov(2, 10)
        .addi(2, -8)
        .mov(3, 10)
        .addi(3, -16)
        .movi(4, 0)
        .call(F_MAP_UPDATE)
        .allow();
    a.finish()
}

impl Accounting {
    pub fn new() -> std::io::Result<Accounting> {
        let bytes = map_create(BPF_MAP_TYPE_LRU_HASH, 8, BYTES_VALUE, BYTES_ENTRIES, b"synnet_bytes")?;
        let conns = map_create(BPF_MAP_TYPE_LRU_HASH, CONNS_KEY, CONNS_VALUE, CONNS_ENTRIES, b"synnet_conns")?;
        let closed = map_create(BPF_MAP_TYPE_LRU_HASH, 8, 8, 65536, b"synnet_closed")?;
        let root = std::fs::File::open("/sys/fs/cgroup")?;
        let mut keep = Vec::new();
        let mut attach = |code: Vec<u8>, name: &[u8], ptype: u32, at: u32| -> std::io::Result<()> {
            let prog = load_typed(&code, name, ptype, at)?;
            keep.push(link(&prog, &root, at)?);
            keep.push(prog);
            Ok(())
        };
        attach(skb_prog(&bytes, 0), b"synnet_rx\0", PROG_CGROUP_SKB, AT_INGRESS)?;
        attach(skb_prog(&bytes, 8), b"synnet_tx\0", PROG_CGROUP_SKB, AT_EGRESS)?;
        attach(release_prog(&closed), b"synnet_rel\0", PROG_CGROUP_SOCK, AT_SOCK_RELEASE)?;
        // Имя процесса: проверить, даёт ли ядро get_current_comm cgroup-программам.
        let has_comm = load_typed(&addr_prog(&conns, false, true), b"synnet_probe\0", PROG_CGROUP_SOCK_ADDR, AT_CONNECT4).is_ok();
        for (v6, at, name) in [(false, AT_CONNECT4, &b"synnet_c4\0"[..]), (true, AT_CONNECT6, b"synnet_c6\0"), (false, AT_SENDMSG4, b"synnet_s4\0"), (true, AT_SENDMSG6, b"synnet_s6\0")] {
            attach(addr_prog(&conns, v6, has_comm), name, PROG_CGROUP_SOCK_ADDR, at)?;
        }
        Ok(Accounting {
            bytes: Map { fd: bytes, key: 8, value: BYTES_VALUE as usize },
            conns: Map { fd: conns, key: CONNS_KEY as usize, value: CONNS_VALUE as usize },
            closed: Map { fd: closed, key: 8, value: 8 },
            has_comm,
            _keep: keep,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumps() {
        let mut a = Asm::default();
        a.jeqi(1, 0, "x").movi(0, 0).movi(0, 0).label("x").allow();
        let c = a.finish();
        assert_eq!(i16::from_ne_bytes([c[2], c[3]]), 2);
    }
}
