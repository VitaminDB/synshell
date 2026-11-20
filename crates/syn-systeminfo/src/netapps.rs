//! Программы учёта трафика: имя и значок по ключу synnetd (имя исполняемого
//! файла), имена пользователей, время и промежутки дней по местному времени.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use syngui::prelude::*;
use synshell_common::xdg::{self, DesktopEntry};

#[derive(Clone)]
pub struct AppInfo {
    pub name: String,
    pub icon: Option<PathBuf>,
}

/// Имя процесса программы по её команде: `/usr/bin/qbittorrent %U` → `qbittorrent`.
fn exe_of(e: &DesktopEntry) -> String {
    let w = e.exec.split_whitespace().find(|w| !w.contains('=') && *w != "env").unwrap_or("");
    w.rsplit('/').next().unwrap_or(w).to_string()
}

static CACHE: Mutex<Option<HashMap<String, AppInfo>>> = Mutex::new(None);

/// Ярлык программы по ключу synnetd: имя исполняемого файла, id ярлыка или WM_CLASS.
pub fn info(key: &str) -> AppInfo {
    if key.is_empty() {
        return AppInfo { name: t!("Система"), icon: None };
    }
    let mut c = CACHE.lock().unwrap();
    let map = c.get_or_insert_with(HashMap::new);
    if let Some(i) = map.get(key) {
        return i.clone();
    }
    let k = key.to_lowercase();
    let apps = xdg::apps();
    let e = apps
        .iter()
        .filter(|e| e.android.is_none() && !e.exec.contains("://"))
        .find(|e| exe_of(e) == key)
        .or_else(|| apps.iter().find(|e| e.android.is_none() && (e.id.to_lowercase() == k || e.id.to_lowercase().ends_with(&format!(".{k}")) || e.wm_class.to_lowercase() == k)));
    let i = AppInfo { name: e.map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| key.to_string()), icon: e.and_then(|e| xdg::lookup_icon(&e.icon)) };
    map.insert(key.to_string(), i.clone());
    i
}

/// Имя пользователя по uid.
pub fn user(uid: u32) -> String {
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return uid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().into_owned()
}

pub fn my_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn tm(ms: u64) -> libc::tm {
    let t = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}

/// Время записи: сегодня — `14:03:22`, раньше — `19.11 14:03`.
pub fn when(ms: u64) -> String {
    let a = tm(ms);
    let b = tm(now_ms());
    if a.tm_yday == b.tm_yday && a.tm_year == b.tm_year {
        format!("{:02}:{:02}:{:02}", a.tm_hour, a.tm_min, a.tm_sec)
    } else {
        format!("{:02}.{:02} {:02}:{:02}", a.tm_mday, a.tm_mon + 1, a.tm_hour, a.tm_min)
    }
}

/// Текущий час по местному времени.
pub fn hour_now() -> usize {
    tm(now_ms()).tm_hour.clamp(0, 23) as usize
}

/// Полная дата и время.
pub fn when_full(ms: u64) -> String {
    let a = tm(ms);
    format!("{:02}.{:02}.{} {:02}:{:02}:{:02}", a.tm_mday, a.tm_mon + 1, a.tm_year + 1900, a.tm_hour, a.tm_min, a.tm_sec)
}

/// Начало и конец дня `ГГГГ-ММ-ДД` по местному времени, мс.
pub fn day_range(day: &str) -> Option<(u64, u64)> {
    let mut it = day.split('-').map(|x| x.parse::<i32>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let mut t: libc::tm = unsafe { std::mem::zeroed() };
    t.tm_year = y - 1900;
    t.tm_mon = m - 1;
    t.tm_mday = d;
    t.tm_isdst = -1;
    let start = unsafe { libc::mktime(&mut t) };
    t = unsafe { std::mem::zeroed() };
    t.tm_year = y - 1900;
    t.tm_mon = m - 1;
    t.tm_mday = d + 1;
    t.tm_isdst = -1;
    let end = unsafe { libc::mktime(&mut t) };
    (start > 0 && end > start).then(|| (start as u64 * 1000, end as u64 * 1000 - 1))
}

/// Начало сегодняшнего дня, мс.
pub fn today_start() -> u64 {
    day_range(&crate::sampler::day_str(now_ms() / 1000)).map(|r| r.0).unwrap_or(0)
}

/// Короткая подпись дня: `20.11`, день недели.
pub fn day_label(day: &str) -> (String, String) {
    let Some((start, _)) = day_range(day) else { return (day.to_string(), String::new()) };
    let a = tm(start);
    let wd = [n_!("вс"), n_!("пн"), n_!("вт"), n_!("ср"), n_!("чт"), n_!("пт"), n_!("сб")][a.tm_wday.clamp(0, 6) as usize];
    (format!("{:02}.{:02}", a.tm_mday, a.tm_mon + 1), syngui::i18n::t(wd))
}

/// Название канала по имени интерфейса и виду.
pub fn iface_label(iface: &str, uplinks: &[syn_netd::Uplink]) -> String {
    let kind = uplinks.iter().find(|u| u.iface == iface).map(|u| u.kind.as_str()).unwrap_or_else(|| {
        if iface.starts_with("wl") {
            "wifi"
        } else if iface.starts_with("en") || iface.starts_with("eth") {
            if iface.contains('u') { "usb" } else { "ethernet" }
        } else if iface.starts_with("wg") || iface.starts_with("tun") {
            "vpn"
        } else {
            ""
        }
    });
    match kind {
        "wifi" => t!("Wi-Fi"),
        "usb" => t!("USB-модем"),
        "ethernet" => t!("Кабель"),
        "mobile" => t!("Мобильная сеть"),
        "vpn" => t!("VPN"),
        _ => iface.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        let (a, b) = day_range("2026-11-20").unwrap();
        assert!(b > a && b - a < 26 * 3600 * 1000 && b - a > 22 * 3600 * 1000);
        assert_eq!(crate::sampler::day_str(a / 1000), "2026-11-20");
        assert_eq!(crate::sampler::day_str(b / 1000), "2026-11-20");
        assert!(day_range("x").is_none());
    }
}
