//! Даты как номер дня от 1970-01-01 (местное время): арифметика без часовых поясов.

/// День: число дней от 1970-01-01.
pub type Day = i32;

/// Сегодня по местному времени.
pub fn today() -> Day {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // SAFETY: localtime_r с корректными буферами.
    let tm = unsafe {
        let t: libc::time_t = now as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    from_ymd(tm.tm_year + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32)
}

/// Текущее местное время в минутах от полуночи (с шагом 5 минут).
pub fn now_minutes() -> u16 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // SAFETY: localtime_r с корректными буферами.
    let tm = unsafe {
        let t: libc::time_t = now as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    ((tm.tm_hour * 60 + tm.tm_min) / 5 * 5) as u16
}

/// Год, месяц (1–12), число → день (алгоритм Хиннанта).
pub fn from_ymd(y: i32, m: u32, d: u32) -> Day {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// День → (год, месяц 1–12, число).
pub fn ymd(z: Day) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// День недели: 0 — понедельник … 6 — воскресенье.
pub fn weekday(d: Day) -> u32 {
    // 1970-01-01 — четверг
    (d + 3).rem_euclid(7) as u32
}

/// Месяц как число месяцев от 0000-01 (для перелистывания).
pub fn month_index(d: Day) -> i32 {
    let (y, m, _) = ymd(d);
    y * 12 + m as i32 - 1
}

pub fn month_start(idx: i32) -> Day {
    from_ymd(idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1, 1)
}

pub fn days_in_month(idx: i32) -> i32 {
    month_start(idx + 1) - month_start(idx)
}

pub fn month_name(m: u32) -> String {
    let names = ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"];
    syngui::i18n::t(names[(m as usize).clamp(1, 12) - 1])
}

/// Месяц в родительном падеже («12 октября»).
pub fn month_gen(m: u32) -> String {
    let names = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    syngui::i18n::t(names[(m as usize).clamp(1, 12) - 1])
}

pub fn month_short(m: u32) -> String {
    let names = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
    syngui::i18n::t(names[(m as usize).clamp(1, 12) - 1])
}

/// Короткие имена дней недели с понедельника.
pub fn weekday_short(wd: u32) -> String {
    let names = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];
    syngui::i18n::t(names[wd as usize % 7])
}

pub fn weekday_name(wd: u32) -> String {
    let names = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];
    syngui::i18n::t(names[wd as usize % 7])
}

/// «12 октября».
pub fn long(d: Day) -> String {
    let (_, m, dd) = ymd(d);
    format!("{dd} {}", month_gen(m))
}

/// «12 окт».
pub fn short(d: Day) -> String {
    let (_, m, dd) = ymd(d);
    format!("{dd} {}", month_short(m))
}

/// «12–18 окт» или «28 сен – 4 окт».
pub fn range(a: Day, b: Day) -> String {
    let (_, ma, da) = ymd(a);
    let (_, mb, db) = ymd(b);
    if ma == mb {
        format!("{da}–{db} {}", month_short(mb))
    } else {
        format!("{} – {}", short(a), short(b))
    }
}

/// «сегодня», «завтра», «через 5 дней», «3 дня назад».
pub fn relative(d: Day, today: Day) -> String {
    let n = d - today;
    match n {
        0 => syngui::i18n::t("сегодня"),
        1 => syngui::i18n::t("завтра"),
        -1 => syngui::i18n::t("вчера"),
        n if n > 0 => syngui::tn!(n, "через {n} день", "через {n} дня", "через {n} дней"),
        n => syngui::tn!(-n, "{n} день назад", "{n} дня назад", "{n} дней назад"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for d in -1000..30000 {
            let (y, m, dd) = ymd(d);
            assert_eq!(from_ymd(y, m, dd), d);
        }
        assert_eq!(ymd(0), (1970, 1, 1));
        assert_eq!(weekday(from_ymd(2026, 10, 13)), 1);
    }
}
