//! Свой гостевой агент (крейт syn-vm-agent): посредник на хосте и клиент.
//!
//! Посредник (`syn-vm --agent-broker МАШИНА`, отдельная сессия на машину) держит единственное
//! постоянное соединение с портом virtio-serial агента (сокет chardev QEMU) и обслуживает любое
//! число клиентов на `agent.sock`: номера операций клиентов переводятся в свои, ответы гостя
//! возвращаются тому, кто спросил. Нет очереди «агент занят» и переподключений к гостю на
//! каждый запрос. Журналы гостя посредник пишет в `guest.log` рядом с машиной.
//!
//! Агент перезапустился или гость перезагрузился (новые метки в `Hello`) — незавершённые
//! операции получают честную ошибку.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::json;
use syn_vm_agent::{hex, read_frame, write_frame, Digest, Entry, Frame, Msg, Sha256, CHUNK, PORT};
use syngui::t;

use crate::model::Vm;

pub fn port_path(vm: &Vm) -> PathBuf {
    vm.run_dir().join("agent-port.sock")
}

/// Адрес агента в сети гостя (guestfwd QEMU) — должен совпадать с syn_vm_agent::NET_ADDR.
pub const NET_ADDR: &str = syn_vm_agent::NET_ADDR;

/// Сокет канала агента по сети (guestfwd).
pub fn net_path(vm: &Vm) -> PathBuf {
    vm.run_dir().join("agent-net.sock")
}

pub fn broker_path(vm: &Vm) -> PathBuf {
    vm.run_dir().join("agent.sock")
}

/// Журнал гостя (OutputDebugString, журнал событий, journald).
pub fn log_path(vm: &Vm) -> PathBuf {
    vm.dir().join("guest.log")
}

/// Отметка «в госте стоит свой агент» (последний Hello) — по ней --wait-boot ждёт его, а не qemu-ga.
fn seen_path(vm: &Vm) -> PathBuf {
    vm.dir().join("agent.json")
}

pub fn installed(vm: &Vm) -> bool {
    seen_path(vm).exists()
}

/// Аргументы QEMU: порт агента на шине virtio-serial `vser`.
pub fn qemu_args(vm: &Vm) -> Vec<String> {
    vec![
        "-chardev".into(),
        format!("socket,id=synagent,path={},server=on,wait=off", port_path(vm).display()),
        "-device".into(),
        format!("virtserialport,bus=vser.0,chardev=synagent,name={PORT},id=synagentport"),
    ]
}

/// Порт агента у работающей машины, запущенной без него, — добавить на ходу (QMP).
fn ensure_port(vm: &Vm) -> Result<()> {
    if port_path(vm).exists() {
        return Ok(());
    }
    let path = port_path(vm).to_string_lossy().into_owned();
    let mut q = crate::qmp::Conn::of(vm)?;
    q.call("chardev-add", json!({"id": "synagent", "backend": {"type": "socket", "data": {"addr": {"type": "unix", "data": {"path": path}}, "server": true, "wait": false}}}))?;
    q.call("device_add", json!({"driver": "virtserialport", "bus": "vser.0", "chardev": "synagent", "name": PORT, "id": "synagentport"}))?;
    tracing::info!("агент: порт {PORT} добавлен в работающую машину");
    Ok(())
}

// ─── Посредник ──────────────────────────────────────────────────────────

type Tx = Arc<Mutex<UnixStream>>;

#[derive(Default)]
struct Broker {
    guest: Option<Tx>,
    hello: Option<Msg>,
    user: Option<String>,
    /// Адреса гостя (`Net`).
    ips: Vec<String>,
    next: u64,
    /// Свой номер → (клиент, его номер).
    routes: HashMap<u64, (u64, u64)>,
    /// (клиент, его номер) → свой.
    back: HashMap<(u64, u64), u64>,
    /// Номера запущенных команд (клиент ушёл — завершить их).
    execs: HashSet<u64>,
    clients: HashMap<u64, Tx>,
    log_subs: HashSet<u64>,
    clip_subs: HashSet<u64>,
}

