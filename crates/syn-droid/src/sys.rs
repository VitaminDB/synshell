//! Тонкие обёртки над системными вызовами для рантайма контейнера.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub fn cstr(p: impl AsRef<Path>) -> CString {
    CString::new(p.as_ref().as_os_str().as_bytes()).expect("путь с NUL")
}

fn errno<T: Default + PartialEq + From<i8>>(r: T, what: impl FnOnce() -> String) -> Result<T> {
    if r == T::from(-1) {
        let e = io::Error::last_os_error();
        bail!("{}: {e}", what());
    }
    Ok(r)
}

pub fn mount(src: &str, target: impl AsRef<Path>, fstype: &str, flags: libc::c_ulong, data: &str) -> Result<()> {
    let t = target.as_ref();
    let (s, ty, d) = (CString::new(src)?, CString::new(fstype)?, CString::new(data)?);
    let r = unsafe {
        libc::mount(
            s.as_ptr(),
            cstr(t).as_ptr(),
            if fstype.is_empty() { std::ptr::null() } else { ty.as_ptr() },
            flags,
            if data.is_empty() { std::ptr::null() } else { d.as_ptr().cast() },
        )
    };
    errno(r, || format!("mount {src} → {} ({fstype})", t.display())).map(drop)
}

pub fn bind(src: impl AsRef<Path>, target: impl AsRef<Path>, recursive: bool) -> Result<()> {
    let flags = libc::MS_BIND | if recursive { libc::MS_REC } else { 0 };
    mount(&src.as_ref().to_string_lossy(), target, "", flags, "")
}

/// Сделать уже смонтированное связывание только для чтения.
pub fn remount_ro(target: impl AsRef<Path>) -> Result<()> {
    mount("", target, "", libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY, "")
}

/// Точка монтирования-файл (для привязки узлов и сокетов).
pub fn touch(p: &Path) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    if !p.exists() {
        File::create(p).with_context(|| format!("создать {}", p.display()))?;
    }
    Ok(())
}

// --- loop -------------------------------------------------------------------------------------------

const LOOP_CTL_GET_FREE: libc::c_ulong = 0x4C82;
const LOOP_CONFIGURE: libc::c_ulong = 0x4C0A;
const LO_FLAGS_READ_ONLY: u32 = 1;
const LO_FLAGS_AUTOCLEAR: u32 = 4;

#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

#[repr(C)]
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo64,
    reserved: [u64; 8],
}

/// Подключить файл к свободному loop (только чтение, autoclear: устройство освободится с последним
/// размонтированием — то есть само, когда умрёт пространство имён контейнера). Узел `/dev/loopN` в ядре без
/// devtmpfs может ещё не существовать — тогда создаём свой в `dir`. Возвращённый дескриптор держать открытым
/// до монтирования: autoclear отключает устройство при последнем закрытии, если оно не смонтировано.
pub fn loop_attach(file: &Path, dir: &Path) -> Result<(PathBuf, File)> {
    let backing = File::open(file).with_context(|| format!("{}", file.display()))?;
    let ctl = OpenOptions::new().read(true).write(true).open("/dev/loop-control").context("/dev/loop-control")?;
    let n = errno(unsafe { libc::ioctl(ctl.as_raw_fd(), LOOP_CTL_GET_FREE as _) }, || "LOOP_CTL_GET_FREE".into())?;
    let mut node = PathBuf::from(format!("/dev/loop{n}"));
    if !node.exists() {
        std::fs::create_dir_all(dir)?;
        node = dir.join(format!("loop{n}"));
        let _ = std::fs::remove_file(&node);
        let dev = libc::makedev(7, n as u32);
        errno(unsafe { libc::mknod(cstr(&node).as_ptr(), libc::S_IFBLK | 0o600, dev) }, || {
            format!("mknod {}", node.display())
        })?;
    }
    let dev = OpenOptions::new().read(true).open(&node).with_context(|| format!("{}", node.display()))?;
    let mut cfg: LoopConfig = unsafe { std::mem::zeroed() };
    cfg.fd = backing.as_raw_fd() as u32;
    cfg.info.lo_flags = LO_FLAGS_READ_ONLY | LO_FLAGS_AUTOCLEAR;
    let name = file.as_os_str().as_bytes();
    let l = name.len().min(63);
    cfg.info.lo_file_name[..l].copy_from_slice(&name[..l]);
    errno(unsafe { libc::ioctl(dev.as_raw_fd(), LOOP_CONFIGURE as _, &cfg) }, || {
        format!("LOOP_CONFIGURE {}", node.display())
    })?;
    Ok((node, dev))
}

/// Смонтировать образ (ext4 или erofs) только для чтения через loop.
pub fn mount_image(img: &Path, target: &Path, loopdir: &Path) -> Result<()> {
    let (dev, _hold) = loop_attach(img, loopdir)?;
    let dev = dev.to_string_lossy();
    let flags = libc::MS_RDONLY | libc::MS_NODEV;
    mount(&dev, target, "ext4", flags, "").or_else(|e| mount(&dev, target, "erofs", flags, "").map_err(|_| e))
}

// --- binderfs ---------------------------------------------------------------------------------------

/// `_IOWR('b', 1, struct binderfs_device)`; структура — имя[256] + major + minor.
const BINDER_CTL_ADD: libc::c_ulong = (3 << 30) | (264 << 16) | ((b'b' as libc::c_ulong) << 8) | 1;

#[repr(C)]
struct BinderfsDevice {
    name: [u8; 256],
    major: u32,
    minor: u32,
}

