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
    }
}
