//! Нейропроцессор (NPU). Qualcomm — HTP внутри вычислительного DSP: состояние CDSP
//! (`/sys/class/remoteproc`, имя `…cdsp`), температура NSP (зоны `nspss-*`), демон synnpu-qnn
//! (рабочие процессы — по одному PD на CDSP) и кэш графов `/var/cache/synnpu`; загрузку HTP ядро
//! не сообщает. Intel и прочие на `accel` (`/sys/class/accel/accel*/device`): занятость по
//! `npu_busy_time_us`, частота, память.

use crate::Sys;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Npu {
    pub name: String,
    /// `qualcomm`, `intel`, `accel` (неизвестный драйвер accel).
    pub vendor: String,
    /// Включён ли сопроцессор (у Qualcomm — CDSP запущен).
    pub online: bool,
    /// Состояние как его даёт ядро (`running`, `offline`, `D0`…).
    pub state: Option<String>,
    pub busy_percent: Option<f32>,
    pub cur_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
    /// Память под задачи NPU, байт (Intel — `npu_memory_utilization`, Qualcomm — память демона).
    pub mem_bytes: Option<u64>,
    pub temp_c: Option<f32>,
    pub daemon: Option<Daemon>,
}

/// Демон synnpu-qnn: главный процесс и рабочие (по соединению, у каждого свой PD на CDSP).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Daemon {
    pub pid: i32,
    pub workers: Vec<i32>,
    /// RSS всех процессов демона, КБ.
    pub rss_kb: u64,
}

/// Счётчик занятости: разность `npu_busy_time_us` между снимками.
#[derive(Debug, Default)]
pub struct NpuSampler {
    sys: Sys,
    last: Option<(u64, std::time::Instant)>,
}

impl NpuSampler {
    pub fn new(sys: Sys) -> Self {
        Self { sys, last: None }
    }

    pub fn sample(&mut self) -> Option<Npu> {
        if let Some(n) = qualcomm(&self.sys) {
            return Some(n);
        }
        let (mut n, busy_us) = accel(&self.sys)?;
        let now = std::time::Instant::now();
        if let Some(b) = busy_us {
            if let Some((b0, t0)) = self.last {
                let dt = now.duration_since(t0).as_micros().max(1) as f64;
                n.busy_percent = Some((b.saturating_sub(b0) as f64 / dt * 100.0).clamp(0.0, 100.0) as f32);
            }
            self.last = Some((b, now));
        }
        Some(n)
    }
}

fn qualcomm(sys: &Sys) -> Option<Npu> {
    let base = "/sys/class/remoteproc";
    let rp = sys.list(base).into_iter().find(|r| sys.read(format!("{base}/{r}/name")).is_some_and(|n| n.contains("cdsp")))?;
    let state = sys.read(format!("{base}/{rp}/state"));
    let temps = crate::thermal::sensors_matching(sys, |n| n.starts_with("nspss") || n.starts_with("nsp-"));
    let daemon = daemon(sys);
    Some(Npu {
        name: "Qualcomm Hexagon NPU".into(),
        vendor: "qualcomm".into(),
        online: state.as_deref() == Some("running"),
        mem_bytes: daemon.as_ref().map(|d| d.rss_kb * 1024),
        state,
        temp_c: temps.into_iter().map(|s| s.celsius).reduce(f32::max),
        daemon,
        ..Default::default()
    })
}

/// Процессы synnpu-qnn: главный — тот, чей родитель не synnpu-qnn.
fn daemon(sys: &Sys) -> Option<Daemon> {
    let mut procs: Vec<(i32, i32, u64)> = Vec::new();
    for p in sys.list("/proc").into_iter().filter_map(|p| p.parse::<i32>().ok()) {
        if sys.read(format!("/proc/{p}/comm")).as_deref() != Some("synnpu-qnn") {
            continue;
        }
        let Some(st) = crate::procs::read_stat(sys, p) else { continue };
        procs.push((p, st.ppid, st.rss_kb));
    }
    let main = procs.iter().find(|(_, ppid, _)| !procs.iter().any(|(q, _, _)| q == ppid))?.0;
    Some(Daemon {
        pid: main,
        workers: procs.iter().filter(|(p, _, _)| *p != main).map(|(p, _, _)| *p).collect(),
        rss_kb: procs.iter().map(|(_, _, r)| r).sum(),
    })
}

