//! Процессы: память (PSS/RSS) и загрузка процессора по pid — для бейджей
//! запущенных приложений.

use crate::Sys;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProcStat {
    /// Память процесса, КБ: PSS (честная доля разделяемой), иначе RSS.
    pub memory_kb: u64,
    /// Загрузка процессора с прошлого снимка, % всего процессора (как общий счётчик:
    /// 100% — все ядра целиком, а не одно, как в top).
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
    last: HashMap<u64, u64>,
    last_total: Option<u64>,
}

impl ProcSampler {
    pub fn new(sys: Sys) -> Self {
        Self { sys, last: HashMap::new(), last_total: None }
    }

    /// Для каждого pid — процесс вместе с потомками.
    pub fn sample(&mut self, pids: &[i32]) -> HashMap<i32, ProcStat> {
        let groups: Vec<(u64, Source)> = pids.iter().map(|&p| (p as u64, Source::Pids(vec![p]))).collect();
        self.sample_groups(&groups).into_iter().map(|(k, v)| (k as i32, v)).collect()
    }

    /// Для каждого ключа (окна) — сумма по его источнику; загрузка — разность с прошлым
    /// снимком того же ключа.
    pub fn sample_groups(&mut self, groups: &[(u64, Source)]) -> HashMap<u64, ProcStat> {
        let total = total_ticks(&self.sys);
        let dt_total = match (total, self.last_total) {
            (Some(t), Some(l)) if t > l => Some((t - l) as f32),
            _ => None,
        };
        let mut out = HashMap::new();
        let mut seen = HashMap::new();
        for (key, src) in groups {
            let (mem, ticks) = match src {
                Source::Pids(roots) => self.pids_usage(roots),
                Source::Cgroup(dir) => cgroup_usage(&self.sys, dir),
            };
            let cpu = match (dt_total, self.last.get(key)) {
                // Тики /proc/stat — сумма по всем ядрам: доля от всего процессора.
                (Some(dt), Some(&prev)) => (ticks.saturating_sub(prev) as f32 / dt * 100.0).min(100.0),
                _ => 0.0,
            };
            seen.insert(*key, ticks);
            out.insert(*key, ProcStat { memory_kb: mem, cpu_percent: cpu });
        }
        self.last = seen;
        self.last_total = total;
        out
    }

    /// Процессы и потомки до трёх поколений (браузер: zygote → вкладки): память КБ, тики.
    fn pids_usage(&self, roots: &[i32]) -> (u64, u64) {
        let mut group = roots.to_vec();
        let mut frontier = roots.to_vec();
        for _ in 0..3 {
            let next: Vec<i32> = frontier.iter().flat_map(|p| children_of(&self.sys, *p)).collect();
            if next.is_empty() {
                break;
            }
            group.extend(&next);
            frontier = next;
        }
        group.sort_unstable();
        group.dedup();
        let mut mem = 0;
        let mut ticks = 0;
        for p in &group {
            mem += memory_kb(&self.sys, *p).unwrap_or(0);
            ticks += cpu_ticks(&self.sys, *p).unwrap_or(0);
        }
        (mem, ticks)
    }
}

/// Что считать за приложение окна.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Процессы с потомками.
    Pids(Vec<i32>),
    /// Вся cgroup v2 (каталог в /sys/fs/cgroup) — контейнер целиком.
    Cgroup(String),
}

/// Память (`memory.current`) и процессор (`cpu.stat` usage_usec → тики) cgroup.
fn cgroup_usage(sys: &Sys, dir: &str) -> (u64, u64) {
    let mem = sys.read_num::<u64>(format!("{dir}/memory.current")).unwrap_or(0) / 1024;
    let usec: u64 = sys
        .read(format!("{dir}/cpu.stat"))
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok()))
        .unwrap_or(0);
    // SAFETY: sysconf без побочных эффектов.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    (mem, usec * hz / 1_000_000)
}

/// Поля `/proc/<pid>/stat`, нужные списку процессов.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PidStat {
    pub name: String,
    pub state: char,
    pub ppid: i32,
    /// utime + stime, тики.
    pub ticks: u64,
    pub threads: u32,
    /// Момент старта, тики с загрузки.
    pub start: u64,
    pub rss_kb: u64,
}

pub fn read_stat(sys: &Sys, pid: i32) -> Option<PidStat> {
    let s = sys.read(format!("/proc/{pid}/stat"))?;
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let f: Vec<&str> = s[close + 1..].split_whitespace().collect();
    let num = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok());
    Some(PidStat {
        name: s[open + 1..close].to_string(),
        state: f.first()?.chars().next()?,
        ppid: f.get(1)?.parse().ok()?,
        ticks: num(11)? + num(12)?,
        threads: num(17).unwrap_or(1) as u32,
        start: num(19).unwrap_or(0),
        rss_kb: num(21).unwrap_or(0) * page_kb(),
    })
}

/// Процесс в общем списке.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    /// Командная строка через пробел (у потоков ядра пусто).
    pub cmdline: String,
    pub user: String,
    pub uid: u32,
    pub state: char,
    pub threads: u32,
    pub rss_kb: u64,
    /// Доля всего процессора с прошлого снимка, %.
    pub cpu_percent: f32,
    /// Работает, с.
    pub age_secs: u64,
}

impl Process {
    /// Поток ядра (kthreadd и его дети).
    pub fn is_kernel(&self) -> bool {
        self.pid == 2 || self.ppid == 2
    }
}

