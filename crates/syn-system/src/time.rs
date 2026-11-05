//! Дата, время и часовой пояс: systemd-timedated (`org.freedesktop.timedate1`
//! на системной шине) — пояс, синхронизация по сети (NTP), ручная установка
//! часов; список поясов и их смещения — из `/usr/share/zoneinfo`; пояс по
//! внешнему IP — `curl` к геосервисам (для «Часовой пояс автоматически»).
//!
//! Вызовы блокирующие — из фонового потока. Изменения просят polkit
//! интерактивно (агент оболочки спросит пароль, если правило не разрешает).

use std::process::{Command, Stdio};

use zbus::blocking::Connection;

use crate::Sys;
use synshell_tr::t;

const DEST: &str = "org.freedesktop.timedate1";
const PATH: &str = "/org/freedesktop/timedate1";

/// Состояние часов системы.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeStatus {
    /// `Europe/Moscow`; пусто — не задан (UTC).
    pub timezone: String,
    /// Синхронизация по сети включена.
    pub ntp: bool,
    /// Служба синхронизации (systemd-timesyncd и т. п.) установлена.
    pub can_ntp: bool,
    /// Часы уже сверены с сервером времени.
    pub synced: bool,
    /// timedated доступен — менять пояс и время можно.
    pub service: bool,
}

fn conn() -> Result<Connection, String> {
    Connection::system().map_err(|e| t!("системная шина D-Bus: {e}", e = e))
}

fn err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => match msg {
            Some(m) if !m.is_empty() => m,
            _ => name.to_string(),
        },
        e => e.to_string(),
    }
}

/// Пояс по ссылке `/etc/localtime` (без timedated).
fn localtime_link(sys: &Sys) -> String {
    std::fs::read_link(sys.path("/etc/localtime"))
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().into_owned();
            s.split_once("zoneinfo/").map(|(_, z)| z.to_string())
        })
        .unwrap_or_default()
}

/// Прочитать состояние. Без timedated — пояс по `/etc/localtime`,
/// `service = false`.
pub fn status() -> TimeStatus {
    let fallback = || TimeStatus { timezone: localtime_link(&Sys::host()), ..Default::default() };
    let Ok(c) = conn() else { return fallback() };
    let r = c.call_method(Some(DEST), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(DEST,));
    let Ok(m) = r else { return fallback() };
    let Ok(p) = m.body().deserialize::<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>() else { return fallback() };
    let b = |k: &str| p.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
    let tz = p.get("Timezone").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
    TimeStatus { timezone: tz, ntp: b("NTP"), can_ntp: b("CanNTP"), synced: b("NTPSynchronized"), service: true }
}

/// Сменить часовой пояс системы.
pub fn set_timezone(tz: &str) -> Result<(), String> {
    conn()?.call_method(Some(DEST), PATH, Some(DEST), "SetTimezone", &(tz, true)).map(|_| ()).map_err(err)
}

/// Включить или выключить синхронизацию времени по сети.
pub fn set_ntp(on: bool) -> Result<(), String> {
    conn()?.call_method(Some(DEST), PATH, Some(DEST), "SetNTP", &(on, true)).map(|_| ()).map_err(err)
}

/// Установить часы (секунды Unix, UTC). Только при выключенной синхронизации.
pub fn set_time(unix: i64) -> Result<(), String> {
    let usec = unix.saturating_mul(1_000_000);
    conn()?.call_method(Some(DEST), PATH, Some(DEST), "SetTime", &(usec, false, true)).map(|_| ()).map_err(err)
}

/// Секунды Unix → местные (год, месяц, день, час, минута, смещение от UTC в
/// секундах) в текущем поясе системы (перечитывается).
pub fn local_parts(unix: i64) -> (i32, u32, u32, u32, u32, i32) {
    extern "C" {
        fn tzset();
    }
    // SAFETY: tzset/localtime_r с корректным буфером.
    let tm = unsafe {
        tzset();
        let t = unix as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32, tm.tm_gmtoff as i32)
}

/// Местные дата и время (в текущем поясе системы) → секунды Unix.
pub fn local_to_unix(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<i64> {
    extern "C" {
        fn tzset();
    }
    // SAFETY: tzset/mktime с корректной структурой.
    unsafe {
        tzset();
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month as i32 - 1;
        tm.tm_mday = day as i32;
        tm.tm_hour = hour as i32;
        tm.tm_min = minute as i32;
        tm.tm_isdst = -1;
        let t = libc::mktime(&mut tm);
        (t != -1).then_some(t as i64)
    }
}

// ─── Пояса ──────────────────────────────────────────────────────────────────

/// Канонические пояса (`Z`-строки `tzdata.zi`), по алфавиту; нет файла —
/// `zone1970.tab`.
pub fn zones(sys: &Sys) -> Vec<String> {
    let mut v: Vec<String> = match sys.read("/usr/share/zoneinfo/tzdata.zi") {
        Some(t) => t.lines().filter_map(|l| l.strip_prefix("Z ")).filter_map(|l| l.split_whitespace().next()).map(String::from).collect(),
        None => sys
            .read("/usr/share/zoneinfo/zone1970.tab")
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split('\t').nth(2))
            .map(String::from)
            .collect(),
    };
    // Устаревшие имена вида «EST5EDT», «Etc/…» в выборе не нужны; UTC — да.
    v.retain(|z| z.contains('/') && !z.starts_with("Etc/"));
    v.push("UTC".into());
    v.sort();
    v.dedup();
    v
}

