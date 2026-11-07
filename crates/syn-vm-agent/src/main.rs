//! syn-vm-agent — гостевой агент syn-vm (служба Windows / служба systemd в Linux).
//!
//! Держит порт virtio-serial `org.syn.vm.agent.0` и выполняет запросы хоста: команды (в том
//! числе в интерактивном сеансе вошедшего пользователя) с потоковым выводом, файлы и каталоги
//! с SHA-256, журналы гостя (OutputDebugString, журнал событий Windows, journald), события
//! «агент запущен» и «пользователь вошёл».
//!
//! Запуск: `syn-vm-agent service` — службой (Windows: SCM; Linux: systemd), `syn-vm-agent run` —
//! в консоли, `syn-vm-agent install` — поставить службой (копия в Program Files или
//! /usr/local/bin), `syn-vm-agent uninstall`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use syn_vm_agent::{hex, read_frame, Digest, Entry, Frame, Msg, Sha256, CHUNK, VERSION};

#[cfg(unix)]
#[path = "sys_unix.rs"]
mod sys;
#[cfg(windows)]
#[path = "sys_win.rs"]
mod sys;

/// Запрос на запуск программы.
pub struct ExecReq {
    pub argv: Vec<String>,
    pub user: bool,
    pub cwd: Option<String>,
    pub env: Vec<String>,
    pub stdin: bool,
}

/// Запущенный процесс: вывод, ввод и ожидание кода.
pub struct Proc {
    pub pid: u32,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
    pub stdin: Option<Box<dyn Write + Send>>,
    pub wait: Box<dyn FnOnce() -> i32 + Send>,
}

// ─── Связь с хостом ─────────────────────────────────────────────────────

/// Писатель текущего соединения (после переоткрытия порта — новый).
fn link() -> &'static Mutex<Option<Box<dyn Write + Send>>> {
    static L: OnceLock<Mutex<Option<Box<dyn Write + Send>>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(None))
}

/// Отправить кадр (нет связи — молча пропустить: хост узнает о потере по Hello).
pub fn send(f: Frame) {
    let mut l = link().lock().unwrap();
    if let Some(w) = l.as_mut() {
        if let Err(e) = syn_vm_agent::write_frame(w, &f) {
            log(&format!("запись в порт: {e}"));
            *l = None;
        }
    }
}

static LOGS_ON: AtomicBool = AtomicBool::new(false);

/// Строка журнала гостя хосту (если он подписан).
pub fn guest_log(src: &str, text: &str) {
    if LOGS_ON.load(Ordering::Relaxed) {
        send(Frame::with_data(Msg::Log { src: src.into() }, text.as_bytes().to_vec()));
    }
}

/// Свой журнал агента (stderr; в Windows — ещё файл в ProgramData).
pub fn log(s: &str) {
    sys::agent_log(s);
}

// ─── Вспомогательный процесс в сеансе пользователя ──────────────────────
//
// Служба (Windows: сеанс 0) не видит буфер обмена и экран пользователя: это делает её копия
// `syn-vm-agent session`, запущенная в интерактивном сеансе. Обмен — кадрами протокола через
// stdin/stdout копии.

/// Очередь кадров копии в сеансе пользователя (нет — копия не запущена). Пишет её отдельный
/// поток: основной цикл чтения порта никогда не ждёт копию (зависшая копия раньше могла
/// остановить приём от хоста).
fn helper() -> &'static Mutex<Option<std::sync::mpsc::Sender<Frame>>> {
    static H: OnceLock<Mutex<Option<std::sync::mpsc::Sender<Frame>>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

/// Последние Share и Mode от хоста — новой копии (после входа пользователя) отдаются сразу.
fn last_for_helper() -> &'static Mutex<Vec<Frame>> {
    static L: OnceLock<Mutex<Vec<Frame>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Vec::new()))
}

fn to_helper(f: &Frame) {
    let mut h = helper().lock().unwrap();
    if let Some(tx) = h.as_ref() {
        if tx.send(f.clone()).is_err() {
            *h = None;
        }
    }
}

fn remember_for_helper(f: &Frame) {
    let mut l = last_for_helper().lock().unwrap();
    // Папок несколько — своя запись на каждую цель; режим экрана — одна.
    let same = |a: &Msg, b: &Msg| match (a, b) {
        (Msg::Share { target: x, .. }, Msg::Share { target: y, .. }) => x == y,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    };
    l.retain(|x| !same(&x.msg, &f.msg));
    l.push(f.clone());
}

