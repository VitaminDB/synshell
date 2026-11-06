//! Windows: служба SCM, порт `\\.\Global\org.syn.vm.agent.0` (overlapped — чтение и запись
//! одновременно из разных потоков), процессы (в том числе в сеансе вошедшего пользователя:
//! WTSQueryUserToken + CreateProcessAsUserW на winsta0\default), OutputDebugString
//! (DBWIN глобально и в сеансе пользователя), журнал событий (Application, System — ошибки).

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::EventLog::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::RemoteDesktop::*;
use windows_sys::Win32::System::Services::*;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

use crate::{ExecReq, Proc};

const SERVICE: &str = "syn-vm-agent";
const GENERIC_RW: u32 = 0x8000_0000 | 0x4000_0000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

fn last_err() -> io::Error {
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

// ─── Журнал агента ──────────────────────────────────────────────────────

fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("syn-vm")
}

pub fn agent_log(s: &str) {
    eprintln!("{s}");
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("agent.log");
    if std::fs::metadata(&path).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("agent.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} {s}");
    }
}

// ─── Сведения о системе ─────────────────────────────────────────────────

pub fn os_name() -> String {
    let out = Command::new("cmd").args(["/c", "ver"]).creation_flags(CREATE_NO_WINDOW).output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Windows".into())
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// Метка загрузки: счётчик загрузок Windows (BootId в реестре); нет — время старта системы.
pub fn boot_id() -> String {
    let out = Command::new("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters", "/v", "BootId"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    if let Some(id) = out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find(|l| l.contains("BootId")).and_then(|l| l.split_whitespace().last().map(str::to_string))) {
        return format!("bootid-{id}");
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let up = unsafe { GetTickCount64() } / 1000;
    format!("{}", (now.saturating_sub(up) + 30) / 60 * 60)
}

// ─── Порт ───────────────────────────────────────────────────────────────

struct Port(HANDLE);
unsafe impl Send for Port {}
unsafe impl Sync for Port {}
impl Drop for Port {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Одна сторона порта (чтение или запись) со своим событием overlapped.
struct PortIo {
    port: Arc<Port>,
    ev: HANDLE,
}
unsafe impl Send for PortIo {}

impl PortIo {
    fn new(port: Arc<Port>) -> io::Result<PortIo> {
        let ev = unsafe { CreateEventW(null(), 1, 0, null()) };
        if ev.is_null() {
            return Err(last_err());
        }
        Ok(PortIo { port, ev })
    }

    fn io(&mut self, read: bool, buf: *mut u8, len: usize) -> io::Result<usize> {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.ev;
        let mut n = 0u32;
        let ok = unsafe {
            if read {
                ReadFile(self.port.0, buf, len.min(u32::MAX as usize) as u32, &mut n, &mut ov)
            } else {
                WriteFile(self.port.0, buf, len.min(u32::MAX as usize) as u32, &mut n, &mut ov)
            }
        };
        if ok == 0 {
            let e = unsafe { GetLastError() };
            if e != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(e as i32));
            }
            if unsafe { GetOverlappedResult(self.port.0, &ov, &mut n, 1) } == 0 {
                return Err(last_err());
            }
        }
        Ok(n as usize)
    }
}

impl Drop for PortIo {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.ev) };
    }
}

impl Read for PortIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.io(true, buf.as_mut_ptr(), buf.len())?;
            if n > 0 {
                return Ok(n);
            }
            // Хост не подключён — порт отдаёт 0 байт; подождать.
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Write for PortIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io(false, buf.as_ptr() as *mut u8, buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn open_port() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let name = wide(&format!(r"\\.\Global\{}", syn_vm_agent::PORT));
    let h = unsafe { CreateFileW(name.as_ptr(), GENERIC_RW, 0, null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return Err(last_err());
    }
    let port = Arc::new(Port(h));
    Ok((Box::new(PortIo::new(port.clone())?), Box::new(PortIo::new(port)?)))
}

// ─── Сеанс пользователя ─────────────────────────────────────────────────

fn console_session() -> Option<u32> {
    let s = unsafe { WTSGetActiveConsoleSessionId() };
    (s != 0xFFFF_FFFF && s != 0).then_some(s)
}

pub fn session_user() -> Option<String> {
    let sid = console_session()?;
    let mut buf: *mut u16 = null_mut();
    let mut n = 0u32;
    if unsafe { WTSQuerySessionInformationW(null_mut(), sid, WTSUserName, &mut buf, &mut n) } == 0 {
        return None;
    }
    let name = from_wide(buf);
    unsafe { WTSFreeMemory(buf as *mut c_void) };
    // Имя есть, но токена нет — сеанс ещё не вошёл (экран входа).
    let mut tok: HANDLE = null_mut();
    if unsafe { WTSQueryUserToken(sid, &mut tok) } == 0 {
        return None;
    }
    unsafe { CloseHandle(tok) };
    (!name.is_empty()).then_some(name)
}

// ─── Процессы ───────────────────────────────────────────────────────────

/// Строка команды по правилам разбора MSVCRT.
fn quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"', '\n']) {
        return a.into();
    }
    let mut s = String::from("\"");
    let mut bs = 0;
    for c in a.chars() {
        if c == '\\' {
            bs += 1;
            continue;
        }
        if c == '"' {
            s.push_str(&"\\".repeat(bs * 2 + 1));
        } else {
            s.push_str(&"\\".repeat(bs));
        }
        bs = 0;
        s.push(c);
    }
    s.push_str(&"\\".repeat(bs * 2));
    s.push('"');
    s
}