impl Broker {
    fn to_client(&mut self, gid: u64, msg: Msg, data: Vec<u8>) {
        let Some(&(cid, lid)) = self.routes.get(&gid) else { return };
        let fin = msg.is_final();
        if let Some(tx) = self.clients.get(&cid) {
            let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::with_data(msg.with_id(lid), data));
        }
        if fin {
            self.routes.remove(&gid);
            self.back.remove(&(cid, lid));
            self.execs.remove(&gid);
        }
    }

    /// Прервать все операции с ошибкой (перезапуск агента, перезагрузка гостя, обрыв).
    fn fail_all(&mut self, why: &str) {
        for gid in self.routes.keys().copied().collect::<Vec<_>>() {
            self.to_client(gid, Msg::Error { id: 0, msg: why.to_string() }, Vec::new());
        }
    }
}

fn append_log(vm: &Vm, line: &str) {
    let path = log_path(vm);
    if std::fs::metadata(&path).map(|m| m.len() > 32 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = chrono_like();
        let _ = writeln!(f, "{t} {line}");
    }
}

/// Местное время ЧЧ:ММ:СС.мс для журнала.
fn chrono_like() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = now.as_secs() as libc::time_t;
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec, now.subsec_millis())
}

/// `syn-vm --agent-broker МАШИНА`: работает, пока работает машина.
pub fn run_broker(vm: &Vm) -> Result<()> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(vm.run_dir())?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(vm.run_dir().join("agent-broker.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(()); // уже работает другой посредник
    }
    if crate::qemu::pid(vm).is_none() {
        bail!("{}", t!("«{name}» не запущена", name = vm.name));
    }
    if let Err(e) = ensure_port(vm) {
        tracing::warn!("агент: порт virtio-serial: {e}");
    }
    let bp = broker_path(vm);
    let _ = std::fs::remove_file(&bp);
    let listener = UnixListener::bind(&bp).with_context(|| bp.display().to_string())?;
    let st = Arc::new(Mutex::new(Broker::default()));

    // Клиенты.
    {
        let (st, vm) = (st.clone(), vm.clone());
        std::thread::spawn(move || {
            for (cid, s) in listener.incoming().flatten().enumerate() {
                let cid = cid as u64 + 1;
                let Ok(w) = s.try_clone() else { continue };
                st.lock().unwrap().clients.insert(cid, Arc::new(Mutex::new(w)));
                let (st, vm) = (st.clone(), vm.clone());
                std::thread::spawn(move || serve_client(&vm, &st, cid, s));
            }
        });
    }

    // Гость: два канала — порт virtio-serial и сеть (guestfwd). Рабочий — тот, откуда пришло
    // приветствие агента; каждый переподключается сам, пока машина работает.
    for (path, kind) in [(port_path(vm), "virtio"), (net_path(vm), "net")] {
        let (st, vm) = (st.clone(), vm.clone());
        std::thread::spawn(move || transport(&vm, &st, &path, kind));
    }
    while crate::qemu::pid(vm).is_some() {
        std::thread::sleep(Duration::from_secs(2));
        // Пакет обновлён (наш exe заменён) — уйти: клиент запустит новый посредник.
        if std::fs::read_link("/proc/self/exe").is_ok_and(|p| p.to_string_lossy().ends_with(" (deleted)")) {
            tracing::info!("агент: syn-vm обновлён — посредник перезапускается");
            let _ = std::fs::remove_file(&bp);
            std::process::exit(0);
        }
    }
    st.lock().unwrap().fail_all(&t!("машина выключилась"));
    let _ = std::fs::remove_file(&bp);
    Ok(())
}