/// Держать копию в сеансе пользователя, пока он вошёл (Windows).
fn session_manager() {
    if !sys::USES_HELPER {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(2));
        if helper().lock().unwrap().is_some() || sys::session_user().is_none() {
            continue;
        }
        let Ok(me) = std::env::current_exe() else { continue };
        let req = ExecReq { argv: vec![me.to_string_lossy().into_owned(), "session".into()], user: true, cwd: None, env: vec![], stdin: true };
        let p = match sys::spawn(&req) {
            Ok(p) => p,
            Err(e) => {
                log(&format!("копия в сеансе пользователя: {e}"));
                continue;
            }
        };
        log(&format!("копия в сеансе пользователя: pid {}", p.pid));
        let Some(mut stdin) = p.stdin else { continue };
        let (tx, rx) = std::sync::mpsc::channel::<Frame>();
        std::thread::spawn(move || {
            for f in rx {
                if syn_vm_agent::write_frame(&mut stdin, &f).is_err() {
                    break;
                }
            }
        });
        *helper().lock().unwrap() = Some(tx);
        for f in last_for_helper().lock().unwrap().clone() {
            to_helper(&f);
        }
        let mut out = p.stdout;
        let mut err = p.stderr;
        std::thread::spawn(move || {
            let mut b = [0u8; 4096];
            while matches!(err.read(&mut b), Ok(n) if n > 0) {}
        });
        let wait = p.wait;
        std::thread::spawn(move || {
            // Из копии — только буфер обмена (хосту как есть).
            while let Ok(f) = read_frame(&mut out) {
                if matches!(f.msg, Msg::Clip { .. }) {
                    send(f);
                }
            }
            *helper().lock().unwrap() = None;
            let code = wait();
            log(&format!("копия в сеансе пользователя завершилась ({code})"));
        });
    });
}

fn instance() -> u64 {
    static I: OnceLock<u64> = OnceLock::new();
    *I.get_or_init(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        t ^ (std::process::id() as u64) << 32
    })
}

/// Хост подписан на журналы.
pub fn logs_on() -> bool {
    LOGS_ON.load(Ordering::Relaxed)
}

/// Метка загрузки — один раз за запуск агента (не меняется от часов).
pub fn boot() -> String {
    static B: OnceLock<String> = OnceLock::new();
    B.get_or_init(sys::boot_id).clone()
}

fn hello() -> Frame {
    Frame::new(Msg::Hello {
        version: VERSION,
        agent: env!("CARGO_PKG_VERSION").into(),
        os: sys::os_name(),
        host: sys::hostname(),
        boot: boot(),
        instance: instance(),
        user: sys::session_user(),
    })
}

// ─── Операции ───────────────────────────────────────────────────────────

/// Живые команды: ввод и pid (для Kill).
struct Running {
    pid: u32,
    stdin: Option<Box<dyn Write + Send>>,
}

fn procs() -> &'static Mutex<HashMap<u64, Running>> {
    static P: OnceLock<Mutex<HashMap<u64, Running>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Файлы, которые пишет хост.
struct Upload {
    file: std::fs::File,
    path: String,
    hash: Sha256,
    size: u64,
    want: u64,
}

fn uploads() -> &'static Mutex<HashMap<u64, Upload>> {
    static U: OnceLock<Mutex<HashMap<u64, Upload>>> = OnceLock::new();
    U.get_or_init(|| Mutex::new(HashMap::new()))
}

fn err(id: u64, msg: impl Into<String>) {
    send(Frame::new(Msg::Error { id, msg: msg.into() }));
}

fn exec(id: u64, req: ExecReq) {
    if req.argv.is_empty() {
        return err(id, "пустая команда");
    }
    let p = match sys::spawn(&req) {
        Ok(p) => p,
        Err(e) => return err(id, format!("{}: {e}", req.argv[0])),
    };
    send(Frame::new(Msg::Started { id, pid: p.pid }));
    procs().lock().unwrap().insert(id, Running { pid: p.pid, stdin: p.stdin });
    let pump = |mut r: Box<dyn Read + Send>, fd: u8| {
        std::thread::spawn(move || {
            let mut buf = vec![0u8; CHUNK];
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => send(Frame::with_data(Msg::Out { id, fd }, buf[..n].to_vec())),
                }
            }
        })
    };
    let (o, e) = (pump(p.stdout, 1), pump(p.stderr, 2));
    let wait = p.wait;
    std::thread::spawn(move || {
        let code = wait();
        // Весь вывод — до кода возврата (потомки, унаследовавшие канал, держат его открытым —
        // не ждать их дольше пары секунд).
        for h in [o, e] {
            let t = std::time::Instant::now();
            while !h.is_finished() && t.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        procs().lock().unwrap().remove(&id);
        send(Frame::new(Msg::Exit { id, code }));
    });
}

