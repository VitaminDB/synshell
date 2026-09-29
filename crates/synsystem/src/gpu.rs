//! Графический процессор: частота и загрузка. Qualcomm KGSL
//! (`/sys/class/kgsl/kgsl-3d0`), AMD (`gpu_busy_percent`), Intel (`gt_cur_freq_mhz`).

use crate::Sys;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gpu {
    pub name: Option<String>,
    pub busy_percent: Option<f32>,
    pub cur_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
}

pub fn read(sys: &Sys) -> Option<Gpu> {
    let k = "/sys/class/kgsl/kgsl-3d0";
    if sys.path(k).exists() {
        let hz = |f: &str| sys.read_num::<u64>(format!("{k}/{f}")).map(|v| (v / 1_000_000) as u32);
        return Some(Gpu {
            name: sys.read(format!("{k}/gpu_model")),
            // «3 %».
            busy_percent: sys.read(format!("{k}/gpu_busy_percentage")).and_then(|s| s.trim_end_matches('%').trim().parse().ok()),
            cur_mhz: hz("gpuclk"),
            max_mhz: hz("max_gpuclk"),
        });
    }
    for card in sys.list("/sys/class/drm").into_iter().filter(|c| c.starts_with("card") && !c.contains('-')) {
        let d = format!("/sys/class/drm/{card}/device");
        if let Some(busy) = sys.read_num::<f32>(format!("{d}/gpu_busy_percent")) {
            // amdgpu: pp_dpm_sclk — строка с «*» текущая.
            let cur = sys.read(format!("{d}/pp_dpm_sclk")).and_then(|s| {
                s.lines().find(|l| l.ends_with('*')).and_then(|l| l.split_whitespace().nth(1)?.trim_end_matches("Mhz").parse().ok())
            });
            return Some(Gpu { name: Some("AMD".into()), busy_percent: Some(busy), cur_mhz: cur, max_mhz: None });
        }
        let g = format!("/sys/class/drm/{card}");
        if let Some(cur) = sys.read_num::<u32>(format!("{g}/gt_cur_freq_mhz")) {
            return Some(Gpu { name: Some("Intel".into()), busy_percent: None, cur_mhz: Some(cur), max_mhz: sys.read_num(format!("{g}/gt_max_freq_mhz")) });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kgsl() {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path().join("sys/class/kgsl/kgsl-3d0");
        std::fs::create_dir_all(&k).unwrap();
        std::fs::write(k.join("gpu_model"), "Adreno730v3").unwrap();
        std::fs::write(k.join("gpu_busy_percentage"), "3 %").unwrap();
        std::fs::write(k.join("gpuclk"), "285000000").unwrap();
        std::fs::write(k.join("max_gpuclk"), "900000000").unwrap();
        let g = read(&Sys::at(dir.path())).unwrap();
        assert_eq!(g.busy_percent, Some(3.0));
        assert_eq!(g.cur_mhz, Some(285));
        assert_eq!(g.max_mhz, Some(900));
    }
}
