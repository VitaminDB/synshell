//! Сетевые интерфейсы: адреса IPv4 для анонсов, какой интерфейс — USB,
//! состояние кабеля.

use std::net::Ipv4Addr;

use synshell_common::link::{Transport, UsbInfo};

#[derive(Debug, Clone)]
pub struct Iface {
    pub name: String,
    pub addr: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub usb: bool,
}

impl Iface {
    pub fn broadcast(&self) -> Ipv4Addr {
        let a = u32::from(self.addr);
        let m = u32::from(self.mask);
        Ipv4Addr::from(a | !m)
    }
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let m = u32::from(self.mask);
        u32::from(self.addr) & m == u32::from(ip) & m
    }
}

/// Поднятые интерфейсы с IPv4 (без loopback).
pub fn list() -> Vec<Iface> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return out;
    }
    let mut p = ifap;
    while !p.is_null() {
        let ifa = unsafe { &*p };
        p = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() {
            continue;
        }
        if unsafe { (*ifa.ifa_addr).sa_family } as i32 != libc::AF_INET {
            continue;
        }
        let flags = ifa.ifa_flags as i32;
        if flags & libc::IFF_UP == 0 || flags & libc::IFF_LOOPBACK != 0 {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
        let msk = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in) };
        let addr = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
        let mask = Ipv4Addr::from(u32::from_be(msk.sin_addr.s_addr));
        let usb = is_usb(&name);
        out.push(Iface { name, addr, mask, usb });
    }
    unsafe { libc::freeifaddrs(ifap) };
    out
}

/// Интерфейс USB-сети: у хоста — устройство на шине USB (cdc_ncm,
/// rndis_host), у телефона — функция гаджета (`usb0` → `gadget.0`).
pub fn is_usb(name: &str) -> bool {
    let dev = std::path::Path::new("/sys/class/net").join(name).join("device");
    match std::fs::canonicalize(&dev) {
        Ok(p) => {
            let s = p.to_string_lossy();
            s.contains("/usb") || s.contains("gadget")
        }
        // Гаджет без ссылки device (старые ядра) — по имени.
        Err(_) => name.starts_with("usb") || name.starts_with("rndis"),
    }
}

/// Чем устройство с адресом `ip` достижимо: USB, если адрес в подсети
/// USB-интерфейса.
pub fn transport_for(ip: std::net::IpAddr, ifaces: &[Iface]) -> Transport {
    let std::net::IpAddr::V4(v4) = ip else { return Transport::Wifi };
    if ifaces.iter().any(|i| i.usb && i.contains(v4)) {
        Transport::Usb
    } else {
        Transport::Wifi
    }
}

/// Мы — гаджет USB (телефон): есть UDC.
pub fn is_gadget() -> bool {
    std::fs::read_dir("/sys/class/udc").is_ok_and(|mut d| d.next().is_some())
}

/// Состояние кабеля: у гаджета — UDC `configured` (хост опознал), у
/// хоста — поднят USB-интерфейс с адресом.
pub fn usb_info(ifaces: &[Iface]) -> UsbInfo {
    let iface = ifaces.iter().find(|i| i.usb);
    let cable = if is_gadget() {
        std::fs::read_dir("/sys/class/udc")
            .map(|d| {
                d.flatten().any(|e| std::fs::read_to_string(e.path().join("state")).is_ok_and(|s| s.trim() == "configured"))
            })
            .unwrap_or(false)
    } else {
        iface.is_some_and(|i| carrier(&i.name))
    };
    UsbInfo { cable, interface: iface.map(|i| i.name.clone()), address: iface.map(|i| i.addr.to_string()) }
}

fn carrier(name: &str) -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{name}/carrier")).is_ok_and(|s| s.trim() == "1")
}

/// Батарея: (процент, заряжается).
pub fn battery() -> Option<(u8, bool)> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery") {
            let cap: u8 = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()?;
            let st = std::fs::read_to_string(p.join("status")).unwrap_or_default();
            let charging = matches!(st.trim(), "Charging" | "Full");
            return Some((cap.min(100), charging));
        }
    }
    None
}