pub fn spawn(req: &ExecReq) -> io::Result<Proc> {
    if req.user {
        return spawn_user(req);
    }
    let mut c = Command::new(&req.argv[0]);
    c.args(&req.argv[1..]).creation_flags(CREATE_NO_WINDOW);
    for kv in &req.env {
        if let Some((k, v)) = kv.split_once('=') {
            c.env(k, v);
        }
    }
    if let Some(d) = &req.cwd {
        c.current_dir(d);
    }
    c.stdin(if req.stdin { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut ch = c.spawn()?;
    let pid = ch.id();
    let stdout = Box::new(ch.stdout.take().unwrap());
    let stderr = Box::new(ch.stderr.take().unwrap());
    let stdin = ch.stdin.take().map(|s| Box::new(s) as Box<dyn Write + Send>);
    Ok(Proc { pid, stdout, stderr, stdin, wait: Box::new(move || ch.wait().ok().and_then(|s| s.code()).unwrap_or(-1)) })
}

struct H(HANDLE);
unsafe impl Send for H {}
impl H {
    fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Канал: (наш конец, конец ребёнка — наследуемый).
fn pipe(child_reads: bool) -> io::Result<(H, H)> {
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 };
    let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
    if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
        return Err(last_err());
    }
    let (ours, theirs) = if child_reads { (w, r) } else { (r, w) };
    unsafe { SetHandleInformation(ours, HANDLE_FLAG_INHERIT, 0) };
    Ok((H(ours), H(theirs)))
}

/// Запуск в интерактивном сеансе вошедшего пользователя (окна, выходы DXGI его рабочего стола).
fn spawn_user(req: &ExecReq) -> io::Result<Proc> {
    let sid = console_session().ok_or_else(|| io::Error::other("нет интерактивного сеанса"))?;
    let mut tok: HANDLE = null_mut();
    if unsafe { WTSQueryUserToken(sid, &mut tok) } == 0 {
        return Err(io::Error::other(format!("пользователь не вошёл в сеанс {sid} ({})", last_err())));
    }
    let tok = H(tok);
    let mut prim: HANDLE = null_mut();
    if unsafe { DuplicateTokenEx(tok.0, 0x0200_0000, null(), SecurityIdentification, TokenPrimary, &mut prim) } == 0 {
        return Err(last_err());
    }
    let prim = H(prim);
    // Окружение пользователя + переданные переменные.
    let mut envp: *mut c_void = null_mut();
    let mut vars: Vec<(String, String)> = Vec::new();
    if unsafe { CreateEnvironmentBlock(&mut envp, prim.0, 0) } != 0 {
        let mut p = envp as *const u16;
        loop {
            let s = from_wide(p);
            if s.is_empty() {
                break;
            }
            unsafe { p = p.add(s.encode_utf16().count() + 1) };
            if let Some((k, v)) = s.split_once('=').filter(|(k, _)| !k.is_empty()) {
                vars.push((k.into(), v.into()));
            } else {
                vars.push((s, String::new()));
            }
        }
        unsafe { DestroyEnvironmentBlock(envp) };
    }
    for kv in &req.env {
        if let Some((k, v)) = kv.split_once('=') {
            vars.retain(|(a, _)| !a.eq_ignore_ascii_case(k));
            vars.push((k.into(), v.into()));
        }
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in &vars {
        let kv = if v.is_empty() && k.starts_with('=') { k.clone() } else { format!("{k}={v}") };
        block.extend(kv.encode_utf16());
        block.push(0);
    }
    block.push(0);

    let (out_r, out_w) = pipe(false)?;
    let (err_r, err_w) = pipe(false)?;
    let (in_w, in_r) = if req.stdin {
        let (w, r) = pipe(true)?;
        (Some(w), r)
    } else {
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 };
        let nul = wide("NUL");
        (None, H(unsafe { CreateFileW(nul.as_ptr(), 0x8000_0000, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, null_mut()) }))
    };
    let mut desktop = wide(r"winsta0\default");
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.lpDesktop = desktop.as_mut_ptr();
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = in_r.0;
    si.hStdOutput = out_w.0;
    si.hStdError = err_w.0;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut cmd = wide(&req.argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let cwd = req.cwd.as_deref().map(wide);
    let ok = unsafe {
        CreateProcessAsUserW(
            prim.0,
            null(),
            cmd.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            block.as_ptr() as *const c_void,
            cwd.as_ref().map(|c| c.as_ptr()).unwrap_or(null()),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(last_err());
    }
    drop((out_w, err_w, in_r));
    unsafe { CloseHandle(pi.hThread) };
    let hp = H(pi.hProcess);
    let take = |h: H| -> std::fs::File {
        let f = unsafe { std::fs::File::from_raw_handle(h.0) };
        std::mem::forget(h);
        f
    };
    Ok(Proc {
        pid: pi.dwProcessId,
        stdout: Box::new(take(out_r)),
        stderr: Box::new(take(err_r)),
        stdin: in_w.map(|w| Box::new(take(w)) as Box<dyn Write + Send>),
        wait: Box::new(move || {
            let mut code = 0u32;
            unsafe {
                WaitForSingleObject(hp.raw(), INFINITE);
                GetExitCodeProcess(hp.raw(), &mut code);
            }
            code as i32
        }),
    })
}

/// Завершить процесс с потомками.
pub fn kill(pid: u32) {
    let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &pid.to_string()]).creation_flags(CREATE_NO_WINDOW).output();
}

// ─── Журналы ────────────────────────────────────────────────────────────

/// Объекты DBWIN с доступом для всех (иначе процессы пользователя не откроют их).
fn open_sa() -> (Box<SECURITY_DESCRIPTOR>, SECURITY_ATTRIBUTES) {
    let mut sd: Box<SECURITY_DESCRIPTOR> = Box::new(unsafe { std::mem::zeroed() });
    unsafe {
        InitializeSecurityDescriptor(&mut *sd as *mut _ as PSECURITY_DESCRIPTOR, 1);
        SetSecurityDescriptorDacl(&mut *sd as *mut _ as PSECURITY_DESCRIPTOR, 1, null(), 0);
    }
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: &mut *sd as *mut _ as *mut c_void, bInheritHandle: 0 };
    (sd, sa)
}

