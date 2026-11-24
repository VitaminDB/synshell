//! Песочница: вложенный synwm с оболочкой — для проверок агентом, не трогая
//! живой сеанс. Свой D-Bus сеанса (имена трея и уведомлений не
//! пересекаются с живой оболочкой), свои настройки (копия) и журналы.
//! Устройство synlink `sandbox` (или `sandbox:ИМЯ`): снимки, ввод, окна, exec
//! (команды запускаются внутри: WAYLAND_DISPLAY, DISPLAY, D-Bus песочницы).
//!
//! Описание запущенной — `$XDG_RUNTIME_DIR/synlink-sandbox/ИМЯ.json`.

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use synshell_common::action::Action;
use synshell_common::ipc::{Client, Request};
use synshell_tr::t;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sandbox {
    pub name: String,
    /// Сокет IPC вложенного synwm.
    pub socket: PathBuf,
    /// `wayland-N` внутри.
    pub wayland: String,
    /// `:N` Xwayland внутри (пусто — X11 нет).
    pub display: String,
    pub dbus: String,
    pub dir: PathBuf,
    pub form_factor: String,
    pub size: String,
    pub synwm_pid: u32,
    pub dbus_pid: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Opts {
    pub name: String,
    /// `1600x900`.
    pub size: String,
    /// desktop, phone, tablet, game.
    pub form_factor: String,
    /// Своя оболочка вместо [general] shell (путь к бинарнику для проверки сборки).
    pub shell: Option<String>,
    /// Свой synwm (по умолчанию — из PATH).
    pub synwm: Option<String>,
}

fn base() -> PathBuf {
    synshell_common::paths::runtime_dir().join("synlink-sandbox")
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Устройство → имя песочницы: `sandbox` → default, `sandbox:x` → x.
pub fn device_name(device: &str) -> Option<String> {
    if device == "sandbox" {
        return Some("default".into());
    }
    device.strip_prefix("sandbox:").filter(|n| valid_name(n)).map(String::from)
}

fn alive(pid: u32) -> bool {
    pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

/// Запущенная песочница (описание есть, synwm жив, сокет на месте).
pub fn load(name: &str) -> Option<Sandbox> {
    let s: Sandbox = serde_json::from_slice(&std::fs::read(base().join(format!("{name}.json"))).ok()?).ok()?;
    (alive(s.synwm_pid) && s.socket.exists()).then_some(s)
}

pub fn list() -> Vec<Sandbox> {
    let Ok(rd) = std::fs::read_dir(base()) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(String::from))
        .filter_map(|n| load(&n))
        .collect()
}

impl Sandbox {
    /// Переменные окружения программ внутри песочницы.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut v = vec![
            ("WAYLAND_DISPLAY".to_string(), self.wayland.clone()),
            ("DBUS_SESSION_BUS_ADDRESS".to_string(), self.dbus.clone()),
            ("SYNSHELL_SOCKET".to_string(), self.socket.to_string_lossy().into_owned()),
            ("SYNSHELL_CONFIG_DIR".to_string(), self.dir.join("config").to_string_lossy().into_owned()),
            ("XDG_STATE_HOME".to_string(), self.dir.join("state").to_string_lossy().into_owned()),
            ("SYNSHELL_FORM_FACTOR".to_string(), self.form_factor.clone()),
            ("SYNKEYBOARD_SOCKET".to_string(), kb_socket(&self.name)),
        ];
        if !self.display.is_empty() {
            v.push(("DISPLAY".to_string(), self.display.clone()));
        }
        v
    }

    /// argv для запуска внутри: `env K=V … argv`.
    pub fn wrap(&self, argv: Vec<String>) -> Vec<String> {
        let mut out = vec!["env".to_string()];
        out.extend(self.env().into_iter().map(|(k, v)| format!("{k}={v}")));
        out.extend(argv);
        out
    }

    pub fn touch(&self) -> bool {
        matches!(self.form_factor.as_str(), "phone" | "tablet")
    }
}

/// Сокет экранной клавиатуры — короткий путь (длинный не влезает в sun_path).
fn kb_socket(name: &str) -> String {
    synshell_common::paths::runtime_dir().join(format!("synkb-sb-{name}.sock")).to_string_lossy().into_owned()
}

