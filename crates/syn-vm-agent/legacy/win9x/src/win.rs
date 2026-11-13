//! Импорты Win32, которые есть в Windows 95 (функции только NT — через GetProcAddress), и обёртки.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use alloc::string::String;
use alloc::vec::Vec;

pub type HANDLE = isize;
pub const INVALID: HANDLE = -1;

#[repr(C)]
pub struct Dcb {
    pub len: u32,
    pub baud: u32,
    pub flags: u32,
    pub reserved: u16,
    pub xon_lim: u16,
    pub xoff_lim: u16,
    pub byte_size: u8,
    pub parity: u8,
    pub stop_bits: u8,
    pub xon: u8,
    pub xoff: u8,
    pub err: u8,
    pub eof: u8,
    pub evt: u8,
    pub reserved1: u16,
}

#[repr(C)]
pub struct CommTimeouts {
    pub read_interval: u32,
    pub read_mult: u32,
    pub read_const: u32,
    pub write_mult: u32,
    pub write_const: u32,
}

#[repr(C)]
pub struct FindData {
    pub attrs: u32,
    pub created: [u32; 2],
    pub accessed: [u32; 2],
    pub written: [u32; 2],
    pub size_high: u32,
    pub size_low: u32,
    pub reserved: [u32; 2],
    pub name: [u8; 260],
    pub alt: [u8; 14],
}

#[repr(C)]
pub struct StartupInfo {
    pub cb: u32,
    pub reserved: *const u8,
    pub desktop: *const u8,
    pub title: *const u8,
    pub x: u32,
    pub y: u32,
    pub xs: u32,
    pub ys: u32,
    pub xc: u32,
    pub yc: u32,
    pub fill: u32,
    pub flags: u32,
    pub show: u16,
    pub reserved2: u16,
    pub reserved3: *const u8,
    pub stdin: HANDLE,
    pub stdout: HANDLE,
    pub stderr: HANDLE,
}

#[repr(C)]
pub struct ProcessInfo {
    pub process: HANDLE,
    pub thread: HANDLE,
    pub pid: u32,
    pub tid: u32,
}

#[repr(C)]
pub struct SecurityAttributes {
    pub len: u32,
    pub sd: *const u8,
    pub inherit: i32,
}

#[repr(C)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub dow: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub ms: u16,
}

#[repr(C)]
pub struct OsVersionInfo {
    pub size: u32,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub platform: u32,
    pub csd: [u8; 128],
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    pub fn ExitProcess(code: u32) -> !;
    pub fn GetTickCount() -> u32;
    pub fn Sleep(ms: u32);
    pub fn GetProcessHeap() -> HANDLE;
    pub fn HeapAlloc(heap: HANDLE, flags: u32, size: usize) -> *mut u8;
    pub fn HeapReAlloc(heap: HANDLE, flags: u32, p: *mut u8, size: usize) -> *mut u8;
    pub fn HeapFree(heap: HANDLE, flags: u32, p: *mut u8) -> i32;
    pub fn CreateFileA(name: *const u8, access: u32, share: u32, sa: *const SecurityAttributes, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    pub fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, read: *mut u32, ov: *mut u8) -> i32;
    pub fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut u8) -> i32;
    pub fn CloseHandle(h: HANDLE) -> i32;
    pub fn GetFileSize(h: HANDLE, high: *mut u32) -> u32;
    pub fn GetCommState(h: HANDLE, dcb: *mut Dcb) -> i32;
    pub fn SetCommState(h: HANDLE, dcb: *const Dcb) -> i32;
    pub fn SetCommTimeouts(h: HANDLE, t: *const CommTimeouts) -> i32;
    pub fn SetupComm(h: HANDLE, inq: u32, outq: u32) -> i32;
    pub fn FindFirstFileA(name: *const u8, data: *mut FindData) -> HANDLE;
    pub fn FindNextFileA(h: HANDLE, data: *mut FindData) -> i32;
    pub fn FindClose(h: HANDLE) -> i32;
    pub fn CreateDirectoryA(name: *const u8, sa: *const u8) -> i32;
    pub fn GetFileAttributesA(name: *const u8) -> u32;
    pub fn DeleteFileA(name: *const u8) -> i32;
    pub fn CopyFileA(from: *const u8, to: *const u8, fail_if_exists: i32) -> i32;
    pub fn CreateProcessA(app: *const u8, cmd: *mut u8, pa: *const SecurityAttributes, ta: *const SecurityAttributes, inherit: i32, flags: u32, env: *const u8, cwd: *const u8, si: *const StartupInfo, pi: *mut ProcessInfo) -> i32;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> i32;
    pub fn TerminateProcess(h: HANDLE, code: u32) -> i32;
    pub fn GetEnvironmentVariableA(name: *const u8, buf: *mut u8, n: u32) -> u32;
    pub fn GetTempPathA(n: u32, buf: *mut u8) -> u32;
    pub fn GetModuleFileNameA(m: isize, buf: *mut u8, n: u32) -> u32;
    pub fn GetVersionExA(v: *mut OsVersionInfo) -> i32;
    pub fn GetComputerNameA(buf: *mut u8, n: *mut u32) -> i32;
    pub fn SetSystemTime(t: *const SystemTime) -> i32;
    pub fn GetLastError() -> u32;
    pub fn GetModuleHandleA(name: *const u8) -> isize;
    pub fn GetProcAddress(m: isize, name: *const u8) -> *const u8;
    pub fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    pub fn WideCharToMultiByte(cp: u32, flags: u32, w: *const u16, wn: i32, s: *mut u8, n: i32, def: *const u8, used: *mut i32) -> i32;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn SetFilePointer(h: HANDLE, lo: i32, hi: *mut i32, method: u32) -> u32;
    pub fn GetWindowsDirectoryA(buf: *mut u8, n: u32) -> u32;
}

