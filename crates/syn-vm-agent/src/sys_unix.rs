//! Linux: порт `/dev/virtio-ports/…`, процессы, сеанс пользователя (logind), journald.

use std::io::{self, BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::{ExecReq, Proc};

const BIN: &str = "/usr/local/bin/syn-vm-agent";
const UNIT: &str = "/etc/systemd/system/syn-vm-agent.service";

pub fn agent_log(s: &str) {
    eprintln!("{s}");
}

pub fn os_name() -> String {
    let rel = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    rel.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|s| s.trim_matches('"').to_string()).unwrap_or_else(|| "Linux".into())
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string()
}

pub fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string()
}

pub fn open_port() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let path = format!("/dev/virtio-ports/{}", syn_vm_agent::PORT);
    let f = std::fs::OpenOptions::new().read(true).write(true).open(&path)?;
    let w = f.try_clone()?;
    Ok((Box::new(EofWait(f)), Box::new(w)))
}

/// Пока хост не подключён, порт отдаёт конец потока сразу — не крутить цикл впустую.
struct EofWait(std::fs::File);

impl Read for EofWait {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.0.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}

/// Активный графический (или любой) сеанс: имя и uid.
fn active_session() -> Option<(String, u32)> {
    let out = Command::new("loginctl").args(["list-sessions", "--no-legend"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(String, u32, bool)> = None;
    for l in text.lines() {
        let id = l.split_whitespace().next()?;
        let show = Command::new("loginctl").args(["show-session", id, "-p", "Name", "-p", "User", "-p", "State", "-p", "Type", "-p", "Class"]).output().ok()?;
        let s = String::from_utf8_lossy(&show.stdout);
        let get = |k: &str| s.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
        if get("Class") != "user" || !matches!(get("State").as_str(), "active" | "online") {
            continue;
        }
        let graphic = matches!(get("Type").as_str(), "wayland" | "x11");
        let Ok(uid) = get("User").parse::<u32>() else { continue };
        if best.as_ref().is_none_or(|b| graphic && !b.2) {
            best = Some((get("Name"), uid, graphic));
        }
    }
    best.map(|(n, u, _)| (n, u))
}

pub fn session_user() -> Option<String> {
    active_session().map(|s| s.0)
}

pub fn spawn(req: &ExecReq) -> io::Result<Proc> {
    let mut c = if req.user {
        let (name, uid) = active_session().ok_or_else(|| io::Error::other("нет вошедшего пользователя"))?;
        let run = format!("/run/user/{uid}");
        let mut c = Command::new("runuser");
        c.args(["-u", &name, "--", "env"]);
        c.arg(format!("XDG_RUNTIME_DIR={run}"));
        c.arg(format!("DBUS_SESSION_BUS_ADDRESS=unix:path={run}/bus"));
        if let Some(w) = std::fs::read_dir(&run).ok().and_then(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))) {
            c.arg(format!("WAYLAND_DISPLAY={w}"));
        }
        c.arg("DISPLAY=:0");
        c.args(&req.env);
        c.args(&req.argv);
        c
    } else {
        let mut c = Command::new(&req.argv[0]);
        c.args(&req.argv[1..]);
        for kv in &req.env {
            if let Some((k, v)) = kv.split_once('=') {
                c.env(k, v);
            }
        }
        c
    };
    if let Some(d) = &req.cwd {
        c.current_dir(d);
    }
    c.stdin(if req.stdin { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Своя группа процессов — Kill завершает и потомков.
    c.process_group(0);
    let mut ch = c.spawn()?;
    let pid = ch.id();
    let stdout = Box::new(ch.stdout.take().unwrap());
    let stderr = Box::new(ch.stderr.take().unwrap());
    let stdin = ch.stdin.take().map(|s| Box::new(s) as Box<dyn Write + Send>);
    Ok(Proc {
        pid,
        stdout,
        stderr,
        stdin,
        wait: Box::new(move || match ch.wait() {
            Ok(st) => {
                use std::os::unix::process::ExitStatusExt;
                st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0))
            }
            Err(_) => -1,
        }),
    })
}

