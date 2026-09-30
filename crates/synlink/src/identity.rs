//! Кто мы: ключ и самоподписанный сертификат (отпечаток — личность
//! устройства), имя и вид машины; спаренные устройства.
//!
//! Всё в `~/.local/share/synlink/`: `key.pem`, `cert.der`, `peers.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use synshell_common::link::DeviceKind;

pub fn data_dir() -> PathBuf {
    synshell_common::paths::data_home().join("synlink")
}

pub struct Identity {
    pub cert: Vec<u8>,
    pub key_der: Vec<u8>,
    /// SHA-256 сертификата, hex.
    pub fingerprint: String,
    /// Первые 16 знаков отпечатка.
    pub id: String,
}

pub fn fingerprint(cert_der: &[u8]) -> String {
    hex(&Sha256::digest(cert_der))
}

pub fn id_of(fp: &str) -> String {
    fp[..16].to_string()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir)?;
        set_private(&dir);
        let key_path = dir.join("key.pem");
        let cert_path = dir.join("cert.der");
        let (key, cert) = match (std::fs::read_to_string(&key_path), std::fs::read(&cert_path)) {
            (Ok(pem), Ok(cert)) => (rcgen::KeyPair::from_pem(&pem).context("key.pem")?, cert),
            _ => {
                let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
                let mut params = rcgen::CertificateParams::new(vec!["synlink".to_string()])?;
                params.not_before = rcgen::date_time_ymd(2024, 1, 1);
                params.not_after = rcgen::date_time_ymd(2124, 1, 1);
                params.distinguished_name.push(rcgen::DnType::CommonName, "synlink");
                let cert = params.self_signed(&key)?.der().to_vec();
                write_private(&key_path, key.serialize_pem().as_bytes())?;
                write_private(&cert_path, &cert)?;
                tracing::info!("создан ключ устройства");
                (key, cert)
            }
        };
        let fingerprint = fingerprint(&cert);
        Ok(Self { id: id_of(&fingerprint), fingerprint, key_der: key.serialize_der(), cert })
    }
}

/// Имя машины по умолчанию: `/etc/hostname`, иначе uname.
pub fn hostname() -> String {
    if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } == 0 {
        let c = unsafe { std::ffi::CStr::from_ptr(u.nodename.as_ptr()) };
        return c.to_string_lossy().into_owned();
    }
    "synshell".into()
}

/// Красивое имя: `PRETTY_HOSTNAME` из `/etc/machine-info`, модель из
/// device-tree (телефоны; «Diting based on Qualcomm…» → «Diting»), модель
/// из DMI (у ноутбуков `product_name` бывает кодом — тогда версия), иначе хост.
pub fn default_name() -> String {
    if let Ok(s) = std::fs::read_to_string("/etc/machine-info") {
        for l in s.lines() {
            if let Some(v) = l.strip_prefix("PRETTY_HOSTNAME=") {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .ok()
            .map(|s| s.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(m) = read("/proc/device-tree/model") {
        let m = m.split(" based on ").next().unwrap_or(&m).trim().to_string();
        if !m.is_empty() && !m.starts_with("Qualcomm") {
            return m;
        }
    }
    let generic = |s: &str| {
        s.contains("To be filled") || s.eq_ignore_ascii_case("System Product Name") || s.eq_ignore_ascii_case("Default string")
    };
    let name = read("/sys/devices/virtual/dmi/id/product_name").filter(|s| !generic(s));
    let version = read("/sys/devices/virtual/dmi/id/product_version").filter(|s| !generic(s) && s.contains(' '));
    // «83F5» — код модели, «Legion Pro 7 16IAX10H» — имя.
    match (name, version) {
        (Some(n), Some(v)) if !n.contains(' ') => return v,
        (Some(n), _) => return n,
        (None, Some(v)) => return v,
        _ => {}
    }
    hostname()
}

pub fn device_kind(cfg: &synshell_common::config::Config) -> DeviceKind {
    use synshell_common::config::FormFactor;
    if cfg.process_form_factor() == FormFactor::Phone {
        return DeviceKind::Phone;
    }
    // Есть батарея — ноутбук.
    if let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") {
        for e in rd.flatten() {
            if std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery") {
                return DeviceKind::Laptop;
            }
        }
    }
    DeviceKind::Desktop
}

/// Спаренное устройство.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Trusted {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub fingerprint: String,
    pub user: String,
    #[serde(default)]
    pub uid: u32,
    pub home: String,
    pub ssh_key: Option<String>,
    pub ssh_host_key: Option<String>,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    /// Последние адреса (для соединения без поиска).
    #[serde(default)]
    pub addrs: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Peers {
    pub peers: BTreeMap<String, Trusted>,
}

impl Peers {
    fn path() -> PathBuf {
        data_dir().join("peers.json")
    }
    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        match serde_json::to_vec_pretty(self) {
            Ok(b) => {
                if let Err(e) = write_private(&Self::path(), &b) {
                    tracing::warn!(?e, "peers.json не записан");
                }
            }
            Err(e) => tracing::warn!(?e, "peers.json"),
        }
    }
    /// Устройство по id, имени или началу id.
    pub fn find(&self, key: &str) -> Option<&Trusted> {
        self.peers
            .get(key)
            .or_else(|| self.peers.values().find(|p| p.name.eq_ignore_ascii_case(key) || crate::ssh::slug(&p.name) == key))
            .or_else(|| {
                // phone / desktop / laptop / tablet — если такое одно.
                let kind: DeviceKind = serde_json::from_value(serde_json::Value::String(key.to_string())).ok()?;
                let mut it = self.peers.values().filter(|p| p.kind == kind);
                let first = it.next();
                if it.next().is_some() {
                    None
                } else {
                    first
                }
            })
            .or_else(|| {
                let mut it = self.peers.values().filter(|p| p.id.starts_with(key));
                let first = it.next();
                if it.next().is_some() {
                    None
                } else {
                    first
                }
            })
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&tmp)?;
        f.write_all(data)?;
    }
    std::fs::rename(&tmp, path)
}

fn set_private(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}
