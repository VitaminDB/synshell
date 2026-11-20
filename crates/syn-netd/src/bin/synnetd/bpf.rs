//! BPF-программа cgroup (`BPF_CGROUP_INET_SOCK_CREATE`): каждому новому
//! сокету IPv4/IPv6 процессов cgroup ставит метку (`SO_MARK`). Метка
//! участвует в выборе маршрута уже при `connect()`/первой отправке, поэтому
//! и адрес источника берётся от нужного канала. Программа — четыре
//! инструкции, собранные вручную (без libbpf и компилятора BPF):
//!
//! ```text
//! r2 = MARK
//! *(u32 *)(r1 + offsetof(struct bpf_sock, mark)) = r2
//! r0 = 1          // разрешить создание сокета
//! exit
//! ```
//!
//! Прикреплённая программа остаётся на cgroup и после выхода службы.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const BPF_PROG_LOAD: libc::c_long = 5;
const BPF_PROG_ATTACH: libc::c_long = 8;
const BPF_PROG_QUERY: libc::c_long = 16;
const BPF_PROG_TYPE_CGROUP_SOCK: u32 = 9;
const BPF_CGROUP_INET_SOCK_CREATE: u32 = 2;
const BPF_F_ALLOW_OVERRIDE: u32 = 1;
const BPF_F_ALLOW_MULTI: u32 = 2;
/// `offsetof(struct bpf_sock, mark)`: bound_dev_if, family, type, protocol — по 4 байта.
const OFF_MARK: i16 = 16;

pub fn insn(code: u8, dst: u8, src: u8, off: i16, imm: i32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0] = code;
    b[1] = (dst & 0xf) | (src << 4);
    b[2..4].copy_from_slice(&off.to_ne_bytes());
    b[4..8].copy_from_slice(&imm.to_ne_bytes());
    b
}

pub fn bpf(cmd: libc::c_long, attr: &mut [u8]) -> libc::c_long {
    unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), attr.len() as libc::c_uint) }
}

pub fn put32(a: &mut [u8], off: usize, v: u32) {
    a[off..off + 4].copy_from_slice(&v.to_ne_bytes());
}

pub fn put64(a: &mut [u8], off: usize, v: u64) {
    a[off..off + 8].copy_from_slice(&v.to_ne_bytes());
}

/// Загрузить программу, ставящую метку `mark`.
pub fn load(mark: u32) -> std::io::Result<OwnedFd> {
    let mut code = Vec::with_capacity(32);
    code.extend_from_slice(&insn(0xb7, 2, 0, 0, mark as i32)); // mov64 r2, imm
    code.extend_from_slice(&insn(0x63, 1, 2, OFF_MARK, 0)); // stx w [r1+16], r2
    code.extend_from_slice(&insn(0xb7, 0, 0, 0, 1)); // mov64 r0, 1
    code.extend_from_slice(&insn(0x95, 0, 0, 0, 0)); // exit
    load_code(&code, b"synnet_mark\0")
}

/// Загрузить программу `BPF_CGROUP_INET_SOCK_CREATE`.
fn load_code(code: &[u8], name: &[u8]) -> std::io::Result<OwnedFd> {
    load_typed(code, name, BPF_PROG_TYPE_CGROUP_SOCK, BPF_CGROUP_INET_SOCK_CREATE)
}