/// Все процессы с загрузкой процессора (разность тиков между снимками).
#[derive(Debug, Default)]
pub struct ProcessTable {
    sys: Sys,
    last: HashMap<i32, (u64, u64)>,
    last_total: Option<u64>,
    cmdlines: HashMap<(i32, u64), String>,
    users: HashMap<u32, String>,
}

impl ProcessTable {
    pub fn new(sys: Sys) -> Self {
        let users = sys
            .read("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
            })
            .collect();
        Self { sys, users, ..Default::default() }
    }

    pub fn sample(&mut self) -> Vec<Process> {
        use std::os::unix::fs::MetadataExt;
        let total = total_ticks(&self.sys);
        let dt = match (total, self.last_total) {
            (Some(t), Some(l)) if t > l => Some((t - l) as f32),
            _ => None,
        };
        // SAFETY: sysconf без побочных эффектов.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
        let uptime = crate::cpu::uptime_secs(&self.sys).unwrap_or(0);
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        let mut cmd_seen = HashMap::new();
        for pid in self.sys.list("/proc").into_iter().filter_map(|p| p.parse::<i32>().ok()) {
            let Some(st) = read_stat(&self.sys, pid) else { continue };
            let uid = std::fs::metadata(self.sys.path(format!("/proc/{pid}"))).map(|m| m.uid()).unwrap_or(0);
            let cpu = match (dt, self.last.get(&pid)) {
                (Some(dt), Some(&(start, prev))) if start == st.start => (st.ticks.saturating_sub(prev) as f32 / dt * 100.0).min(100.0),
                _ => 0.0,
            };
            seen.insert(pid, (st.start, st.ticks));
            let key = (pid, st.start);
            let cmdline = match self.cmdlines.remove(&key) {
                Some(c) => c,
                None => self.sys.read(format!("/proc/{pid}/cmdline")).map(|c| c.replace('\0', " ").trim().to_string()).unwrap_or_default(),
            };
            cmd_seen.insert(key, cmdline.clone());
            out.push(Process {
                pid,
                ppid: st.ppid,
                user: self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()),
                uid,
                name: st.name,
                cmdline,
                state: st.state,
                threads: st.threads,
                rss_kb: st.rss_kb,
                cpu_percent: cpu,
                age_secs: uptime.saturating_sub(st.start / hz),
            });
        }
        self.last = seen;
        self.cmdlines = cmd_seen;
        self.last_total = total;
        out
    }
}

/// Процессы, чьё имя (первое слово cmdline) — `name` или `name:…` (служебные процессы
/// Android-приложения: `org.app:remote`).
pub fn find_by_name(sys: &Sys, name: &str) -> Vec<i32> {
    sys.list("/proc")
        .iter()
        .filter_map(|p| p.parse::<i32>().ok())
        .filter(|p| {
            sys.read(format!("/proc/{p}/cmdline")).is_some_and(|c| {
                let arg0 = c.split('\0').next().unwrap_or("");
                arg0 == name || arg0.strip_prefix(name).is_some_and(|r| r.starts_with(':'))
            })
        })
        .collect()
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

    #[test]
    fn table() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("proc/42")).unwrap();
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        fs::write(dir.path().join("proc/stat"), "cpu  100 0 0 900 0 0 0 0\n").unwrap();
        fs::write(dir.path().join("proc/uptime"), "100.0 1.0\n").unwrap();
        fs::write(dir.path().join("proc/42/stat"), "42 (my app (x)) R 1 42 42 0 -1 4194560 100 0 0 0 70 30 0 0 20 0 4 0 500 1000 256 0").unwrap();
        fs::write(dir.path().join("proc/42/cmdline"), "/usr/bin/app\0--flag\0").unwrap();
        let sys = Sys::at(dir.path());
        let st = read_stat(&sys, 42).unwrap();
        assert_eq!((st.name.as_str(), st.state, st.ppid, st.ticks, st.threads, st.start), ("my app (x)", 'R', 1, 100, 4, 500));
        let mut t = ProcessTable::new(sys);
        let p = t.sample();
        assert_eq!(p[0].cmdline, "/usr/bin/app --flag");
        fs::write(dir.path().join("proc/stat"), "cpu  200 0 0 1000 0 0 0 0\n").unwrap();
        fs::write(dir.path().join("proc/42/stat"), "42 (my app (x)) R 1 42 42 0 -1 4194560 100 0 0 0 120 30 0 0 20 0 4 0 500 1000 256 0").unwrap();
        assert_eq!(t.sample()[0].cpu_percent, 25.0);
    }

    #[test]
    fn cgroup_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let cg = dir.path().join("sys/fs/cgroup/box");
        fs::create_dir_all(&cg).unwrap();
        fs::write(cg.join("memory.current"), "2097152\n").unwrap();
        fs::write(cg.join("cpu.stat"), "usage_usec 3000000\nuser_usec 1\n").unwrap();
        for (pid, cmd) in [(7, "org.app\0"), (8, "org.app:remote\0"), (9, "org.apple\0")] {
            let p = dir.path().join(format!("proc/{pid}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("cmdline"), cmd).unwrap();
        }
        let sys = Sys::at(dir.path());
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;
        assert_eq!(cgroup_usage(&sys, "/sys/fs/cgroup/box"), (2048, 3 * hz));
        let mut v = find_by_name(&sys, "org.app");
        v.sort();
        assert_eq!(v, vec![7, 8]);
    }
}