pub fn kill(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

/// journald: предупреждения и ошибки (ядро и службы).
pub fn start_logs() {
    std::thread::spawn(|| loop {
        let ch = Command::new("journalctl").args(["-f", "-n", "0", "-p", "warning", "-o", "short-iso"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        if let Ok(mut ch) = ch {
            if let Some(out) = ch.stdout.take() {
                for l in io::BufReader::new(out).lines().map_while(Result::ok) {
                    crate::guest_log("journal", &l);
                }
            }
            let _ = ch.wait();
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
}

/// Копия в сеансе пользователя не нужна (буфер обмена — spice-vdagent, монтирует служба).
pub const USES_HELPER: bool = false;

pub fn helper_main() -> io::Result<()> {
    Err(io::Error::other("копия в сеансе пользователя — только в Windows"))
}

/// Общая папка хоста: `virtiofs` или `9p` с меткой hostshare в `target` (пусто — /mnt/hostshare).
pub fn share_prepare(kind: &str, target: &str, source: &str) -> io::Result<()> {
    let target = if target.is_empty() { "/mnt/hostshare" } else { target };
    let tag = if source.is_empty() { "hostshare" } else { source };
    let mounted = Command::new("mountpoint").args(["-q", target]).status().map(|s| s.success()).unwrap_or(false);
    if kind.is_empty() {
        if mounted {
            let _ = Command::new("umount").arg(target).status();
        }
        return Ok(());
    }
    if mounted {
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    let st = match kind {
        "virtiofs" => Command::new("mount").args(["-t", "virtiofs", tag, target]).status()?,
        _ => Command::new("mount").args(["-t", "9p", "-o", "trans=virtio,version=9p2000.L,msize=1048576,cache=mmap", tag, target]).status()?,
    };
    if !st.success() {
        return Err(io::Error::other(format!("mount {kind} {tag} {target}: {st}")));
    }
    crate::log(&format!("общая папка: {kind} {tag} → {target}"));
    Ok(())
}

pub fn set_time(unix_ms: u64) -> io::Result<()> {
    let ts = libc::timespec { tv_sec: (unix_ms / 1000) as _, tv_nsec: ((unix_ms % 1000) * 1_000_000) as libc::c_long };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn service_main() -> io::Result<()> {
    crate::run();
    Ok(())
}

pub fn update(path: &str) -> io::Result<()> {
    let me = std::env::current_exe()?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    std::fs::rename(path, &me)
}

pub fn install() -> io::Result<()> {
    let me = std::env::current_exe()?;
    if me != std::path::Path::new(BIN) {
        let tmp = format!("{BIN}.new");
        std::fs::copy(&me, &tmp)?;
        std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
        std::fs::rename(&tmp, BIN)?;
    }
    std::fs::write(
        UNIT,
        format!(
            "[Unit]\nDescription=syn-vm guest agent\nAfter=dev-virtio\\x2dports-{port}.device\n\n[Service]\nExecStart={BIN} service\nRestart=always\nRestartSec=1\n\n[Install]\nWantedBy=multi-user.target\n",
            port = syn_vm_agent::PORT.replace('.', "\\x2e")
        ),
    )?;
    let ok = |args: &[&str]| Command::new("systemctl").args(args).status().map(|s| s.success()).unwrap_or(false);
    ok(&["daemon-reload"]);
    if !ok(&["enable", "syn-vm-agent.service"]) || !ok(&["restart", "syn-vm-agent.service"]) {
        return Err(io::Error::other("systemctl enable/restart syn-vm-agent не удался"));
    }
    println!("syn-vm-agent: служба установлена ({BIN})");
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let _ = Command::new("systemctl").args(["disable", "--now", "syn-vm-agent.service"]).status();
    let _ = std::fs::remove_file(UNIT);
    let _ = std::fs::remove_file(BIN);
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    Ok(())
}

