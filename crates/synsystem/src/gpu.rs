//! Графический процессор: частота и загрузка. Qualcomm KGSL
//! (`/sys/class/kgsl/kgsl-3d0`), AMD (`gpu_busy_percent`), Intel (`gt_cur_freq_mhz`),
//! mainline-GPU через devfreq (`/sys/class/devfreq/<…gpu>`: drm/msm Adreno, panfrost Mali; нагрузка — атрибут `load`
//! из патча ядра synmobile, без него — только частота).

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
    devfreq_gpu(sys)
}

/// GPU с devfreq (mainline): узел `/sys/class/devfreq/<имя>`, имя которого содержит «gpu».
fn devfreq_gpu(sys: &Sys) -> Option<Gpu> {
    let base = "/sys/class/devfreq";
    let dev = sys.list(base).into_iter().find(|n| n.contains("gpu"))?;
    let d = format!("{base}/{dev}");
    let mhz = |f: &str| sys.read_num::<u64>(format!("{d}/{f}")).map(|v| (v / 1_000_000) as u32);
    Some(Gpu {
        name: gpu_name(sys.read(format!("{d}/device/of_node/compatible")).as_deref()),
        busy_percent: sys.read_num::<f32>(format!("{d}/load")),
        cur_mhz: mhz("cur_freq"),
        max_mhz: mhz("max_freq"),
    })
}

/// Имя по первой строке compatible: «qcom,adreno-505.0» → «Adreno 505», «arm,mali-bifrost» → «Mali Bifrost».
fn gpu_name(compatible: Option<&str>) -> Option<String> {
    let first = compatible?.split('\0').next()?.trim();
    let model = first.split_once(',').map_or(first, |(_, m)| m);
    if let Some(n) = model.strip_prefix("adreno-") {
        return Some(format!("Adreno {}", n.split('.').next().unwrap_or(n)));
    }
    if let Some(n) = model.strip_prefix("mali-") {
        let mut c = n.chars();
        let n = c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default();
        return Some(format!("Mali {n}"));
    }
    (!model.is_empty()).then(|| model.to_string())
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

    #[test]
    fn devfreq_adreno() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("sys/class/devfreq/1c00000.gpu");
        std::fs::create_dir_all(d.join("device/of_node")).unwrap();
        std::fs::write(d.join("device/of_node/compatible"), "qcom,adreno-505.0\0qcom,adreno\0").unwrap();
        std::fs::write(d.join("cur_freq"), "400000000\n").unwrap();
        std::fs::write(d.join("max_freq"), "450000000\n").unwrap();
        std::fs::write(d.join("load"), "37\n").unwrap();
        let g = read(&Sys::at(dir.path())).unwrap();
        assert_eq!(g.name.as_deref(), Some("Adreno 505"));
        assert_eq!(g.busy_percent, Some(37.0));
        assert_eq!((g.cur_mhz, g.max_mhz), (Some(400), Some(450)));
    }
}
