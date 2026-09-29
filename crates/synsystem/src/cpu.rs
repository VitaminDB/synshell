//! Процессор: загрузка по ядрам (`/proc/stat`), частоты и кластеры
//! (`cpufreq/policy*`), модель.

use crate::Sys;

/// Одно ядро в снимке.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Core {
    pub index: usize,
    /// Загрузка с прошлого снимка, 0..100.
    pub usage: f32,
    pub cur_mhz: Option<u32>,
    pub min_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
    /// Номер кластера (политики cpufreq): ядра с общей частотой.
    pub cluster: usize,
    /// Класс ядра по максимальной частоте: 0 — самые медленные (LITTLE,
    /// E-ядра), дальше быстрее (big, prime, P-ядра). Для цвета на графике —
    /// у Intel каждое ядро своя политика, кластеры там ничего не говорят.
    pub tier: usize,
    pub online: bool,
}

/// Снимок процессора.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CpuSnapshot {
    /// Общая загрузка, 0..100.
    pub usage: f32,
    pub cores: Vec<Core>,
}

/// Кластер ядер с общей частотой (big.LITTLE).
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub policy: String,
    pub cpus: Vec<usize>,
    pub min_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
    pub governor: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Times {
    total: u64,
    idle: u64,
}

fn parse_times(line: &str) -> Option<Times> {
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let idle = v.get(3)? + v.get(4).unwrap_or(&0);
    // guest/guest_nice уже входят в user/nice.
    let total = v.iter().take(8).sum();
    Some(Times { total, idle })
}

/// Загрузка по разности двух снимков `/proc/stat`.
#[derive(Debug, Default)]
pub struct CpuSampler {
    sys: Sys,
    last_total: Option<Times>,
    last_cores: Vec<Option<Times>>,
    clusters: Vec<Cluster>,
}

impl CpuSampler {
    pub fn new(sys: Sys) -> Self {
        let clusters = clusters(&sys);
        Self { sys, last_total: None, last_cores: Vec::new(), clusters }
    }

    pub fn clusters(&self) -> &[Cluster] {
        &self.clusters
    }

    /// Снимок; первый вызов даёт нулевую загрузку (не с чем сравнить).
    pub fn sample(&mut self) -> CpuSnapshot {
        let stat = self.sys.read("/proc/stat").unwrap_or_default();
        let usage_of = |prev: Option<Times>, now: Times| -> f32 {
            let Some(p) = prev else { return 0.0 };
            let dt = now.total.saturating_sub(p.total).max(1) as f32;
            let di = now.idle.saturating_sub(p.idle) as f32;
            ((1.0 - di / dt) * 100.0).clamp(0.0, 100.0)
        };
        let mut snap = CpuSnapshot::default();
        let mut per_core: Vec<(usize, Times)> = Vec::new();
        for line in stat.lines() {
            if let Some(rest) = line.strip_prefix("cpu") {
                // «cpu  …» — сумма, «cpu3 …» — ядро (номер сразу за «cpu»).
                let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                let Some(t) = parse_times(line) else { continue };
                if id.is_empty() {
                    snap.usage = usage_of(self.last_total, t);
                    self.last_total = Some(t);
                } else if let Ok(i) = id.parse::<usize>() {
                    per_core.push((i, t));
                }
            }
        }
        let n = self.possible_cores().max(per_core.iter().map(|(i, _)| i + 1).max().unwrap_or(0));
        if self.last_cores.len() < n {
            self.last_cores.resize(n, None);
        }
        for i in 0..n {
            // Выключенного ядра нет в /proc/stat.
            let now = per_core.iter().find(|(c, _)| *c == i).map(|(_, t)| *t);
            let usage = now.map(|t| usage_of(self.last_cores[i], t)).unwrap_or(0.0);
            self.last_cores[i] = now;
            let base = format!("/sys/devices/system/cpu/cpu{i}/cpufreq");
            let khz = |f: &str| self.sys.read_num::<u32>(format!("{base}/{f}")).map(|k| k / 1000);
            let cluster = self.clusters.iter().position(|c| c.cpus.contains(&i)).unwrap_or(0);
            let cl = self.clusters.get(cluster);
            snap.cores.push(Core {
                index: i,
                usage,
                cur_mhz: khz("scaling_cur_freq"),
                min_mhz: khz("cpuinfo_min_freq").or(cl.and_then(|c| c.min_mhz)),
                max_mhz: khz("cpuinfo_max_freq").or(cl.and_then(|c| c.max_mhz)),
                cluster,
                tier: 0,
                online: now.is_some(),
            });
        }
        let mut maxes: Vec<u32> = snap.cores.iter().filter_map(|c| c.max_mhz).collect();
        maxes.sort_unstable();
        maxes.dedup();
        for c in &mut snap.cores {
            c.tier = c.max_mhz.and_then(|m| maxes.iter().position(|x| *x == m)).unwrap_or(0);
        }
        snap
    }

