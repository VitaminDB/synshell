//! USB-модемы (телефон в режиме USB-модема), которые никто не настраивает:
//! dhcpcd с `allowinterfaces`, нет NetworkManager и systemd-networkd на этом
//! интерфейсе. Служба поднимает интерфейс и получает адрес сама — свой
//! DHCPv4-клиент на пакетном сокете (порт 68 занят dhcpcd, а ответ сервера
//! может прийти на ещё не назначенный адрес). IPv6 ядро настраивает само по
//! объявлениям маршрутизатора, как только интерфейс поднят.
//!
//! Маршрут по умолчанию — с метрикой [`crate::route::USB_METRIC`], DNS —
//! systemd-resolved (`resolvctl dns`), если он запущен.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::route::{ip, USB_METRIC};

/// Драйверы USB-модемов (сеть телефона по USB).
pub const USB_MODEM_DRIVERS: &[&str] = &["rndis_host", "cdc_ether", "cdc_ncm", "cdc_eem", "ipheth"];
/// Сколько ждать, не настроит ли интерфейс кто-то другой.
const GRACE: Duration = Duration::from_secs(5);

/// USB-модемы: интерфейсы с драйвером из [`USB_MODEM_DRIVERS`], кроме
/// функции гаджета (это мы — телефон, раздающий сеть по USB).
pub fn usb_modems() -> Vec<String> {
    // Проверка: `SYNNETD_TEST_USB=veth1,veth2` — считать интерфейсы модемами.
    if let Ok(t) = std::env::var("SYNNETD_TEST_USB") {
        return t.split(',').filter(|s| std::path::Path::new(&format!("/sys/class/net/{s}")).exists()).map(str::to_string).collect();
    }
    let mut v = Vec::new();
    for e in std::fs::read_dir("/sys/class/net").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        let driver = std::fs::read_link(p.join("device/driver")).ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        if !USB_MODEM_DRIVERS.contains(&driver.as_str()) {
            continue;
        }
        if std::fs::canonicalize(p.join("device")).is_ok_and(|d| d.to_string_lossy().contains("gadget")) {
            continue;
        }
        v.push(name);
    }
    v.sort();
    v
}

/// Интерфейсом управляет кто-то ещё.
fn managed_elsewhere(iface: &str) -> bool {
    // NetworkManager сам поднимает USB-модемы.
    if nm_running() {
        return true;
    }
    // systemd-networkd: состояние ссылки.
    if let Ok(idx) = std::fs::read_to_string(format!("/sys/class/net/{iface}/ifindex")) {
        if let Ok(s) = std::fs::read_to_string(format!("/run/systemd/netif/links/{}", idx.trim())) {
            if s.lines().any(|l| l.starts_with("ADMIN_STATE=") && !l.ends_with("unmanaged")) {
                return true;
            }
        }
    }
    false
}

fn nm_running() -> bool {
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
        e.file_name().to_str().is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "NetworkManager")
    })
}

fn has_ipv4(iface: &str) -> bool {
    ip(&["-4", "-o", "addr", "show", "dev", iface, "scope", "global"]).is_ok_and(|s| !s.trim().is_empty())
}

struct Worker {
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Dhcp {
    seen: HashMap<String, Instant>,
    workers: HashMap<String, Worker>,
    ours: Arc<Mutex<Vec<String>>>,
}

impl Dhcp {
    /// Адрес интерфейса получен службой.
    pub fn ours(&self) -> Arc<Mutex<Vec<String>>> {
        self.ours.clone()
    }

