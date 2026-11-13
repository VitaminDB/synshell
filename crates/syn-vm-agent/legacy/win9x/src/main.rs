//! SYNAGENT.EXE — гостевой агент syn-vm для Windows 95/98/ME и XP.
//!
//! Тот же протокол, что у агента Windows 7–11 и Linux (кадры SVA1 + JSON, крейт syn-vm-agent без std):
//! команды (вывод — по мере появления), файлы туда и обратно, список каталога, часы. Канал — COM2
//! (у syn-vm это isa-serial с сокетом agent-com.sock): драйверы не нужны ни одной системе.
//!
//! `SYNAGENT.EXE install` — поставить: копия в C:\SYNVM, автозапуск (Run — в сеансе пользователя; из
//! RunServices у 9x сеансы DOS уходили во весь экран), затем шаги `install.txt` каталога системы на диске (драйверы, заплатки, утилиты)
//! с продолжением после перезагрузок. `SYNAGENT.EXE run` — работать без установки (portable).

#![no_std]
#![no_main]

extern crate alloc;

mod msvcrt;
mod win;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

use syn_vm_agent::{decode_body, hex, Digest, Entry, Frame, Msg, Sha256, MAGIC, MAX_FRAME, VERSION};
use win::*;

// ─── память и паника ────────────────────────────────────────────────────

struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { HeapAlloc(GetProcessHeap(), 0, l.size().max(1)) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        unsafe { HeapAlloc(GetProcessHeap(), 8, l.size().max(1)) }
    }
    unsafe fn dealloc(&self, p: *mut u8, _l: Layout) {
        unsafe { HeapFree(GetProcessHeap(), 0, p) };
    }
    unsafe fn realloc(&self, p: *mut u8, _l: Layout, size: usize) -> *mut u8 {
        unsafe { HeapReAlloc(GetProcessHeap(), 0, p, size.max(1)) }
    }
}

#[global_allocator]
static HEAP: Heap = Heap;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log(&format!("паника: {info}"));
    unsafe { ExitProcess(3) }
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetCommandLineA() -> *const u8;
    fn GetSystemTime(t: *mut SystemTime);
    fn GetCurrentProcessId() -> u32;
    fn CreateMutexA(sa: *const u8, owner: i32, name: *const u8) -> isize;
    fn CreateThread(sa: *const u8, stack: usize, start: unsafe extern "system" fn(*mut u8) -> u32, param: *mut u8, flags: u32, tid: *mut u32) -> HANDLE;
    fn InitializeCriticalSection(cs: *mut u8);
    fn EnterCriticalSection(cs: *mut u8);
    fn LeaveCriticalSection(cs: *mut u8);
}

// ─── общее для потоков: отправка в порт и запущенные команды ────────────

/// CRITICAL_SECTION (24 байта у x86) — с запасом.
static mut CS: [u8; 64] = [0; 64];
/// Запущенные команды: (номер операции, процесс) — Kill снимает по номеру.
static mut PROCS: Vec<(u64, HANDLE)> = Vec::new();

#[allow(static_mut_refs)]
fn locked<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        EnterCriticalSection(CS.as_mut_ptr());
        let r = f();
        LeaveCriticalSection(CS.as_mut_ptr());
        r
    }
}

/// Кадры хосту от всех потоков: пишет в порт только главный поток (между чтениями) — запись в COM из
/// другого потока, пока главный ждёт в ReadFile, у синхронного дескриптора стоит (Wine — навсегда).
static mut OUTQ: Vec<Frame> = Vec::new();

/// Кадр хосту — из любого потока (уйдёт в ближайшие 200 мс, ответы главного потока — сразу).
#[allow(static_mut_refs)]
fn tx(f: &Frame) {
    locked(|| unsafe { OUTQ.push(f.clone()) });
}

/// Главный поток: отправить накопленное.
#[allow(static_mut_refs)]
fn flush(port: &mut Port) {
    let q = locked(|| unsafe { core::mem::take(&mut OUTQ) });
    for f in &q {
        port.send(f);
    }
}

#[allow(static_mut_refs)]
fn proc_add(id: u64, h: HANDLE) {
    if id != 0 {
        locked(|| unsafe { PROCS.push((id, h)) });
    }
}

#[allow(static_mut_refs)]
fn proc_remove(id: u64) {
    locked(|| unsafe { PROCS.retain(|p| p.0 != id) });
}

/// Снять команду по номеру (Kill от хоста: тайм-аут --exec).
#[allow(static_mut_refs)]
fn proc_kill(id: u64) {
    locked(|| unsafe {
        for p in PROCS.iter().filter(|p| p.0 == id) {
            TerminateProcess(p.1, 124);
        }
    });
}

