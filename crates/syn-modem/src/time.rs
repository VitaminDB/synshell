//! Местное время для показа SMS и журнала звонков.

use synshell_tr::{n_, t};
/// Разложенное местное время.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// День от начала эпохи в местном поясе (для «сегодня/вчера»).
    pub days: i64,
}

pub fn local(t: i64) -> Local {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as libc::time_t;
    unsafe { libc::localtime_r(&tt, &mut tm) };
    Local {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        days: (t + tm.tm_gmtoff as i64).div_euclid(86400),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

const MONTHS: [&str; 12] = [n_!("янв"), n_!("фев"), n_!("мар"), n_!("апр"), n_!("мая"), n_!("июн"), n_!("июл"), n_!("авг"), n_!("сен"), n_!("окт"), n_!("ноя"), n_!("дек")];

/// Коротко для списков: «14:05», «вчера», «3 окт», «3 окт 2025».
pub fn short(t: i64) -> String {
    let l = local(t);
    let n = local(now());
    if l.days == n.days {
        format!("{:02}:{:02}", l.hour, l.minute)
    } else if l.days == n.days - 1 {
        t!("вчера").into()
    } else if l.year == n.year {
        format!("{} {}", l.day, MONTHS[(l.month - 1) as usize])
    } else {
        format!("{} {} {}", l.day, MONTHS[(l.month - 1) as usize], l.year)
    }
}

/// Заголовок дня в переписке: «Сегодня», «Вчера», «3 октября».
pub fn day_title(t: i64) -> String {
    const FULL: [&str; 12] =
        [n_!("января"), n_!("февраля"), n_!("марта"), n_!("апреля"), n_!("мая"), n_!("июня"), n_!("июля"), n_!("августа"), n_!("сентября"), n_!("октября"), n_!("ноября"), n_!("декабря")];
    let l = local(t);
    let n = local(now());
    if l.days == n.days {
        t!("Сегодня").into()
    } else if l.days == n.days - 1 {
        t!("Вчера").into()
    } else if l.year == n.year {
        format!("{} {}", l.day, FULL[(l.month - 1) as usize])
    } else {
        format!("{} {} {}", l.day, FULL[(l.month - 1) as usize], l.year)
    }
}

pub fn hm(t: i64) -> String {
    let l = local(t);
    format!("{:02}:{:02}", l.hour, l.minute)
}

/// Длительность «1:05» / «1:02:03».
pub fn duration(s: u32) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
