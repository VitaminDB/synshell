//! Время: `strftime` из libc (локаль и часовой пояс системы) и тикающий
//! сигнал `ShellCtx::now`.

use std::ffi::CString;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ctx::ShellCtx;

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Локальное время в формате strftime (`%H:%M`, `%a, %d %b`).
pub fn format(ts: i64, fmt: &str) -> String {
    let Ok(cfmt) = CString::new(fmt) else { return String::new() };
    // SAFETY: localtime_r/strftime с корректными буферами.
    unsafe {
        let t: libc::time_t = ts as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        let mut buf = vec![0u8; 256];
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm);
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// Год, месяц (1..12), день — для календаря.
pub fn ymd(ts: i64) -> (i32, u32, u32) {
    // SAFETY: см. `format`.
    unsafe {
        let t: libc::time_t = ts as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
    }
}

/// Тикать `now` на границе каждой секунды.
pub fn start(ctx: ShellCtx) {
    // Локаль для strftime (названия дней/месяцев).
    // SAFETY: вызывается один раз на старте из главного потока.
    unsafe {
        libc::setlocale(libc::LC_TIME, c"".as_ptr());
    }
    fn next_delay() -> Duration {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        Duration::from_millis(1000 - (now.subsec_millis() as u64).min(999))
    }
    syngui_layer::add_timer(next_delay(), move || {
        ctx.now.set(unix_now());
        Some(next_delay())
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        let s = super::format(0, "%Y");
        assert!(s == "1970" || s == "1969", "{s:?}");
        assert_eq!(super::format(3600 * 5 + 60 * 7, "%M"), "07");
    }
}