/// Один канал к агенту: соединиться, поздороваться, читать кадры; оборвалось — снова.
fn transport(vm: &Vm, st: &Arc<Mutex<Broker>>, path: &Path, kind: &str) {
    let mut last_note = Instant::now() - Duration::from_secs(60);
    loop {
        if !path.exists() {
            if kind == "virtio" {
                let _ = ensure_port(vm);
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        let s = match UnixStream::connect(path) {
            Ok(s) => s,
            Err(e) => {
                if last_note.elapsed() > Duration::from_secs(30) {
                    tracing::warn!("агент ({kind}): {}: {e}", path.display());
                    last_note = Instant::now();
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let Ok(w) = s.try_clone() else { continue };
        let tx: Tx = Arc::new(Mutex::new(w));
        let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::new(Msg::Hi));
        let mut r = BufReader::with_capacity(256 * 1024, s);
        while let Ok(f) = read_frame(&mut r) {
            // Приветствие — этот канал рабочий.
            if matches!(f.msg, Msg::Hello { .. }) {
                let mut b = st.lock().unwrap();
                if !b.guest.as_ref().is_some_and(|g| Arc::ptr_eq(g, &tx)) {
                    append_log(vm, &format!("[agent] канал: {kind}"));
                    b.guest = Some(tx.clone());
                }
            }
            guest_frame(vm, st, &tx, f);
        }
        {
            let mut b = st.lock().unwrap();
            if b.guest.as_ref().is_some_and(|g| Arc::ptr_eq(g, &tx)) {
                b.guest = None;
                b.fail_all(&t!("связь с агентом гостя прервалась"));
            }
        }
        if crate::qemu::pid(vm).is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn guest_frame(vm: &Vm, st: &Arc<Mutex<Broker>>, tx: &Tx, f: Frame) {
    let mut b = st.lock().unwrap();
    match f.msg {
        Msg::Hello { ref boot, instance, ref user, ref agent, ref os, .. } => {
            if let Some(Msg::Hello { boot: ob, instance: oi, .. }) = &b.hello {
                if ob != boot {
                    b.fail_all(&t!("гость перезагрузился посреди операции"));
                } else if *oi != instance {
                    b.fail_all(&t!("агент гостя перезапустился посреди операции"));
                }
            }
            let new = !matches!(&b.hello, Some(Msg::Hello { instance: oi, .. }) if *oi == instance);
            if new {
                append_log(vm, &format!("[agent] на связи: syn-vm-agent {agent}, {os}, пользователь: {}", user.as_deref().unwrap_or("—")));
            }
            b.user = user.clone();
            b.hello = Some(f.msg.clone());
            let _ = std::fs::write(seen_path(vm), serde_json::to_vec_pretty(&f.msg).unwrap_or_default());
            let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::new(Msg::Logs { on: true }));
            // Часы гостя — по хосту (RTC после загрузки, паузы, снимка может врать).
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::new(Msg::SetTime { unix_ms: ms }));
            send_share(vm, tx);
        }
        Msg::Session { ref user } => {
            append_log(vm, &format!("[agent] сеанс: {}", user.as_deref().unwrap_or("вышел")));
            // Вошёл — подключить общую папку в его сеансе (агент передаст её своей копии).
            if user.is_some() {
                send_share(vm, tx);
            }
            b.user = user.clone();
            if let Some(Msg::Hello { user: u, .. }) = b.hello.as_mut() {
                *u = user.clone();
            }
        }
        Msg::Net { ref ips } => {
            if b.ips != *ips {
                append_log(vm, &format!("[agent] адреса: {}", if ips.is_empty() { "—".to_string() } else { ips.join(", ") }));
                b.ips = ips.clone();
            }
        }
        Msg::Clip { .. } => {
            for cid in b.clip_subs.clone() {
                if let Some(c) = b.clients.get(&cid) {
                    let _ = write_frame(&mut *c.lock().unwrap(), &f);
                }
            }
        }
        Msg::Log { ref src } => {
            let text = String::from_utf8_lossy(&f.data);
            for l in text.lines().filter(|l| !l.trim().is_empty()) {
                append_log(vm, &format!("[{src}] {l}"));
            }
            for cid in b.log_subs.clone() {
                if let Some(c) = b.clients.get(&cid) {
                    let _ = write_frame(&mut *c.lock().unwrap(), &f);
                }
            }
        }
        ref m => {
            if let Some(gid) = m.id() {
                let msg = f.msg.clone();
                b.to_client(gid, msg, f.data);
            }
        }
    }
}

fn serve_client(vm: &Vm, st: &Arc<Mutex<Broker>>, cid: u64, s: UnixStream) {
    let _ = vm;
    let mut r = BufReader::new(s);
    while let Ok(f) = read_frame(&mut r) {
        let mut b = st.lock().unwrap();
        match f.msg {
            Msg::Status { id } => {
                let reply = Frame::new(Msg::State { id, hello: b.hello.clone().map(Box::new), user: b.user.clone(), ips: b.ips.clone() });
                if let Some(c) = b.clients.get(&cid) {
                    let _ = write_frame(&mut *c.lock().unwrap(), &reply);
                }
            }
            // Подписка на буфер обмена гостя.
            Msg::Clip { ref mime } if mime == "x-subscribe" => {
                b.clip_subs.insert(cid);
            }
            Msg::SetTime { .. } | Msg::Clip { .. } | Msg::Mode { .. } => {
                if let Some(g) = b.guest.clone() {
                    drop(b);
                    let _ = write_frame(&mut *g.lock().unwrap(), &f);
                }
            }
            Msg::Logs { on } => {
                if on {
                    b.log_subs.insert(cid);
                } else {
                    b.log_subs.remove(&cid);
                }
            }
            ref m => {
                let Some(lid) = m.id() else { continue };
                let gid = match b.back.get(&(cid, lid)) {
                    Some(&g) => g,
                    None => {
                        b.next += 1;
                        let g = b.next;
                        b.routes.insert(g, (cid, lid));
                        b.back.insert((cid, lid), g);
                        g
                    }
                };
                if matches!(m, Msg::Exec { .. }) {
                    b.execs.insert(gid);
                }
                let out = Frame::with_data(f.msg.clone().with_id(gid), f.data);
                match b.guest.clone() {
                    Some(g) => {
                        drop(b);
                        if write_frame(&mut *g.lock().unwrap(), &out).is_err() {
                            st.lock().unwrap().to_client(gid, Msg::Error { id: 0, msg: t!("связь с агентом гостя прервалась") }, Vec::new());
                        }
                    }
                    None => b.to_client(gid, Msg::Error { id: 0, msg: t!("агент гостя не на связи") }, Vec::new()),
                }
            }
        }
    }
    // Клиент ушёл: его команды завершить, номера освободить.
    let mut b = st.lock().unwrap();
    b.clients.remove(&cid);
    b.log_subs.remove(&cid);
    b.clip_subs.remove(&cid);
    let mine: Vec<u64> = b.routes.iter().filter(|(_, (c, _))| *c == cid).map(|(g, _)| *g).collect();
    for g in mine {
        if b.execs.remove(&g) {
            if let Some(tx) = b.guest.clone() {
                let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::new(Msg::Kill { id: g }));
            }
        }
        if let Some((c, l)) = b.routes.remove(&g) {
            b.back.remove(&(c, l));
        }
    }
}

/// Общая папка машины — агенту (настройки перечитываются: могли поменяться на ходу).
fn send_share(vm: &Vm, tx: &Tx) {
    let Ok(v) = crate::cli::find(&vm.id) else { return };
    let kind = match v.share() {
        Some(crate::model::ShareMode::Smb) if v.os.is_windows() => "smb",
        Some(crate::model::ShareMode::Virtiofs) if !v.os.is_windows() => "virtiofs",
        Some(crate::model::ShareMode::P9) if !v.os.is_windows() => "9p",
        _ => return,
    };
    // Windows — диски с Z: вниз, Linux — /mnt/hostshare, /mnt/hostshare1…
    for (i, source) in v.share_names(kind == "smb").into_iter().enumerate() {
        let target = if kind == "smb" { format!("{}:", (b'Z' - i.min(20) as u8) as char) } else if i == 0 { "/mnt/hostshare".into() } else { format!("/mnt/hostshare{i}") };
        let _ = write_frame(&mut *tx.lock().unwrap(), &Frame::new(Msg::Share { kind: kind.into(), target, source }));
    }
}

// ─── Клиент ─────────────────────────────────────────────────────────────

/// Сведения агента (последний Hello).
#[derive(Debug, Clone)]
pub struct Info {
    pub instance: u64,
    pub agent: String,
    pub os: String,
    pub host: String,
    pub user: Option<String>,
    pub ips: Vec<String>,
}

pub struct Client {
    w: UnixStream,
    r: BufReader<UnixStream>,
    next: u64,
}

/// Итог команды.
#[derive(Debug, Default)]
pub struct ExecOut {
    pub code: i32,
    pub timed_out: bool,
}

impl Client {
    /// Соединиться с посредником машины (нет — запустить его).
    pub fn connect(vm: &Vm) -> Result<Client> {
        if !vm.guest_agent {
            bail!("{}", t!("«{name}»: канал гостевого агента выключен в настройках (guest_agent = true)", name = vm.name));
        }
        let bp = broker_path(vm);
        let mut spawned = false;
        let t0 = Instant::now();
        loop {
            if let Ok(s) = UnixStream::connect(&bp) {
                let w = s.try_clone()?;
                return Ok(Client { w, r: BufReader::with_capacity(256 * 1024, s), next: 0 });
            }
            if !spawned {
                crate::qemu::spawn_watcher(vm, "--agent-broker");
                spawned = true;
            }
            if t0.elapsed() > Duration::from_secs(10) {
                bail!("{}", t!("«{name}»: посредник агента не запустился (syn-vm --agent-broker {id})", name = vm.name, id = vm.id));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn send(&mut self, f: Frame) -> Result<()> {
        write_frame(&mut self.w, &f).context(t!("посредник агента закрыл соединение"))
    }

    fn recv(&mut self, timeout: Option<Duration>) -> Result<Frame> {
        self.r.get_ref().set_read_timeout(timeout)?;
        read_frame(&mut self.r).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => anyhow::anyhow!("timeout"),
            _ => anyhow::anyhow!(t!("посредник агента закрыл соединение")),
        })
    }

    /// Кадр операции `id` (чужие пропускаются); ошибка гостя — ошибкой.
    fn recv_for(&mut self, id: u64, timeout: Option<Duration>) -> Result<Frame> {
        loop {
            let f = self.recv(timeout)?;
            if f.msg.id() != Some(id) {
                continue;
            }
            if let Msg::Error { msg, .. } = &f.msg {
                bail!("{msg}");
            }
            return Ok(f);
        }
    }

    /// Агент на связи (`None` — нет) и пользователь сеанса.
    pub fn info(&mut self) -> Result<Option<Info>> {
        let id = self.id();
        self.send(Frame::new(Msg::Status { id }))?;
        let f = self.recv_for(id, Some(Duration::from_secs(5)))?;
        let Msg::State { hello, user, ips, .. } = f.msg else { bail!("status") };
        Ok(hello.and_then(|h| match *h {
            Msg::Hello { agent, os, host, instance, .. } => Some(Info { instance, agent, os, host, user: user.clone(), ips: ips.clone() }),
            _ => None,
        }))
    }

    /// Ждать агента (`user` — и вошедшего пользователя).
    pub fn wait_ready(&mut self, user: bool, timeout: Duration) -> Result<Info> {
        let t0 = Instant::now();
        loop {
            if let Some(i) = self.info()? {
                if !user || i.user.is_some() {
                    return Ok(i);
                }
            }
            if t0.elapsed() > timeout {
                bail!("{}", if user { t!("агент гостя на связи, но пользователь не вошёл") } else { t!("агент гостя не вышел на связь") });
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Команда с потоковым выводом: `out(fd, данные)`. Тайм-аут — команда завершается, код 124.
    #[allow(clippy::too_many_arguments)]
    pub fn exec(&mut self, argv: &[String], user: bool, cwd: Option<String>, env: &[String], input: Option<&[u8]>, timeout: Duration, out: &mut dyn FnMut(u8, &[u8])) -> Result<ExecOut> {
        let id = self.id();
        self.send(Frame::new(Msg::Exec { id, argv: argv.to_vec(), user, cwd, env: env.to_vec(), stdin: input.is_some() }))?;
        if let Some(inp) = input {
            for c in inp.chunks(CHUNK) {
                self.send(Frame::with_data(Msg::Stdin { id }, c.to_vec()))?;
            }
            self.send(Frame::new(Msg::Stdin { id }))?;
        }
        let deadline = Instant::now() + timeout;
        let mut killed = false;
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
            let f = match self.recv_for(id, Some(if killed { Duration::from_secs(10) } else { left })) {
                Ok(f) => f,
                Err(e) if e.to_string() == "timeout" => {
                    if killed {
                        return Ok(ExecOut { code: 124, timed_out: true });
                    }
                    self.send(Frame::new(Msg::Kill { id }))?;
                    killed = true;
                    continue;
                }
                Err(e) => return Err(e),
            };
            match f.msg {
                Msg::Out { fd, .. } => out(fd, &f.data),
                Msg::Exit { code, .. } => return Ok(ExecOut { code: if killed { 124 } else { code }, timed_out: killed }),
                _ => {}
            }
        }
    }

    /// Файл на гость: размер и SHA-256 сверяются с гостем.
    pub fn put_file(&mut self, local: &Path, guest: &str) -> Result<u64> {
        let f = std::fs::File::open(local).with_context(|| local.display().to_string())?;
        let size = f.metadata()?.len();
        self.put_reader(f, size, guest)
    }

    /// Данные из памяти — в файл гостя.
    pub fn put_bytes(&mut self, data: &[u8], guest: &str) -> Result<u64> {
        self.put_reader(data, data.len() as u64, guest)
    }

    fn put_reader(&mut self, mut f: impl Read, size: u64, guest: &str) -> Result<u64> {
        let id = self.id();
        self.send(Frame::new(Msg::Put { id, path: guest.to_string(), size }))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            self.send(Frame::with_data(Msg::Data { id }, buf[..n].to_vec()))?;
        }
        self.send(Frame::new(Msg::PutEnd { id }))?;
        let mine = hex(&h.finalize());
        match self.recv_for(id, Some(Duration::from_secs(600)))?.msg {
            Msg::Done { size: s, sha256, .. } if s == size && sha256 == mine => Ok(size),
            Msg::Done { size: s, sha256, .. } => bail!("{}", t!("{p}: в госте {s} байт, SHA-256 {g} — ожидалось {n} байт, {m}", p = guest, s = s, g = sha256, n = size, m = mine)),
            m => bail!("put: {m:?}"),
        }
    }

    pub fn mkdir(&mut self, guest: &str) -> Result<()> {
        let id = self.id();
        self.send(Frame::new(Msg::Mkdir { id, path: guest.to_string() }))?;
        self.recv_for(id, Some(Duration::from_secs(60)))?;
        Ok(())
    }

    /// Файл или каталог на гость. Итог — байт.
    pub fn push(&mut self, local: &Path, guest: &str) -> Result<u64> {
        if !local.is_dir() {
            return self.put_file(local, guest);
        }
        let mut total = 0;
        self.mkdir(guest)?;
        let mut stack = vec![local.to_path_buf()];
        while let Some(d) = stack.pop() {
            let mut items: Vec<_> = std::fs::read_dir(&d)?.flatten().collect();
            items.sort_by_key(|e| e.file_name());
            for e in items {
                let p = e.path();
                let rel = p.strip_prefix(local).unwrap_or(&p).to_string_lossy().into_owned();
                let g = format!("{}/{}", guest.trim_end_matches(['/', '\\']), rel);
                if p.is_dir() {
                    self.mkdir(&g)?;
                    stack.push(p);
                } else {
                    total += self.put_file(&p, &g)?;
                }
            }
        }
        Ok(total)
    }

    pub fn list(&mut self, guest: &str) -> Result<Vec<Entry>> {
        let id = self.id();
        self.send(Frame::new(Msg::List { id, path: guest.to_string() }))?;
        match self.recv_for(id, Some(Duration::from_secs(120)))?.msg {
            Msg::Entries { entries, .. } => Ok(entries),
            m => bail!("list: {m:?}"),
        }
    }

    /// Небольшой файл гостя — в память (не больше `max` байт).
    pub fn read(&mut self, guest: &str, max: usize) -> Result<Vec<u8>> {
        let id = self.id();
        self.send(Frame::new(Msg::Get { id, path: guest.to_string() }))?;
        let mut out = Vec::new();
        loop {
            let fr = self.recv_for(id, Some(Duration::from_secs(120)))?;
            match fr.msg {
                Msg::Data { .. } if out.len() < max => out.extend_from_slice(&fr.data[..fr.data.len().min(max - out.len())]),
                Msg::End { .. } => return Ok(out),
                _ => {}
            }
        }
    }

    /// Текст в буфер обмена гостя.
    pub fn send_clip(&mut self, text: &str) -> Result<()> {
        self.send_clip_data("text/plain", text.as_bytes().to_vec())
    }

    /// Содержимое буфера гостю: `text/plain` или `image/png`.
    pub fn send_clip_data(&mut self, mime: &str, data: Vec<u8>) -> Result<()> {
        self.send(Frame::with_data(Msg::Clip { mime: mime.into() }, data))
    }

    /// Разрешение экрана гостя (без своей видеокарты: ChangeDisplaySettings в его сеансе).
    pub fn send_mode(&mut self, width: u32, height: u32) -> Result<()> {
        self.send(Frame::new(Msg::Mode { width, height }))
    }

    /// Подписаться на буфер обмена гостя: `on_clip(mime, данные)` на каждое изменение, до разрыва.
    pub fn follow_clip(&mut self, on_clip: &mut dyn FnMut(&str, Vec<u8>)) -> Result<()> {
        self.send(Frame::new(Msg::Clip { mime: "x-subscribe".into() }))?;
        loop {
            let f = self.recv(None)?;
            if let Msg::Clip { mime } = f.msg {
                on_clip(&mime, f.data);
            }
        }
    }

    /// Часы гостя — по хосту.
    pub fn set_time(&mut self) -> Result<()> {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        self.send(Frame::new(Msg::SetTime { unix_ms: ms }))
    }

    fn get_file(&mut self, guest: &str, local: &Path) -> Result<u64> {
        let id = self.id();
        self.send(Frame::new(Msg::Get { id, path: guest.to_string() }))?;
        if let Some(d) = local.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        let tmp = local.with_extension("syn-vm-part");
        let mut f = std::fs::File::create(&tmp).with_context(|| tmp.display().to_string())?;
        let mut h = Sha256::new();
        let (mut want, mut got) = (0u64, 0u64);
        loop {
            let fr = self.recv_for(id, Some(Duration::from_secs(120)))?;
            match fr.msg {
                Msg::Stat { size, .. } => want = size,
                Msg::Data { .. } => {
                    h.update(&fr.data);
                    got += fr.data.len() as u64;
                    f.write_all(&fr.data)?;
                }
                Msg::End { sha256, .. } => {
                    f.sync_all()?;
                    drop(f);
                    let mine = hex(&h.finalize());
                    if sha256 != mine || got != want {
                        let _ = std::fs::remove_file(&tmp);
                        bail!("{}", t!("{p}: получено {got} байт из {want}, SHA-256 не совпал", p = guest, got = got, want = want));
                    }
                    std::fs::rename(&tmp, local)?;
                    return Ok(got);
                }
                _ => {}
            }
        }
    }

    /// Файл или каталог с гостя. Итог — байт.
    pub fn pull(&mut self, guest: &str, local: &Path) -> Result<u64> {
        let entries = self.list(guest)?;
        if entries.len() == 1 && entries[0].path.is_empty() {
            let dest = if local.is_dir() {
                let name = guest.rsplit(['/', '\\']).next().unwrap_or("file");
                local.join(name)
            } else {
                local.to_path_buf()
            };
            return self.get_file(guest, &dest);
        }
        std::fs::create_dir_all(local)?;
        let mut total = 0;
        let sep = if guest.contains('\\') { '\\' } else { '/' };
        for e in entries {
            let dest = local.join(&e.path);
            if e.dir {
                std::fs::create_dir_all(&dest)?;
            } else {
                let g = format!("{}{sep}{}", guest.trim_end_matches(['/', '\\']), if sep == '\\' { e.path.replace('/', "\\") } else { e.path.clone() });
                total += self.get_file(&g, &dest)?;
            }
        }
        Ok(total)
    }

    /// Обновить агента в госте: новый exe — рядом, затем Update (агент перезапустится).
    pub fn update(&mut self, exe: &Path, guest_exe_dir: &str) -> Result<()> {
        let tmp = format!("{}{}syn-vm-agent.new", guest_exe_dir.trim_end_matches(['/', '\\']), if guest_exe_dir.contains('\\') { "\\" } else { "/" });
        let old = self.info()?.map(|i| i.instance);
        self.put_file(exe, &tmp)?;
        let id = self.id();
        self.send(Frame::new(Msg::Update { id, path: tmp }))?;
        self.recv_for(id, Some(Duration::from_secs(30)))?;
        // Ждать новый экземпляр (служба перезапускает агент), а не прежнее приветствие.
        let t0 = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(300));
            if let Some(i) = self.info()? {
                if Some(i.instance) != old {
                    return Ok(());
                }
            }
            if t0.elapsed() > Duration::from_secs(60) {
                bail!("{}", t!("агент гостя не перезапустился после обновления"));
            }
        }
    }

    /// Подписаться на журналы: строки (`src`, текст) до разрыва.
    pub fn follow_logs(&mut self, on_line: &mut dyn FnMut(&str, &str)) -> Result<()> {
        self.send(Frame::new(Msg::Logs { on: true }))?;
        loop {
            let f = self.recv(None)?;
            if let Msg::Log { src } = &f.msg {
                on_line(src, &String::from_utf8_lossy(&f.data));
            }
        }
    }
}

/// Агент на связи (посредник запускается при необходимости).
pub fn online(vm: &Vm) -> bool {
    Client::connect(vm).and_then(|mut c| c.info()).map(|i| i.is_some()).unwrap_or(false)
}

/// Образ агента для гостя: Windows — syn-vm-agent.exe, Linux — syn-vm-agent.
pub fn agent_binary(windows: bool) -> Option<PathBuf> {
    let name = if windows { "syn-vm-agent.exe" } else { "syn-vm-agent" };
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os(if windows { "SYN_VM_AGENT_EXE" } else { "SYN_VM_AGENT" }) {
        c.push(PathBuf::from(p));
    }
    // Пакет кладёт агентов в /usr/lib (исполняемые файлы; /usr/share — у прежних пакетов).
    c.push(PathBuf::from("/usr/lib/syn-vm/agent").join(name));
    c.push(PathBuf::from("/usr/share/syn-vm/agent").join(name));
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            c.push(dir.join("agent").join(name));
            // Сборка из исходников: target/<профиль>/syn-vm → target/<цель>/<профиль>/…
            if let (Some(profile), Some(target)) = (dir.file_name(), dir.parent()) {
                let t = if windows { "x86_64-pc-windows-gnu" } else { "x86_64-unknown-linux-musl" };
                c.push(target.join(t).join(profile).join(name));
                if !windows {
                    c.push(dir.join(name));
                }
            }
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Ждать загрузки гостя: свой агент на связи (`user` — и пользователь вошёл), если он стоит в
/// госте; иначе — ответа qemu-ga.
pub fn wait_boot(vm: &Vm, user: bool, timeout: Duration) -> Result<()> {
    if !user && !installed(vm) {
        return crate::qga::wait(vm, timeout);
    }
    let t0 = Instant::now();
    let mut c = loop {
        match Client::connect(vm) {
            Ok(c) => break c,
            Err(e) if t0.elapsed() > timeout => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    };
    c.wait_ready(user, timeout.saturating_sub(t0.elapsed()))?;
    Ok(())
}

/// Не идти через свой агент (`--qga`).
pub static FORCE_QGA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Свой агент стоит в госте и на связи — команды qemu-ga (exec, файлы, сведения, выключение)
/// идут через него. Ответ «на связи» помнится 3 с.
pub fn preferred(vm: &Vm) -> bool {
    use std::sync::atomic::Ordering;
    if FORCE_QGA.load(Ordering::Relaxed) || !vm.guest_agent || !installed(vm) || crate::qemu::pid(vm).is_none() {
        return false;
    }
    // Свои инструменты: qemu-ga в госте нет — только агент (посредник поднимется сам).
    if vm.syn_tools() {
        return true;
    }
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = std::sync::OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    if let Some(&(t, v)) = c.lock().unwrap().get(&vm.id) {
        if t.elapsed() < Duration::from_secs(3) {
            return v;
        }
    }
    let v = online(vm);
    c.lock().unwrap().insert(vm.id.clone(), (Instant::now(), v));
    v
}

/// Сведения о госте из приветствия агента.
pub fn guest_info(vm: &Vm) -> Result<crate::qga::GuestInfo> {
    let i = Client::connect(vm)?.info()?.context("агент не на связи")?;
    Ok(crate::qga::GuestInfo { os: i.os, hostname: i.host, ips: i.ips, agent_version: format!("syn-vm-agent {}", i.agent) })
}