/// Вывод консольных программ — в кодовой странице OEM (cp866 у русской Windows) → UTF-8. У Windows 95
/// (нет CP_UTF8) — как есть.
fn oem_to_utf8(b: &[u8]) -> Vec<u8> {
    if b.is_ascii() {
        return b.to_vec();
    }
    unsafe {
        let n = MultiByteToWideChar(1, 0, b.as_ptr(), b.len() as i32, core::ptr::null_mut(), 0);
        if n <= 0 {
            return b.to_vec();
        }
        let mut w = alloc::vec![0u16; n as usize];
        MultiByteToWideChar(1, 0, b.as_ptr(), b.len() as i32, w.as_mut_ptr(), n);
        let m = WideCharToMultiByte(65001, 0, w.as_ptr(), n, core::ptr::null_mut(), 0, core::ptr::null(), core::ptr::null_mut());
        if m <= 0 {
            return b.to_vec();
        }
        let mut out = alloc::vec![0u8; m as usize];
        WideCharToMultiByte(65001, 0, w.as_ptr(), n, out.as_mut_ptr(), m, core::ptr::null(), core::ptr::null_mut());
        out
    }
}

/// Команда хоста — в своём потоке: зависшая не держит остальные.
struct Job {
    id: u64,
    cmd: String,
    cwd: Option<String>,
}

unsafe extern "system" fn exec_thread(p: *mut u8) -> u32 {
    let job = unsafe { alloc::boxed::Box::from_raw(p as *mut Job) };
    let id = job.id;
    tx(&Frame::new(Msg::Started { id, pid: 0 }));
    let (code, _) = run_capture(&job.cmd, job.cwd.as_deref(), 0, id, &mut |chunk| {
        if !chunk.is_empty() {
            tx(&Frame::with_data(Msg::Out { id, fd: 1 }, oem_to_utf8(chunk)));
        }
        false
    });
    tx(&Frame::new(Msg::Exit { id, code }));
    0
}

