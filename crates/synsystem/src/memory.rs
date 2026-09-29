//! Память (`/proc/meminfo`).

use crate::Sys;

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

/// «1.5 ГБ» из килобайт.
pub fn human_kb(kb: u64) -> String {
    let mb = kb as f64 / 1024.0;
    if mb >= 1024.0 {
        format!("{:.1} ГБ", mb / 1024.0)
    } else {
        format!("{mb:.0} МБ")
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
    }
}