    /// Шаг: новые модемы поднять, через [`GRACE`] без адреса — свой DHCP;
    /// ушедшие — забыть.
    pub fn tick(&mut self, enabled: bool) -> bool {
        let modems = usb_modems();
        let mut changed = false;
        self.seen.retain(|k, _| modems.contains(k));
        let gone: Vec<String> = self.workers.keys().filter(|k| !enabled || !modems.contains(k)).cloned().collect();
        for k in gone {
            if let Some(w) = self.workers.remove(&k) {
                w.stop.store(true, Ordering::SeqCst);
                changed = true;
            }
        }
        if !enabled {
            return changed;
        }
        for m in modems {
            if self.workers.contains_key(&m) {
                continue;
            }
            let first = *self.seen.entry(m.clone()).or_insert_with(|| {
                if !managed_elsewhere(&m) {
                    // Интерфейс модема никто не поднимает.
                    let _ = ip(&["link", "set", "dev", &m, "up"]);
                }
                Instant::now()
            });
            if first.elapsed() < GRACE || has_ipv4(&m) || managed_elsewhere(&m) {
                continue;
            }
            tracing::info!("{m}: USB-модем без адреса — свой DHCP");
            let stop = Arc::new(AtomicBool::new(false));
            let (st, ours, name) = (stop.clone(), self.ours.clone(), m.clone());
            let _ = std::thread::Builder::new().name(format!("dhcp-{m}")).spawn(move || {
                client(&name, &st, &ours);
                ours.lock().unwrap().retain(|x| *x != name);
            });
            self.workers.insert(m, Worker { stop });
            changed = true;
        }
        changed
    }
}

// ─── Клиент ──────────────────────────────────────────────────────────────────

const DISCOVER: u8 = 1;
const OFFER: u8 = 2;
const REQUEST: u8 = 3;
const ACK: u8 = 5;
const NAK: u8 = 6;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lease {
    pub addr: [u8; 4],
    pub prefix: u8,
    pub router: Option<[u8; 4]>,
    pub dns: Vec<[u8; 4]>,
    pub server: [u8; 4],
    pub lease: u32,
    pub t1: u32,
    pub t2: u32,
    pub mtu: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub kind: u8,
    pub xid: u32,
    pub yiaddr: [u8; 4],
    pub lease: Lease,
}

fn ipstr(a: [u8; 4]) -> String {
    std::net::Ipv4Addr::from(a).to_string()
}

/// Сообщение клиента.
pub fn build(kind: u8, xid: u32, mac: [u8; 6], ciaddr: [u8; 4], requested: Option<[u8; 4]>, server: Option<[u8; 4]>, host: &str) -> Vec<u8> {
    let mut m = vec![0u8; 240];
    m[0] = 1; // BOOTREQUEST
    m[1] = 1; // Ethernet
    m[2] = 6;
    m[4..8].copy_from_slice(&xid.to_be_bytes());
    m[10..12].copy_from_slice(&0x8000u16.to_be_bytes()); // ответ широковещательно
    m[12..16].copy_from_slice(&ciaddr);
    m[28..34].copy_from_slice(&mac);
    m[236..240].copy_from_slice(&[99, 130, 83, 99]);
    m.extend_from_slice(&[53, 1, kind]);
    m.extend_from_slice(&[61, 7, 1]);
    m.extend_from_slice(&mac);
    if let Some(r) = requested {
        m.extend_from_slice(&[50, 4]);
        m.extend_from_slice(&r);
    }
    if let Some(s) = server {
        m.extend_from_slice(&[54, 4]);
        m.extend_from_slice(&s);
    }
    if !host.is_empty() && host.len() < 64 {
        m.extend_from_slice(&[12, host.len() as u8]);
        m.extend_from_slice(host.as_bytes());
    }
    m.extend_from_slice(&[55, 8, 1, 3, 6, 15, 26, 51, 58, 59]);
    m.push(255);
    // Минимальный размер BOOTP.
    if m.len() < 300 {
        m.resize(300, 0);
    }
    m
}

/// Разобрать ответ сервера (полезная нагрузка UDP).
pub fn parse(b: &[u8], mac: [u8; 6]) -> Option<Reply> {
    if b.len() < 240 || b[0] != 2 || b[28..34] != mac || b[236..240] != [99, 130, 83, 99] {
        return None;
    }
    let xid = u32::from_be_bytes(b[4..8].try_into().ok()?);
    let yiaddr: [u8; 4] = b[16..20].try_into().ok()?;
    let mut kind = 0;
    let mut l = Lease { addr: yiaddr, prefix: 24, ..Default::default() };
    let mut i = 240;
    while i < b.len() {
        let code = b[i];
        if code == 0 {
            i += 1;
            continue;
        }
        if code == 255 || i + 1 >= b.len() {
            break;
        }
        let len = b[i + 1] as usize;
        let Some(v) = b.get(i + 2..i + 2 + len) else { break };
        let ip4 = |v: &[u8]| -> Option<[u8; 4]> { v.get(..4).and_then(|x| x.try_into().ok()) };
        let u32v = |v: &[u8]| v.get(..4).map(|x| u32::from_be_bytes(x.try_into().unwrap()));
        match code {
            53 if len >= 1 => kind = v[0],
            1 => {
                if let Some(m) = ip4(v) {
                    l.prefix = u32::from_be_bytes(m).count_ones() as u8;
                }
            }
            3 => l.router = ip4(v),
            6 => l.dns = v.chunks_exact(4).map(|c| c.try_into().unwrap()).collect(),
            26 if len >= 2 => l.mtu = Some(u16::from_be_bytes([v[0], v[1]])),
            51 => l.lease = u32v(v).unwrap_or(0),
            54 => l.server = ip4(v).unwrap_or_default(),
            58 => l.t1 = u32v(v).unwrap_or(0),
            59 => l.t2 = u32v(v).unwrap_or(0),
            _ => {}
        }
        i += 2 + len;
    }
    if l.lease == 0 {
        l.lease = 3600;
    }
    if l.t1 == 0 || l.t1 >= l.lease {
        l.t1 = l.lease / 2;
    }
    if l.t2 == 0 || l.t2 >= l.lease || l.t2 <= l.t1 {
        l.t2 = l.lease / 8 * 7;
    }
    Some(Reply { kind, xid, yiaddr, lease: l })
}

fn csum(b: &[u8]) -> u16 {
    let mut s: u32 = 0;
    for c in b.chunks(2) {
        s += u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32;
    }
    while s > 0xffff {
        s = (s & 0xffff) + (s >> 16);
    }
    !(s as u16)
}

/// IPv4 + UDP 68→67 вокруг сообщения DHCP.
pub fn wrap(src: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let total = 20 + 8 + payload.len();
    let mut p = vec![0u8; 28];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&src);
    p[16..20].copy_from_slice(&[255, 255, 255, 255]);
    let c = csum(&p[..20]);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p[20..22].copy_from_slice(&68u16.to_be_bytes());
    p[22..24].copy_from_slice(&67u16.to_be_bytes());
    p[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    // Контрольная сумма UDP для IPv4 необязательна (0).
    p.extend_from_slice(payload);
    p
}

/// Полезная нагрузка UDP на порт 68 из IP-пакета.
pub fn unwrap(pkt: &[u8]) -> Option<&[u8]> {
    if pkt.len() < 28 || pkt[0] >> 4 != 4 || pkt[9] != 17 {
        return None;
    }
    let ihl = (pkt[0] & 0xf) as usize * 4;
    let udp = pkt.get(ihl..)?;
    if udp.len() < 8 || u16::from_be_bytes([udp[2], udp[3]]) != 68 {
        return None;
    }
    let len = (u16::from_be_bytes([udp[4], udp[5]]) as usize).min(udp.len());
    udp.get(8..len)
}

struct Sock {
    fd: i32,
    ifindex: i32,
}

impl Drop for Sock {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

impl Sock {
    fn open(iface: &str) -> std::io::Result<Sock> {
        let ifindex: i32 = std::fs::read_to_string(format!("/sys/class/net/{iface}/ifindex"))?.trim().parse().map_err(std::io::Error::other)?;
        let proto = (libc::ETH_P_IP as u16).to_be();
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, proto as i32) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let s = Sock { fd, ifindex };
        let mut sa: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sa.sll_family = libc::AF_PACKET as u16;
        sa.sll_protocol = proto;
        sa.sll_ifindex = ifindex;
        if unsafe { libc::bind(fd, (&sa as *const libc::sockaddr_ll).cast(), std::mem::size_of::<libc::sockaddr_ll>() as u32) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let tv = libc::timeval { tv_sec: 1, tv_usec: 0 };
        unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, (&tv as *const libc::timeval).cast(), std::mem::size_of::<libc::timeval>() as u32) };
        Ok(s)
    }