/// Каталог установки на системном диске (`C:\SYNVM`).
fn home() -> String {
    let mut buf = [0u8; 260];
    let n = unsafe { GetWindowsDirectoryA(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let drive = if n >= 2 { from_ansi(&buf[..2]) } else { "C:".into() };
    format!("{drive}\\SYNVM")
}

/// Журнал агента (`C:\SYNVM\AGENT.LOG`, дописывается).
fn log(line: &str) {
    let path = format!("{}\\AGENT.LOG", home());
    mkdirs(&home());
    unsafe {
        let h = CreateFileA(cz(&path).as_ptr(), GENERIC_WRITE, FILE_SHARE_RW, core::ptr::null(), 4 /* OPEN_ALWAYS */, 0, 0);
        if h == INVALID {
            return;
        }
        SetFilePointer(h, 0, core::ptr::null_mut(), 2);
        let mut s = to_ansi(line);
        s.extend_from_slice(b"\r\n");
        let mut w = 0;
        WriteFile(h, s.as_ptr(), s.len() as u32, &mut w, core::ptr::null_mut());
        CloseHandle(h);
    }
}

// ─── вход ───────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    let cmd = unsafe { from_ansi(zstr(core::slice::from_raw_parts(GetCommandLineA(), 1024))) };
    let args = split_args(&cmd);
    let verb = args.get(1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    let code = match verb.as_str() {
        "install" | "/install" => install(),
        _ => run(),
    };
    unsafe { ExitProcess(code) }
}

/// Разбор командной строки: пробелы, кавычки.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' | '\t' if !quoted => {
                if any {
                    out.push(core::mem::take(&mut cur));
                    any = false;
                }
            }
            '\0' => break,
            _ => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

// ─── установка ──────────────────────────────────────────────────────────

/// Копия в C:\SYNVM, автозапуск, шаги install.txt и запуск агента.
fn install() -> u32 {
    let home = home();
    if !mkdirs(&home) {
        return 2;
    }
    let me = self_path();
    let exe = format!("{home}\\SYNAGENT.EXE");
    if !me.eq_ignore_ascii_case(&exe) {
        // Обновление: прежний агент как раз уходит, его exe ещё занят — повторять до 15 с.
        let mut tries = 0;
        while unsafe { CopyFileA(cz(&me).as_ptr(), cz(&exe).as_ptr(), 0) } == 0 {
            tries += 1;
            if tries > 30 {
                log(&format!("установка: не удалось скопировать {me} → {exe}"));
                return 2;
            }
            unsafe { Sleep(500) };
        }
    }
    // RUN.PIF (свойства сеанса DOS для команд) — рядом с агентом на диске.
    let pif = format!("{}\\RUN.PIF", parent(&me));
    if exists(&pif) {
        unsafe { CopyFileA(cz(&pif).as_ptr(), cz(&format!("{home}\\RUN.PIF")).as_ptr(), 0) };
    }
    // Каталог системы на диске: …\WIN9X\AGENT\SYNAGENT.EXE → …\WIN9X.
    let sys = parent(&parent(&me));
    let manifest = format!("{sys}\\install.txt");
    if exists(&manifest) {
        let setup = format!("{home}\\SETUP");
        copy_tree(&sys, &setup);
        write_file(&format!("{home}\\STEP.TXT"), b"0");
        log(&format!("установка: шаги {manifest} → {setup}"));
    }
    let run = format!("{exe} run");
    // Автозапуск — в сеансе пользователя (Run): из RunServices (до входа, вне рабочего стола) любой
    // сеанс DOS уходил во весь экран, сколько ни настраивай PIF. Прежнюю запись RunServices — убрать.
    let key = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    reg_delete("Software\\Microsoft\\Windows\\CurrentVersion\\RunServices", "syn-vm-agent");
    if !reg_set(key, "syn-vm-agent", &run) {
        log("установка: автозапуск не записан");
    }
    log(&format!("установка: {exe}, автозапуск {key}"));
    spawn_detached(&run, None);
    0
}

fn parent(p: &str) -> String {
    match p.trim_end_matches('\\').rfind('\\') {
        Some(i) => p[..i].to_string(),
        None => p.to_string(),
    }
}

/// Скопировать каталог целиком.
fn copy_tree(from: &str, to: &str) {
    mkdirs(to);
    for (name, dir) in read_dir(from) {
        let (s, d) = (format!("{from}\\{name}"), format!("{to}\\{name}"));
        if dir {
            copy_tree(&s, &d);
        } else {
            unsafe { CopyFileA(cz(&s).as_ptr(), cz(&d).as_ptr(), 0) };
        }
    }
}

/// Имена в каталоге: (имя, каталог ли).
fn read_dir(dir: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut fd: FindData = unsafe { core::mem::zeroed() };
    let h = unsafe { FindFirstFileA(cz(&format!("{dir}\\*.*")).as_ptr(), &mut fd) };
    if h == INVALID {
        return out;
    }
    loop {
        let name = from_ansi(zstr(&fd.name));
        if name != "." && name != ".." {
            out.push((name, fd.attrs & ATTR_DIR != 0));
        }
        if unsafe { FindNextFileA(h, &mut fd) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    out
}

/// Запустить без ожидания (без окна консоли).
fn spawn_detached(cmd: &str, cwd: Option<&str>) -> bool {
    let mut c = cz(cmd);
    let si = StartupInfo { cb: core::mem::size_of::<StartupInfo>() as u32, flags: 1, show: 0, ..zeroed_si() };
    let mut pi: ProcessInfo = unsafe { core::mem::zeroed() };
    let cwd = cwd.map(cz);
    let ok = unsafe { CreateProcessA(core::ptr::null(), c.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 0x8 /* DETACHED_PROCESS */, core::ptr::null(), cwd.as_ref().map_or(core::ptr::null(), |c| c.as_ptr()), &si, &mut pi) } != 0;
    if ok {
        unsafe {
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
    }
    ok
}

fn zeroed_si() -> StartupInfo {
    unsafe { core::mem::zeroed() }
}

/// Система для условий шага (`if=95,98,me,xp`).
fn os_tag() -> &'static str {
    let v = version();
    match (v.platform, v.major, v.minor) {
        (2, 5, 0) => "2000",
        (2, 5, _) => "xp",
        (2, _, _) => "nt",
        (_, 4, 0) => "95",
        (_, 4, 10) => "98",
        (_, 4, 90) => "me",
        _ => "9x",
    }
}

/// Шаги install.txt (копия в C:\SYNVM\SETUP) с места, где остановились; `reboot` — перезагрузить
/// после шага и продолжить со следующего.
fn continue_setup() {
    let home = home();
    let step_path = format!("{home}\\STEP.TXT");
    let Some(step) = read_file(&step_path) else { return };
    let Ok(mut n) = core::str::from_utf8(&step).unwrap_or("x").trim().parse::<usize>() else { return };
    let setup = format!("{home}\\SETUP");
    let Some(text) = read_file(&format!("{setup}\\install.txt")) else {
        unsafe { DeleteFileA(cz(&step_path).as_ptr()) };
        return;
    };
    let text = from_ansi(&text);
    let steps: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let tag = os_tag();
    while n < steps.len() {
        let mut line = steps[n];
        let mut reboot = false;
        let mut skip = false;
        loop {
            if let Some(rest) = line.strip_prefix("reboot ") {
                reboot = true;
                line = rest.trim_start();
            } else if let Some(rest) = line.strip_prefix("if=") {
                let (cond, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                skip |= !cond.split(',').any(|c| c.eq_ignore_ascii_case(tag));
                line = rest.trim_start();
            } else {
                break;
            }
        }
        if skip {
            log(&format!("шаг {}: пропущен ({tag}): {line}", n + 1));
        } else {
            log(&format!("шаг {}: {line}", n + 1));
            let (code, out) = run_capture(line, Some(&setup), 0, 0, &mut |_| false);
            if !out.is_empty() {
                log(&from_ansi(&out));
            }
            if code != 0 {
                log(&format!("шаг {}: код {code} — установка остановлена", n + 1));
                write_file(&step_path, format!("failed {}", n + 1).as_bytes());
                return;
            }
        }
        n += 1;
        write_file(&step_path, format!("{n}").as_bytes());
        if reboot && !skip {
            log("перезагрузка, затем — следующий шаг");
            win::reboot();
            unsafe { ExitProcess(0) };
        }
    }
    unsafe { DeleteFileA(cz(&step_path).as_ptr()) };
    log("установка закончена");
}

// ─── команды ────────────────────────────────────────────────────────────

/// Запустить команду и ждать: вывод (stdout и stderr) — в файл, по мере появления — в `out` (и пустой
/// кусок раз в 200 мс; `true` от `out` — снять команду). `.bat` и встроенные команды — через COMSPEC.
/// Итог — код и весь вывод.
fn run_capture(cmd: &str, cwd: Option<&str>, timeout_ms: u32, id: u64, out: &mut dyn FnMut(&[u8]) -> bool) -> (i32, Vec<u8>) {
    let mut tmp = [0u8; 260];
    let n = unsafe { GetTempPathA(tmp.len() as u32, tmp.as_mut_ptr()) } as usize;
    let tmp_dir = if n > 0 { from_ansi(&tmp[..n]) } else { format!("{}\\", home()) };
    let file = format!("{tmp_dir}SVA{}.OUT", unsafe { GetTickCount() } & 0xFFFF);
    let sa = SecurityAttributes { len: core::mem::size_of::<SecurityAttributes>() as u32, sd: core::ptr::null(), inherit: 1 };
    let h = unsafe { CreateFileA(cz(&file).as_ptr(), GENERIC_WRITE, FILE_SHARE_RW, &sa, CREATE_ALWAYS, 0, 0) };
    let first = cmd.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    // Windows 9x: команды DOS (COMMAND.COM, .bat, встроенные) — сеанс DOS, у которого свой вывод не в
    // наш файл, и который без PIF уходит во весь экран. Поэтому — C:\SYNVM\RUN.BAT (вывод и код —
    // файлами) через RUN.PIF (окно свёрнуто, закрыть по выходу), если он есть.
    let dos = !is_nt() && (first.ends_with(".bat") || is_builtin(&first) || first.ends_with("command.com") || first == "command");
    if dos {
        if h != INVALID {
            unsafe { CloseHandle(h) };
        }
        return run_dos(cmd, cwd, &file, timeout_ms, id, out);
    }
    let line = if first.ends_with(".bat") || first.ends_with(".cmd") || is_builtin(&first) {
        let shell = env("COMSPEC").unwrap_or_else(|| if is_nt() { "cmd.exe".into() } else { "command.com".into() });
        format!("{shell} /c {cmd}")
    } else {
        cmd.to_string()
    };
    let mut c = cz(&line);
    let si = StartupInfo { cb: core::mem::size_of::<StartupInfo>() as u32, flags: 0x100 | 1, show: 0, stdin: 0, stdout: h, stderr: h, ..zeroed_si() };
    let mut pi: ProcessInfo = unsafe { core::mem::zeroed() };
    let cwd = cwd.map(cz);
    let flags = if is_nt() { 0x0800_0000 /* CREATE_NO_WINDOW */ } else { 0 };
    let ok = unsafe { CreateProcessA(core::ptr::null(), c.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 1, flags, core::ptr::null(), cwd.as_ref().map_or(core::ptr::null(), |c| c.as_ptr()), &si, &mut pi) } != 0;
    if ok {
        proc_add(id, pi.process);
    }
    if h != INVALID {
        unsafe { CloseHandle(h) };
    }
    if !ok {
        let e = unsafe { GetLastError() };
        unsafe { DeleteFileA(cz(&file).as_ptr()) };
        return (-1, format!("не запустилось: {line} (ошибка {e})").into_bytes());
    }
    let t0 = unsafe { GetTickCount() };
    let mut sent = 0usize;
    let mut all = Vec::new();
    let code = loop {
        let done = unsafe { WaitForSingleObject(pi.process, 200) } == 0;
        let mut kill = false;
        if let Some(d) = read_file(&file) {
            if d.len() > sent {
                kill |= out(&d[sent..]);
                sent = d.len();
            }
            all = d;
        }
        kill |= out(&[]);
        if kill && !done {
            unsafe { TerminateProcess(pi.process, 124) };
            break 124;
        }
        if done {
            let mut c = 0u32;
            unsafe { GetExitCodeProcess(pi.process, &mut c) };
            break c as i32;
        }
        if timeout_ms > 0 && unsafe { GetTickCount() }.wrapping_sub(t0) > timeout_ms {
            unsafe { TerminateProcess(pi.process, 1) };
            break -2;
        }
    };
    proc_remove(id);
    unsafe {
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
        DeleteFileA(cz(&file).as_ptr());
    }
    (code, all)
}

/// Команда DOS у Windows 9x: RUN.BAT с выводом в файл и кодом (лестница IF ERRORLEVEL — у COMMAND.COM
/// нет %ERRORLEVEL%), запуск через RUN.PIF (свёрнутое окно, закрыть по выходу — делается в свойствах
/// RUN.BAT и лежит на диске рядом с агентом) или COMMAND.COM /C.
fn run_dos(cmd: &str, cwd: Option<&str>, out_file: &str, timeout_ms: u32, id: u64, out: &mut dyn FnMut(&[u8]) -> bool) -> (i32, Vec<u8>) {
    let home = home();
    let code_file = format!("{home}\\RUN.ERR");
    unsafe { DeleteFileA(cz(&code_file).as_ptr()) };
    let mut bat = String::from("@echo off\r\n");
    if let Some(d) = cwd {
        if d.len() >= 2 && d.as_bytes()[1] == b':' {
            bat += &format!("{}\r\n", &d[..2]);
        }
        bat += &format!("cd {d}\r\n");
    }
    // Сам COMMAND.COM /C x — выполнить x.
    let inner = match cmd.split_once(' ') {
        Some((f, rest)) if f.to_ascii_lowercase().ends_with("command.com") || f.eq_ignore_ascii_case("command") => rest.trim_start().strip_prefix("/c ").or_else(|| rest.trim_start().strip_prefix("/C ")).unwrap_or(rest),
        _ => cmd,
    };
    bat += &format!("{inner} > {out_file}\r\n");
    bat += &format!("echo 0 > {code_file}\r\n");
    for n in 1..=255 {
        bat += &format!("if errorlevel {n} echo {n} > {code_file}\r\n");
    }
    let bat_path = format!("{home}\\RUN.BAT");
    write_file(&bat_path, &to_ansi(&bat));
    let pif = format!("{home}\\RUN.PIF");
    let line = if exists(&pif) {
        pif
    } else {
        let shell = env("COMSPEC").unwrap_or_else(|| "command.com".into());
        format!("{shell} /c {bat_path}")
    };
    let mut c = cz(&line);
    // Свёрнуто: свёрнутый сеанс DOS — оконный, во весь экран не уходит.
    let si = StartupInfo { cb: core::mem::size_of::<StartupInfo>() as u32, flags: 1, show: 7 /* SW_SHOWMINNOACTIVE */, ..zeroed_si() };
    let mut pi: ProcessInfo = unsafe { core::mem::zeroed() };
    let ok = unsafe { CreateProcessA(core::ptr::null(), c.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 0, core::ptr::null(), core::ptr::null(), &si, &mut pi) } != 0;
    if ok {
        proc_add(id, pi.process);
    }
    if !ok {
        let e = unsafe { GetLastError() };
        return (-1, format!("не запустилось: {line} (ошибка {e})").into_bytes());
    }
    let t0 = unsafe { GetTickCount() };
    let mut sent = 0usize;
    let mut all = Vec::new();
    let finished = loop {
        let done = unsafe { WaitForSingleObject(pi.process, 200) } == 0;
        let mut kill = false;
        if let Some(d) = read_file(out_file) {
            if d.len() > sent {
                kill |= out(&d[sent..]);
                sent = d.len();
            }
            all = d;
        }
        kill |= out(&[]);
        if done {
            break true;
        }
        if kill || (timeout_ms > 0 && unsafe { GetTickCount() }.wrapping_sub(t0) > timeout_ms) {
            unsafe { TerminateProcess(pi.process, 124) };
            break false;
        }
    };
    proc_remove(id);
    unsafe {
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
        DeleteFileA(cz(out_file).as_ptr());
    }
    if !finished {
        return (124, all);
    }
    let code = read_file(&code_file).and_then(|b| core::str::from_utf8(&b).ok().and_then(|s| s.trim().parse::<i32>().ok())).unwrap_or(0);
    (code, all)
}

fn is_builtin(c: &str) -> bool {
    ["dir", "copy", "del", "erase", "type", "echo", "set", "ver", "md", "mkdir", "rd", "rmdir", "ren", "move", "cd", "cls", "vol", "path", "date", "time"].contains(&c)
}

/// Командная строка из argv (кавычки у аргументов с пробелами).
fn join_argv(argv: &[String]) -> String {
    let mut s = String::new();
    for a in argv {
        if !s.is_empty() {
            s.push(' ');
        }
        if a.contains(' ') && !a.starts_with('"') {
            s.push('"');
            s.push_str(a);
            s.push('"');
        } else {
            s.push_str(a);
        }
    }
    s
}

// ─── канал COM2 ─────────────────────────────────────────────────────────

/// Порт COM2 для прямой работы с UART.
const UART: u16 = 0x2F8;

fn outb(port: u16, v: u8) {
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack)) }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack)) };
    v
}

