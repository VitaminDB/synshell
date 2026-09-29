//! Аккумуляторы (`/sys/class/power_supply`): сводка для строки состояния и
//! подробности для экрана ресурсов и «Оборудования».

use crate::Sys;

/// Сводка по всем системным батареям (без батарей мышей и т. п.).
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
    pub full: bool,
    /// Оценка до разряда/заряда, минуты.
    pub minutes: Option<u32>,
    /// Мощность, Вт (положительная), если драйвер её даёт.
    pub power_w: Option<f32>,
}

/// Одна батарея подробно.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatteryInfo {
    pub name: String,
    pub status: String,
    pub percent: Option<u32>,
    pub voltage_v: Option<f32>,
    /// Ток, А (знак — как у драйвера: у части драйверов разряд отрицателен).
    pub current_a: Option<f32>,
    pub power_w: Option<f32>,
    pub temp_c: Option<f32>,
    pub health: Option<String>,
    pub cycles: Option<u32>,
    pub technology: Option<String>,
    /// Ёмкость сейчас / при выпуске, Вт·ч или А·ч (как у драйвера).
    pub full: Option<f32>,
    pub full_design: Option<f32>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

impl BatteryInfo {
    /// Износ: ёмкость сейчас от паспортной, %.
    pub fn wear_percent(&self) -> Option<u32> {
        match (self.full, self.full_design) {
            (Some(f), Some(d)) if d > 0.0 => Some((f / d * 100.0).round() as u32),
            _ => None,
        }
    }
}

const PS: &str = "/sys/class/power_supply";

fn is_system_battery(sys: &Sys, name: &str) -> bool {
    sys.read(format!("{PS}/{name}/type")).as_deref() == Some("Battery")
        && sys.read(format!("{PS}/{name}/scope")).as_deref() != Some("Device")
}

/// Подробности по каждой системной батарее.
pub fn batteries(sys: &Sys) -> Vec<BatteryInfo> {
    sys.list(PS)
        .into_iter()
        .filter(|n| is_system_battery(sys, n))
        .map(|name| {
            let f = |k: &str| format!("{PS}/{name}/{k}");
            let micro = |k: &str| sys.read_num::<f64>(f(k)).map(|v| (v / 1e6) as f32);
            let voltage_v = micro("voltage_now");
            let current_a = micro("current_now");
            let power_w = micro("power_now").or_else(|| Some(voltage_v? * current_a?.abs()));
            let (full, full_design) = match micro("energy_full") {
                Some(e) => (Some(e), micro("energy_full_design")),
                None => (micro("charge_full"), micro("charge_full_design")),
            };
            BatteryInfo {
                status: sys.read(f("status")).unwrap_or_default(),
                percent: sys.read_num(f("capacity")),
                voltage_v,
                current_a,
                power_w,
                // Десятые доли градуса.
                temp_c: sys.read_num::<f32>(f("temp")).map(|t| t / 10.0),
                health: sys.read(f("health")),
                cycles: sys.read_num(f("cycle_count")),
                technology: sys.read(f("technology")),
                full,
                full_design,
                manufacturer: sys.read(f("manufacturer")),
                model: sys.read(f("model_name")),
                name,
            }
        })
        .collect()
}

/// Напряжение аккумулятора из АЦП PMIC (`vbat` в IIO), В — на телефонах,
/// где драйвер батареи не поднят (нет ADSP).
pub fn adc_voltage(sys: &Sys) -> Option<f32> {
    for dev in sys.list("/sys/bus/iio/devices") {
        let base = format!("/sys/bus/iio/devices/{dev}");
        for f in sys.list(&base) {
            if f.contains("vbat") && f.ends_with("_input") {
                if let Some(uv) = sys.read_num::<f64>(format!("{base}/{f}")) {
                    return Some((uv / 1e6) as f32);
                }
            }
        }
    }
    None
}

/// Сводка: сумма по батареям.
pub fn read(sys: &Sys) -> Option<Battery> {
    let mut total_now = 0f64;
    let mut total_full = 0f64;
    let mut charging = false;
    let mut full = true;
    let mut power = 0f64;
    let mut power_w = None::<f32>;
    let mut found = false;
    for name in sys.list(PS) {
        if !is_system_battery(sys, &name) {
            continue;
        }
        found = true;
        let num = |k: &str| sys.read_num::<f64>(format!("{PS}/{name}/{k}"));
        let (now, fl) = match (num("energy_now"), num("energy_full")) {
            (Some(n), Some(f)) => (n, f),
            _ => match (num("charge_now"), num("charge_full")) {
                (Some(n), Some(f)) => (n, f),
                _ => (num("capacity").unwrap_or(0.0), 100.0),
            },
        };
        total_now += now;
        total_full += fl;
        let p = num("power_now").or_else(|| num("current_now").map(f64::abs)).unwrap_or(0.0);
        power += p;
        if let Some(pw) = num("power_now") {
            power_w = Some(power_w.unwrap_or(0.0) + (pw.abs() / 1e6) as f32);
        } else if let (Some(v), Some(c)) = (num("voltage_now"), num("current_now")) {
            power_w = Some(power_w.unwrap_or(0.0) + (v / 1e6 * c.abs() / 1e6) as f32);
        }
        let st = sys.read(format!("{PS}/{name}/status")).unwrap_or_default();
        charging |= st == "Charging";
        full &= st == "Full" || st == "Not charging";
    }
    if !found || total_full <= 0.0 {
        return None;
    }
    let percent = (total_now / total_full * 100.0).round().clamp(0.0, 100.0) as u32;
    let minutes = (power > 0.0).then(|| {
        let hours = if charging { (total_full - total_now) / power } else { total_now / power };
        (hours * 60.0) as u32
    });
    Some(Battery { percent, charging, full, minutes, power_w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn laptop_battery() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("sys/class/power_supply/BAT0");
        fs::create_dir_all(&b).unwrap();
        for (k, v) in [
            ("type", "Battery"),
            ("status", "Discharging"),
            ("energy_now", "30000000"),
            ("energy_full", "60000000"),
            ("energy_full_design", "80000000"),
            ("power_now", "15000000"),
            ("capacity", "50"),
            ("cycle_count", "120"),
        ] {
            fs::write(b.join(k), v).unwrap();
        }
        // Батарея мыши не в счёт.
        let m = dir.path().join("sys/class/power_supply/hid-mouse");
        fs::create_dir_all(&m).unwrap();
        fs::write(m.join("type"), "Battery").unwrap();
        fs::write(m.join("scope"), "Device").unwrap();
        let sys = Sys::at(dir.path());
        let s = read(&sys).unwrap();
        assert_eq!(s.percent, 50);
        assert_eq!(s.minutes, Some(120));
        assert_eq!(s.power_w, Some(15.0));
        let d = batteries(&sys);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].wear_percent(), Some(75));
        assert_eq!(d[0].cycles, Some(120));
    }

    #[test]
    fn no_battery() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&Sys::at(dir.path())).is_none());
    }
}
