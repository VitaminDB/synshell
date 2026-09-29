//! Wi-Fi: общий интерфейс [`WifiBackend`] и две реализации — iwd (чтение по
//! D-Bus `net.connman.iwd`, подключение `iwctl --passphrase`, без своего
//! агента) и NetworkManager (`nmcli`). `[wifi] backend = auto` — кто есть
//! на системной шине (iwd, затем NetworkManager).
//!
//! Все вызовы блокирующие — из фонового потока.

use std::collections::HashMap;
use std::process::{Command, Stdio};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    pub ssid: String,
    /// 0..100.
    pub strength: u8,
    /// `open`, `psk`, `8021x`, `wep`.
    pub security: String,
    pub known: bool,
    pub connected: bool,
}

impl Network {
    pub fn secure(&self) -> bool {
        self.security != "open" && !self.security.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WifiState {
    pub backend: String,
    /// Есть беспроводное устройство.
    pub present: bool,
    pub powered: bool,
    pub device: Option<String>,
    pub connected: Option<String>,
    /// `connected`, `connecting`, `disconnected`…
    pub status: String,
    pub ip: Option<String>,
    pub networks: Vec<Network>,
}

pub trait WifiBackend: Send {
    fn name(&self) -> &'static str;
    fn state(&self) -> Result<WifiState, String>;
    fn scan(&self) -> Result<(), String>;
    fn set_powered(&self, on: bool) -> Result<(), String>;
    /// Подключиться; для защищённой новой сети — пароль.
    fn connect(&self, ssid: &str, passphrase: Option<&str>) -> Result<(), String>;
    fn disconnect(&self) -> Result<(), String>;
    fn forget(&self, ssid: &str) -> Result<(), String>;
}

pub(crate) fn has_name(conn: &Connection, name: &str) -> bool {
    let r = conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,));
    r.ok().and_then(|m| m.body().deserialize::<bool>().ok()).unwrap_or(false)
}

/// Бэкенд по настройке `[wifi] backend`: `auto`, `iwd`, `networkmanager`.
pub fn backend(choice: &str) -> Option<Box<dyn WifiBackend>> {
    let conn = Connection::system().ok()?;
    let iwd = has_name(&conn, "net.connman.iwd");
    let nm = has_name(&conn, "org.freedesktop.NetworkManager");
    match choice {
        "iwd" if iwd => Some(Box::new(Iwd { conn })),
        "networkmanager" | "nm" if nm => Some(Box::new(Nm)),
        // NetworkManager поверх iwd управляет сам — через него.
        "auto" | "" if nm => Some(Box::new(Nm)),
        "auto" | "" if iwd => Some(Box::new(Iwd { conn })),
        _ => None,
    }
}

// ─── Службы ─────────────────────────────────────────────────────────────────

/// Служба Wi-Fi (systemd-юнит) и её состояние.
#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    pub unit: &'static str,
    pub name: &'static str,
    /// Юнит есть в системе (пакет установлен).
    pub installed: bool,
    pub active: bool,
    pub enabled: bool,
}

const SERVICES: [(&str, &str); 2] = [("iwd.service", "iwd"), ("NetworkManager.service", "NetworkManager")];

/// Состояние служб iwd и NetworkManager по `systemctl show`.
pub fn services() -> Vec<Service> {
    SERVICES
        .iter()
        .map(|(unit, name)| {
            let out = Command::new("systemctl")
                .args(["show", unit, "-p", "LoadState", "-p", "ActiveState", "-p", "UnitFileState"])
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let get = |k: &str| out.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).unwrap_or("").to_string();
            Service {
                unit,
                name,
                installed: get("LoadState") == "loaded",
                active: get("ActiveState") == "active",
                enabled: matches!(get("UnitFileState").as_str(), "enabled" | "static" | "alias"),
            }
        })
        .collect()
}

fn is_root() -> bool {
    // SAFETY: geteuid без побочных эффектов.
    unsafe { libc::geteuid() == 0 }
}

/// Запустить службу (`systemctl enable --now`) или остановить (`stop`):
/// от root напрямую, иначе через pkexec.
pub fn service_control(unit: &str, start: bool) -> Result<(), String> {
    let args: Vec<&str> = if start { vec!["enable", "--now", unit] } else { vec!["stop", unit] };
    let mut c = if is_root() {
        Command::new("systemctl")
    } else if crate::util::which("pkexec") {
        let mut c = Command::new("pkexec");
        c.arg("systemctl");
        c
    } else {
        return Err("нужны права root: pkexec не найден".into());
    };
    let out = c.args(&args).env("LC_ALL", "C").stdin(Stdio::null()).output().map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if e.is_empty() { format!("systemctl: код {}", out.status.code().unwrap_or(-1)) } else { e })
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).env("LC_ALL", "C").stdin(Stdio::null()).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if e.is_empty() { format!("{cmd}: код {}", out.status.code().unwrap_or(-1)) } else { e })
    }
}

// ─── iwd ────────────────────────────────────────────────────────────────────

