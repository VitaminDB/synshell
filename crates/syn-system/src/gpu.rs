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

/// Видеокарта подробно — для экрана ресурсов (все GPU системы: встроенная и дискретная).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuDevice {
    pub name: String,
    /// Драйвер ядра: `kgsl`, `msm`, `amdgpu`, `i915`, `xe`, `nvidia`, …
    pub driver: String,
    pub busy_percent: Option<f32>,
    pub cur_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
    /// Видеопамять занято / всего, байт. У мобильных GPU память общая с системной.
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub shared_memory: bool,
    pub temp_c: Option<f32>,
    pub power_w: Option<f32>,
    /// Дискретная карта спит (runtime PM) — её не будили ради цифр.
    pub asleep: bool,
}

/// Все GPU. Спящую дискретную NVIDIA не будит: `nvidia-smi` зовётся, только когда она активна.
pub fn devices(sys: &Sys) -> Vec<GpuDevice> {
    let mut out = Vec::new();
    let k = "/sys/class/kgsl/kgsl-3d0";
    if sys.path(k).exists() {
        let hz = |f: &str| sys.read_num::<u64>(format!("{k}/{f}")).map(|v| (v / 1_000_000) as u32);
        let model = sys.read(format!("{k}/gpu_model")).unwrap_or_else(|| "Adreno".into());
        out.push(GpuDevice {
            // «Adreno730v3» → «Adreno 730».
            name: model.strip_prefix("Adreno").map(|n| format!("Adreno {}", n.split('v').next().unwrap_or(n))).unwrap_or(model),
            driver: "kgsl".into(),
            busy_percent: sys.read(format!("{k}/gpu_busy_percentage")).and_then(|s| s.trim_end_matches('%').trim().parse().ok()),
            cur_mhz: hz("gpuclk"),
            max_mhz: hz("max_gpuclk"),
            mem_used: sys.read_num("/sys/class/kgsl/kgsl/page_alloc"),
            mem_total: None,
            shared_memory: true,
            temp_c: sys.read_num::<f32>(format!("{k}/temp")).map(|t| if t > 1000.0 { t / 1000.0 } else { t }),
            ..Default::default()
        });
        return out;
    }
    for card in sys.list("/sys/class/drm").into_iter().filter(|c| c.starts_with("card") && !c.contains('-')) {
        let d = format!("/sys/class/drm/{card}/device");
        let driver = std::fs::read_link(sys.path(format!("{d}/driver")))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let asleep = sys.read(format!("{d}/power/runtime_status")).as_deref() == Some("suspended");
        let slot = sys.read(format!("{d}/uevent")).and_then(|u| u.lines().find_map(|l| l.strip_prefix("PCI_SLOT_NAME=").map(String::from)));
        let mut g = GpuDevice { driver: driver.clone(), asleep, ..Default::default() };
        match driver.as_str() {
            "nvidia" => {
                g.name = slot.as_deref().and_then(pci_name).unwrap_or_else(|| "NVIDIA".into());
                if !asleep {
                    if let Some(n) = nvidia_smi(slot.as_deref()) {
                        g = GpuDevice { driver: g.driver, asleep: false, ..n };
                    }
                }
            }
            "amdgpu" => {
                g.name = slot.as_deref().and_then(pci_name).unwrap_or_else(|| "AMD Radeon".into());
                g.busy_percent = sys.read_num(format!("{d}/gpu_busy_percent"));
                g.cur_mhz = sys.read(format!("{d}/pp_dpm_sclk")).and_then(|s| {
                    s.lines().find(|l| l.ends_with('*')).and_then(|l| l.split_whitespace().nth(1)?.trim_end_matches("Mhz").parse().ok())
                });
                g.mem_used = sys.read_num(format!("{d}/mem_info_vram_used"));
                g.mem_total = sys.read_num(format!("{d}/mem_info_vram_total"));
                // Встроенная Radeon: «видеопамять» — кусок системной.
                g.shared_memory = g.mem_total.is_some_and(|t| t <= 2 << 30);
                let hw = format!("{d}/hwmon");
                if let Some(h) = sys.list(&hw).into_iter().next() {
                    g.temp_c = sys.read_num::<f32>(format!("{hw}/{h}/temp1_input")).map(|t| t / 1000.0);
                    g.power_w = sys.read_num::<f32>(format!("{hw}/{h}/power1_average")).map(|p| p / 1e6);
                }
            }
            "i915" | "xe" => {
                g.name = slot.as_deref().and_then(pci_name).unwrap_or_else(|| "Intel Graphics".into());
                let c = format!("/sys/class/drm/{card}");
                g.cur_mhz = sys.read_num(format!("{c}/gt_act_freq_mhz")).filter(|m: &u32| *m > 0).or_else(|| sys.read_num(format!("{c}/gt_cur_freq_mhz")));
                g.max_mhz = sys.read_num(format!("{c}/gt_max_freq_mhz"));
                g.shared_memory = true;
            }
            _ => {
                // Встроенные в SoC (msm, panfrost, …) — частота и нагрузка через devfreq.
                if let Some(df) = devfreq_gpu(sys) {
                    g.name = df.name.unwrap_or_else(|| driver.clone());
                    g.busy_percent = df.busy_percent;
                    g.cur_mhz = df.cur_mhz;
                    g.max_mhz = df.max_mhz;
                    g.shared_memory = true;
                } else {
                    continue;
                }
            }
        }
        if !out.iter().any(|x: &GpuDevice| x.name == g.name && x.driver == g.driver) {
            out.push(g);
        }
    }
    out
}

