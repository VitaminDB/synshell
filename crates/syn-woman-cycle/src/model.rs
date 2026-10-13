//! Отметки по дням, настройки и прогноз цикла. Хранится локально:
//! `~/.local/share/synshell/woman-cycle/data.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::date::Day;

/// Симптомы и самочувствие — биты `DayLog::sym`.
pub const SYMPTOMS: &[(u32, &str, &str)] = &[
    (1 << 0, "Спазмы", "\u{EA0B}"),
    (1 << 1, "Головная боль", "\u{EA4A}"),
    (1 << 2, "Чувствительная грудь", "\u{E87D}"),
    (1 << 3, "Вздутие", "\u{E6DD}"),
    (1 << 4, "Акне", "\u{E87C}"),
    (1 << 5, "Усталость", "\u{E19C}"),
    (1 << 6, "Бессонница", "\u{EF44}"),
    (1 << 7, "Тяга к сладкому", "\u{E7E9}"),
    (1 << 8, "Тошнота", "\u{F220}"),
    (1 << 9, "Высокое либидо", "\u{E3E4}"),
];

/// Настроение — `DayLog::mood` (0 — не отмечено).
pub const MOODS: &[(u8, &str, &str)] = &[(1, "Отлично", "\u{E813}"), (2, "Спокойно", "\u{E812}"), (3, "Грустно", "\u{E811}"), (4, "Тревожно", "\u{E7F3}"), (5, "Раздражение", "\u{E814}")];

/// Обильность: 1 — мажущие, 2 — слабые, 3 — средние, 4 — обильные.
pub const FLOWS: &[(u8, &str)] = &[(1, "Мажущие"), (2, "Слабые"), (3, "Средние"), (4, "Обильные")];

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DayLog {
    /// 0 — нет месячных.
    #[serde(default)]
    pub flow: u8,
    #[serde(default)]
    pub sym: u32,
    #[serde(default)]
    pub mood: u8,
}

impl DayLog {
    pub fn is_empty(&self) -> bool {
        self.flow == 0 && self.sym == 0 && self.mood == 0
    }
}

/// Близость: день, необязательное время (минуты от полуночи) и защита (если отмечена).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Intimacy {
    pub id: u64,
    pub day: Day,
    #[serde(default)]
    pub time: Option<u16>,
    #[serde(default)]
    pub protected: Option<bool>,
}

impl Intimacy {
    /// «21:30» или пусто.
    pub fn time_text(&self) -> Option<String> {
        self.time.map(|m| format!("{:02}:{:02}", m / 60, m % 60))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Длина цикла, если истории мало или автоподсчёт выключен.
    pub cycle_len: i32,
    pub period_len: i32,
    /// От овуляции до месячных.
    pub luteal: i32,
    /// Считать длины по истории (последние 6 циклов).
    pub auto: bool,
    pub theme: ThemeMode,
    pub scale: f32,
    pub fullscreen: bool,
    pub monday: bool,
    pub show_fertile: bool,
    pub show_ovulation: bool,
    /// Отметки близости в календаре (можно скрыть).
    pub show_intimacy: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { cycle_len: 28, period_len: 5, luteal: 14, auto: true, theme: ThemeMode::System, scale: 1.0, fullscreen: true, monday: true, show_fertile: true, show_ovulation: true, show_intimacy: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub onboarded: bool,
    pub settings: Settings,
    /// Отметки по дням (ключ — `Day`).
    pub days: BTreeMap<Day, DayLog>,
    /// День, когда отмечено «месячные закончились» (остаток прогноза не показывается).
    pub ended: Option<Day>,
    /// Близость по дням (в порядке дня и времени).
    pub intimacy: Vec<Intimacy>,
}

fn store_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/share"));
    base.join("synshell/woman-cycle/data.json")
}

pub fn load() -> Data {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(d: &Data) {
    let p = store_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    let r = std::fs::write(&tmp, serde_json::to_string_pretty(d).unwrap_or_default()).and_then(|_| std::fs::rename(&tmp, &p));
    if let Err(e) = r {
        tracing::warn!("данные не сохранены: {e}");
    }
}

// ── Прогноз ───────────────────────────────────────────────────────────────

/// Что показать в дне календаря (по убыванию важности).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Period,
    /// Прогноз месячных (в т. ч. остаток идущих).
    Predicted,
    Ovulation,
    Fertile,
    None,
}

/// Фаза цикла сегодня.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Period,
    Follicular,
    Fertile,
    Ovulation,
    Luteal,
    /// Месячные не начались в срок.
    Late,
}

