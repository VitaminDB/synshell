//! ssh между спаренными устройствами без знания адресов: `ssh phone`
//! идёт через `ProxyCommand synlink proxy <id>` — туннель внутри
//! соединения synlink (по USB или Wi-Fi, какое есть) до sshd устройства.
//!
//! При спаривании ключ ssh той стороны попадает в `~/.ssh/authorized_keys`
//! (с пометкой `synlink:<id>`, забывание устройства его убирает), ключ хоста
//! — в свой known_hosts. Если системный sshd не запущен (десктоп без
//! sudo), демон поднимает sshd от пользователя на 127.0.0.1:22022 — только
//! для туннеля, снаружи он не виден.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::identity::{data_dir, Trusted};

const USER_SSHD_PORT: u16 = 22022;

fn ssh_dir() -> PathBuf {
    data_dir().join("ssh")
}

pub fn whoami() -> (String, u32, String) {
    let uid = unsafe { libc::getuid() };
    let pw = unsafe { libc::getpwuid(uid) };
    let (name, home) = if pw.is_null() {
        (std::env::var("USER").unwrap_or_default(), synshell_common::paths::home().to_string_lossy().into_owned())
    } else {
        unsafe {
            (
                std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned(),
                std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned(),
            )
        }
    };
    (name, uid, home)
}

fn keygen(path: &std::path::Path, comment: &str) -> Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let st = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .status()
        .context("ssh-keygen")?;
    if !st.success() {
        bail!("ssh-keygen: {st}");
    }
    Ok(())
}

/// Ключ ssh этой машины для входа на устройства (открытая часть).
pub fn ensure_key() -> Result<String> {
    let key = ssh_dir().join("id_ed25519");
    if !key.exists() {
        let (user, ..) = whoami();
        keygen(&key, &format!("{user}@{} synlink", crate::identity::hostname()))?;
    }
    Ok(std::fs::read_to_string(key.with_extension("pub"))?.trim().to_string())
}

fn system_sshd() -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], 22).into(), Duration::from_millis(300)).is_ok()
}

/// Ключ хоста того sshd, до которого пойдёт туннель.
pub fn host_key() -> Option<String> {
    if system_sshd() {
        return std::fs::read_to_string("/etc/ssh/ssh_host_ed25519_key.pub").ok().map(|s| s.trim().to_string());
    }
    let hk = ssh_dir().join("host_ed25519");
    if !hk.exists() && keygen(&hk, "synlink host").is_err() {
        return None;
    }
    std::fs::read_to_string(hk.with_extension("pub")).ok().map(|s| s.trim().to_string())
}

static USER_SSHD: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// Порт sshd для туннеля: системный (22) или свой, запущенный по требованию.
pub async fn sshd_port() -> Result<u16> {
    if tokio::task::spawn_blocking(system_sshd).await? {
        return Ok(22);
    }
    {
        let mut g = USER_SSHD.lock().unwrap();
        let alive = g.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_none());
        if !alive {
            *g = Some(start_user_sshd()?);
        }
    }
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", USER_SSHD_PORT)).await.is_ok() {
            return Ok(USER_SSHD_PORT);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("sshd пользователя не запустился")
}

