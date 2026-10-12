//! Температуры: зоны `/sys/class/thermal`, датчики hwmon. На телефонах
//! зон больше сотни, у многих чтение даёт ошибку — такие пропускаются.

use crate::Sys;

#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    /// Тип зоны или `hwmon:метка`.
    pub name: String,
    pub celsius: f32,
}

/// Все читаемые температуры (зоны thermal и hwmon).
pub fn sensors(sys: &Sys) -> Vec<Sensor> {
    let mut out = Vec::new();
    let base = "/sys/class/thermal";
    for z in sys.list(base).into_iter().filter(|z| z.starts_with("thermal_zone")) {
        let Some(t) = sys.read_num::<f64>(format!("{base}/{z}/temp")) else { continue };
        let name = sys.read(format!("{base}/{z}/type")).unwrap_or(z);
        // Зоны-счётчики (bcl-*, ibat-*) — не температуры.
        if name.contains("bcl") || name.contains("ibat") || name.contains("vbat") {
            continue;
        }
        out.push(Sensor { name, celsius: (t / 1000.0) as f32 });
    }
    let hw = "/sys/class/hwmon";
    for h in sys.list(hw) {
        let chip = sys.read(format!("{hw}/{h}/name")).unwrap_or_else(|| h.clone());
        for f in sys.list(format!("{hw}/{h}")) {
            let Some(idx) = f.strip_prefix("temp").and_then(|r| r.strip_suffix("_input")) else { continue };
            let Some(t) = sys.read_num::<f64>(format!("{hw}/{h}/{f}")) else { continue };
            let label = sys.read(format!("{hw}/{h}/temp{idx}_label")).unwrap_or_else(|| format!("temp{idx}"));
            out.push(Sensor { name: format!("{chip}:{label}"), celsius: (t / 1000.0) as f32 });
        }
    }
    out
}

/// Зоны thermal, чей тип подходит под `pred` (читается только их температура).
pub fn sensors_matching(sys: &Sys, pred: impl Fn(&str) -> bool) -> Vec<Sensor> {
    let base = "/sys/class/thermal";
    sys.list(base)
        .into_iter()
        .filter(|z| z.starts_with("thermal_zone"))
        .filter_map(|z| {
            let name = sys.read(format!("{base}/{z}/type"))?;
            if !pred(&name) {
                return None;
            }
            let t = sys.read_num::<f64>(format!("{base}/{z}/temp"))?;
            Some(Sensor { name, celsius: (t / 1000.0) as f32 })
        })
        .collect()
}

/// Температура процессора: максимум по зонам CPU (`cpu*`, `x86_pkg_temp`,
/// `k10temp`, `coretemp`).
pub fn cpu_celsius(sensors: &[Sensor]) -> Option<f32> {
    sensors
        .iter()
        .filter(|s| {
            let n = s.name.to_ascii_lowercase();
            n.starts_with("cpu") || n.contains("x86_pkg") || n.starts_with("k10temp") || n.starts_with("coretemp") || n.contains("tctl")
        })
        .map(|s| s.celsius)
        .fold(None, |m, t| Some(m.map_or(t, |m: f32| m.max(t))))
}