/// Завершённый или текущий цикл.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cycle {
    pub start: Day,
    pub period: i32,
    /// Длина до следующего начала; `None` — текущий.
    pub len: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub today: Day,
    pub cycle: i32,
    pub period: i32,
    pub luteal: i32,
    pub cycles: Vec<Cycle>,
    /// Прогноз начала ближайших месячных и следующих (12 шт.).
    pub next: Vec<Day>,
    /// Дней задержки (0 — нет).
    pub late: i32,
    /// Идут ли месячные сегодня (отмечены сегодня или вчера и ещё не кончились по средней длине).
    pub ongoing: bool,
    /// Ожидаемый конец идущих месячных.
    pub ongoing_end: Option<Day>,
    show_fertile: bool,
    show_ovulation: bool,
    pub show_intimacy: bool,
    days: BTreeMap<Day, DayLog>,
    intimacy: Vec<Intimacy>,
}

/// Подряд идущие дни месячных (разрыв до 2 дней — тот же период): (начало, конец).
fn runs(days: &BTreeMap<Day, DayLog>) -> Vec<(Day, Day)> {
    let mut out: Vec<(Day, Day)> = Vec::new();
    for (&d, l) in days {
        if l.flow == 0 {
            continue;
        }
        match out.last_mut() {
            Some((_, e)) if d - *e <= 2 => *e = d,
            _ => out.push((d, d)),
        }
    }
    out
}

fn mean(v: &[i32]) -> Option<i32> {
    if v.is_empty() {
        None
    } else {
        Some((v.iter().sum::<i32>() as f32 / v.len() as f32).round() as i32)
    }
}

impl Forecast {
    pub fn new(data: &Data, today: Day) -> Self {
        let s = &data.settings;
        let rs: Vec<(Day, Day)> = runs(&data.days).into_iter().filter(|r| r.0 <= today).collect();
        let mut cycles: Vec<Cycle> = Vec::new();
        for (i, r) in rs.iter().enumerate() {
            let len = rs.get(i + 1).map(|n| n.0 - r.0);
            cycles.push(Cycle { start: r.0, period: r.1 - r.0 + 1, len });
        }
        let lens: Vec<i32> = cycles.iter().filter_map(|c| c.len).filter(|l| (18..=60).contains(l)).rev().take(6).collect();
        let pers: Vec<i32> = cycles.iter().rev().filter(|c| c.len.is_some()).map(|c| c.period).filter(|p| (1..=12).contains(p)).take(6).collect();
        let cycle = if s.auto { mean(&lens).unwrap_or(s.cycle_len) } else { s.cycle_len }.clamp(18, 60);
        let period = if s.auto { mean(&pers).unwrap_or(s.period_len) } else { s.period_len }.clamp(1, 12);
        let luteal = s.luteal.clamp(8, 18).min(cycle - 4);

        let mut next = Vec::new();
        let mut late = 0;
        let mut ongoing = false;
        let mut ongoing_end = None;
        if let Some(last) = cycles.last() {
            let end = last.start + last.period - 1;
            if today - end <= 1 && today - last.start < 12 && data.ended.is_none_or(|e| e < last.start) {
                ongoing = true;
                ongoing_end = Some((last.start + period - 1).max(end).max(today));
            }
            let mut p = last.start + cycle;
            if p <= today {
                late = today - p + 1;
                p = today + 1;
            }
            for _ in 0..12 {
                next.push(p);
                p += cycle;
            }
        }
        Forecast { today, cycle, period, luteal, cycles, next, late, ongoing, ongoing_end, show_fertile: s.show_fertile, show_ovulation: s.show_ovulation, show_intimacy: s.show_intimacy, days: data.days.clone(), intimacy: data.intimacy.clone() }
    }

    pub fn has_data(&self) -> bool {
        !self.cycles.is_empty()
    }

    /// Близость в этот день (по времени, без времени — в конце).
    pub fn intimacy(&self, d: Day) -> Vec<Intimacy> {
        let mut v: Vec<Intimacy> = self.intimacy.iter().filter(|i| i.day == d).copied().collect();
        v.sort_by_key(|i| (i.time.is_none(), i.time, i.id));
        v
    }

    /// Близость за дни `from..=to`: всего и в фертильные дни.
    pub fn intimacy_stats(&self, from: Day, to: Day) -> (usize, usize) {
        let list: Vec<&Intimacy> = self.intimacy.iter().filter(|i| i.day >= from && i.day <= to).collect();
        let fertile = list.iter().filter(|i| self.chance(i.day) > 0).count();
        (list.len(), fertile)
    }

    pub fn log(&self, d: Day) -> DayLog {
        self.days.get(&d).copied().unwrap_or_default()
    }

    /// Начало цикла, в который входит день, и начало следующего (факт или прогноз).
    pub fn cycle_of(&self, d: Day) -> Option<(Day, Day)> {
        let mut starts: Vec<Day> = self.cycles.iter().map(|c| c.start).collect();
        starts.extend(self.next.iter().copied());
        let i = starts.iter().rposition(|&s| s <= d)?;
        // при задержке у текущего цикла овуляция — по ожидавшемуся сроку
        let last = self.cycles.len().checked_sub(1);
        let next = if self.late > 0 && Some(i) == last { starts[i] + self.cycle } else { starts.get(i + 1).copied().unwrap_or(starts[i] + self.cycle) };
        Some((starts[i], next))
    }