    fn send(&self, pkt: &[u8]) -> std::io::Result<()> {
        let mut sa: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sa.sll_family = libc::AF_PACKET as u16;
        sa.sll_protocol = (libc::ETH_P_IP as u16).to_be();
        sa.sll_ifindex = self.ifindex;
        sa.sll_halen = 6;
        sa.sll_addr[..6].copy_from_slice(&[0xff; 6]);
        let n = unsafe { libc::sendto(self.fd, pkt.as_ptr().cast(), pkt.len(), 0, (&sa as *const libc::sockaddr_ll).cast(), std::mem::size_of::<libc::sockaddr_ll>() as u32) };
        if n < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Ответ с нужным `xid` за `wait`.
    fn recv(&self, mac: [u8; 6], xid: u32, wait: Duration, stop: &AtomicBool) -> Option<Reply> {
        let t0 = Instant::now();
        let mut buf = [0u8; 1600];
        while t0.elapsed() < wait && !stop.load(Ordering::SeqCst) {
            let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n <= 0 {
                continue;
            }
            if let Some(r) = unwrap(&buf[..n as usize]).and_then(|p| parse(p, mac)) {
                if r.xid == xid {
                    return Some(r);
                }
            }
        }
        None
    }
}

fn mac_of(iface: &str) -> Option<[u8; 6]> {
    let s = std::fs::read_to_string(format!("/sys/class/net/{iface}/address")).ok()?;
    let v: Vec<u8> = s.trim().split(':').filter_map(|h| u8::from_str_radix(h, 16).ok()).collect();
    v.try_into().ok()
}

fn xid() -> u32 {
    let mut b = [0u8; 4];
    unsafe { libc::getrandom(b.as_mut_ptr().cast(), 4, 0) };
    u32::from_ne_bytes(b)
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Спать до `secs`, просыпаясь проверить остановку и наличие интерфейса.
fn sleep_until(deadline: Instant, iface: &str, stop: &AtomicBool) -> bool {
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) || !std::path::Path::new(&format!("/sys/class/net/{iface}")).exists() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    true
}

fn resolved_running() -> bool {
    std::path::Path::new("/run/systemd/resolve/io.systemd.Resolve").exists()
}

fn configure(iface: &str, l: &Lease) {
    let addr = format!("{}/{}", ipstr(l.addr), l.prefix);
    let life = l.lease.to_string();
    if let Err(e) = ip(&["-4", "addr", "replace", &addr, "dev", iface, "valid_lft", &life, "preferred_lft", &life]) {
        tracing::warn!("{e}");
    }
    if let Some(m) = l.mtu.filter(|m| *m >= 576) {
        let _ = ip(&["link", "set", "dev", iface, "mtu", &m.to_string()]);
    }
    if let Some(r) = l.router {
        if let Err(e) = ip(&["-4", "route", "replace", "default", "via", &ipstr(r), "dev", iface, "metric", &USB_METRIC.to_string(), "proto", "dhcp"]) {
            tracing::warn!("{e}");
        }
    }
    if !l.dns.is_empty() && resolved_running() {
        let mut a = vec!["dns".to_string(), iface.to_string()];
        a.extend(l.dns.iter().map(|d| ipstr(*d)));
        let _ = std::process::Command::new("resolvectl").args(&a).status();
    }
}

fn deconfigure(iface: &str, l: &Lease) {
    if !std::path::Path::new(&format!("/sys/class/net/{iface}")).exists() {
        return;
    }
    let _ = ip(&["-4", "route", "del", "default", "dev", iface, "metric", &USB_METRIC.to_string()]);
    let _ = ip(&["-4", "addr", "del", &format!("{}/{}", ipstr(l.addr), l.prefix), "dev", iface]);
    if resolved_running() {
        let _ = std::process::Command::new("resolvectl").args(["revert", iface]).status();
    }
}

/// Клиент одного интерфейса: до остановки или исчезновения интерфейса.
fn client(iface: &str, stop: &AtomicBool, ours: &Mutex<Vec<String>>) {
    let Some(mac) = mac_of(iface) else { return };
    let host = hostname();
    let mut backoff = 4u64;
    let mut bound: Option<(Lease, Instant)> = None;
    loop {
        if stop.load(Ordering::SeqCst) || !std::path::Path::new(&format!("/sys/class/net/{iface}")).exists() {
            break;
        }
        let sock = match Sock::open(iface) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("{iface}: {e}");
                if !sleep_until(Instant::now() + Duration::from_secs(5), iface, stop) {
                    break;
                }
                continue;
            }
        };
        match bound.take() {
            None => {
                // DISCOVER → OFFER → REQUEST → ACK.
                let x = xid();
                let _ = sock.send(&wrap([0; 4], &build(DISCOVER, x, mac, [0; 4], None, None, &host)));
                let Some(offer) = sock.recv(mac, x, Duration::from_secs(backoff), stop).filter(|r| r.kind == OFFER) else {
                    backoff = (backoff * 2).min(32);
                    continue;
                };
                let _ = sock.send(&wrap([0; 4], &build(REQUEST, x, mac, [0; 4], Some(offer.yiaddr), Some(offer.lease.server), &host)));
                match sock.recv(mac, x, Duration::from_secs(4), stop) {
                    Some(r) if r.kind == ACK => {
                        backoff = 4;
                        tracing::info!("{iface}: адрес {}/{} шлюз {:?} на {} с", ipstr(r.lease.addr), r.lease.prefix, r.lease.router.map(ipstr), r.lease.lease);
                        configure(iface, &r.lease);
                        ours.lock().unwrap().push(iface.to_string());
                        bound = Some((r.lease, Instant::now()));
                    }
                    _ => backoff = (backoff * 2).min(32),
                }
            }
            Some((l, got)) => {
                // Продление с T1, повтор раз в 30 с до конца аренды.
                let t1 = got + Duration::from_secs(l.t1 as u64);
                if !sleep_until(t1, iface, stop) {
                    deconfigure(iface, &l);
                    break;
                }
                let end = got + Duration::from_secs(l.lease as u64);
                let mut renewed = None;
                let mut nak = false;
                while Instant::now() < end && !stop.load(Ordering::SeqCst) {
                    let x = xid();
                    let _ = sock.send(&wrap(l.addr, &build(REQUEST, x, mac, l.addr, None, None, &host)));
                    match sock.recv(mac, x, Duration::from_secs(4), stop) {
                        Some(r) if r.kind == ACK => {
                            renewed = Some(r.lease);
                            break;
                        }
                        Some(r) if r.kind == NAK => {
                            nak = true;
                            break;
                        }
                        _ => {}
                    }
                    if !sleep_until(Instant::now() + Duration::from_secs(30).min(end.saturating_duration_since(Instant::now())), iface, stop) {
                        break;
                    }
                }
                match renewed {
                    Some(n) => {
                        if n.addr != l.addr {
                            deconfigure(iface, &l);
                        }
                        configure(iface, &n);
                        bound = Some((n, Instant::now()));
                    }
                    None => {
                        tracing::info!("{iface}: аренда {} кончилась{}", ipstr(l.addr), if nak { " (NAK)" } else { "" });
                        deconfigure(iface, &l);
                        ours.lock().unwrap().retain(|x| x != iface);
                    }
                }
            }
        }
    }
    if let Some((l, _)) = bound {
        deconfigure(iface, &l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x3a, 0xf3, 0xdb, 0x35, 0x9b, 0x89];

    fn offer(kind: u8, xid: u32) -> Vec<u8> {
        let mut m = vec![0u8; 240];
        m[0] = 2;
        m[4..8].copy_from_slice(&xid.to_be_bytes());
        m[16..20].copy_from_slice(&[192, 168, 42, 77]);
        m[28..34].copy_from_slice(&MAC);
        m[236..240].copy_from_slice(&[99, 130, 83, 99]);
        m.extend_from_slice(&[53, 1, kind, 1, 4, 255, 255, 255, 0, 3, 4, 192, 168, 42, 129, 6, 8, 8, 8, 8, 8, 1, 1, 1, 1, 51, 4, 0, 0, 0x0e, 0x10, 54, 4, 192, 168, 42, 129, 255]);
        m
    }

    #[test]
    fn parse_offer() {
        let r = parse(&offer(OFFER, 77), MAC).unwrap();
        assert_eq!(r.kind, OFFER);
        assert_eq!(r.xid, 77);
        assert_eq!(r.lease.addr, [192, 168, 42, 77]);
        assert_eq!(r.lease.prefix, 24);
        assert_eq!(r.lease.router, Some([192, 168, 42, 129]));
        assert_eq!(r.lease.dns, vec![[8, 8, 8, 8], [1, 1, 1, 1]]);
        assert_eq!(r.lease.lease, 3600);
        assert_eq!((r.lease.t1, r.lease.t2), (1800, 3150));
        assert_eq!(r.lease.server, [192, 168, 42, 129]);
        // Чужой MAC — не нам.
        assert!(parse(&offer(OFFER, 77), [0; 6]).is_none());
    }

    #[test]
    fn wrap_unwrap() {
        let msg = build(DISCOVER, 5, MAC, [0; 4], None, None, "host");
        assert_eq!(msg.len(), 300);
        assert_eq!(&msg[240..243], &[53, 1, DISCOVER]);
        let pkt = wrap([0; 4], &msg);
        // Контрольная сумма заголовка IPv4 сходится.
        assert_eq!(csum(&pkt[..20]), 0);
        // Ответ сервера (порт назначения 68) разворачивается.
        let mut reply = pkt.clone();
        reply[20..22].copy_from_slice(&67u16.to_be_bytes());
        reply[22..24].copy_from_slice(&68u16.to_be_bytes());
        assert_eq!(unwrap(&reply).unwrap(), &msg[..]);
        assert!(unwrap(&pkt).is_none());
    }
}