/// Слушать OutputDebugString в пространстве имён `prefix` («Global\», «Session\N\»),
/// пока `alive()`.
fn dbwin(prefix: String, alive: impl Fn() -> bool) {
    let (_sd, sa) = open_sa();
    let n = |s: &str| wide(&format!("{prefix}{s}"));
    unsafe {
        let buf_ready = H(CreateEventW(&sa, 0, 0, n("DBWIN_BUFFER_READY").as_ptr()));
        let data_ready = H(CreateEventW(&sa, 0, 0, n("DBWIN_DATA_READY").as_ptr()));
        let map = H(CreateFileMappingW(INVALID_HANDLE_VALUE, &sa, PAGE_READWRITE, 0, 4096, n("DBWIN_BUFFER").as_ptr()));
        let busy = GetLastError() == ERROR_ALREADY_EXISTS;
        if buf_ready.0.is_null() || data_ready.0.is_null() || map.0.is_null() {
            crate::log(&format!("DBWIN {prefix}: {}", last_err()));
            return;
        }
        if busy {
            crate::log(&format!("DBWIN {prefix}: уже слушает другая программа (DebugView?)"));
        }
        let view = MapViewOfFile(map.0, FILE_MAP_READ, 0, 0, 4096);
        if view.Value.is_null() {
            crate::log(&format!("DBWIN {prefix}: {}", last_err()));
            return;
        }
        let base = view.Value as *const u8;
        crate::log(&format!("DBWIN {prefix}: слушаю"));
        while alive() {
            SetEvent(buf_ready.0);
            if WaitForSingleObject(data_ready.0, 1000) == WAIT_OBJECT_0 {
                let pid = u32::from_le_bytes([*base, *base.add(1), *base.add(2), *base.add(3)]);
                let mut len = 0;
                while len < 4092 && *base.add(4 + len) != 0 {
                    len += 1;
                }
                let text = String::from_utf8_lossy(std::slice::from_raw_parts(base.add(4), len)).trim_end().to_string();
                if !text.is_empty() {
                    crate::guest_log("debug", &format!("[{pid}] {text}"));
                }
            }
        }
        UnmapViewOfFile(view);
    }
}

