use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::daemon::Daemon;
use crate::netif::Iface;

const QUIET: Duration = Duration::from_secs(20);
const RECHECK: Duration = Duration::from_secs(300);
const MAX_HOSTS: u32 = 1023;

pub async fn run(d: Arc<Daemon>) {
    let mut key: Option<(String, Ipv4Addr)> = None;
    let mut quiet_since = Instant::now();
    let mut probed: Option<Instant> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let iface = crate::netif::list().into_iter().find(|i| !i.usb && is_wireless(&i.name));
        let k = iface.as_ref().map(|i| (i.name.clone(), i.addr));
        if k != key {
            key = k;
            quiet_since = Instant::now();
            probed = None;
            d.set_wifi_isolated(false);
        }
        let Some(i) = iface else { continue };
        if d.sleeping.load(Ordering::Relaxed) || d.wifi_alive() {
            quiet_since = Instant::now();
            probed = None;
            d.set_wifi_isolated(false);
            continue;
        }
        if quiet_since.elapsed() < QUIET || probed.is_some_and(|t| t.elapsed() < RECHECK) {
            continue;
        }
        probed = Some(Instant::now());
        if let Some(alone) = probe(&i).await {
            d.set_wifi_isolated(alone && !d.wifi_alive());
        }
    }
}

fn is_wireless(name: &str) -> bool {
    let p = std::path::Path::new("/sys/class/net").join(name);
    p.join("wireless").exists() || p.join("phy80211").exists()
}

async fn probe(i: &Iface) -> Option<bool> {
    let mask = u32::from(i.mask);
    let hosts = !mask;
    if !(2..=MAX_HOSTS).contains(&hosts) {
        return None;
    }
    let net = u32::from(i.addr) & mask;
    let gw = gateway(&i.name);
    let sock = tokio::net::UdpSocket::bind(SocketAddr::from((i.addr, 0))).await.ok()?;
    for h in 1..hosts {
        let ip = Ipv4Addr::from(net | h);
        if ip != i.addr && Some(ip) != gw {
            let _ = sock.send_to(&[0], SocketAddr::from((ip, 9))).await;
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let arp = neighbours(&i.name);
    let gw_mac = gw.and_then(|g| arp.iter().find(|(ip, _)| *ip == g).map(|(_, m)| m.clone()));
    let others = arp.iter().filter(|(ip, mac)| Some(*ip) != gw && *ip != i.addr && Some(mac) != gw_mac.as_ref()).count();
    tracing::debug!(iface = %i.name, others, "проверка изоляции Wi-Fi");
    Some(others == 0)
}

fn gateway(iface: &str) -> Option<Ipv4Addr> {
    let s = std::fs::read_to_string("/proc/net/route").ok()?;
    s.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || f[0] != iface || f[1] != "00000000" {
            return None;
        }
        let v = u32::from_str_radix(f[2], 16).ok()?;
        Some(Ipv4Addr::from(v.to_ne_bytes())).filter(|g| !g.is_unspecified())
    })
}

fn neighbours(iface: &str) -> Vec<(Ipv4Addr, String)> {
    let Ok(s) = std::fs::read_to_string("/proc/net/arp") else { return Vec::new() };
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 6 || f[5] != iface {
                return None;
            }
            let flags = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).ok()?;
            if flags & 2 == 0 || f[3] == "00:00:00:00:00:00" {
                return None;
            }
            Some((f[0].parse().ok()?, f[3].to_lowercase()))
        })
        .collect()
}