fn accel(sys: &Sys) -> Option<(Npu, Option<u64>)> {
    let base = "/sys/class/accel";
    let a = sys.list(base).into_iter().next()?;
    let d = format!("{base}/{a}/device");
    let driver = std::fs::read_link(sys.path(format!("{d}/driver"))).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let intel = driver == "intel_vpu" || sys.path(format!("{d}/npu_busy_time_us")).exists();
    let state = sys.read(format!("{d}/power_state")).or_else(|| sys.read(format!("{d}/power/runtime_status")));
    let n = Npu {
        name: if intel { "Intel NPU".into() } else if driver.is_empty() { a.clone() } else { driver.clone() },
        vendor: if intel { "intel" } else { "accel" }.into(),
        // D0 — включён; runtime PM усыпляет простаивающий NPU (D3hot).
        online: state.as_deref().is_none_or(|s| s == "D0" || s == "active"),
        state,
        cur_mhz: sys.read_num(format!("{d}/npu_current_frequency_mhz")),
        max_mhz: sys.read_num(format!("{d}/npu_max_frequency_mhz")),
        mem_bytes: sys.read_num(format!("{d}/npu_memory_utilization")),
        ..Default::default()
    };
    Some((n, sys.read_num(format!("{d}/npu_busy_time_us"))))
}

/// Размер каталога (кэш графов NPU), байт.
pub fn dir_size(sys: &Sys, dir: &str) -> u64 {
    fn walk(p: &std::path::Path, depth: u32) -> u64 {
        let Ok(rd) = std::fs::read_dir(p) else { return 0 };
        rd.flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() && depth < 6 => walk(&e.path(), depth + 1),
                Ok(m) if m.is_file() => m.len(),
                _ => 0,
            })
            .sum()
    }
    walk(&sys.path(dir), 0)
}

/// Кэш скомпилированных графов synnpu.
pub const GRAPH_CACHE: &str = "/var/cache/synnpu";

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn qualcomm_cdsp() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let rp = r.join("sys/class/remoteproc/remoteproc1");
        fs::create_dir_all(&rp).unwrap();
        fs::write(rp.join("name"), "32300000.remoteproc-cdsp").unwrap();
        fs::write(rp.join("state"), "running").unwrap();
        let z = r.join("sys/class/thermal/thermal_zone7");
        fs::create_dir_all(&z).unwrap();
        fs::write(z.join("type"), "nspss-1").unwrap();
        fs::write(z.join("temp"), "45300").unwrap();
        for (pid, ppid) in [(10, 1), (11, 10), (12, 10)] {
            let p = r.join(format!("proc/{pid}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("comm"), "synnpu-qnn").unwrap();
            fs::write(p.join("stat"), format!("{pid} (synnpu-qnn) S {ppid} 1 1 0 -1 0 0 0 0 0 5 5 0 0 20 0 3 0 100 1000000 256 0")).unwrap();
        }
        let n = NpuSampler::new(Sys::at(r)).sample().unwrap();
        assert!(n.online);
        assert_eq!(n.temp_c, Some(45.3));
        let d = n.daemon.unwrap();
        assert_eq!((d.pid, d.workers.len()), (10, 2));
    }

    #[test]
    fn intel_busy() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("sys/class/accel/accel0/device");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("npu_busy_time_us"), "1000").unwrap();
        fs::write(d.join("npu_max_frequency_mhz"), "1600").unwrap();
        fs::write(d.join("power_state"), "D3hot").unwrap();
        let mut s = NpuSampler::new(Sys::at(dir.path()));
        let n = s.sample().unwrap();
        assert_eq!((n.vendor.as_str(), n.online, n.max_mhz, n.busy_percent), ("intel", false, Some(1600), None));
        fs::write(d.join("npu_busy_time_us"), "1000000000").unwrap();
        assert_eq!(s.sample().unwrap().busy_percent, Some(100.0));
    }
}
