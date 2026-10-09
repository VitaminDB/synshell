//! VPN через NetworkManager (nmcli): соединения WireGuard и VPN-плагинов (OpenVPN и др.) — список,
//! подключение, импорт файла конфигурации, новое WireGuard-соединение, удаление, автоподключение.

use std::process::Command;
use synshell_tr::t;

/// Соединение VPN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vpn {
    pub name: String,
    pub uuid: String,
    /// «wireguard» или «vpn» (плагин, например OpenVPN).
    pub kind: String,
    pub active: bool,
    pub autoconnect: bool,
}

/// Поля нового соединения WireGuard (как в конфиге wg-quick).
#[derive(Debug, Clone, Default)]
pub struct WireGuard {
    pub name: String,
    pub private_key: String,
    /// Адрес интерфейса: «10.0.0.2/32».
    pub address: String,
    pub dns: String,
    pub peer_public_key: String,
    /// «host:port».
    pub endpoint: String,
    /// «0.0.0.0/0, ::/0» — весь трафик через VPN.
    pub allowed_ips: String,
    pub preshared_key: String,
}

fn nmcli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nmcli").args(args).output().map_err(|e| format!("nmcli: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { t!("nmcli {v}: ошибка", v = args.join(" ")) } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Разбить строку `nmcli -t` по «:» с учётом экранирования «\:».
fn fields(line: &str) -> Vec<String> {
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

pub fn list() -> Result<Vec<Vpn>, String> {
    let s = nmcli(&["-t", "-f", "NAME,UUID,TYPE,ACTIVE,AUTOCONNECT", "connection", "show"])?;
    Ok(s.lines()
        .map(fields)
        .filter(|f| f.len() >= 5 && (f[2] == "wireguard" || f[2] == "vpn"))
        .map(|f| Vpn {
            name: f[0].clone(),
            uuid: f[1].clone(),
            kind: f[2].clone(),
            active: f[3] == "yes",
            autoconnect: f[4] == "yes",
        })
        .collect())
}

pub fn up(uuid: &str) -> Result<(), String> {
    nmcli(&["connection", "up", "uuid", uuid]).map(|_| ())
}

pub fn down(uuid: &str) -> Result<(), String> {
    nmcli(&["connection", "down", "uuid", uuid]).map(|_| ())
}

pub fn delete(uuid: &str) -> Result<(), String> {
    nmcli(&["connection", "delete", "uuid", uuid]).map(|_| ())
}

pub fn set_autoconnect(uuid: &str, on: bool) -> Result<(), String> {
    nmcli(&["connection", "modify", "uuid", uuid, "connection.autoconnect", if on { "yes" } else { "no" }]).map(|_| ())
}

/// Импорт файла: `.conf` — WireGuard, `.ovpn` — OpenVPN (нужен networkmanager-openvpn).
pub fn import(path: &str) -> Result<(), String> {
    let p = path.trim();
    if !std::path::Path::new(p).is_file() {
        return Err(t!("нет файла {p}", p = p));
    }
    let kind = if p.ends_with(".ovpn") {
        "openvpn"
    } else if p.ends_with(".conf") {
        "wireguard"
    } else {
        return Err(t!("поддерживаются .conf (WireGuard) и .ovpn (OpenVPN)").into());
    };
    nmcli(&["connection", "import", "type", kind, "file", p]).map(|_| ())
}

/// Новое WireGuard-соединение: конфиг wg-quick во временном файле и импорт (имя файла = имя интерфейса).
pub fn add_wireguard(w: &WireGuard) -> Result<(), String> {
    let name: String = w.name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(15).collect();
    if name.is_empty() {
        return Err(t!("имя — латиницей, до 15 символов").into());
    }
    if w.private_key.trim().is_empty() || w.peer_public_key.trim().is_empty() || w.endpoint.trim().is_empty() {
        return Err(t!("нужны закрытый ключ, открытый ключ сервера и адрес сервера").into());
    }
    let mut conf = format!("[Interface]\nPrivateKey = {}\n", w.private_key.trim());
    if !w.address.trim().is_empty() {
        conf.push_str(&format!("Address = {}\n", w.address.trim()));
    }
    if !w.dns.trim().is_empty() {
        conf.push_str(&format!("DNS = {}\n", w.dns.trim()));
    }
    conf.push_str(&format!("\n[Peer]\nPublicKey = {}\nEndpoint = {}\n", w.peer_public_key.trim(), w.endpoint.trim()));
    let allowed = if w.allowed_ips.trim().is_empty() { "0.0.0.0/0, ::/0" } else { w.allowed_ips.trim() };
    conf.push_str(&format!("AllowedIPs = {allowed}\nPersistentKeepalive = 25\n"));
    if !w.preshared_key.trim().is_empty() {
        conf.push_str(&format!("PresharedKey = {}\n", w.preshared_key.trim()));
    }
    let dir = std::env::temp_dir().join(format!("synvpn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{name}.conf"));
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&file).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut f, conf.as_bytes()).map_err(|e| e.to_string())?;
    }
    let r = nmcli(&["connection", "import", "type", "wireguard", "file", &file.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&dir);
    r.map(|_| ())
}

/// Пара ключей WireGuard (закрытый, открытый) через `wg`.
pub fn wireguard_keys() -> Result<(String, String), String> {
    let private = Command::new("wg").arg("genkey").output().map_err(|e| t!("wg: {e} (нужен wireguard-tools)", e = e))?;
    let private = String::from_utf8_lossy(&private.stdout).trim().to_string();
    let mut child = Command::new("wg")
        .arg("pubkey")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), private.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok((private, String::from_utf8_lossy(&out.stdout).trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_fields_escape() {
        assert_eq!(fields(r"Work\:VPN:uuid-1:vpn:no:yes"), vec!["Work:VPN", "uuid-1", "vpn", "no", "yes"]);
    }
}