/// Смонтировать binderfs (если ещё нет) и создать в нём узлы `names` (существующие — оставить).
pub fn binderfs_nodes(root: &str, names: &[&str]) -> Result<()> {
    let ctl = Path::new(root).join("binder-control");
    if !ctl.exists() {
        std::fs::create_dir_all(root)?;
        mount("binder", root, "binder", 0, "stats=global")?;
    }
    let f = OpenOptions::new().read(true).write(true).open(&ctl).context("binder-control")?;
    for n in names {
        if Path::new(root).join(n).exists() {
            continue;
        }
        let mut d = BinderfsDevice { name: [0; 256], major: 0, minor: 0 };
        d.name[..n.len()].copy_from_slice(n.as_bytes());
        errno(unsafe { libc::ioctl(f.as_raw_fd(), BINDER_CTL_ADD as _, &mut d) }, || format!("binderfs: {n}"))?;
    }
    Ok(())
}

// --- права процесса -----------------------------------------------------------------------------

/// Capabilities, которые остаются у Android (как `lxc.cap.keep` Waydroid); остальные убираются из
/// ограничивающего набора.
const KEEP_CAPS: &[i32] = &[
    0,  // chown
    1,  // dac_override
    2,  // dac_read_search
    3,  // fowner
    4,  // fsetid
    5,  // kill
    6,  // setgid
    7,  // setuid
    8,  // setpcap
    10, // net_bind_service
    12, // net_admin
    13, // net_raw
    14, // ipc_lock
    18, // sys_chroot
    19, // sys_ptrace
    21, // sys_admin
    23, // sys_nice
    24, // sys_resource
    25, // sys_time
    27, // mknod
    30, // audit_control
    34, // syslog
    35, // wake_alarm
    36, // block_suspend
];

pub fn drop_caps() -> Result<()> {
    for cap in 0..64 {
        if KEEP_CAPS.contains(&cap) {
            continue;
        }
        let r = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap as libc::c_ulong, 0, 0, 0) };
        if r != 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINVAL) {
                break; // дальше ядро capabilities не знает
            }
            bail!("PR_CAPBSET_DROP {cap}: {e}");
        }
    }
    Ok(())
}

/// Фильтр seccomp (как `waydroid.seccomp`): загрузка модулей ядра, kexec, reboot, swap — запрещены;
/// подкрутка часов и ключи ядра — «успешно» без действия; open_by_handle_at — ENOSYS. Только aarch64.
pub fn install_seccomp() -> Result<()> {
    #[cfg(target_arch = "aarch64")]
    {
        const AUDIT_ARCH_AARCH64: u32 = 0xC000_00B7;
        // (номер, ответ) — номера системных вызовов aarch64
        let rules: &[(u32, u32)] = &[
            (105, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // init_module
            (273, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // finit_module
            (106, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // delete_module
            (104, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // kexec_load
            (294, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // kexec_file_load
            (142, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // reboot
            (224, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // swapon
            (225, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32), // swapoff
            (265, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32), // open_by_handle_at
            (171, libc::SECCOMP_RET_ERRNO),                       // adjtimex
            (266, libc::SECCOMP_RET_ERRNO),                       // clock_adjtime
            (112, libc::SECCOMP_RET_ERRNO),                       // clock_settime
            (170, libc::SECCOMP_RET_ERRNO),                       // settimeofday
            (217, libc::SECCOMP_RET_ERRNO),                       // add_key
            (218, libc::SECCOMP_RET_ERRNO),                       // request_key
            (219, libc::SECCOMP_RET_ERRNO),                       // keyctl
        ];
        let stmt = |code: u16, k: u32| libc::sock_filter { code, jt: 0, jf: 0, k };
        let jeq = |k: u32, jt: u8, jf: u8| libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt,
            jf,
            k,
        };
        let ld = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
        let ret = (libc::BPF_RET | libc::BPF_K) as u16;
        let mut p = vec![
            stmt(ld, 4), // seccomp_data.arch
            jeq(AUDIT_ARCH_AARCH64, 1, 0),
            stmt(ret, libc::SECCOMP_RET_ALLOW),
            stmt(ld, 0), // seccomp_data.nr
        ];
        for (nr, action) in rules {
            p.push(jeq(*nr, 0, 1));
            p.push(stmt(ret, *action));
        }
        p.push(stmt(ret, libc::SECCOMP_RET_ALLOW));
        let prog = libc::sock_fprog { len: p.len() as u16, filter: p.as_mut_ptr() };
        errno(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }, || "no_new_privs".into())?;
        errno(
            unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER as libc::c_ulong, &prog) },
            || "seccomp".into(),
        )?;
    }
    Ok(())
}

pub fn sethostname(name: &str) -> Result<()> {
    errno(unsafe { libc::sethostname(name.as_ptr().cast(), name.len()) }, || "sethostname".into()).map(drop)
}

/// Сменить корень на `root` (уже точку монтирования) и отцепить старый.
pub fn pivot_root(root: &Path) -> Result<()> {
    std::env::set_current_dir(root)?;
    let dot = cstr(".");
    errno(unsafe { libc::syscall(libc::SYS_pivot_root, dot.as_ptr(), dot.as_ptr()) as i32 }, || {
        "pivot_root".into()
    })?;
    errno(unsafe { libc::umount2(dot.as_ptr(), libc::MNT_DETACH) }, || "umount старого корня".into())?;
    std::env::set_current_dir("/")?;
    Ok(())
}

/// pidfd процесса (для setns и надёжной проверки «жив ли»).
pub fn pidfd_open(pid: i32) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let r = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(r as i32) })
}