    fn possible_cores(&self) -> usize {
        // «0-7» → 8.
        self.sys
            .read("/sys/devices/system/cpu/possible")
            .and_then(|s| parse_cpu_list(&s).into_iter().max())
            .map(|m| m + 1)
            .unwrap_or(0)
    }
}

/// «0-3,7» → [0,1,2,3,7]; также «0 1 2 3».
pub fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in s.split([',', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.parse::<usize>().ok()),
        }
    }
    out
}

/// Кластеры по политикам cpufreq.
pub fn clusters(sys: &Sys) -> Vec<Cluster> {
    let base = "/sys/devices/system/cpu/cpufreq";
    sys.list(base)
        .into_iter()
        .filter(|p| p.starts_with("policy"))
        .map(|p| {
            let f = |k: &str| format!("{base}/{p}/{k}");
            Cluster {
                cpus: sys.read(f("related_cpus")).map(|s| parse_cpu_list(&s)).unwrap_or_default(),
                min_mhz: sys.read_num::<u32>(f("cpuinfo_min_freq")).map(|k| k / 1000),
                max_mhz: sys.read_num::<u32>(f("cpuinfo_max_freq")).map(|k| k / 1000),
                governor: sys.read(f("scaling_governor")),
                policy: p,
            }
        })
        .collect()
}

/// Модель процессора: x86 — `model name`; ARM — `Hardware` или модель
/// устройства из device tree (`/proc/cpuinfo` без названия).
pub fn model(sys: &Sys) -> Option<String> {
    let info = sys.read("/proc/cpuinfo").unwrap_or_default();
    let field = |k: &str| {
        info.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    field("model name").or_else(|| field("Hardware")).or_else(|| {
        let dt = sys.read("/proc/device-tree/model")?;
        let dt = dt.trim_end_matches('\0').trim();
        // «Diting based on Qualcomm Technologies, Inc SM8475» → SoC.
        Some(dt.rsplit_once(" based on ").map(|(_, soc)| soc.to_string()).unwrap_or_else(|| dt.to_string()))
    })
}

/// Модель устройства (device tree), если есть.
pub fn device_model(sys: &Sys) -> Option<String> {
    sys.read("/proc/device-tree/model").map(|s| s.trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn phone() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("proc/device-tree")).unwrap();
        fs::write(r.join("proc/device-tree/model"), "Diting based on Qualcomm Technologies, Inc SM8475\0").unwrap();
        fs::write(r.join("proc/cpuinfo"), "processor\t: 0\nCPU part\t: 0xd46\n").unwrap();
        fs::create_dir_all(r.join("sys/devices/system/cpu")).unwrap();
        fs::write(r.join("sys/devices/system/cpu/possible"), "0-7\n").unwrap();
        for (p, cpus, max) in [("policy0", "0 1 2 3", 2016000), ("policy4", "4 5 6", 2745600), ("policy7", "7", 3187200)] {
            let d = r.join("sys/devices/system/cpu/cpufreq").join(p);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("related_cpus"), cpus).unwrap();
            fs::write(d.join("cpuinfo_max_freq"), max.to_string()).unwrap();
            fs::write(d.join("cpuinfo_min_freq"), "300000").unwrap();
            fs::write(d.join("scaling_governor"), "performance").unwrap();
            for c in parse_cpu_list(cpus) {
                let cd = r.join(format!("sys/devices/system/cpu/cpu{c}/cpufreq"));
                fs::create_dir_all(&cd).unwrap();
                fs::write(cd.join("scaling_cur_freq"), max.to_string()).unwrap();
            }
        }
        dir
    }

    fn stat(r: &std::path::Path, busy: u64, idle: u64) {
        let mut s = format!("cpu  {} 0 0 {} 0 0 0 0 0 0\n", busy * 8, idle * 8);
        for i in 0..8 {
            s += &format!("cpu{i} {busy} 0 0 {idle} 0 0 0 0 0 0\n");
        }
        fs::write(r.join("proc/stat"), s).unwrap();
    }

    #[test]
    fn cores_clusters_usage() {
        let dir = phone();
        stat(dir.path(), 100, 900);
        let mut s = CpuSampler::new(Sys::at(dir.path()));
        assert_eq!(s.clusters().len(), 3);
        let first = s.sample();
        assert_eq!(first.cores.len(), 8);
        assert_eq!(first.usage, 0.0);
        stat(dir.path(), 150, 950);
        let snap = s.sample();
        assert!((snap.usage - 50.0).abs() < 0.1, "{}", snap.usage);
        assert_eq!(snap.cores[7].cluster, 2);
        assert_eq!((snap.cores[0].tier, snap.cores[4].tier, snap.cores[7].tier), (0, 1, 2));
        assert_eq!(snap.cores[7].max_mhz, Some(3187));
        assert_eq!(snap.cores[5].cur_mhz, Some(2745));
    }

    #[test]
    fn arm_model() {
        let dir = phone();
        assert_eq!(model(&Sys::at(dir.path())).as_deref(), Some("Qualcomm Technologies, Inc SM8475"));
        assert_eq!(parse_cpu_list("0-2,7"), [0, 1, 2, 7]);
    }
}