struct Port {
    /// Дескриптор COM2 Windows; `INVALID` — UART напрямую (Windows 9x, которая порта не знает).
    h: HANDLE,
    buf: Vec<u8>,
}

impl Port {
    /// UART 0x2F8 напрямую: у Windows 9x, установленной без этого порта, VCOMM его не держит, и
    /// программе Win32 IN/OUT к нему разрешены. Есть ли там UART — по регистру scratch.
    fn open_raw() -> Option<Port> {
        if is_nt() {
            return None;
        }
        outb(UART + 7, 0x5A);
        if inb(UART + 7) != 0x5A {
            return None;
        }
        outb(UART + 1, 0);
        outb(UART + 3, 0x80);
        outb(UART, 1); // 115200
        outb(UART + 1, 0);
        outb(UART + 3, 0x03);
        outb(UART + 2, 0xC7);
        outb(UART + 4, 0x03);
        log("COM2: Windows порта не знает — UART 0x2F8 напрямую");
        Some(Port { h: INVALID, buf: Vec::new() })
    }

    fn open() -> Option<Port> {
        unsafe {
            let h = CreateFileA(b"COM2\0".as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0);
            if h == INVALID {
                return Port::open_raw();
            }
            SetupComm(h, 65536, 65536);
            let mut d: Dcb = core::mem::zeroed();
            d.len = core::mem::size_of::<Dcb>() as u32;
            GetCommState(h, &mut d);
            d.baud = 115_200;
            // fBinary, DTR включён, RTS включён, без программного и аппаратного управления потоком.
            d.flags = 1 | (1 << 4) | (1 << 12);
            d.byte_size = 8;
            d.parity = 0;
            d.stop_bits = 0;
            SetCommState(h, &d);
            // Чтение: вернуться, как только есть байты, или через 200 мс.
            let t = CommTimeouts { read_interval: u32::MAX, read_mult: u32::MAX, read_const: 200, write_mult: 0, write_const: 0 };
            SetCommTimeouts(h, &t);
            Some(Port { h, buf: Vec::new() })
        }
    }

