//! Мелкие кроссплатформенные утилиты: время и форматирование.

use std::time::{SystemTime, UNIX_EPOCH};

/// Текущее время в epoch-миллисекундах. Работает и на desktop, и на Android.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Локальное «сейчас» как (год, месяц 1–12, день, час, минута).
pub fn now_local() -> (i32, u32, u32, u32, u32) {
    let t = (now_ms() / 1000) as libc::time_t;
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (
            tm.tm_year + 1900,
            (tm.tm_mon + 1) as u32,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
        )
    }
}

/// Смещение локального часового пояса от UTC в секундах (восток — положительно).
fn local_offset_secs() -> i64 {
    let t = (now_ms() / 1000) as libc::time_t;
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i64
    }
}

/// Число дней от эпохи Unix для гражданской даты (алгоритм Хиннанта).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (if m > 2 { m - 3 } else { m + 9 }) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Локальные (год, месяц, день, час, минута) → epoch-миллисекунды (UTC).
pub fn local_civil_to_epoch_ms(y: i32, m: u32, d: u32, h: u32, min: u32) -> u64 {
    let days = days_from_civil(y, m, d);
    let secs = days * 86_400 + (h as i64) * 3600 + (min as i64) * 60 - local_offset_secs();
    (secs.max(0) as u64) * 1000
}

/// Локальный час суток (0–23) для момента `ms`.
pub fn local_hour(ms: u64) -> u32 {
    let secs = ms as i64 / 1000 + local_offset_secs();
    secs.rem_euclid(86_400) as u32 / 3600
}

/// Локальные (час, минута) для момента `ms`.
pub fn local_hm(ms: u64) -> (u32, u32) {
    let day_secs = (ms as i64 / 1000 + local_offset_secs()).rem_euclid(86_400);
    ((day_secs / 3600) as u32, ((day_secs % 3600) / 60) as u32)
}

/// Индекс локального дня (число дней от эпохи в локальном поясе).
pub fn local_day_index(ms: u64) -> i64 {
    (ms as i64 / 1000 + local_offset_secs()).div_euclid(86_400)
}

/// День недели по индексу дня, понедельник = 0 (эпоха 1970-01-01 — четверг).
pub fn weekday_mon0(day_index: i64) -> usize {
    (day_index + 3).rem_euclid(7) as usize
}

/// Индекс дня → гражданская дата (год, месяц 1–12, день). Обратный к `days_from_civil`.
pub fn civil_from_days(day_index: i64) -> (i32, u32, u32) {
    let z = day_index + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Разбить секунды на (дни, часы, минуты, секунды).
pub fn split_dhms(total_secs: u64) -> (u64, u64, u64, u64) {
    let d = total_secs / 86_400;
    let h = (total_secs % 86_400) / 3_600;
    let m = (total_secs % 3_600) / 60;
    let s = total_secs % 60;
    (d, h, m, s)
}

/// Формат `ЧЧ:ММ:СС` (или `Д д ЧЧ:ММ:СС`, если есть дни).
pub fn fmt_hms(total_secs: u64) -> String {
    let (d, h, m, s) = split_dhms(total_secs);
    if d > 0 {
        syngui::t!("{d} д {hms}", d = d, hms = format!("{h:02}:{m:02}:{s:02}"))
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// Компактный формат крупной длительности: «3 дн 4 ч» / «5 ч 12 мин» / «42 мин».
pub fn fmt_duration_compact_ru(total_secs: u64) -> String {
    let (d, h, m, _s) = split_dhms(total_secs);
    if d > 0 {
        format!("{d} дн {h} ч")
    } else if h > 0 {
        format!("{h} ч {m} мин")
    } else {
        format!("{m} мин")
    }
}

/// Округление денежной суммы до двух знаков для отображения.
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Целое число с разделением тысяч неразрывным пробелом: `1 234 567`.
pub fn group_int(n: i64) -> String {
    let neg = n < 0;
    let digits = n.abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::new();
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push('\u{202f}'); // narrow no-break space
        }
        out.push(*b as char);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

/// Денежная сумма с символом валюты: `1 234 ₽`.
pub fn fmt_money(v: f64, symbol: &str) -> String {
    format!("{} {}", group_int(v.round() as i64), symbol)
}