fn start_user_sshd() -> Result<std::process::Child> {
    let dir = ssh_dir();
    let hk = dir.join("host_ed25519");
    if !hk.exists() {
        keygen(&hk, "synlink host")?;
    }
    let (_, _, home) = whoami();
    let cfg = dir.join("sshd_config");
    let sftp = ["/usr/lib/ssh/sftp-server", "/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .unwrap_or("internal-sftp");
    std::fs::write(
        &cfg,
        format!(
            "# sshd пользователя для туннелей synlink (только 127.0.0.1)\n\
             Port {USER_SSHD_PORT}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile none\nUsePAM no\n\
             StrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n\
             PubkeyAuthentication yes\nAuthorizedKeysFile {home}/.ssh/authorized_keys\nPrintMotd no\n\
             Subsystem sftp {sftp}\n",
            hk.display()
        ),
    )?;
    let sshd = ["/usr/bin/sshd", "/usr/sbin/sshd"].into_iter().find(|p| std::path::Path::new(p).exists()).context("нет sshd")?;
    tracing::info!("запускаю sshd пользователя на 127.0.0.1:{USER_SSHD_PORT}");
    Ok(std::process::Command::new(sshd)
        .arg("-D")
        .arg("-e")
        .arg("-f")
        .arg(&cfg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("запуск sshd")?)
}

pub fn stop_user_sshd() {
    if let Some(mut c) = USER_SSHD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn authorized_keys() -> PathBuf {
    synshell_common::paths::home().join(".ssh/authorized_keys")
}

fn ensure_ssh_home() -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let d = synshell_common::paths::home().join(".ssh");
    std::fs::create_dir_all(&d)?;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    Ok(d)
}

/// Разрешить вход по ключу устройства.
pub fn authorize(id: &str, name: &str, key: &str) -> Result<()> {
    ensure_ssh_home()?;
    let path = authorized_keys();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let tag = format!("synlink:{id}");
    let mut lines: Vec<String> = old.lines().filter(|l| !l.trim_end().ends_with(&tag)).map(String::from).collect();
    // Комментарий ключа заменяем своей пометкой (по ней и удаляем).
    let mut parts = key.split_whitespace();
    let (Some(t), Some(k)) = (parts.next(), parts.next()) else { bail!("странный ключ ssh") };
    lines.push(format!("{t} {k} {}-{tag}", slug(name)));
    std::fs::write(&path, lines.join("\n") + "\n")?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn unauthorize(id: &str) {
    let path = authorized_keys();
    let Ok(old) = std::fs::read_to_string(&path) else { return };
    let tag = format!("synlink:{id}");
    let new: Vec<&str> = old.lines().filter(|l| !l.trim_end().ends_with(&tag)).collect();
    if new.len() != old.lines().count() {
        let _ = std::fs::write(&path, new.join("\n") + "\n");
    }
}

/// Имя для ssh и путей: латиница, цифры и дефисы.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "device".into()
    } else {
        s
    }
}

pub fn host_alias(t: &Trusted) -> String {
    slug(&t.name)
}

fn kind_alias(k: synshell_common::link::DeviceKind) -> &'static str {
    use synshell_common::link::DeviceKind::*;
    match k {
        Phone => "phone",
        Tablet => "tablet",
        Laptop => "laptop",
        Desktop => "desktop",
    }
}

/// Переписать `~/.local/share/synlink/ssh/config` и подключить его к
/// `~/.ssh/config` (строка `Include` в начале, один раз).
pub fn write_config(peers: &[Trusted]) {
    if let Err(e) = write_config_inner(peers) {
        tracing::warn!("ssh config: {e:#}");
    }
}

fn write_config_inner(peers: &[Trusted]) -> Result<()> {
    let dir = ssh_dir();
    std::fs::create_dir_all(&dir)?;
    let exe = crate::self_exe();
    let key = dir.join("id_ed25519");
    let known = dir.join("known_hosts");
    let mut cfg = String::from("# Спаренные устройства synlink — файл пишет демон, правки затрутся.\n");
    let mut kh = String::new();
    for t in peers {
        let mut names = vec![host_alias(t), format!("{}.synlink", t.id)];
        let ka = kind_alias(t.kind);
        if peers.iter().filter(|p| kind_alias(p.kind) == ka).count() == 1 {
            names.push(ka.to_string());
        }
        let alias = format!("synlink-{}", t.id);
        cfg.push_str(&format!(
            "\nHost {}\n    HostName {}\n    User {}\n    ProxyCommand {} proxy {}\n    IdentityFile {}\n    IdentitiesOnly yes\n    HostKeyAlias {alias}\n    UserKnownHostsFile {}\n    StrictHostKeyChecking {}\n    ServerAliveInterval 10\n",
            names.join(" "),
            host_alias(t),
            t.user,
            exe.display(),
            t.id,
            key.display(),
            known.display(),
            if t.ssh_host_key.is_some() { "yes" } else { "accept-new" },
        ));
        if let Some(hk) = &t.ssh_host_key {
            let mut p = hk.split_whitespace();
            if let (Some(a), Some(b)) = (p.next(), p.next()) {
                kh.push_str(&format!("{alias} {a} {b}\n"));
            }
        }
    }
    std::fs::write(dir.join("config"), cfg)?;
    std::fs::write(&known, kh)?;
    let sshd = ensure_ssh_home()?;
    let main = sshd.join("config");
    let include = format!("Include {}", dir.join("config").display());
    let old = std::fs::read_to_string(&main).unwrap_or_default();
    if !old.lines().any(|l| l.trim() == include) {
        let new = format!("# synlink: спаренные устройства (ssh phone, ssh <имя>)\n{include}\n\n{old}");
        std::fs::write(&main, new)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