pub struct Iwd {
    conn: Connection,
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn prop_str(p: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    p.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

fn prop_bool(p: &HashMap<String, OwnedValue>, k: &str) -> Option<bool> {
    p.get(k).and_then(|v| bool::try_from(v).ok())
}

fn prop_path(p: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    p.get(k).and_then(|v| OwnedObjectPath::try_from(v.try_clone().ok()?).ok()).map(|o| o.to_string())
}

impl Iwd {
    fn objects(&self) -> Result<Objects, String> {
        let m = self
            .conn
            .call_method(Some("net.connman.iwd"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
            .map_err(|e| e.to_string())?;
        m.body().deserialize::<Objects>().map_err(|e| e.to_string())
    }

    /// Станция и устройство: (путь, имя интерфейса).
    fn station(&self, objs: &Objects) -> Option<(String, String)> {
        objs.iter().find_map(|(path, ifs)| {
            ifs.get("net.connman.iwd.Station")?;
            let name = ifs.get("net.connman.iwd.Device").and_then(|d| prop_str(d, "Name")).unwrap_or_default();
            Some((path.to_string(), name))
        })
    }

    fn call(&self, path: &str, iface: &str, method: &str) -> Result<(), String> {
        self.conn.call_method(Some("net.connman.iwd"), path, Some(iface), method, &()).map(|_| ()).map_err(|e| e.to_string())
    }
}

impl WifiBackend for Iwd {
    fn name(&self) -> &'static str {
        "iwd"
    }

    fn state(&self) -> Result<WifiState, String> {
        let objs = self.objects()?;
        let mut st = WifiState { backend: "iwd".into(), ..Default::default() };
        let Some((sp, dev)) = self.station(&objs).or_else(|| {
            // Устройство есть, но выключено (станции нет).
            objs.iter().find_map(|(p, ifs)| ifs.get("net.connman.iwd.Device").map(|d| (p.to_string(), prop_str(d, "Name").unwrap_or_default())))
        }) else {
            return Ok(st);
        };
        st.present = true;
        st.device = Some(dev.clone());
        let ifs = &objs.iter().find(|(p, _)| p.as_str() == sp).unwrap().1;
        st.powered = ifs.get("net.connman.iwd.Device").and_then(|d| prop_bool(d, "Powered")).unwrap_or(false);
        let station = ifs.get("net.connman.iwd.Station");
        st.status = station.and_then(|s| prop_str(s, "State")).unwrap_or_else(|| "off".into());
        let connected = station.and_then(|s| prop_path(s, "ConnectedNetwork"));
        // Сила сигнала — из GetOrderedNetworks (dBm × 100).
        let mut strength: HashMap<String, i16> = HashMap::new();
        if station.is_some() {
            if let Ok(m) = self.conn.call_method(Some("net.connman.iwd"), sp.as_str(), Some("net.connman.iwd.Station"), "GetOrderedNetworks", &()) {
                if let Ok(v) = m.body().deserialize::<Vec<(OwnedObjectPath, i16)>>() {
                    strength = v.into_iter().map(|(p, s)| (p.to_string(), s)).collect();
                }
            }
        }
        for (path, ifs) in &objs {
            let Some(n) = ifs.get("net.connman.iwd.Network") else { continue };
            let ssid = prop_str(n, "Name").unwrap_or_default();
            let dbm = strength.get(path.as_str()).copied().unwrap_or(-10000) as f32 / 100.0;
            let is_conn = connected.as_deref() == Some(path.as_str());
            if is_conn {
                st.connected = Some(ssid.clone());
            }
            st.networks.push(Network {
                strength: (((dbm + 100.0) * 2.0).clamp(0.0, 100.0)) as u8,
                security: prop_str(n, "Type").unwrap_or_default(),
                known: prop_path(n, "KnownNetwork").is_some(),
                connected: is_conn,
                ssid,
            });
        }
        st.networks.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.strength.cmp(&a.strength)));
        st.ip = ip_of(&dev);
        Ok(st)
    }

    fn scan(&self) -> Result<(), String> {
        let objs = self.objects()?;
        let (sp, _) = self.station(&objs).ok_or("нет станции Wi-Fi")?;
        // Уже идёт — не ошибка.
        match self.call(&sp, "net.connman.iwd.Station", "Scan") {
            Err(e) if e.contains("InProgress") || e.contains("Busy") => Ok(()),
            r => r,
        }
    }

    fn set_powered(&self, on: bool) -> Result<(), String> {
        let objs = self.objects()?;
        let path = objs.iter().find(|(_, i)| i.contains_key("net.connman.iwd.Device")).map(|(p, _)| p.to_string()).ok_or("нет устройства Wi-Fi")?;
        let v = zbus::zvariant::Value::from(on);
        self.conn
            .call_method(Some("net.connman.iwd"), path.as_str(), Some("org.freedesktop.DBus.Properties"), "Set", &("net.connman.iwd.Device", "Powered", v))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn connect(&self, ssid: &str, passphrase: Option<&str>) -> Result<(), String> {
        let st = self.state()?;
        let dev = st.device.ok_or("нет устройства Wi-Fi")?;
        let mut args: Vec<&str> = Vec::new();
        if let Some(p) = passphrase {
            args.extend(["--passphrase", p]);
        }
        args.extend(["station", &dev, "connect", ssid]);
        run("iwctl", &args).map(|_| ())
    }

    fn disconnect(&self) -> Result<(), String> {
        let objs = self.objects()?;
        let (sp, _) = self.station(&objs).ok_or("нет станции Wi-Fi")?;
        self.call(&sp, "net.connman.iwd.Station", "Disconnect")
    }

    fn forget(&self, ssid: &str) -> Result<(), String> {
        let objs = self.objects()?;
        let path = objs
            .iter()
            .find(|(_, ifs)| ifs.get("net.connman.iwd.KnownNetwork").and_then(|k| prop_str(k, "Name")).as_deref() == Some(ssid))
            .map(|(p, _)| p.to_string())
            .ok_or("сеть не сохранена")?;
        self.call(&path, "net.connman.iwd.KnownNetwork", "Forget")
    }
}

/// IPv4 интерфейса (`ip -4 -o addr`).
fn ip_of(dev: &str) -> Option<String> {
    let out = run("ip", &["-4", "-o", "addr", "show", "dev", dev]).ok()?;
    out.split_whitespace().skip_while(|w| *w != "inet").nth(1).map(|s| s.split('/').next().unwrap_or(s).to_string())
}

// ─── NetworkManager ─────────────────────────────────────────────────────────

pub struct Nm;

/// Разбор `nmcli -t`: поля через `:`, `\:` — двоеточие в значении.
pub fn nm_split(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for c in line.chars() {
        if esc {
            out.last_mut().unwrap().push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out
}

impl WifiBackend for Nm {
    fn name(&self) -> &'static str {
        "NetworkManager"
    }

    fn state(&self) -> Result<WifiState, String> {
        let mut st = WifiState { backend: "NetworkManager".into(), ..Default::default() };
        let radio = run("nmcli", &["-t", "radio", "wifi"])?;
        st.powered = radio.trim() == "enabled";
        for line in run("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])?.lines() {
            let f = nm_split(line);
            if f.get(1).map(String::as_str) == Some("wifi") {
                st.present = true;
                st.device = f.first().cloned();
                st.status = f.get(2).cloned().unwrap_or_default();
                break;
            }
        }
        if !st.present {
            return Ok(st);
        }
        let known: Vec<String> = run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])?
            .lines()
            .map(nm_split)
            .filter(|f| f.get(1).is_some_and(|t| t.contains("wireless")))
            .filter_map(|f| f.first().cloned())
            .collect();
        let mut seen = std::collections::HashSet::new();
        for line in run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"])?.lines() {
            let f = nm_split(line);
            let ssid = f.get(1).cloned().unwrap_or_default();
            if ssid.is_empty() || !seen.insert(ssid.clone()) {
                continue;
            }
            let connected = f.first().is_some_and(|x| x == "*");
            if connected {
                st.connected = Some(ssid.clone());
            }
            let sec = f.get(3).cloned().unwrap_or_default();
            st.networks.push(Network {
                strength: f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                security: if sec.is_empty() || sec == "--" { "open".into() } else if sec.contains("802.1X") { "8021x".into() } else { "psk".into() },
                known: known.contains(&ssid),
                connected,
                ssid,
            });
        }
        st.networks.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.strength.cmp(&a.strength)));
        st.ip = st.device.as_deref().and_then(ip_of);
        Ok(st)
    }

    fn scan(&self) -> Result<(), String> {
        run("nmcli", &["device", "wifi", "rescan"]).map(|_| ()).or_else(|e| if e.contains("not allowed") || e.contains("Scanning") { Ok(()) } else { Err(e) })
    }

    fn set_powered(&self, on: bool) -> Result<(), String> {
        run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]).map(|_| ())
    }

    fn connect(&self, ssid: &str, passphrase: Option<&str>) -> Result<(), String> {
        let known = run("nmcli", &["-t", "-f", "NAME", "connection", "show"]).unwrap_or_default();
        if passphrase.is_none() && known.lines().any(|l| nm_split(l).first().map(String::as_str) == Some(ssid)) {
            return run("nmcli", &["connection", "up", "id", ssid]).map(|_| ());
        }
        let mut args = vec!["device", "wifi", "connect", ssid];
        if let Some(p) = passphrase {
            args.extend(["password", p]);
        }
        run("nmcli", &args).map(|_| ())
    }

    fn disconnect(&self) -> Result<(), String> {
        let st = self.state()?;
        let dev = st.device.ok_or("нет устройства Wi-Fi")?;
        run("nmcli", &["device", "disconnect", &dev]).map(|_| ())
    }

    fn forget(&self, ssid: &str) -> Result<(), String> {
        run("nmcli", &["connection", "delete", "id", ssid]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nmcli_split() {
        assert_eq!(super::nm_split("*:My\\:Net:80:WPA2"), ["*", "My:Net", "80", "WPA2"]);
    }
}