/// Загрузить программу типа `prog_type` для точки `attach_type`.
pub fn load_typed(code: &[u8], name: &[u8], prog_type: u32, attach_type: u32) -> std::io::Result<OwnedFd> {
    let license = c"GPL";
    let mut log = vec![0u8; 16384];
    let mut attr = [0u8; 144];
    put32(&mut attr, 0, prog_type);
    put32(&mut attr, 4, (code.len() / 8) as u32);
    put64(&mut attr, 8, code.as_ptr() as u64);
    put64(&mut attr, 16, license.as_ptr() as u64);
    put32(&mut attr, 24, 1); // log_level
    put32(&mut attr, 28, log.len() as u32);
    put64(&mut attr, 32, log.as_mut_ptr() as u64);
    let n = name.len().min(16);
    attr[48..48 + n].copy_from_slice(&name[..n]);
    put32(&mut attr, 68, attach_type); // expected_attach_type
    let fd = bpf(BPF_PROG_LOAD, &mut attr);
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        let msg = String::from_utf8_lossy(&log[..log.iter().position(|b| *b == 0).unwrap_or(0)]).trim().to_string();
        return Err(std::io::Error::new(e.kind(), format!("BPF_PROG_LOAD: {e} {msg}")));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

/// Сколько программ создания сокетов прикреплено к самой cgroup.
pub fn attached_count(cgroup: &std::fs::File) -> std::io::Result<u32> {
    let mut attr = [0u8; 64];
    put32(&mut attr, 0, cgroup.as_raw_fd() as u32);
    put32(&mut attr, 4, BPF_CGROUP_INET_SOCK_CREATE);
    if bpf(BPF_PROG_QUERY, &mut attr) < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(u32::from_ne_bytes(attr[24..28].try_into().unwrap()))
}

/// Прикрепить программу к cgroup (каталог в /sys/fs/cgroup), если там ещё
/// нет своей.
pub fn attach(cgroup_dir: &std::path::Path, prog: &OwnedFd) -> std::io::Result<()> {
    let dir = std::fs::File::open(cgroup_dir)?;
    if attached_count(&dir)? > 0 {
        return Ok(());
    }
    let mut last = None;
    // Родитель с программой без флагов не даёт прикрепить свою; MULTI
    // уживается с программами systemd и других служб.
    for flags in [BPF_F_ALLOW_MULTI, BPF_F_ALLOW_OVERRIDE] {
        let mut attr = [0u8; 64];
        put32(&mut attr, 0, dir.as_raw_fd() as u32);
        put32(&mut attr, 4, prog.as_raw_fd() as u32);
        put32(&mut attr, 8, BPF_CGROUP_INET_SOCK_CREATE);
        put32(&mut attr, 12, flags);
        if bpf(BPF_PROG_ATTACH, &mut attr) == 0 {
            return Ok(());
        }
        last = Some(std::io::Error::last_os_error());
    }
    Err(last.unwrap())
}

// ─── Метка по процессу (без гонки с переносом cgroup) ───────────────────────
//
// Перенос процесса в cgroup занимает 1–5 мс после exec (миграция cgroup в
// ядре), а короткая программа успевает за это время открыть соединение.
// Поэтому на корневую cgroup вешается ещё одна программа: метка берётся из
// таблицы `tgid → метка`, куда служба заносит процесс сразу при событии exec
// (микросекунды) и его потомков — при fork:
//
// ```text
// r6 = r1
// r0 = get_current_pid_tgid(); r0 >>= 32
// *(u32 *)(r10 - 4) = r0
// r0 = map_lookup_elem(map, r10 - 4)
// if r0 == 0 goto out
// *(u32 *)(r6 + 16) = *(u32 *)r0
// out: r0 = 1; exit
// ```
//
// Программа прикреплена через BPF link: служба завершилась (даже аварийно) —
// дескриптор закрыт и программа снята сама.

const BPF_MAP_CREATE: libc::c_long = 0;
const BPF_MAP_UPDATE_ELEM: libc::c_long = 2;
const BPF_MAP_DELETE_ELEM: libc::c_long = 3;
const BPF_LINK_CREATE: libc::c_long = 28;
const BPF_MAP_TYPE_HASH: u32 = 1;
const FUNC_MAP_LOOKUP_ELEM: i32 = 1;
const FUNC_GET_CURRENT_PID_TGID: i32 = 14;

/// Таблица `tgid → метка` и программа, прикреплённая к корневой cgroup.
pub struct TgidMarks {
    map: OwnedFd,
    _prog: OwnedFd,
    _link: OwnedFd,
}

impl TgidMarks {
    pub fn new() -> std::io::Result<TgidMarks> {
        let mut attr = [0u8; 72];
        put32(&mut attr, 0, BPF_MAP_TYPE_HASH);
        put32(&mut attr, 4, 4);
        put32(&mut attr, 8, 4);
        put32(&mut attr, 12, 16384);
        attr[28..28 + 12].copy_from_slice(b"synnet_tgid\0"); // map_name
        let fd = bpf(BPF_MAP_CREATE, &mut attr);
        if fd < 0 {
            return Err(std::io::Error::other(format!("BPF_MAP_CREATE: {}", std::io::Error::last_os_error())));
        }
        let map = unsafe { OwnedFd::from_raw_fd(fd as i32) };

        let mut code = Vec::with_capacity(14 * 8);
        code.extend_from_slice(&insn(0xbf, 6, 1, 0, 0)); // r6 = r1
        code.extend_from_slice(&insn(0x85, 0, 0, 0, FUNC_GET_CURRENT_PID_TGID));
        code.extend_from_slice(&insn(0x77, 0, 0, 0, 32)); // r0 >>= 32
        code.extend_from_slice(&insn(0x63, 10, 0, -4, 0)); // *(u32*)(r10-4) = r0
        code.extend_from_slice(&insn(0xbf, 2, 10, 0, 0)); // r2 = r10
        code.extend_from_slice(&insn(0x07, 2, 0, 0, -4)); // r2 += -4
        code.extend_from_slice(&insn(0x18, 1, 1, 0, map.as_raw_fd())); // r1 = map (ld_imm64, BPF_PSEUDO_MAP_FD)
        code.extend_from_slice(&insn(0, 0, 0, 0, 0));
        code.extend_from_slice(&insn(0x85, 0, 0, 0, FUNC_MAP_LOOKUP_ELEM));
        code.extend_from_slice(&insn(0x15, 0, 0, 2, 0)); // if r0 == 0 goto +2
        code.extend_from_slice(&insn(0x61, 1, 0, 0, 0)); // r1 = *(u32*)(r0+0)
        code.extend_from_slice(&insn(0x63, 6, 1, OFF_MARK, 0)); // ctx->mark = r1
        code.extend_from_slice(&insn(0xb7, 0, 0, 0, 1)); // r0 = 1
        code.extend_from_slice(&insn(0x95, 0, 0, 0, 0)); // exit
        let prog = load_code(&code, b"synnet_tgid\0")?;

        let root = std::fs::File::open("/sys/fs/cgroup")?;
        let mut attr = [0u8; 64];
        put32(&mut attr, 0, prog.as_raw_fd() as u32);
        put32(&mut attr, 4, root.as_raw_fd() as u32);
        put32(&mut attr, 8, BPF_CGROUP_INET_SOCK_CREATE);
        let fd = bpf(BPF_LINK_CREATE, &mut attr);
        if fd < 0 {
            return Err(std::io::Error::other(format!("BPF_LINK_CREATE: {}", std::io::Error::last_os_error())));
        }
        Ok(TgidMarks { map, _prog: prog, _link: unsafe { OwnedFd::from_raw_fd(fd as i32) } })
    }

    pub fn set(&self, tgid: i32, mark: u32) {
        let key = tgid as u32;
        let mut attr = [0u8; 32];
        put32(&mut attr, 0, self.map.as_raw_fd() as u32);
        put64(&mut attr, 8, &key as *const u32 as u64);
        put64(&mut attr, 16, &mark as *const u32 as u64);
        bpf(BPF_MAP_UPDATE_ELEM, &mut attr);
    }

    pub fn remove(&self, tgid: i32) {
        let key = tgid as u32;
        let mut attr = [0u8; 32];
        put32(&mut attr, 0, self.map.as_raw_fd() as u32);
        put64(&mut attr, 8, &key as *const u32 as u64);
        bpf(BPF_MAP_DELETE_ELEM, &mut attr);
    }
}
