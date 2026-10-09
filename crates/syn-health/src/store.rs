//! Шаги по дням и часам (`~/.local/share/synshell/health/steps.json`), настройки
//! (`~/.config/synshell/health.toml`) и счётчик: шагомер SLPI через источник платформы
//! `/usr/lib/syndroid/sensors-source steps` (строки `S n` — шагов с включения).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SOURCE: &str = "/usr/lib/syndroid/sensors-source";

/// День: шаги всего и по часам.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    pub total: u32,
    pub hours: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steps {
    /// Дата «2026-10-09» → день.
    pub days: BTreeMap<String, Day>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Цель шагов в день.
    pub goal: u32,
    /// Длина шага, м.
    pub step_m: f32,
    /// Вес, кг (калории).
    pub weight_kg: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { goal: 8000, step_m: 0.75, weight_kg: 75.0 }
    }
}

fn data_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/share"));
    base.join("synshell/health/steps.json")
}

fn settings_path() -> std::path::PathBuf {
    synshell_common::paths::config_dir().join("health.toml")
}

impl Steps {
    pub fn load() -> Steps {
        std::fs::read_to_string(data_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = data_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }

    /// Прибавить шаги к часу `hour` дня `date`.
    pub fn add(&mut self, date: &str, hour: usize, n: u32) {
        let d = self.days.entry(date.to_string()).or_default();
        if d.hours.len() < 24 {
            d.hours.resize(24, 0);
        }
        d.hours[hour.min(23)] += n;
        d.total += n;
    }

    pub fn day(&self, date: &str) -> Day {
        let mut d = self.days.get(date).cloned().unwrap_or_default();
        d.hours.resize(24, 0);
        d
    }
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(settings_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = settings_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, toml::to_string(self).unwrap_or_default());
    }

    pub fn km(&self, steps: u32) -> f32 {
        steps as f32 * self.step_m / 1000.0
    }

    /// Ходьба: ≈0,5 ккал на килограмм веса на километр.
    pub fn kcal(&self, steps: u32) -> f32 {
        self.km(steps) * self.weight_kg * 0.5
    }
}

/// Местные дата и час сейчас (и `days_ago` дней назад).
pub fn local_date(days_ago: i64) -> (String, usize) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - days_ago * 86400;
    // SAFETY: localtime_r с корректными буферами.
    let tm = unsafe {
        let t: libc::time_t = now as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    (format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday), tm.tm_hour as usize)
}

/// День недели (0 — понедельник) для даты N дней назад.
pub fn weekday(days_ago: i64) -> usize {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - days_ago * 86400;
    // SAFETY: localtime_r с корректными буферами.
    let wd = unsafe {
        let t: libc::time_t = now as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_wday
    };
    ((wd + 6) % 7) as usize
}

/// Счётчик в фоне (`syn-health --track`, автозапуск сеанса): шаги источника — в файл.
/// Нет источника (компьютер) — выход; источник упал — перезапуск через минуту.
pub fn track() {
    if !std::path::Path::new(SOURCE).exists() && std::env::var_os("SYN_HEALTH_SOURCE").is_none() {
        tracing::info!("шагомера нет ({SOURCE}) — счётчик не нужен");
        return;
    }
    let src = std::env::var("SYN_HEALTH_SOURCE").unwrap_or_else(|_| SOURCE.to_string());
    loop {
        let child = Command::new(&src).arg("steps").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("шагомер: {e}");
                std::thread::sleep(Duration::from_secs(60));
                continue;
            }
        };
        let _stdin = child.stdin.take();
        let Some(out) = child.stdout.take() else { return };
        let mut prev: Option<u64> = None;
        let mut steps = Steps::load();
        let mut saved = Instant::now();
        let mut dirty = false;
        for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
            let mut it = line.split_whitespace();
            if it.next() != Some("S") {
                continue;
            }
            let Some(n) = it.next().and_then(|v| v.parse::<f64>().ok()).map(|v| v.max(0.0) as u64) else { continue };
            // счёт источника с его запуска; первый отчёт — точка отсчёта
            let delta = match prev {
                Some(p) if n >= p => n - p,
                Some(_) => n,
                None => 0,
            };
            prev = Some(n);
            if delta > 0 && delta < 20000 {
                let (date, hour) = local_date(0);
                // файл мог обновить кто-то ещё (сброс в программе) — свежая копия
                if saved.elapsed() > Duration::from_secs(60) {
                    steps = Steps::load();
                }
                steps.add(&date, hour, delta as u32);
                dirty = true;
            }
            if dirty && saved.elapsed() > Duration::from_secs(10) {
                steps.save();
                saved = Instant::now();
                dirty = false;
            }
        }
        if dirty {
            steps.save();
        }
        let _ = child.wait();
        tracing::warn!("шагомер: источник завершился, перезапуск через минуту");
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_estimates() {
        let mut s = Steps::default();
        s.add("2026-10-09", 9, 1200);
        s.add("2026-10-09", 9, 300);
        s.add("2026-10-09", 18, 2000);
        let d = s.day("2026-10-09");
        assert_eq!(d.total, 3500);
        assert_eq!(d.hours[9], 1500);
        assert_eq!(s.day("2026-10-08").total, 0);
        let set = Settings::default();
        assert!((set.km(10000) - 7.5).abs() < 0.01);
        assert!((set.kcal(10000) - 281.25).abs() < 0.1);
    }
}