/// Ошибки журнала событий `channel` (новые записи).
fn eventlog(channel: &str) {
    unsafe {
        let ev = H(CreateEventW(null(), 1, 1, null()));
        let ch = wide(channel);
        let q = wide("*[System[(Level=1 or Level=2)]]");
        let sub = EvtSubscribe(0, ev.0, ch.as_ptr(), q.as_ptr(), 0, null(), None, EvtSubscribeToFutureEvents as u32);
        if sub == 0 {
            crate::log(&format!("журнал событий {channel}: {}", last_err()));
            return;
        }
        crate::log(&format!("журнал событий {channel}: слушаю"));
        loop {
            WaitForSingleObject(ev.0, INFINITE);
            loop {
                let mut evs = [0isize; 16];
                let mut got = 0u32;
                if EvtNext(sub, 16, evs.as_mut_ptr(), 0, 0, &mut got) == 0 {
                    break;
                }
                for &e in &evs[..got as usize] {
                    let text = render_event(e);
                    EvtClose(e);
                    crate::guest_log(&format!("event:{channel}"), &text);
                }
            }
            ResetEvent(ev.0);
        }
    }
}

unsafe fn render_event(e: isize) -> String {
    let mut used = 0u32;
    let mut count = 0u32;
    EvtRender(0, e, EvtRenderEventXml as u32, 0, null_mut(), &mut used, &mut count);
    let mut buf = vec![0u16; used as usize / 2 + 1];
    if EvtRender(0, e, EvtRenderEventXml as u32, (buf.len() * 2) as u32, buf.as_mut_ptr() as *mut c_void, &mut used, &mut count) == 0 {
        return String::new();
    }
    let xml = from_wide(buf.as_ptr());
    let attr = |tag: &str, a: &str| -> String {
        xml.find(&format!("<{tag}"))
            .and_then(|i| {
                let rest = &xml[i..];
                let end = rest.find('>')?;
                let el = &rest[..end];
                let j = el.find(&format!("{a}="))? + a.len() + 2;
                let q = el.as_bytes().get(j - 1).copied()? as char;
                el[j..].find(q).map(|k| el[j..j + k].to_string())
            })
            .unwrap_or_default()
    };
    let text_of = |tag: &str| -> String {
        xml.find(&format!("<{tag}>")).and_then(|i| xml[i + tag.len() + 2..].find('<').map(|k| xml[i + tag.len() + 2..i + tag.len() + 2 + k].to_string())).unwrap_or_default()
    };
    let provider = attr("Provider", "Name");
    let id = text_of("EventID");
    let level = text_of("Level");
    let time = attr("TimeCreated", "SystemTime");
    let mut msg = String::new();
    let pw = wide(&provider);
    let meta = EvtOpenPublisherMetadata(0, pw.as_ptr(), null(), 0, 0);
    if meta != 0 {
        let mut n = 0u32;
        EvtFormatMessage(meta, e, 0, 0, null(), EvtFormatMessageEvent as u32, 0, null_mut(), &mut n);
        if n > 0 {
            let mut b = vec![0u16; n as usize + 1];
            if EvtFormatMessage(meta, e, 0, 0, null(), EvtFormatMessageEvent as u32, b.len() as u32, b.as_mut_ptr(), &mut n) != 0 {
                msg = from_wide(b.as_ptr());
            }
        }
        EvtClose(meta);
    }
    let lvl = match level.as_str() {
        "1" => "критическая",
        "2" => "ошибка",
        _ => "событие",
    };
    format!("{time} {lvl} {provider} {id}: {}", msg.trim().replace("\r\n", " ").replace('\n', " "))
}