/// Пояс существует (файл TZif в `zoneinfo`). Отсекает мусор от геосервиса.
pub fn zone_exists(sys: &Sys, tz: &str) -> bool {
    let ok = !tz.is_empty()
        && tz.len() < 64
        && !tz.contains("..")
        && !tz.starts_with('/')
        && tz.chars().all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c));
    ok && std::fs::read(sys.path(format!("/usr/share/zoneinfo/{tz}"))).is_ok_and(|b| b.starts_with(b"TZif"))
}

/// Стандартное (зимнее) смещение пояса от UTC, секунды к востоку —
/// из POSIX-строки в хвосте файла TZif.
pub fn std_offset(sys: &Sys, tz: &str) -> Option<i32> {
    let data = std::fs::read(sys.path(format!("/usr/share/zoneinfo/{tz}"))).ok()?;
    let text = data.strip_suffix(b"\n")?;
    let start = text.iter().rposition(|&b| b == b'\n')? + 1;
    parse_posix_offset(std::str::from_utf8(&text[start..]).ok()?)
}

/// Смещение из POSIX TZ (`MSK-3`, `<+05>-5`, `CET-1CEST,…`): секунды к
/// востоку от UTC (в POSIX знак обратный).
pub fn parse_posix_offset(s: &str) -> Option<i32> {
    let rest = if let Some(r) = s.strip_prefix('<') {
        &r[r.find('>')? + 1..]
    } else {
        let n = s.find(|c: char| !c.is_ascii_alphabetic())?;
        if n < 3 {
            return None;
        }
        &s[n..]
    };
    let (sign, rest) = match rest.as_bytes().first()? {
        b'-' => (-1, &rest[1..]),
        b'+' => (1, &rest[1..]),
        _ => (1, rest),
    };
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == ':')).unwrap_or(rest.len());
    let mut parts = rest[..end].split(':').map(|p| p.parse::<i32>());
    let h = parts.next()?.ok()?;
    let m = parts.next().and_then(|r| r.ok()).unwrap_or(0);
    let sec = parts.next().and_then(|r| r.ok()).unwrap_or(0);
    Some(-sign * (h * 3600 + m * 60 + sec))
}

/// `UTC+05:00`, `UTC−03:30`, `UTC`.
pub fn format_offset(secs: i32) -> String {
    if secs == 0 {
        return "UTC".into();
    }
    let sign = if secs < 0 { '−' } else { '+' };
    let a = secs.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", a / 3600, a % 3600 / 60)
}

/// Подпись пояса: город из имени
/// (`America/Argentina/Buenos_Aires` → `Buenos Aires`) и регион.
pub fn zone_label(tz: &str) -> (String, String) {
    match tz.rsplit_once('/') {
        Some((region, city)) => (city.replace('_', " "), region.replace('_', " ")),
        None => (tz.to_string(), String::new()),
    }
}

// ─── Пояс по IP ─────────────────────────────────────────────────────────────

fn curl(url: &str) -> Option<String> {
    let out = Command::new("curl")
        .args(["-sfL", "--max-time", "8", url])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Часовой пояс по внешнему IP (несколько геосервисов по очереди; нужна
/// сеть). Ответ проверяется по `zoneinfo`.
pub fn detect_timezone() -> Result<String, String> {
    let sys = Sys::host();
    let tries: [(&str, fn(&str) -> Option<String>); 3] = [
        ("http://ip-api.com/line/?fields=timezone", |t| Some(t.to_string())),
        ("https://ipwho.is/?fields=timezone", |t| {
            let j: serde_json::Value = serde_json::from_str(t).ok()?;
            j.get("timezone")?.get("id")?.as_str().map(String::from)
        }),
        ("https://ipapi.co/timezone", |t| Some(t.to_string())),
    ];
    let mut reached = false;
    for (url, parse) in tries {
        let Some(body) = curl(url) else { continue };
        reached = true;
        if let Some(tz) = parse(&body).filter(|tz| zone_exists(&sys, tz)) {
            return Ok(tz);
        }
    }
    Err(if reached { t!("геосервисы не определили пояс").into() } else { t!("нет связи с интернетом").into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_offsets() {
        assert_eq!(parse_posix_offset("MSK-3"), Some(3 * 3600));
        assert_eq!(parse_posix_offset("<+05>-5"), Some(5 * 3600));
        assert_eq!(parse_posix_offset("CET-1CEST,M3.5.0,M10.5.0/3"), Some(3600));
        assert_eq!(parse_posix_offset("EST5EDT,M3.2.0,M11.1.0"), Some(-5 * 3600));
        assert_eq!(parse_posix_offset("<+0530>-5:30"), Some(5 * 3600 + 1800));
        assert_eq!(parse_posix_offset("UTC0"), Some(0));
        assert_eq!(parse_posix_offset(""), None);
    }

    #[test]
    fn offsets_format() {
        assert_eq!(format_offset(0), "UTC");
        assert_eq!(format_offset(5 * 3600), "UTC+05:00");
        assert_eq!(format_offset(-(3 * 3600 + 1800)), "UTC−03:30");
    }

    #[test]
    fn zone_names() {
        assert_eq!(zone_label("America/Argentina/Buenos_Aires"), ("Buenos Aires".into(), "America/Argentina".into()));
        assert_eq!(zone_label("UTC"), ("UTC".into(), String::new()));
    }

    #[test]
    fn zone_check_rejects_garbage() {
        let sys = Sys::host();
        assert!(!zone_exists(&sys, "../../etc/passwd"));
        assert!(!zone_exists(&sys, "{'error': True}"));
        if sys.path("/usr/share/zoneinfo/Europe/Moscow").exists() {
            assert!(zone_exists(&sys, "Europe/Moscow"));
            assert_eq!(std_offset(&sys, "Europe/Moscow"), Some(3 * 3600));
            assert!(zones(&sys).iter().any(|z| z == "Asia/Almaty"));
        }
    }
}