    fn send(&mut self, f: &Frame) {
        let b = f.encode();
        if self.h == INVALID {
            for &c in &b {
                while inb(UART + 5) & 0x20 == 0 {}
                outb(UART, c);
            }
            return;
        }
        let mut off = 0;
        while off < b.len() {
            let mut w = 0u32;
            let n = (b.len() - off).min(4096) as u32;
            if unsafe { WriteFile(self.h, b[off..].as_ptr(), n, &mut w, core::ptr::null_mut()) } == 0 {
                return;
            }
            off += w as usize;
        }
    }

    /// Следующий кадр, если пришёл за ~200 мс.
    fn poll(&mut self) -> Option<Frame> {
        if self.h == INVALID {
            // Опрос: байты, пока идут; ничего за ~200 мс — вернуться (спим по 1 мс).
            let t0 = unsafe { GetTickCount() };
            loop {
                let mut any = false;
                while inb(UART + 5) & 1 != 0 {
                    self.buf.push(inb(UART));
                    any = true;
                }
                if any && self.buf.windows(4).any(|w| w == MAGIC) && self.buf.len() >= 8 {
                    let i = self.buf.windows(4).position(|w| w == MAGIC).unwrap_or(0);
                    if self.buf.len() >= i + 8 {
                        let len = u32::from_le_bytes([self.buf[i + 4], self.buf[i + 5], self.buf[i + 6], self.buf[i + 7]]) as usize;
                        if self.buf.len() >= i + 8 + len {
                            break;
                        }
                    }
                }
                if !any {
                    if unsafe { GetTickCount() }.wrapping_sub(t0) > 200 {
                        break;
                    }
                    unsafe { Sleep(1) };
                }
            }
        } else {
            let mut tmp = [0u8; 4096];
            let mut got = 0u32;
            unsafe { ReadFile(self.h, tmp.as_mut_ptr(), tmp.len() as u32, &mut got, core::ptr::null_mut()) };
            self.buf.extend_from_slice(&tmp[..got as usize]);
        }
        loop {
            let Some(i) = self.buf.windows(4).position(|w| w == MAGIC) else {
                let keep = self.buf.len().min(3);
                self.buf.drain(..self.buf.len() - keep);
                return None;
            };
            self.buf.drain(..i);
            if self.buf.len() < 8 {
                return None;
            }
            let len = u32::from_le_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]) as usize;
            if !(4..=MAX_FRAME).contains(&len) {
                self.buf.drain(..4);
                continue;
            }
            if self.buf.len() < 8 + len {
                return None;
            }
            let body: Vec<u8> = self.buf[8..8 + len].to_vec();
            self.buf.drain(..8 + len);
            if let Some(f) = decode_body(&body) {
                return Some(f);
            }
        }
    }
}

