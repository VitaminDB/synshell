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