/// Температура GPU: зоны `gpu*`.
pub fn gpu_celsius(sensors: &[Sensor]) -> Option<f32> {
    sensors
        .iter()
        .filter(|s| s.name.to_ascii_lowercase().starts_with("gpu") || s.name.starts_with("amdgpu:"))
        .map(|s| s.celsius)
        .fold(None, |m, t| Some(m.map_or(t, |m: f32| m.max(t))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempKind {
    Cpu,
    Gpu,
    Storage,
    Memory,
    Wifi,
    Battery,
    Board,
    Npu,
    Modem,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TempDevice {
    pub kind: TempKind,
    pub name: String,
    pub celsius: f32,
    pub crit: Option<f32>,
}

impl TempDevice {
    pub fn level(&self) -> f32 {
        let crit = self.crit.filter(|c| *c > 40.0 && *c < 150.0).unwrap_or(100.0);
        self.celsius / crit
    }
}

fn hwmon_temp(sys: &Sys, dir: &str, idx: &str, file: &str) -> Option<f32> {
    sys.read_num::<f64>(format!("{dir}/temp{idx}_{file}")).map(|v| (v / 1000.0) as f32)
}

const SOCS: &[(&str, &str)] = &[
    ("SM8750", "Snapdragon 8 Elite"),
    ("SM8650", "Snapdragon 8 Gen 3"),
    ("SM8550", "Snapdragon 8 Gen 2"),
    ("SM8475", "Snapdragon 8+ Gen 1"),
    ("SM8450", "Snapdragon 8 Gen 1"),
    ("SM8350", "Snapdragon 888"),
    ("SM8250", "Snapdragon 865"),
    ("SM8150", "Snapdragon 855"),
    ("SDM845", "Snapdragon 845"),
    ("SM7475", "Snapdragon 7+ Gen 2"),
    ("SM7325", "Snapdragon 778G"),
    ("SM7250", "Snapdragon 765G"),
    ("SM7150", "Snapdragon 730"),
    ("SM6375", "Snapdragon 695"),
    ("SDM660", "Snapdragon 660"),
    ("SDM450", "Snapdragon 450"),
    ("MSM8953", "Snapdragon 625"),
    ("MSM8917", "Snapdragon 425"),
    ("MSM8916", "Snapdragon 410"),
];

fn short_cpu(name: &str) -> String {
    let up = name.to_ascii_uppercase();
    if let Some((_, n)) = SOCS.iter().find(|(id, _)| up.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == *id)) {
        return n.to_string();
    }
    let name = name.trim_start_matches("Qualcomm Technologies, Inc ").trim_start_matches("Qualcomm ");
    let mut s = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    for cut in [" CPU", " @", " with ", " Processor"] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    for pre in ["Intel ", "AMD "] {
        s = s.trim_start_matches(pre).to_string();
    }
    s.split_whitespace().filter(|w| !w.ends_with("-Core")).collect::<Vec<_>>().join(" ")
}

pub fn cpu_short_name(sys: &Sys) -> Option<String> {
    crate::cpu::model(sys).map(|m| short_cpu(&m))
}

pub fn devices(sys: &Sys) -> Vec<TempDevice> {
    let mut out: Vec<TempDevice> = Vec::new();
    let add = |d: TempDevice, out: &mut Vec<TempDevice>| {
        if let Some(x) = out.iter_mut().find(|x| x.kind == d.kind && x.name == d.name) {
            if d.celsius > x.celsius {
                x.celsius = d.celsius;
            }
            x.crit = x.crit.or(d.crit);
        } else {
            out.push(d);
        }
    };
    let hw = "/sys/class/hwmon";
    let mut has_cpu = false;
    for h in sys.list(hw) {
        let dir = format!("{hw}/{h}");
        let chip = sys.read(format!("{dir}/name")).unwrap_or_default();
        let idxs: Vec<String> = sys.list(&dir).into_iter().filter_map(|f| f.strip_prefix("temp").and_then(|r| r.strip_suffix("_input")).map(String::from)).collect();
        if idxs.is_empty() {
            continue;
        }
        let label = |i: &str| sys.read(format!("{dir}/temp{i}_label")).unwrap_or_default();
        let pick = |pref: &[&str]| -> Option<String> {
            pref.iter().find_map(|p| idxs.iter().find(|i| label(i).starts_with(p)).cloned()).or_else(|| idxs.iter().min_by_key(|i| i.parse::<u32>().unwrap_or(99)).cloned())
        };
        let max_of = || idxs.iter().filter_map(|i| hwmon_temp(sys, &dir, i, "input")).fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.max(t))));
        let (kind, name, celsius, crit) = match chip.as_str() {
            "coretemp" | "k10temp" | "zenpower" => {
                let i = pick(&["Package", "Tctl", "Tdie"]).unwrap_or_default();
                has_cpu = true;
                let name = cpu_short_name(sys).unwrap_or_else(|| "CPU".into());
                (TempKind::Cpu, name, max_of(), hwmon_temp(sys, &dir, &i, "crit"))
            }
            "nvme" => {
                let i = pick(&["Composite"]).unwrap_or_default();
                let model = sys
                    .read(format!("{dir}/device/model"))
                    .map(|m| m.trim().split('-').next().unwrap_or("").trim().to_string())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "NVMe".into());
                (TempKind::Storage, model, hwmon_temp(sys, &dir, &i, "input"), hwmon_temp(sys, &dir, &i, "crit"))
            }
            "drivetemp" => {
                let model = sys.read(format!("{dir}/device/model")).map(|m| m.trim().to_string()).unwrap_or_else(|| "SATA".into());
                (TempKind::Storage, model, max_of(), hwmon_temp(sys, &dir, "1", "crit"))
            }
            "spd5118" | "jc42" | "ee1004" => (TempKind::Memory, String::new(), max_of(), hwmon_temp(sys, &dir, "1", "crit")),
            "amdgpu" => (TempKind::Gpu, "AMD Radeon".into(), max_of(), hwmon_temp(sys, &dir, "1", "crit")),
            c if c.starts_with("iwlwifi") || c.starts_with("ath") || c.starts_with("mt7") => (TempKind::Wifi, "Wi-Fi".into(), max_of(), None),
            c if c.starts_with("BAT") => (TempKind::Battery, String::new(), max_of(), None),
            _ => continue,
        };
        let Some(celsius) = celsius.filter(|t| *t > 0.0 && *t < 150.0) else { continue };
        add(TempDevice { kind, name, celsius, crit }, &mut out);
    }
    let base = "/sys/class/thermal";
    for z in sys.list(base).into_iter().filter(|z| z.starts_with("thermal_zone")) {
        let Some(t) = sys.read_num::<f64>(format!("{base}/{z}/temp")) else { continue };
        let t = (t / 1000.0) as f32;
        if !(1.0..150.0).contains(&t) {
            continue;
        }
        let ty = sys.read(format!("{base}/{z}/type")).unwrap_or_default();
        let l = ty.to_ascii_lowercase();
        if l.contains("bcl") || l.contains("ibat") || l.contains("vbat") || l.starts_with("int3400") || l.starts_with("iwlwifi") {
            continue;
        }
        let kind = if l.starts_with("cpu") || l.contains("x86_pkg") || l.starts_with("tcpu") || l.contains("cpuss") || l.starts_with("apc") {
            if has_cpu {
                continue;
            }
            TempKind::Cpu
        } else if l.starts_with("gpu") {
            TempKind::Gpu
        } else if l.starts_with("ddr") {
            TempKind::Memory
        } else if l.starts_with("nspss") || l.contains("npu") {
            TempKind::Npu
        } else if l.starts_with("mdmss") || l.starts_with("modem") {
            TempKind::Modem
        } else if l.contains("battery") || l == "bms" {
            TempKind::Battery
        } else if l.starts_with("sen") || l.starts_with("acpitz") || l.contains("skin") || l.contains("quiet") || l.contains("pch") || l.starts_with("tz") {
            TempKind::Board
        } else if l.contains("nvme") || l.contains("ufs") || l.contains("sdc") || l.starts_with("flash") {
            TempKind::Storage
        } else if l.contains("wlan") || l.contains("wifi") {
            TempKind::Wifi
        } else {
            continue;
        };
        let name = match kind {
            TempKind::Cpu => cpu_short_name(sys).unwrap_or_else(|| "CPU".into()),
            TempKind::Gpu => "GPU".into(),
            TempKind::Storage => "UFS".into(),
            TempKind::Wifi => "Wi-Fi".into(),
            _ => String::new(),
        };
        add(TempDevice { kind, name, celsius: t, crit: None }, &mut out);
    }
    out.sort_by(|a, b| b.level().total_cmp(&a.level()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn zones_and_hwmon() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for (i, ty, t) in [(0, "cpu-1-7", Some("43000")), (1, "gpuss-0", Some("33900")), (2, "pa", None), (3, "pm8350c-bcl-lvl0", Some("0"))] {
            let z = r.join(format!("sys/class/thermal/thermal_zone{i}"));
            fs::create_dir_all(&z).unwrap();
            fs::write(z.join("type"), ty).unwrap();
            if let Some(t) = t {
                fs::write(z.join("temp"), t).unwrap();
            }
        }
        let h = r.join("sys/class/hwmon/hwmon0");
        fs::create_dir_all(&h).unwrap();
        fs::write(h.join("name"), "k10temp").unwrap();
        fs::write(h.join("temp1_input"), "55000").unwrap();
        fs::write(h.join("temp1_label"), "Tctl").unwrap();
        let s = sensors(&Sys::at(r));
        assert_eq!(s.len(), 3);
        assert_eq!(cpu_celsius(&s), Some(55.0));
        assert_eq!(gpu_celsius(&s), Some(33.9));
        let d = devices(&Sys::at(r));
        assert!(d.iter().any(|x| x.kind == TempKind::Cpu && x.celsius == 55.0));
        assert!(d.iter().any(|x| x.kind == TempKind::Gpu));
    }

    #[test]
    fn cpu_names() {
        assert_eq!(short_cpu("Intel(R) Core(TM) Ultra 9 275HX"), "Core Ultra 9 275HX");
        assert_eq!(short_cpu("AMD Ryzen 9 7945HX with Radeon Graphics"), "Ryzen 9 7945HX");
        assert_eq!(short_cpu("AMD Ryzen 7 5800X 8-Core Processor"), "Ryzen 7 5800X");
        assert_eq!(short_cpu("Qualcomm Technologies, Inc SM8475"), "Snapdragon 8+ Gen 1");
        assert_eq!(short_cpu("Qualcomm Technologies, Inc SM9999"), "SM9999");
    }
}