// ─── агент ──────────────────────────────────────────────────────────────

/// Сейчас: секунды с 1970 (UTC).
fn unix_now() -> u64 {
    let mut t: SystemTime = unsafe { core::mem::zeroed() };
    unsafe { GetSystemTime(&mut t) };
    let days = days_from_civil(t.year as i64, t.month as i64, t.day as i64);
    (days * 86400 + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64) as u64
}

/// Дни с 1970-01-01 (алгоритм Говарда Хиннанта).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn hello() -> Msg {
    let v = version();
    let csd = from_ansi(zstr(&v.csd));
    let name = match os_tag() {
        "95" => "Windows 95",
        "98" => "Windows 98",
        "me" => "Windows ME",
        "xp" => "Windows XP",
        "2000" => "Windows 2000",
        _ => "Windows",
    };
    let os = format!("{name} {}.{}.{} {}", v.major, v.minor, v.build & 0xFFFF, csd.trim());
    let mut buf = [0u8; 64];
    let mut n = buf.len() as u32;
    unsafe { GetComputerNameA(buf.as_mut_ptr(), &mut n) };
    let host = from_ansi(&buf[..n as usize]);
    // Метка загрузки: время включения с точностью до минуты.
    let boot = format!("{}", (unix_now() - (unsafe { GetTickCount() } / 1000) as u64) / 60);
    // Метка запуска — одна на процесс (по ней хост отличает перезапуск агента).
    static INSTANCE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    if INSTANCE.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        let v = ((unsafe { GetCurrentProcessId() } as u64) << 32) | unsafe { GetTickCount() } as u64;
        INSTANCE.store(v.max(1), core::sync::atomic::Ordering::Relaxed);
    }
    let instance = INSTANCE.load(core::sync::atomic::Ordering::Relaxed);
    Msg::Hello { version: VERSION, agent: format!("syn-vm-agent-9x {}", env!("CARGO_PKG_VERSION")), os: os.trim().into(), host, boot, instance, user: None, max_chunk: None }
}