fn put_begin(id: u64, path: String, size: u64) {
    let p = std::path::Path::new(&path);
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return err(id, format!("{}: {e}", dir.display()));
        }
    }
    match std::fs::File::create(p) {
        Ok(file) => {
            uploads().lock().unwrap().insert(id, Upload { file, path, hash: Sha256::new(), size: 0, want: size });
        }
        Err(e) => err(id, format!("{path}: {e}")),
    }
}

fn put_data(id: u64, data: &[u8]) {
    let mut u = uploads().lock().unwrap();
    let Some(up) = u.get_mut(&id) else { return };
    if let Err(e) = up.file.write_all(data) {
        let path = up.path.clone();
        u.remove(&id);
        drop(u);
        return err(id, format!("{path}: {e}"));
    }
    up.hash.update(data);
    up.size += data.len() as u64;
}

fn put_end(id: u64) {
    let Some(mut up) = uploads().lock().unwrap().remove(&id) else { return };
    if let Err(e) = up.file.flush().and_then(|_| up.file.sync_all()) {
        return err(id, format!("{}: {e}", up.path));
    }
    if up.size != up.want {
        return err(id, format!("{}: записано {} байт из {}", up.path, up.size, up.want));
    }
    send(Frame::new(Msg::Done { id, size: up.size, sha256: hex(&up.hash.finalize()) }));
}

fn get(id: u64, path: String) {
    std::thread::spawn(move || {
        let mut f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => return err(id, format!("{path}: {e}")),
        };
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        send(Frame::new(Msg::Stat { id, size }));
        let mut h = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    h.update(&buf[..n]);
                    send(Frame::with_data(Msg::Data { id }, buf[..n].to_vec()));
                }
                Err(e) => return err(id, format!("{path}: {e}")),
            }
        }
        send(Frame::new(Msg::End { id, sha256: hex(&h.finalize()) }));
    });
}

fn list(id: u64, path: String) {
    std::thread::spawn(move || {
        let root = std::path::PathBuf::from(&path);
        let md = match std::fs::metadata(&root) {
            Ok(m) => m,
            Err(e) => return err(id, format!("{path}: {e}")),
        };
        let mut entries = Vec::new();
        if md.is_dir() {
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    let Ok(m) = e.metadata() else { continue };
                    let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                    entries.push(Entry { path: rel, dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() } });
                    if m.is_dir() {
                        stack.push(p);
                    }
                }
            }
            entries.sort_by(|a, b| a.path.cmp(&b.path));
        } else {
            entries.push(Entry { path: String::new(), dir: false, size: md.len() });
        }
        send(Frame::new(Msg::Entries { id, entries }));
    });
}