pub fn start(o: &Opts) -> Result<Sandbox> {
    let name = if o.name.is_empty() { "default".to_string() } else { o.name.clone() };
    if !valid_name(&name) {
        bail!("{}", t!("имя песочницы — латиница, цифры, - и _"));
    }
    if let Some(s) = load(&name) {
        return Ok(s);
    }
    let size = if o.size.is_empty() { "1600x900".to_string() } else { o.size.clone() };
    let ff = if o.form_factor.is_empty() { "desktop".to_string() } else { o.form_factor.clone() };
    let dir = base().join(&name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config"))?;
    std::fs::create_dir_all(dir.join("state"))?;
    // Настройки — копия живых: меняются только в песочнице.
    let cfg = synshell_common::paths::config_dir();
    if let Ok(rd) = std::fs::read_dir(&cfg) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) && e.metadata().map(|m| m.len() < 4 << 20).unwrap_or(false) {
                let _ = std::fs::copy(e.path(), dir.join("config").join(e.file_name()));
            }
        }
    }

    // Свой D-Bus сеанса.
    // Без экрана живого сеанса: службы, которые он поднимает по требованию (порталы выбора
    // файлов и др.), иначе открывают окна на живом столе. Экран песочницы — ниже, когда
    // поднимется synwm (dbus-update-activation-environment).
    let mut dbus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1", "--print-pid=1"])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env_remove("SYNSHELL_SOCKET")
        .env_remove("SYNDESKTOP_SOCKET")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("dbus-daemon")?;
    let mut lines = BufReader::new(dbus.stdout.take().context("dbus stdout")?).lines();
    let addr = lines.next().context(t!("dbus-daemon не ответил"))??;
    let dbus_pid = lines.next().and_then(|l| l.ok()).and_then(|l| l.trim().parse().ok()).unwrap_or(dbus.id());
    std::mem::forget(lines);

    let log = dir.join("synwm.out");
    let out = std::fs::File::create(&log)?;
    let mut cmd = Command::new(o.synwm.as_deref().unwrap_or("synwm"));
    cmd.args(["--nested", "--nested-size", &size, "--no-autostart"])
        .env("DBUS_SESSION_BUS_ADDRESS", &addr)
        .env("SYNSHELL_CONFIG_DIR", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("SYNSHELL_FORM_FACTOR", &ff)
        .env("SYNKEYBOARD_SOCKET", kb_socket(&name))
        // своя шина: оболочка регистрирует бэкенд портала (выбор файлов внутри песочницы)
        .env("SYNSHELL_OWN_BUS", "1")
        .env_remove("SYNSHELL_SOCKET")
        .env_remove("SYNDESKTOP_SOCKET")
        .env_remove("DISPLAY")
        .env_remove("_")
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out)
        .process_group(0);
    if let Some(sh) = &o.shell {
        cmd.env("SYNSHELL_SHELL", sh);
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            unsafe { libc::kill(dbus_pid as i32, libc::SIGTERM) };
            bail!("synwm: {e}");
        }
    };
    let synwm_pid = child.id();
    std::mem::forget(child);

    // Сокет, Wayland и X11 — из журнала вложенного synwm.
    let t0 = Instant::now();
    let (mut socket, mut wayland, mut display) = (None, None, None);
    while t0.elapsed() < Duration::from_secs(30) {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let clean = strip_ansi(&text);
        socket = socket.or_else(|| after(&clean, "IPC path=").map(PathBuf::from));
        wayland = wayland.or_else(|| after(&clean, "WAYLAND_DISPLAY socket=").map(|s| s.trim_matches('"').to_string()));
        display = display.or_else(|| after(&clean, "DISPLAY=").filter(|d| d.starts_with(':')).map(String::from));
        if socket.as_ref().is_some_and(|p: &PathBuf| p.exists()) && wayland.is_some() && (display.is_some() || t0.elapsed() > Duration::from_secs(8)) {
            break;
        }
        if !alive(synwm_pid) {
            unsafe { libc::kill(dbus_pid as i32, libc::SIGTERM) };
            bail!("{}", t!("synwm песочницы завершился: {log}", log = tail(&clean, 15)));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let (Some(socket), Some(wayland)) = (socket, wayland) else {
        unsafe {
            libc::kill(synwm_pid as i32, libc::SIGTERM);
            libc::kill(dbus_pid as i32, libc::SIGTERM);
        }
        bail!("{}", t!("synwm песочницы не поднялся за 30 с"));
    };
    // Окружение песочницы — для служб, которые её D-Bus запускает по требованию.
    let mut vars = vec![format!("WAYLAND_DISPLAY={wayland}"), format!("SYNSHELL_CONFIG_DIR={}", dir.join("config").display()), format!("XDG_STATE_HOME={}", dir.join("state").display())];
    if let Some(d) = &display {
        vars.push(format!("DISPLAY={d}"));
    }
    let upd = Command::new("dbus-update-activation-environment").env("DBUS_SESSION_BUS_ADDRESS", &addr).args(&vars).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if !upd.is_ok_and(|s| s.success()) {
        tracing::warn!("dbus-update-activation-environment не сработал: службы песочницы могут открыть окна на живом столе");
    }
    let s = Sandbox {
        name: name.clone(),
        socket,
        wayland,
        display: display.unwrap_or_default(),
        dbus: addr,
        dir,
        form_factor: ff,
        size,
        synwm_pid,
        dbus_pid,
    };
    std::fs::write(base().join(format!("{name}.json")), serde_json::to_vec_pretty(&s)?)?;
    Ok(s)
}

pub fn stop(name: &str) -> Result<()> {
    let path = base().join(format!("{name}.json"));
    let s: Sandbox = serde_json::from_slice(&std::fs::read(&path).with_context(|| t!("песочницы «{name}» нет", name = name))?)?;
    if s.socket.exists() {
        if let Ok(mut c) = Client::connect_to(&s.socket) {
            let _ = c.request(&Request::Action { action: Action::Quit });
        }
    }
    let t0 = Instant::now();
    while alive(s.synwm_pid) && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe {
        if alive(s.synwm_pid) {
            // Группа процессов песочницы: synwm, оболочка, программы внутри.
            libc::kill(-(s.synwm_pid as i32), libc::SIGTERM);
        }
        libc::kill(-(s.dbus_pid as i32), libc::SIGTERM);
        libc::kill(s.dbus_pid as i32, libc::SIGTERM);
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(kb_socket(name));
    let _ = std::fs::remove_dir_all(&s.dir);
    Ok(())
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            for d in it.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Значение после метки до пробела: `IPC path=/run/…sock` → путь.
fn after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let i = text.find(key)? + key.len();
    let v = text[i..].split(|c: char| c.is_whitespace()).next()?;
    (!v.is_empty()).then_some(v)
}

fn tail(s: &str, n: usize) -> String {
    let v: Vec<&str> = s.lines().collect();
    v[v.len().saturating_sub(n)..].join("\n")
}

pub fn describe(s: &Sandbox) -> String {
    format!(
        "{}: {} {}, сокет {}, {} {}, журналы {}",
        s.name,
        s.form_factor,
        s.size,
        s.socket.display(),
        s.wayland,
        if s.display.is_empty() { "без X11".to_string() } else { format!("DISPLAY={}", s.display) },
        Path::new(&s.dir).join("state/synwm").display()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_log() {
        let log = "\x1b[2m2026\x1b[0m INFO synwm::ipc: IPC \x1b[3mpath\x1b[0m\x1b[2m=\x1b[0m/run/user/1000/synwm.wayland-2.sock\nINFO synwm: WAYLAND_DISPLAY socket=\"wayland-2\"\nDISPLAY=:1\n";
        let c = super::strip_ansi(log);
        assert_eq!(super::after(&c, "IPC path="), Some("/run/user/1000/synwm.wayland-2.sock"));
        assert_eq!(super::after(&c, "WAYLAND_DISPLAY socket=").map(|s| s.trim_matches('"')), Some("wayland-2"));
        assert_eq!(super::after(&c, "DISPLAY="), Some(":1"));
        assert_eq!(super::device_name("sandbox").as_deref(), Some("default"));
        assert_eq!(super::device_name("sandbox:t1").as_deref(), Some("t1"));
        assert_eq!(super::device_name("sandbox:../x"), None);
    }
}

/// Запрос к песочнице — прямо здесь, без демона (он мог остаться старым, а
/// перезапуск оборвал бы связь с телефонами). `None` — запрос не к песочнице.
pub fn handle(req: &synshell_common::link::Request) -> Option<Result<synshell_common::link::Response>> {
    use synshell_common::link::{Request as L, Response as R};
    let device = match req {
        L::Screenshot { device, .. } | L::Input { device, .. } | L::Wm { device, .. } | L::Exec { device, .. } => device,
        _ => return None,
    };
    let name = device_name(device)?;
    Some((|| -> Result<R> {
        let s = load(&name).with_context(|| t!("песочница «{name}» не запущена — synlink sandbox start", name = name))?;
        Ok(match req.clone() {
            L::Screenshot { output, path, max_size, .. } => {
                let (o, mut w, mut h, mut png) = crate::services::screenshot(Some(&s.socket), output)?;
                if let Some(m) = max_size.filter(|m| *m > 0) {
                    (w, h, png) = crate::services::downscale_png(&png, m)?;
                }
                let path = path.map(PathBuf::from).unwrap_or_else(|| {
                    let dir = synshell_common::paths::runtime_dir().join("synlink-shots");
                    let _ = std::fs::create_dir_all(&dir);
                    dir.join(format!("sandbox-{name}-{}.png", crate::identity::now()))
                });
                std::fs::write(&path, &png)?;
                R::Screenshot { path: path.to_string_lossy().into_owned(), width: w, height: h, output: o }
            }
            L::Input { output, events, .. } => {
                crate::services::input(Some(&s.socket), output, events)?;
                R::Ok
            }
            L::Wm { wm, .. } => R::Wm { wm: crate::services::wm(Some(&s.socket), wm)? },
            L::Exec { argv, cwd, timeout_ms, .. } => {
                let (code, stdout, stderr) = run(&s, &argv, cwd.as_deref(), None, Duration::from_millis(timeout_ms.unwrap_or(120_000)))?;
                R::Exec { code, stdout, stderr }
            }
            _ => unreachable!(),
        })
    })())
}

/// Команда внутри песочницы (с данными на stdin), с тайм-аутом.
pub fn run(s: &Sandbox, argv: &[String], cwd: Option<&str>, stdin: Option<Vec<u8>>, timeout: Duration) -> Result<(i32, String, String)> {
    use std::io::{Read, Write};
    let (prog, args) = argv.split_first().context(t!("пустая команда"))?;
    let mut cmd = Command::new(prog);
    cmd.args(args).envs(s.env()).env_remove("_").stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().with_context(|| prog.clone())?;
    if let Some(data) = stdin {
        let mut w = child.stdin.take().context("stdin")?;
        std::thread::spawn(move || {
            let _ = w.write_all(&data);
        });
    }
    let mut out = child.stdout.take().context("stdout")?;
    let mut err = child.stderr.take().context("stderr")?;
    // Вывод — в потоках: запущенная внутри программа (setsid -f) держит трубы
    // открытыми и после выхода команды, ждать её конца нельзя.
    let (tx_o, rx_o) = std::sync::mpsc::channel();
    let (tx_e, rx_e) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let mut b = [0u8; 8192];
        while let Ok(n) = out.read(&mut b) {
            if n == 0 { break; }
            v.extend_from_slice(&b[..n]);
            let _ = tx_o.send(v.clone());
        }
        let _ = tx_o.send(v);
    });
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let mut b = [0u8; 8192];
        while let Ok(n) = err.read(&mut b) {
            if n == 0 { break; }
            v.extend_from_slice(&b[..n]);
            let _ = tx_e.send(v.clone());
        }
        let _ = tx_e.send(v);
    });
    let t0 = Instant::now();
    let code = loop {
        if let Some(st) = child.try_wait()? {
            break st.code().unwrap_or(-1);
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break -1;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Последний снимок вывода: до конца трубы (или 300 мс, если её держит потомок).
    let last = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        let mut v = Vec::new();
        let end = Instant::now() + Duration::from_millis(300);
        while let Ok(x) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            v = x;
        }
        v
    };
    let (o, e) = (last(rx_o), last(rx_e));
    Ok((code, String::from_utf8_lossy(&o).into_owned(), String::from_utf8_lossy(&e).into_owned()))
}