/// Приём файла (Put → Data… → PutEnd).
struct Upload {
    id: u64,
    h: HANDLE,
    size: u64,
    sha: Sha256,
    /// Когда пришёл последний кусок (тики): брошенная передача закрывается через минуту.
    t: u32,
}

/// Рабочий цикл: шаги установки, затем кадры с COM2.
fn run() -> u32 {
    // Один агент на систему: второй (автозапуск и ручной run, обновление) за COM2 не борется.
    // Прежний может как раз уходить (обновление) — подождать его до 15 с.
    let mut tries = 0;
    loop {
        let m = unsafe { CreateMutexA(core::ptr::null(), 1, b"syn-vm-agent-9x\0".as_ptr()) };
        if unsafe { GetLastError() } != 183 {
            break; // ERROR_ALREADY_EXISTS — нет
        }
        unsafe { CloseHandle(m) };
        tries += 1;
        if tries > 15 {
            return 0;
        }
        unsafe { Sleep(1000) };
    }
    continue_setup();
    let mut port = loop {
        match Port::open() {
            Some(p) => break p,
            None => {
                log("COM2 не открывается — повтор через 5 с");
                unsafe { Sleep(5000) };
            }
        }
    };
    log(&format!("агент запущен ({})", os_tag()));
    #[allow(static_mut_refs)]
    unsafe {
        InitializeCriticalSection(CS.as_mut_ptr());
    }
    tx(&Frame::new(hello()));
    let mut up: Option<Upload> = None;
    loop {
        flush(&mut port);
        let f = port.poll();
        // Брошенная передача (хост оборвал по тайм-ауту) — закрыть файл, иначе он останется занятым.
        if up.as_ref().is_some_and(|u| unsafe { GetTickCount() }.wrapping_sub(u.t) > 60_000) {
            close_upload(&mut up);
        }
        let Some(f) = f else { continue };
        handle(&mut port, &mut up, f);
        flush(&mut port);
    }
}

fn err(_port: &mut Port, id: u64, msg: String) {
    tx(&Frame::new(Msg::Error { id, msg }));
}

fn close_upload(up: &mut Option<Upload>) {
    if let Some(u) = up.take() {
        unsafe { CloseHandle(u.h) };
        log(&format!("передача {} не закончена — файл закрыт", u.id));
    }
}