#[link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    pub fn ExitWindowsEx(flags: u32, reason: u32) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    pub fn RegCreateKeyExA(key: isize, sub: *const u8, r: u32, class: *const u8, opts: u32, sam: u32, sa: *const u8, out: *mut isize, disp: *mut u32) -> i32;
    pub fn RegSetValueExA(key: isize, name: *const u8, r: u32, ty: u32, data: *const u8, n: u32) -> i32;
    pub fn RegCloseKey(key: isize) -> i32;
    pub fn RegOpenKeyExA(key: isize, sub: *const u8, opts: u32, sam: u32, out: *mut isize) -> i32;
    pub fn RegDeleteValueA(key: isize, name: *const u8) -> i32;
}

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const OPEN_EXISTING: u32 = 3;
pub const CREATE_ALWAYS: u32 = 2;
pub const FILE_SHARE_RW: u32 = 3;
pub const ATTR_DIR: u32 = 0x10;
pub const NO_ATTRS: u32 = 0xFFFF_FFFF;
pub const HKLM: isize = 0x8000_0002u32 as i32 as isize;

/// Строка с нулём в конце.
pub fn cz(s: &str) -> Vec<u8> {
    let mut v = to_ansi(s);
    v.push(0);
    v
}

/// UTF-8 → кодовая страница системы (ANSI). Латиница — как есть; остальное — через UTF-16 (CP_UTF8 у
/// Windows 98/NT4 и новее; у Windows 95 нелатинские имена не переводятся).
pub fn to_ansi(s: &str) -> Vec<u8> {
    if s.is_ascii() {
        return s.as_bytes().to_vec();
    }
    unsafe {
        let n = MultiByteToWideChar(65001, 0, s.as_ptr(), s.len() as i32, core::ptr::null_mut(), 0);
        if n <= 0 {
            return s.as_bytes().to_vec();
        }
        let mut w = alloc::vec![0u16; n as usize];
        MultiByteToWideChar(65001, 0, s.as_ptr(), s.len() as i32, w.as_mut_ptr(), n);
        let m = WideCharToMultiByte(0, 0, w.as_ptr(), n, core::ptr::null_mut(), 0, core::ptr::null(), core::ptr::null_mut());
        let mut out = alloc::vec![0u8; m.max(0) as usize];
        WideCharToMultiByte(0, 0, w.as_ptr(), n, out.as_mut_ptr(), m, core::ptr::null(), core::ptr::null_mut());
        out
    }
}

/// Кодовая страница системы → UTF-8 (имена файлов, вывод команд).
pub fn from_ansi(b: &[u8]) -> String {
    if b.is_ascii() {
        return String::from_utf8_lossy(b).into_owned();
    }
    unsafe {
        let n = MultiByteToWideChar(0, 0, b.as_ptr(), b.len() as i32, core::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(b).into_owned();
        }
        let mut w = alloc::vec![0u16; n as usize];
        MultiByteToWideChar(0, 0, b.as_ptr(), b.len() as i32, w.as_mut_ptr(), n);
        String::from_utf16_lossy(&w)
    }
}

/// Строка из буфера до нуля.
pub fn zstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