pub fn start_logs() {
    std::thread::spawn(|| dbwin(r"Global\".into(), || true));
    // OutputDebugString процессов сеанса пользователя (DWM с UMD, игры) — в его пространстве имён.
    std::thread::spawn(|| {
        static CUR: AtomicU32 = AtomicU32::new(0);
        loop {
            let s = console_session().unwrap_or(0);
            if s != CUR.load(Ordering::Relaxed) {
                CUR.store(s, Ordering::Relaxed);
                if s != 0 {
                    std::thread::spawn(move || dbwin(format!(r"Session\{s}\"), move || CUR.load(Ordering::Relaxed) == s));
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
    for ch in ["System", "Application"] {
        std::thread::spawn(move || eventlog(ch));
    }
}

// ─── Служба ─────────────────────────────────────────────────────────────

static STATUS: AtomicUsize = AtomicUsize::new(0);

fn set_status(state: SERVICE_STATUS_CURRENT_STATE) {
    let st = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN } else { 0 },
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    unsafe { SetServiceStatus(STATUS.load(Ordering::Relaxed) as SERVICE_STATUS_HANDLE, &st) };
}

unsafe extern "system" fn handler(ctrl: u32, _t: u32, _d: *mut c_void, _c: *mut c_void) -> u32 {
    if ctrl == SERVICE_CONTROL_STOP || ctrl == SERVICE_CONTROL_SHUTDOWN {
        set_status(SERVICE_STOPPED);
        std::process::exit(0);
    }
    NO_ERROR
}

unsafe extern "system" fn svc_main(_argc: u32, _argv: *mut windows_sys::core::PWSTR) {
    let name = wide(SERVICE);
    let h = RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), null());
    STATUS.store(h as usize, Ordering::Relaxed);
    set_status(SERVICE_RUNNING);
    crate::run();
}

pub fn service_main() -> io::Result<()> {
    let mut name = wide(SERVICE);
    let table = [SERVICE_TABLE_ENTRYW { lpServiceName: name.as_mut_ptr(), lpServiceProc: Some(svc_main) }, SERVICE_TABLE_ENTRYW { lpServiceName: null_mut(), lpServiceProc: None }];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        return Err(last_err());
    }
    Ok(())
}

// ─── Установка и обновление ─────────────────────────────────────────────

fn install_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("syn-vm")
}

fn sc(args: &[&str]) -> (bool, String) {
    match Command::new("sc.exe").args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => (false, e.to_string()),
    }
}

fn stop_service() {
    sc(&["stop", SERVICE]);
    for _ in 0..60 {
        let (_, q) = sc(&["query", SERVICE]);
        if q.contains("STOPPED") || q.contains("1060") || q.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Заменить свой exe новым (работающий exe переименовывается — NTFS это позволяет).
pub fn update(path: &str) -> io::Result<()> {
    let me = std::env::current_exe()?;
    let old = me.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&me, &old)?;
    if let Err(e) = std::fs::rename(path, &me) {
        let _ = std::fs::rename(&old, &me);
        return Err(e);
    }
    Ok(())
}

pub fn install() -> io::Result<()> {
    let me = std::env::current_exe()?;
    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("syn-vm-agent.exe");
    let exists = sc(&["query", SERVICE]).0;
    if exists {
        stop_service();
    }
    if !me.eq(&dest) {
        let _ = std::fs::remove_file(dest.with_extension("old.exe"));
        if dest.exists() {
            std::fs::rename(&dest, dest.with_extension("old.exe"))?;
        }
        std::fs::copy(&me, &dest)?;
    }
    let bin = format!("\"{}\" service", dest.display());
    if !exists {
        let (ok, out) = sc(&["create", SERVICE, "binPath=", &bin, "start=", "auto", "DisplayName=", "syn-vm guest agent"]);
        if !ok {
            return Err(io::Error::other(format!("sc create: {out}")));
        }
    } else {
        sc(&["config", SERVICE, "binPath=", &bin, "start=", "auto"]);
    }
    sc(&["description", SERVICE, "syn-vm: commands, files and logs for the host"]);
    sc(&["failure", SERVICE, "reset=", "60", "actions=", "restart/1000/restart/1000/restart/3000"]);
    let (ok, out) = sc(&["start", SERVICE]);
    if !ok && !out.contains("1056") {
        return Err(io::Error::other(format!("sc start: {out}")));
    }
    println!("syn-vm-agent: служба установлена ({})", dest.display());
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    stop_service();
    sc(&["delete", SERVICE]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quoting() {
        assert_eq!(quote("a"), "a");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote(r"C:\x y\"), r#""C:\x y\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(""), "\"\"");
    }
}