/// Название PCI-устройства по `lspci -mm` (кэш на время жизни процесса).
fn pci_name(slot: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().ok()?.get(slot) {
        return v.clone();
    }
    // `00:02.0 "VGA compatible controller" "Intel Corporation" "Arrow Lake-S [Intel Graphics]" …`
    let name = crate::util::output("lspci", &["-mm", "-s", slot]).and_then(|s| {
        let q: Vec<&str> = s.split('"').collect();
        let vendor = q.get(3)?.split_whitespace().next()?.to_string();
        let dev = q.get(5)?.to_string();
        // Маркетинговое имя — в квадратных скобках.
        let dev = dev.rsplit_once('[').and_then(|(_, r)| r.strip_suffix(']')).map(String::from).unwrap_or(dev);
        Some(if dev.starts_with(&vendor) { dev } else { format!("{vendor} {dev}") })
    });
    cache.lock().ok()?.insert(slot.to_string(), name.clone());
    name
}

fn nvidia_smi(slot: Option<&str>) -> Option<GpuDevice> {
    let q = "name,utilization.gpu,clocks.gr,clocks.max.gr,memory.used,memory.total,temperature.gpu,power.draw,pci.bus_id";
    let out = crate::util::output("nvidia-smi", &[&format!("--query-gpu={q}"), "--format=csv,noheader,nounits"])?;
    let line = out
        .lines()
        .find(|l| slot.is_none_or(|s| l.to_ascii_lowercase().ends_with(&s.to_ascii_lowercase())))
        .or_else(|| out.lines().next())?;
    let f: Vec<&str> = line.split(',').map(str::trim).collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    Some(GpuDevice {
        name: f.first()?.to_string(),
        busy_percent: num(1).map(|v| v as f32),
        cur_mhz: num(2).map(|v| v as u32),
        max_mhz: num(3).map(|v| v as u32),
        mem_used: num(4).map(|v| (v * 1048576.0) as u64),
        mem_total: num(5).map(|v| (v * 1048576.0) as u64),
        temp_c: num(6).map(|v| v as f32),
        power_w: num(7).map(|v| v as f32),
        ..Default::default()
    })
}

