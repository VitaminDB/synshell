//! Процессы: память (PSS/RSS) и загрузка процессора по pid — для бейджей
//! запущенных приложений.

use crate::Sys;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProcStat {
    /// Память процесса, КБ: PSS (честная доля разделяемой), иначе RSS.
    pub memory_kb: u64,
    /// Загрузка процессора с прошлого снимка, % одного ядра × ядра (как top:
    /// 200% — два ядра целиком).
    pub cpu_percent: f32,
}

/// Процесс и его потомки (приложение из нескольких процессов — браузер).
fn children_of(sys: &Sys, pid: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let tasks = sys.list(format!("/proc/{pid}/task"));
    for t in tasks {
        if let Some(c) = sys.read(format!("/proc/{pid}/task/{t}/children")) {
            out.extend(c.split_whitespace().filter_map(|p| p.parse::<i32>().ok()));
        }
    }
    out
}

fn memory_kb(sys: &Sys, pid: i32) -> Option<u64> {
    if let Some(s) = sys.read(format!("/proc/{pid}/smaps_rollup")) {
        if let Some(v) = s.lines().find(|l| l.starts_with("Pss:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()) {
            return Some(v);
        }
    }
    let statm = sys.read(format!("/proc/{pid}/statm"))?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * page_kb())
}

fn page_kb() -> u64 {
    // SAFETY: sysconf без побочных эффектов.
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 { p as u64 / 1024 } else { 4 }
}

/// utime+stime в тиках.
fn cpu_ticks(sys: &Sys, pid: i32) -> Option<u64> {
    let s = sys.read(format!("/proc/{pid}/stat"))?;
    // Имя в скобках может содержать пробелы — считаем поля после «)».
    let rest = &s[s.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

fn total_ticks(sys: &Sys) -> Option<u64> {
    let s = sys.read("/proc/stat")?;
    Some(s.lines().next()?.split_whitespace().skip(1).take(8).filter_map(|x| x.parse::<u64>().ok()).sum())
}

/// Снимки по pid: загрузка — разность с прошлым снимком.
#[derive(Debug, Default)]
pub struct ProcSampler {
    sys: Sys,
    last: HashMap<i32, u64>,
    last_total: Option<u64>,
    cores: usize,
}

impl ProcSampler {
    pub fn new(sys: Sys) -> Self {
        // SAFETY: sysconf без побочных эффектов.
        let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_CONF) };
        Self { sys, last: HashMap::new(), last_total: None, cores: n.max(1) as usize }
    }

    /// Для каждого pid — процесс вместе с потомками.
    pub fn sample(&mut self, pids: &[i32]) -> HashMap<i32, ProcStat> {
        let total = total_ticks(&self.sys);
        let dt_total = match (total, self.last_total) {
            (Some(t), Some(l)) if t > l => Some((t - l) as f32),
            _ => None,
        };
        let mut out = HashMap::new();
        let mut seen = HashMap::new();
        for &pid in pids {
            // Процесс и потомки до трёх поколений (браузер: zygote → вкладки).
            let mut group = vec![pid];
            let mut frontier = vec![pid];
            for _ in 0..3 {
                let next: Vec<i32> = frontier.iter().flat_map(|p| children_of(&self.sys, *p)).collect();
                if next.is_empty() {
                    break;
                }
                group.extend(&next);
                frontier = next;
            }
            let mut mem = 0;
            let mut ticks = 0;
            for p in &group {
                mem += memory_kb(&self.sys, *p).unwrap_or(0);
                ticks += cpu_ticks(&self.sys, *p).unwrap_or(0);
            }
            let cpu = match (dt_total, self.last.get(&pid)) {
                // Тики /proc/stat — сумма по всем ядрам.
                (Some(dt), Some(&prev)) => ticks.saturating_sub(prev) as f32 / dt * 100.0 * self.cores as f32,
                _ => 0.0,
            };
            seen.insert(pid, ticks);
            out.insert(pid, ProcStat { memory_kb: mem, cpu_percent: cpu });
        }
        self.last = seen;
        self.last_total = total;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stat_fields_after_comm() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("proc/42");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("stat"), "42 (my app (x)) S 1 42 42 0 -1 4194560 100 0 0 0 70 30 0 0 20 0 1 0").unwrap();
        fs::write(p.join("smaps_rollup"), "Rss: 2000 kB\nPss:  1500 kB\n").unwrap();
        let sys = Sys::at(dir.path());
        assert_eq!(cpu_ticks(&sys, 42), Some(100));
        assert_eq!(memory_kb(&sys, 42), Some(1500));
    }
}
