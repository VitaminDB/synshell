//! Время: `strftime` из libc (локаль и часовой пояс системы) и тикающий
//! сигнал `ShellCtx::now`. Пояс перечитывается (`tzset`) при каждом
//! форматировании: его меняют на ходу («Дата и время», автоопределение).

use std::ffi::CString;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ctx::ShellCtx;

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Локальное время в формате strftime (`%H:%M`, `%a, %d %b`).
pub fn format(ts: i64, fmt: &str) -> String {
    let Ok(cfmt) = CString::new(fmt) else { return String::new() };
    let Some(tm) = local(ts) else { return String::new() };
    // SAFETY: strftime с корректными буферами.
    unsafe {
        let mut buf = vec![0u8; 256];
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm);
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// Разбить время в текущем поясе системы. `localtime_r` сам пояс не
/// перечитывает — `tzset` сверяет `/etc/localtime` (дёшево: stat).
fn local(ts: i64) -> Option<libc::tm> {
    extern "C" {
        fn tzset();
    }
    // SAFETY: tzset/localtime_r с корректным буфером.
    unsafe {
        tzset();
        let t: libc::time_t = ts as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&t, &mut tm).is_null()).then_some(tm)
    }
}

/// Год, месяц (1..12), день — для календаря.
pub fn ymd(ts: i64) -> (i32, u32, u32) {
    let tm = local(ts).unwrap_or_else(|| unsafe { std::mem::zeroed() });
    (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
}

/// Смещение текущего пояса от UTC в момент `ts`, секунды к востоку.
pub fn utc_offset(ts: i64) -> i32 {
    local(ts).map(|tm| tm.tm_gmtoff as i32).unwrap_or(0)
}

/// Тикать `now` на границе каждой секунды.
pub fn start(ctx: ShellCtx) {
    // Локаль strftime (названия дней и месяцев) ставит язык интерфейса —
    // `synshell_common::i18n::apply`.
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