/// Видеопамять по процессам, байт: pid → занято. KGSL — `/sys/class/kgsl/kgsl/proc/<pid>/kernel`
/// (выделено драйвером под процесс, без импортированных буферов); остальные — `drm-total-*` /
/// `drm-memory-*` из fdinfo открытых `/dev/dri/*` (по разу на клиента DRM).
pub fn process_memory(sys: &Sys) -> Vec<(i32, u64)> {
    let kp = "/sys/class/kgsl/kgsl/proc";
    if sys.path(kp).exists() {
        let mut v: Vec<(i32, u64)> = sys
            .list(kp)
            .into_iter()
            .filter_map(|p| Some((p.parse().ok()?, sys.read_num::<u64>(format!("{kp}/{p}/kernel"))? + sys.read_num::<u64>(format!("{kp}/{p}/user")).unwrap_or(0))))
            .filter(|(_, b)| *b > 0)
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        return v;
    }
    let mut v = Vec::new();
    for pid in sys.list("/proc").into_iter().filter_map(|p| p.parse::<i32>().ok()) {
        let fd_dir = format!("/proc/{pid}/fd");
        let mut clients = std::collections::HashSet::new();
        let mut total = 0u64;
        for fd in sys.list(&fd_dir) {
            let Ok(target) = std::fs::read_link(sys.path(format!("{fd_dir}/{fd}"))) else { continue };
            if !target.starts_with("/dev/dri/") {
                continue;
            }
            let Some(info) = sys.read(format!("/proc/{pid}/fdinfo/{fd}")) else { continue };
            let client = info.lines().find_map(|l| l.strip_prefix("drm-client-id:")).map(|s| s.trim().to_string());
            if let Some(c) = client {
                if !clients.insert(c) {
                    continue;
                }
            }
            total += drm_fdinfo_bytes(&info);
        }
        if total > 0 {
            v.push((pid, total));
        }
    }
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// Сумма памяти клиента DRM из fdinfo: `drm-total-<регион>` (новые ядра) или `drm-memory-<регион>`
/// (amdgpu), «123 KiB».
fn drm_fdinfo_bytes(info: &str) -> u64 {
    let has_total = info.lines().any(|l| l.starts_with("drm-total-"));
    let prefix = if has_total { "drm-total-" } else { "drm-memory-" };
    info.lines()
        .filter(|l| l.starts_with(prefix))
        .filter_map(|l| {
            let mut it = l.split_once(':')?.1.split_whitespace();
            let n: u64 = it.next()?.parse().ok()?;
            Some(match it.next().unwrap_or("") {
                "KiB" => n << 10,
                "MiB" => n << 20,
                "GiB" => n << 30,
                _ => n,
            })
        })
        .sum()
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
    fn kgsl_device_and_procs() {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path().join("sys/class/kgsl/kgsl-3d0");
        std::fs::create_dir_all(&k).unwrap();
        std::fs::write(k.join("gpu_model"), "Adreno730v3").unwrap();
        std::fs::write(k.join("temp"), "49700").unwrap();
        let g = dir.path().join("sys/class/kgsl/kgsl");
        std::fs::create_dir_all(g.join("proc/42")).unwrap();
        std::fs::write(g.join("page_alloc"), "363028480").unwrap();
        std::fs::write(g.join("proc/42/kernel"), "46342144").unwrap();
        std::fs::write(g.join("proc/42/user"), "0").unwrap();
        let sys = Sys::at(dir.path());
        let d = devices(&sys);
        assert_eq!(d[0].name, "Adreno 730");
        assert_eq!(d[0].temp_c, Some(49.7));
        assert_eq!(d[0].mem_used, Some(363028480));
        assert_eq!(process_memory(&sys), vec![(42, 46342144)]);
    }

    #[test]
    fn fdinfo_sizes() {
        assert_eq!(drm_fdinfo_bytes("drm-driver: i915\ndrm-total-system0: 2 MiB\ndrm-resident-system0: 1 MiB\n"), 2 << 20);
        assert_eq!(drm_fdinfo_bytes("drm-memory-vram: 100 KiB\ndrm-memory-gtt: 4 KiB\n"), 104 << 10);
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