fn handle(port: &mut Port, up: &mut Option<Upload>, f: Frame) {
    match f.msg {
        Msg::Hi => {
            // Хост переподключился — незаконченная передача уже не продолжится.
            close_upload(up);
            tx(&Frame::new(hello()));
        }
        Msg::Ping { id } => tx(&Frame::new(Msg::Pong { id })),
        Msg::Exec { id, argv, cwd, .. } => {
            let job = alloc::boxed::Box::new(Job { id, cmd: join_argv(&argv), cwd });
            let p = alloc::boxed::Box::into_raw(job) as *mut u8;
            let mut tid = 0;
            let h = unsafe { CreateThread(core::ptr::null(), 0, exec_thread, p, 0, &mut tid) };
            if h == 0 {
                drop(unsafe { alloc::boxed::Box::from_raw(p as *mut Job) });
                err(port, id, String::from("поток команды не создан"));
            } else {
                unsafe { CloseHandle(h) };
            }
        }
        Msg::Kill { id } => proc_kill(id),
        Msg::Put { id, path, size } => {
            // Прежняя передача не закончилась (обрыв) — её файл закрыть, иначе он занят.
            close_upload(up);
            if let Some(dir) = path.rfind('\\').map(|i| &path[..i]) {
                mkdirs(dir);
            }
            let h = unsafe { CreateFileA(cz(&path).as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0) };
            if h == INVALID {
                err(port, id, format!("{path}: не открывается на запись (ошибка {})", unsafe { GetLastError() }));
                return;
            }
            let _ = size;
            *up = Some(Upload { id, h, size: 0, sha: Sha256::new(), t: unsafe { GetTickCount() } });
        }
        Msg::Data { id } => {
            if let Some(u) = up.as_mut().filter(|u| u.id == id) {
                let mut w = 0u32;
                unsafe { WriteFile(u.h, f.data.as_ptr(), f.data.len() as u32, &mut w, core::ptr::null_mut()) };
                u.size += f.data.len() as u64;
                u.sha.update(&f.data);
                u.t = unsafe { GetTickCount() };
            }
        }
        Msg::PutEnd { id } => {
            if let Some(u) = up.take().filter(|u| u.id == id) {
                unsafe { CloseHandle(u.h) };
                let sha = hex(&u.sha.finalize());
                tx(&Frame::new(Msg::Done { id, size: u.size, sha256: sha }));
            }
        }
        Msg::Mkdir { id, path } => {
            if mkdirs(&path) {
                tx(&Frame::new(Msg::Done { id, size: 0, sha256: String::new() }));
            } else {
                err(port, id, format!("{path}: каталог не создан"));
            }
        }
        Msg::Get { id, path } => match read_file(&path) {
            Some(d) => {
                tx(&Frame::new(Msg::Stat { id, size: d.len() as u64 }));
                let mut sha = Sha256::new();
                for c in d.chunks(32 * 1024) {
                    sha.update(c);
                    tx(&Frame::with_data(Msg::Data { id }, c.to_vec()));
                }
                tx(&Frame::new(Msg::End { id, sha256: hex(&sha.finalize()) }));
            }
            None => err(port, id, format!("{path}: не читается")),
        },
        Msg::List { id, path } => {
            let mut entries = Vec::new();
            if is_dir(&path) {
                list(&path, "", &mut entries);
            } else if let Some(d) = read_file(&path) {
                entries.push(Entry { path: String::new(), dir: false, size: d.len() as u64 });
            } else {
                err(port, id, format!("{path}: нет такого файла"));
                return;
            }
            tx(&Frame::new(Msg::Entries { id, entries }));
        }
        Msg::SetTime { unix_ms } => {
            let secs = (unix_ms / 1000) as i64;
            let (y, m, d) = civil_from_days(secs.div_euclid(86400));
            let rem = secs.rem_euclid(86400);
            let t = SystemTime { year: y as u16, month: m as u16, dow: 0, day: d as u16, hour: (rem / 3600) as u16, minute: (rem % 3600 / 60) as u16, second: (rem % 60) as u16, ms: (unix_ms % 1000) as u16 };
            unsafe { SetSystemTime(&t) };
        }
        Msg::Update { id, path } => {
            // Новый exe записан хостом: заменить себя и перезапуститься.
            let exe = format!("{}\\SYNAGENT.EXE", home());
            let me = self_path();
            let target = if me.eq_ignore_ascii_case(&exe) { exe.clone() } else { me };
            // Работающий exe у 9x не перезаписать — новая копия запускается сама и уходит в C:\SYNVM.
            tx(&Frame::new(Msg::Done { id, size: 0, sha256: String::new() }));
            if spawn_detached(&format!("{path} install"), None) {
                log(&format!("обновление: {path} → {target}"));
                unsafe { ExitProcess(0) };
            }
        }
        // Остальное (журналы, буфер обмена, разрешение, общие папки) у старых систем пока не поддержано.
        Msg::Stdin { .. } | Msg::Logs { .. } | Msg::Clip { .. } | Msg::Mode { .. } | Msg::Share { .. } => {}
        Msg::Status { .. } => {}
        _ => {}
    }
}

/// Рекурсивный список каталога (пути через `/`).
fn list(dir: &str, rel: &str, out: &mut Vec<Entry>) {
    for (name, is_dir) in read_dir(dir) {
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let full = format!("{dir}\\{name}");
        if is_dir {
            out.push(Entry { path: r.clone(), dir: true, size: 0 });
            list(&full, &r, out);
        } else {
            let size = read_size(&full);
            out.push(Entry { path: r, dir: false, size });
        }
    }
}

fn read_size(path: &str) -> u64 {
    unsafe {
        let h = CreateFileA(cz(path).as_ptr(), GENERIC_READ, FILE_SHARE_RW, core::ptr::null(), OPEN_EXISTING, 0, 0);
        if h == INVALID {
            return 0;
        }
        let s = GetFileSize(h, core::ptr::null_mut());
        CloseHandle(h);
        s as u64
    }
}