    /// День овуляции цикла с началом следующего `next`.
    pub fn ovulation(&self, next: Day) -> Day {
        next - self.luteal
    }

    /// День цикла (с 1).
    pub fn cycle_day(&self, d: Day) -> Option<i32> {
        self.cycle_of(d).map(|(s, _)| d - s + 1)
    }

    pub fn kind(&self, d: Day) -> Kind {
        if self.log(d).flow > 0 {
            return Kind::Period;
        }
        if d > self.today {
            if let Some(e) = self.ongoing_end {
                if d <= e {
                    return Kind::Predicted;
                }
            }
            if self.next.iter().any(|&p| d >= p && d < p + self.period) {
                return Kind::Predicted;
            }
        }
        let Some((_, next)) = self.cycle_of(d) else {
            return Kind::None;
        };
        let ov = self.ovulation(next);
        if self.show_ovulation && d == ov {
            return Kind::Ovulation;
        }
        if self.show_fertile && d >= ov - 5 && d <= ov + 1 {
            return Kind::Fertile;
        }
        Kind::None
    }

    pub fn phase(&self) -> Option<Phase> {
        let t = self.today;
        if self.log(t).flow > 0 || self.ongoing {
            return Some(Phase::Period);
        }
        if self.late > 0 {
            return Some(Phase::Late);
        }
        let (_, next) = self.cycle_of(t)?;
        let ov = self.ovulation(next);
        Some(if t == ov {
            Phase::Ovulation
        } else if t >= ov - 5 && t <= ov + 1 {
            Phase::Fertile
        } else if t < ov {
            Phase::Follicular
        } else {
            Phase::Luteal
        })
    }

    /// Ближайшая овуляция начиная с сегодняшнего дня.
    pub fn next_ovulation(&self) -> Option<Day> {
        let (_, next) = self.cycle_of(self.today)?;
        let ov = self.ovulation(next);
        if ov >= self.today {
            Some(ov)
        } else {
            self.next.iter().map(|&p| self.ovulation(p + self.cycle)).find(|&o| o >= self.today)
        }
    }

    /// Вероятность зачатия в день: 0 — низкая, 1 — средняя, 2 — высокая.
    pub fn chance(&self, d: Day) -> u8 {
        let Some((_, next)) = self.cycle_of(d) else {
            return 0;
        };
        let ov = self.ovulation(next);
        match d - ov {
            -2..=0 => 2,
            -5..=-3 | 1 => 1,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(starts: &[Day], len: i32) -> Data {
        let mut d = Data::default();
        for &s in starts {
            for i in 0..len {
                d.days.insert(s + i, DayLog { flow: 2, ..Default::default() });
            }
        }
        d
    }

    #[test]
    fn forecast_by_history() {
        let d = data(&[100, 130, 160], 4);
        let f = Forecast::new(&d, 170);
        assert_eq!(f.cycle, 30);
        assert_eq!(f.period, 4);
        assert_eq!(f.next[0], 190);
        assert_eq!(f.kind(190), Kind::Predicted);
        assert_eq!(f.kind(176), Kind::Ovulation);
        assert_eq!(f.kind(172), Kind::Fertile);
        assert_eq!(f.cycle_day(170), Some(11));
        assert_eq!(f.kind(161), Kind::Period);
    }

    #[test]
    fn intimacy_by_day_and_stats() {
        let mut d = data(&[100, 130, 160], 4);
        d.intimacy.push(Intimacy { id: 1, day: 172, time: None, protected: None });
        d.intimacy.push(Intimacy { id: 2, day: 172, time: Some(8 * 60), protected: Some(true) });
        d.intimacy.push(Intimacy { id: 3, day: 165, time: Some(23 * 60), protected: Some(false) });
        let f = Forecast::new(&d, 175);
        let day = f.intimacy(172);
        assert_eq!(day.iter().map(|i| i.id).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(day[0].time_text().as_deref(), Some("08:00"));
        // цикл с 160: 165 — обычный день, 172 — фертильное окно (овуляция 176)
        assert_eq!(f.intimacy_stats(160, 175), (3, 2));
    }

    #[test]
    fn late_period() {
        let d = data(&[100], 5);
        let f = Forecast::new(&d, 130);
        assert_eq!(f.late, 3);
        assert_eq!(f.phase(), Some(Phase::Late));
        assert_eq!(f.next[0], 131);
    }

    #[test]
    fn ongoing_period() {
        let d = data(&[100], 2);
        let f = Forecast::new(&d, 101);
        assert!(f.ongoing);
        assert_eq!(f.kind(103), Kind::Predicted);
        assert_eq!(f.kind(105), Kind::None);
    }
}