fn handle(f: Frame) {
    match f.msg {
        Msg::Hi => send(hello()),
        Msg::Ping { id } => send(Frame::new(Msg::Pong { id })),
        Msg::Exec { id, argv, user, cwd, env, stdin } => exec(id, ExecReq { argv, user, cwd, env, stdin }),
        Msg::Stdin { id } => {
            let mut p = procs().lock().unwrap();
            if let Some(r) = p.get_mut(&id) {
                if f.data.is_empty() {
                    r.stdin = None;
                } else if let Some(w) = r.stdin.as_mut() {
                    if w.write_all(&f.data).and_then(|_| w.flush()).is_err() {
                        r.stdin = None;
                    }
                }
            }
        }
        Msg::Kill { id } => {
            if let Some(pid) = procs().lock().unwrap().get(&id).map(|r| r.pid) {
                sys::kill(pid);
            }
        }
        Msg::Put { id, path, size } => put_begin(id, path, size),
        Msg::Data { id } => put_data(id, &f.data),
        Msg::PutEnd { id } => put_end(id),
        Msg::Mkdir { id, path } => match std::fs::create_dir_all(&path) {
            Ok(()) => send(Frame::new(Msg::Done { id, size: 0, sha256: String::new() })),
            Err(e) => err(id, format!("{path}: {e}")),
        },
        Msg::Get { id, path } => get(id, path),
        Msg::List { id, path } => list(id, path),
        Msg::SetTime { unix_ms } => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            let diff = unix_ms as i64 - now;
            // Расхождение меньше секунды — не трогать.
            if diff.abs() >= 1000 {
                match sys::set_time(unix_ms) {
                    Ok(()) => log(&format!("время: поправка {:+.1} с", diff as f64 / 1000.0)),
                    Err(e) => log(&format!("время: {e}")),
                }
            }
        }
        Msg::Logs { on } => {
            LOGS_ON.store(on, Ordering::Relaxed);
        }
        Msg::Clip { .. } => {
            if sys::USES_HELPER {
                to_helper(&f);
            }
        }
        Msg::Mode { .. } => {
            if sys::USES_HELPER {
                remember_for_helper(&f);
                to_helper(&f);
            }
        }
        Msg::Share { ref kind, ref target, ref source } => {
            // Своё службы (реестр SMB, монтирование в Linux), затем — копии (подключить диск).
            if let Err(e) = sys::share_prepare(kind, target, source) {
                log(&format!("общая папка: {e}"));
            }
            if sys::USES_HELPER {
                remember_for_helper(&f);
                to_helper(&f);
            }
        }
        Msg::Update { id, path } => match sys::update(&path) {
            Ok(()) => {
                send(Frame::new(Msg::Done { id, size: 0, sha256: String::new() }));
                std::thread::sleep(Duration::from_millis(300));
                log("обновление: перезапуск");
                // Служба перезапустит (Windows — действие при сбое, systemd — Restart=always).
                std::process::exit(3);
            }
            Err(e) => err(id, format!("обновление: {e}")),
        },
        _ => {}
    }
}

/// Главный цикл: открыть порт, поздороваться, обрабатывать кадры; связь пропала — заново.
pub fn run() {
    log(&format!("syn-vm-agent {} запущен", env!("CARGO_PKG_VERSION")));
    sys::start_logs();
    session_manager();
    // Пользователь сеанса вошёл или вышел — событие хосту.
    std::thread::spawn(|| {
        let mut last = sys::session_user();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let u = sys::session_user();
            if u != last {
                log(&format!("сеанс: {u:?}"));
                send(Frame::new(Msg::Session { user: u.clone() }));
                last = u;
            }
        }
    });
    static OPENS: AtomicU64 = AtomicU64::new(0);
    loop {
        // Порт virtio-serial (если в госте есть его драйвер), иначе — сеть (guestfwd QEMU).
        let (mut r, w, via) = match sys::open_port() {
            Ok((r, w)) => (r, w, "virtio"),
            Err(pe) => match open_net() {
                Ok((r, w)) => (r, w, "net"),
                Err(ne) => {
                    if OPENS.fetch_add(1, Ordering::Relaxed) % 60 == 0 {
                        log(&format!("порт {}: {pe}; сеть {}: {ne}", syn_vm_agent::PORT, syn_vm_agent::NET_ADDR));
                    }
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            },
        };
        log(&format!("канал: {via}"));
        OPENS.store(0, Ordering::Relaxed);
        *link().lock().unwrap() = Some(w);
        send(hello());
        loop {
            match read_frame(&mut r) {
                Ok(f) => handle(f),
                Err(e) => {
                    log(&format!("чтение порта: {e}"));
                    break;
                }
            }
        }
        *link().lock().unwrap() = None;
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Канал по сети: TCP к хосту через guestfwd.
fn open_net() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    use std::net::{TcpStream, ToSocketAddrs};
    let addr = syn_vm_agent::NET_ADDR.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("адрес"))?;
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_nodelay(true)?;
    let w = s.try_clone()?;
    Ok((Box::new(s), Box::new(w)))
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let r: io::Result<()> = match cmd.as_str() {
        "service" => sys::service_main(),
        "run" => {
            run();
            Ok(())
        }
        "session" => sys::helper_main(),
        "install" => sys::install(),
        "uninstall" => sys::uninstall(),
        "-V" | "--version" => {
            println!("syn-vm-agent {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("syn-vm-agent {} — гостевой агент syn-vm\n  install | uninstall | service | run", env!("CARGO_PKG_VERSION"));
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("syn-vm-agent: {e}");
        std::process::exit(1);
    }
}
