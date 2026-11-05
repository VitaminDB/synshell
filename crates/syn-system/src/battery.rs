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
            let power_w = power_of(voltage_v.map(f64::from), current_a.map(f64::from), micro("power_now").map(f64::from)).map(|p| p as f32);
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

/// Подключено ли внешнее питание (сеть, USB, беспроводная зарядка):
/// `None` — источников питания нет вовсе (настольный компьютер) или нет
/// системной батареи, которую они заряжали бы.
pub fn external_power(sys: &Sys) -> Option<bool> {
    let names = sys.list(PS);
    if !names.iter().any(|n| is_system_battery(sys, n)) {
        return None;
    }
    let mut seen = false;
    let mut online = false;
    for n in &names {
        let ty = sys.read(format!("{PS}/{n}/type")).unwrap_or_default();
        if ty == "Battery" || sys.read(format!("{PS}/{n}/scope")).as_deref() == Some("Device") {
            continue;
        }
        let Some(on) = sys.read_num::<u32>(format!("{PS}/{n}/online")) else { continue };
        seen = true;
        online |= on > 0;
    }
    seen.then_some(online)
}

/// Слушать изменения источников питания (подключили/отключили кабель):
/// `changed` зовётся из потока чтения uevent.
pub fn watch(changed: impl FnMut() + Send + 'static) -> std::io::Result<()> {
    crate::uevent::watch("power-uevent", |m| crate::uevent::has(m, b"SUBSYSTEM=power_supply"), changed)
}

/// Мощность батареи, Вт: U×I, если драйвер даёт напряжение и ток, иначе
/// `power_now`. Драйверы телефонов (qcom battmgr) держат в `power_now`
/// константу (10 Вт), а ток и напряжение у них настоящие.
fn power_of(voltage_v: Option<f64>, current_a: Option<f64>, power_now_w: Option<f64>) -> Option<f64> {
    match (voltage_v, current_a) {
        (Some(v), Some(i)) if v > 0.0 => Some(v * i.abs()),
        _ => power_now_w.map(f64::abs),
    }
}

/// Сводка: сумма по батареям.
///
/// Запас и мощность сводятся к Вт·ч и Вт, чтобы оценка времени не смешивала
/// единицы: у части драйверов есть только `capacity` и `charge_full`
/// (без `charge_now`/`energy_now`).
pub fn read(sys: &Sys) -> Option<Battery> {
    // Сумма процентов, взвешенных ёмкостью (Вт·ч), и сами ёмкости.
    let mut pct_sum = 0f64;
    let mut pct_weight = 0f64;
    let mut now_wh = 0f64;
    let mut full_wh = 0f64;
    let mut energy_known = true;
    let mut charging = false;
    let mut full = true;
    let mut power_w = None::<f64>;
    let mut found = false;
    for name in sys.list(PS) {
        if !is_system_battery(sys, &name) {
            continue;
        }
        found = true;
        let num = |k: &str| sys.read_num::<f64>(format!("{PS}/{name}/{k}"));
        let micro = |k: &str| num(k).map(|v| v / 1e6);
        let volt = micro("voltage_now").filter(|v| *v > 0.0).or_else(|| micro("voltage_min_design")).filter(|v| *v > 0.0);
        let pw = power_of(micro("voltage_now"), micro("current_now"), micro("power_now"));
        // Ёмкость полная и текущая, Вт·ч (по заряду — через напряжение).
        let full_e = micro("energy_full").or_else(|| Some(micro("charge_full")? * volt?)).filter(|f| *f > 0.0);
        let now_e = micro("energy_now").or_else(|| Some(micro("charge_now")? * volt?));
        let cap = num("capacity");
        let pct = match (now_e, full_e) {
            (Some(n), Some(f)) => Some(n / f * 100.0),
            _ => cap,
        };
        let Some(pct) = pct else { continue };
        let w = full_e.unwrap_or(1.0);
        pct_sum += pct.clamp(0.0, 100.0) * w;
        pct_weight += w;
        match full_e {
            Some(f) => {
                full_wh += f;
                now_wh += now_e.unwrap_or(f * pct / 100.0);
            }
            None => energy_known = false,
        }
        if let Some(p) = pw {
            power_w = Some(power_w.unwrap_or(0.0) + p);
        }
        let st = sys.read(format!("{PS}/{name}/status")).unwrap_or_default();
        charging |= st == "Charging";
        full &= st == "Full" || st == "Not charging";
    }
    if !found || pct_weight <= 0.0 {
        return None;
    }
    let percent = (pct_sum / pct_weight).round().clamp(0.0, 100.0) as u32;
    let minutes = match power_w {
        Some(p) if p > 0.05 && energy_known && full_wh > 0.0 && !full => {
            let hours = if charging { (full_wh - now_wh).max(0.0) / p } else { now_wh / p };
            // Больше суток — оценка бессмысленна (ток почти нулевой).
            Some((hours * 60.0).round() as u32).filter(|m| *m < 48 * 60)
        }
        _ => None,
    };
    Some(Battery { percent, charging, full, minutes, power_w: power_w.map(|p| p as f32) })
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
    fn phone_battery_without_charge_now() {
        // qcom battmgr: нет charge_now, power_now — константа 10 Вт.
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("sys/class/power_supply/battery");
        fs::create_dir_all(&b).unwrap();
        for (k, v) in [
            ("type", "Battery"),
            ("status", "Charging"),
            ("capacity", "50"),
            ("charge_full", "4000000"),
            ("current_now", "-2000000"),
            ("voltage_now", "4000000"),
            ("power_now", "10000000"),
        ] {
            fs::write(b.join(k), v).unwrap();
        }
        let s = read(&Sys::at(dir.path())).unwrap();
        assert_eq!(s.percent, 50);
        assert!(s.charging);
        assert_eq!(s.power_w, Some(8.0));
        // 2 А·ч × 4 В = 8 Вт·ч до полного при 8 Вт — час.
        assert_eq!(s.minutes, Some(60));
        assert_eq!(batteries(&Sys::at(dir.path()))[0].power_w, Some(8.0));
    }

    #[test]
    fn external_power_online() {
        let dir = tempfile::tempdir().unwrap();
        let ps = dir.path().join("sys/class/power_supply");
        for (n, kv) in [
            ("battery", &[("type", "Battery"), ("capacity", "50")][..]),
            ("usb", &[("type", "USB_PD"), ("online", "1")][..]),
            ("wireless", &[("type", "Wireless"), ("online", "0")][..]),
        ] {
            fs::create_dir_all(ps.join(n)).unwrap();
            for (k, v) in kv {
                fs::write(ps.join(n).join(k), v).unwrap();
            }
        }
        let sys = Sys::at(dir.path());
        assert_eq!(external_power(&sys), Some(true));
        fs::write(ps.join("usb/online"), "0").unwrap();
        assert_eq!(external_power(&sys), Some(false));
        fs::remove_dir_all(ps.join("battery")).unwrap();
        assert_eq!(external_power(&sys), None);
    }

    #[test]
    fn no_battery() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&Sys::at(dir.path())).is_none());
    }
}
