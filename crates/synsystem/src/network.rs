//! Сеть: состояние подключения и счётчики трафика. Подключение к Wi-Fi —
//! отдельный бэкенд (iwd / NetworkManager, этап Wi-Fi).

use crate::util::{output, which};
use crate::Sys;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    /// `wifi`, `ethernet`, `none`.
    pub kind: String,
    pub connection: String,
    /// Сила сигнала Wi-Fi, 0..100.
    pub signal: Option<u32>,
    pub online: bool,
}

/// Текущее подключение: NetworkManager, иначе состояние интерфейсов.
pub fn read(sys: &Sys) -> Network {
    if which("nmcli") {
        if let Some(s) = output("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) {
            for line in s.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 && parts[1] == "connected" && (parts[0] == "wifi" || parts[0] == "ethernet") {
                    let signal = if parts[0] == "wifi" {
                        output("nmcli", &["-t", "-f", "ACTIVE,SIGNAL", "device", "wifi"])
                            .and_then(|w| w.lines().find(|l| l.starts_with("yes:")).and_then(|l| l[4..].parse().ok()))
                    } else {
                        None
                    };
                    return Network { kind: parts[0].into(), connection: parts[2..].join(":"), signal, online: true };
                }
            }
            // У NetworkManager нет подключений — но интерфейс может вести
            // systemd-networkd (usb0 телефона как unmanaged): смотрим sysfs.
        }
    }
    for name in sys.list("/sys/class/net") {
        if name == "lo" {
            continue;
        }
        if sys.read(format!("/sys/class/net/{name}/operstate")).as_deref() == Some("up") {
            let wifi = sys.path(format!("/sys/class/net/{name}/wireless")).exists();
            return Network { kind: if wifi { "wifi" } else { "ethernet" }.into(), connection: name, signal: None, online: true };
        }
    }
    Network { kind: "none".into(), ..Default::default() }
}

/// Байты принято/передано по всем интерфейсам, кроме `lo`.
pub fn traffic(sys: &Sys) -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    for name in sys.list("/sys/class/net") {
        if name == "lo" {
            continue;
        }
        rx += sys.read_num::<u64>(format!("/sys/class/net/{name}/statistics/rx_bytes")).unwrap_or(0);
        tx += sys.read_num::<u64>(format!("/sys/class/net/{name}/statistics/tx_bytes")).unwrap_or(0);
    }
    (rx, tx)
}

/// Сетевой интерфейс со счётчиками.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Interface {
    pub name: String,
    /// `wifi`, `ethernet`, `mobile`, `usb`, `vpn`, `bridge`, `virtual`.
    pub kind: String,
    pub up: bool,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub mac: Option<String>,
    /// Скорость линии, Мбит/с (Ethernet).
    pub speed_mbps: Option<u32>,
}

/// Интерфейсы, кроме `lo`.
pub fn interfaces(sys: &Sys) -> Vec<Interface> {
    sys.list("/sys/class/net")
        .into_iter()
        .filter(|n| n != "lo")
        .map(|name| {
            let b = format!("/sys/class/net/{name}");
            let has = |f: &str| sys.path(format!("{b}/{f}")).exists();
            let driver = std::fs::read_link(sys.path(format!("{b}/device/driver"))).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            let kind = if has("wireless") || has("phy80211") {
                "wifi"
            } else if ["rmnet", "wwan", "ccmni", "rev_rmnet"].iter().any(|p| name.starts_with(p)) {
                "mobile"
            } else if ["tun", "wg", "tap", "ppp", "ipsec"].iter().any(|p| name.starts_with(p)) || has("tun_flags") {
                "vpn"
            } else if has("bridge") {
                "bridge"
            } else if name.starts_with("usb") || name.starts_with("rndis") || ["rndis_host", "cdc_ether", "cdc_ncm"].contains(&driver.as_str()) {
                "usb"
            } else if !has("device") {
                "virtual"
            } else {
                "ethernet"
            };
            let oper = sys.read(format!("{b}/operstate")).unwrap_or_default();
            Interface {
                kind: kind.into(),
                up: oper == "up" || (oper == "unknown" && sys.read_num::<u32>(format!("{b}/carrier")) == Some(1)),
                rx_bytes: sys.read_num(format!("{b}/statistics/rx_bytes")).unwrap_or(0),
                tx_bytes: sys.read_num(format!("{b}/statistics/tx_bytes")).unwrap_or(0),
                mac: sys.read(format!("{b}/address")).filter(|m| !m.is_empty() && m != "00:00:00:00:00:00"),
                speed_mbps: sys.read_num::<i64>(format!("{b}/speed")).filter(|s| *s > 0).map(|s| s as u32),
                name,
            }
        })
        .collect()
}

/// Адреса интерфейсов по `ip -j addr`: имя → «адрес/префикс».
pub fn addresses() -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    let Some(json) = output("ip", &["-j", "addr"]) else { return out };
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&json) else { return out };
    for i in list {
        let Some(name) = i.get("ifname").and_then(|v| v.as_str()) else { continue };
        let addrs: Vec<String> = i
            .get("addr_info")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter(|x| x.get("scope").and_then(|s| s.as_str()) != Some("link"))
                    .filter_map(|x| Some(format!("{}/{}", x.get("local")?.as_str()?, x.get("prefixlen")?.as_u64()?)))
                    .collect()
            })
            .unwrap_or_default();
        out.insert(name.to_string(), addrs);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn interface_kinds() {
        let dir = tempfile::tempdir().unwrap();
        for (name, extra) in [("wlan0", Some("wireless")), ("rmnet_data0", None), ("lo", None), ("usb0", None)] {
            let b = dir.path().join(format!("sys/class/net/{name}"));
            fs::create_dir_all(b.join("statistics")).unwrap();
            fs::create_dir_all(b.join("device")).unwrap();
            if let Some(e) = extra {
                fs::create_dir_all(b.join(e)).unwrap();
            }
            fs::write(b.join("operstate"), "up").unwrap();
            fs::write(b.join("statistics/rx_bytes"), "10").unwrap();
        }
        let v = interfaces(&Sys::at(dir.path()));
        let kinds: Vec<(&str, &str)> = v.iter().map(|i| (i.name.as_str(), i.kind.as_str())).collect();
        assert_eq!(kinds, [("rmnet_data0", "mobile"), ("usb0", "usb"), ("wlan0", "wifi")]);
        assert!(v.iter().all(|i| i.up && i.rx_bytes == 10));
    }
}