pub fn env(name: &str) -> Option<String> {
    let mut buf = [0u8; 512];
    let n = unsafe { GetEnvironmentVariableA(cz(name).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| from_ansi(&buf[..n]))
}

/// Windows NT (XP) или 9x.
pub fn is_nt() -> bool {
    let mut v: OsVersionInfo = unsafe { core::mem::zeroed() };
    v.size = core::mem::size_of::<OsVersionInfo>() as u32;
    unsafe { GetVersionExA(&mut v) };
    v.platform == 2
}

pub fn version() -> OsVersionInfo {
    let mut v: OsVersionInfo = unsafe { core::mem::zeroed() };
    v.size = core::mem::size_of::<OsVersionInfo>() as u32;
    unsafe { GetVersionExA(&mut v) };
    v
}

pub fn exists(path: &str) -> bool {
    unsafe { GetFileAttributesA(cz(path).as_ptr()) != NO_ATTRS }
}

pub fn is_dir(path: &str) -> bool {
    let a = unsafe { GetFileAttributesA(cz(path).as_ptr()) };
    a != NO_ATTRS && a & ATTR_DIR != 0
}

/// Создать каталог с родителями.
pub fn mkdirs(path: &str) -> bool {
    let p = path.trim_end_matches('\\');
    if p.is_empty() || is_dir(p) {
        return true;
    }
    if let Some(i) = p.rfind('\\') {
        let parent = &p[..i];
        if !parent.is_empty() && !parent.ends_with(':') {
            mkdirs(parent);
        }
    }
    unsafe { CreateDirectoryA(cz(p).as_ptr(), core::ptr::null()) != 0 || is_dir(p) }
}

pub fn read_file(path: &str) -> Option<Vec<u8>> {
    unsafe {
        let h = CreateFileA(cz(path).as_ptr(), GENERIC_READ, FILE_SHARE_RW, core::ptr::null(), OPEN_EXISTING, 0, 0);
        if h == INVALID {
            return None;
        }
        let size = GetFileSize(h, core::ptr::null_mut()) as usize;
        let mut v = alloc::vec![0u8; size];
        let mut got = 0u32;
        let ok = ReadFile(h, v.as_mut_ptr(), size as u32, &mut got, core::ptr::null_mut()) != 0;
        CloseHandle(h);
        ok.then(|| {
            v.truncate(got as usize);
            v
        })
    }
}

pub fn write_file(path: &str, data: &[u8]) -> bool {
    unsafe {
        let h = CreateFileA(cz(path).as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        if h == INVALID {
            return false;
        }
        let mut w = 0u32;
        let ok = data.is_empty() || WriteFile(h, data.as_ptr(), data.len() as u32, &mut w, core::ptr::null_mut()) != 0;
        CloseHandle(h);
        ok && w as usize == data.len()
    }
}

/// Путь своего exe.
pub fn self_path() -> String {
    let mut buf = [0u8; 300];
    let n = unsafe { GetModuleFileNameA(0, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    from_ansi(&buf[..n])
}

/// Строковое значение в HKLM.
pub fn reg_set(sub: &str, name: &str, value: &str) -> bool {
    unsafe {
        let mut k = 0isize;
        let mut d = 0u32;
        if RegCreateKeyExA(HKLM, cz(sub).as_ptr(), 0, core::ptr::null(), 0, 0x2_0006, core::ptr::null(), &mut k, &mut d) != 0 {
            return false;
        }
        let v = cz(value);
        let ok = RegSetValueExA(k, cz(name).as_ptr(), 0, 1, v.as_ptr(), v.len() as u32) == 0;
        RegCloseKey(k);
        ok
    }
}

/// Удалить строковое значение в HKLM (если есть).
pub fn reg_delete(sub: &str, name: &str) {
    unsafe {
        let mut k = 0isize;
        if RegOpenKeyExA(HKLM, cz(sub).as_ptr(), 0, 0x2_0006, &mut k) == 0 {
            RegDeleteValueA(k, cz(name).as_ptr());
            RegCloseKey(k);
        }
    }
}

/// Перезагрузка: у NT сначала право SeShutdownPrivilege (функции advapi32 только NT — через GetProcAddress).
pub fn reboot() {
    unsafe {
        if is_nt() {
            let adv = GetModuleHandleA(b"advapi32.dll\0".as_ptr());
            let open = GetProcAddress(adv, b"OpenProcessToken\0".as_ptr());
            let lookup = GetProcAddress(adv, b"LookupPrivilegeValueA\0".as_ptr());
            let adjust = GetProcAddress(adv, b"AdjustTokenPrivileges\0".as_ptr());
            if !open.is_null() && !lookup.is_null() && !adjust.is_null() {
                let open: unsafe extern "system" fn(HANDLE, u32, *mut HANDLE) -> i32 = core::mem::transmute(open);
                let lookup: unsafe extern "system" fn(*const u8, *const u8, *mut [u32; 2]) -> i32 = core::mem::transmute(lookup);
                let adjust: unsafe extern "system" fn(HANDLE, i32, *const [u32; 4], u32, *mut u8, *mut u32) -> i32 = core::mem::transmute(adjust);
                let mut tok = 0;
                if open(GetCurrentProcess(), 0x28, &mut tok) != 0 {
                    let mut luid = [0u32; 2];
                    if lookup(core::ptr::null(), b"SeShutdownPrivilege\0".as_ptr(), &mut luid) != 0 {
                        // TOKEN_PRIVILEGES: count, LUID, attributes (SE_PRIVILEGE_ENABLED)
                        let tp = [1u32, luid[0], luid[1], 2];
                        adjust(tok, 0, &tp, 0, core::ptr::null_mut(), core::ptr::null_mut());
                    }
                    CloseHandle(tok);
                }
            }
        }
        // EWX_REBOOT | EWX_FORCE
        ExitWindowsEx(0x2 | 0x4, 0);
    }
}
