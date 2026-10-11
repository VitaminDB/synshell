//! Память (`/proc/meminfo`).

use crate::Sys;
use synshell_tr::t;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Memory {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl Memory {
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }

    /// Занято, 0..100.
    pub fn used_percent(&self) -> f32 {
        if self.total_kb == 0 {
            return 0.0;
        }
        self.used_kb() as f32 / self.total_kb as f32 * 100.0
    }

    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
}

pub fn read(sys: &Sys) -> Option<Memory> {
    let s = sys.read("/proc/meminfo")?;
    let get = |k: &str| -> Option<u64> { s.lines().find(|l| l.starts_with(k))?.split_whitespace().nth(1)?.parse().ok() };
    Some(Memory {
        total_kb: get("MemTotal:")?,
        available_kb: get("MemAvailable:").or_else(|| get("MemFree:"))?,
        swap_total_kb: get("SwapTotal:").unwrap_or(0),
        swap_free_kb: get("SwapFree:").unwrap_or(0),
    })
}

/// Подробности `/proc/meminfo`, КБ (нет поля — 0).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Detail {
    pub total: u64,
    pub available: u64,
    pub free: u64,
    pub buffers: u64,
    pub cached: u64,
    pub shmem: u64,
    pub anon: u64,
    pub slab: u64,
    pub slab_reclaimable: u64,
    pub page_tables: u64,
    pub kernel_stack: u64,
    pub dirty: u64,
    pub writeback: u64,
    pub mapped: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub swap_cached: u64,
    pub cma_total: u64,
    pub cma_free: u64,
    pub huge: u64,
}

impl Detail {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }

    /// Кэш файлов, который ядро отдаст по первому требованию.
    pub fn cache(&self) -> u64 {
        (self.cached + self.buffers + self.slab_reclaimable).saturating_sub(self.shmem)
    }

    /// Ядро: slab без освобождаемой части, таблицы страниц, стеки.
    pub fn kernel(&self) -> u64 {
        self.slab.saturating_sub(self.slab_reclaimable) + self.page_tables + self.kernel_stack
    }
}

pub fn detail(sys: &Sys) -> Option<Detail> {
    let s = sys.read("/proc/meminfo")?;
    let mut d = Detail::default();
    for line in s.lines() {
        let Some((k, rest)) = line.split_once(':') else { continue };
        let v: u64 = rest.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let f = match k {
            "MemTotal" => &mut d.total,
            "MemAvailable" => &mut d.available,
            "MemFree" => &mut d.free,
            "Buffers" => &mut d.buffers,
            "Cached" => &mut d.cached,
            "Shmem" => &mut d.shmem,
            "AnonPages" => &mut d.anon,
            "Slab" => &mut d.slab,
            "SReclaimable" => &mut d.slab_reclaimable,
            "PageTables" => &mut d.page_tables,
            "KernelStack" => &mut d.kernel_stack,
            "Dirty" => &mut d.dirty,
            "Writeback" => &mut d.writeback,
            "Mapped" => &mut d.mapped,
            "SwapTotal" => &mut d.swap_total,
            "SwapFree" => &mut d.swap_free,
            "SwapCached" => &mut d.swap_cached,
            "CmaTotal" => &mut d.cma_total,
            "CmaFree" => &mut d.cma_free,
            "Hugetlb" => &mut d.huge,
            _ => continue,
        };
        *f = v;
    }
    if d.available == 0 {
        d.available = d.free + d.cache();
    }
    (d.total > 0).then_some(d)
}

/// Устройства подкачки (`/proc/swaps`): путь, тип (`partition`, `file`), размер и занято, КБ.
/// zram — тип `partition` с путём `/dev/zram*`.
pub fn swaps(sys: &Sys) -> Vec<(String, String, u64, u64)> {
    sys.read("/proc/swaps")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((f.first()?.to_string(), f.get(1)?.to_string(), f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?))
        })
        .collect()
}

/// Нехватка ресурса по PSI (`/proc/pressure/{cpu,memory,io}`): доля времени за 10 с, когда
/// хотя бы одна задача ждала ресурс, %.
pub fn pressure(sys: &Sys, resource: &str) -> Option<f32> {
    let s = sys.read(format!("/proc/pressure/{resource}"))?;
    let line = s.lines().find(|l| l.starts_with("some"))?;
    line.split_whitespace().find_map(|x| x.strip_prefix("avg10="))?.parse().ok()
}

/// «1.5 ГБ» из килобайт.
pub fn human_kb(kb: u64) -> String {
    let mb = kb as f64 / 1024.0;
    if mb >= 1024.0 {
        t!("{v} ГБ", v = format!("{:.1}", mb / 1024.0))
    } else {
        t!("{mb} МБ", mb = format!("{:.0}", mb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("proc")).unwrap();
        std::fs::write(
            dir.path().join("proc/meminfo"),
            "MemTotal:       11724252 kB\nMemFree:  100 kB\nMemAvailable:    5862126 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        )
        .unwrap();
        let m = read(&Sys::at(dir.path())).unwrap();
        assert!((m.used_percent() - 50.0).abs() < 0.01);
        assert_eq!(human_kb(2 * 1024 * 1024), "2.0 ГБ");
        assert_eq!(human_kb(512 * 1024), "512 МБ");
        let d = detail(&Sys::at(dir.path())).unwrap();
        assert_eq!((d.total, d.available, d.free), (11724252, 5862126, 100));
    }

    #[test]
    fn swaps_and_pressure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("proc/pressure")).unwrap();
        std::fs::write(dir.path().join("proc/swaps"), "Filename Type Size Used Priority\n/dev/zram0 partition 11426812 939008 100\n").unwrap();
        std::fs::write(dir.path().join("proc/pressure/memory"), "some avg10=1.50 avg60=0.91 avg300=0.92 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").unwrap();
        let sys = Sys::at(dir.path());
        assert_eq!(swaps(&sys), vec![("/dev/zram0".to_string(), "partition".to_string(), 11426812, 939008)]);
        assert_eq!(pressure(&sys, "memory"), Some(1.5));
    }
}
