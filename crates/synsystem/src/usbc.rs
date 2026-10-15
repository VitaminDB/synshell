use std::path::Path;

use crate::Sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    Default,
    Current1_5,
    Current3_0,
    PowerDelivery,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsbDevice {
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub speed_mbps: Option<u32>,
    pub max_ma: Option<u32>,
    pub self_powered: bool,
    pub children: usize,
}

impl UsbDevice {
    pub fn name(&self) -> Option<String> {
        match (&self.manufacturer, &self.product) {
            (_, Some(p)) => Some(p.clone()),
            (Some(m), None) => Some(m.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsbcPort {
    pub name: String,
    pub position: Option<String>,
    pub mode: OpMode,
    pub power_role: Option<String>,
    pub data_role: Option<String>,
    pub partner: bool,
    pub devices: Vec<UsbDevice>,
    pub input: Option<PowerIn>,
    pub partner_source_w: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerIn {
    pub voltage_v: f32,
    pub current_a: Option<f32>,
    pub current_max_a: Option<f32>,
}

impl PowerIn {
    pub fn watts(&self) -> Option<f32> {
        Some(self.voltage_v * self.current_a.filter(|a| *a > 0.0).or(self.current_max_a)?)
    }
}

impl UsbcPort {
    pub fn connected(&self) -> bool {
        self.partner || self.input.is_some() || !self.devices.is_empty()
    }

    pub fn supplying(&self) -> bool {
        self.input.is_none() && self.connected() && self.data_role.as_deref() != Some("device")
    }

    pub fn out_limit_w(&self) -> Option<f32> {
        match self.mode {
            OpMode::Current1_5 => Some(7.5),
            OpMode::Current3_0 => Some(15.0),
            OpMode::PowerDelivery | OpMode::Unknown => None,
            OpMode::Default => {
                let usb3 = self.devices.iter().any(|d| d.speed_mbps.is_some_and(|s| s >= 5000));
                Some(if usb3 { 4.5 } else { 2.5 })
            }
        }
    }
}

const TC: &str = "/sys/class/typec";
const PS: &str = "/sys/class/power_supply";

fn selected(s: &str) -> Option<String> {
    let a = s.find('[')?;
    let b = s[a..].find(']')? + a;
    Some(s[a + 1..b].to_string())
}

fn link_name(sys: &Sys, p: impl AsRef<Path>) -> Option<String> {
    std::fs::canonicalize(sys.path(p)).ok()?.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn leading_num(s: &str) -> Option<f32> {
    let end = s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len());
    s[..end].parse().ok()
}

fn usb_device(sys: &Sys, dir: &str) -> Option<UsbDevice> {
    let r = |k: &str| sys.read(format!("{dir}/{k}")).filter(|v| !v.is_empty());
    if sys.read(format!("{dir}/devnum")).is_none() && r("product").is_none() && r("idVendor").is_none() {
        return None;
    }
    let attrs = r("bmAttributes").and_then(|v| u8::from_str_radix(&v, 16).ok()).unwrap_or(0);
    let children = if sys.read_num::<u32>(format!("{dir}/maxchild")).unwrap_or(0) > 0 {
        let base = link_name(sys, dir).unwrap_or_default();
        sys.list(dir).iter().filter(|n| n.starts_with(&format!("{base}.")) && !n.contains(':')).count()
    } else {
        0
    };
    Some(UsbDevice {
        product: r("product"),
        manufacturer: r("manufacturer"),
        speed_mbps: r("speed").and_then(|s| leading_num(&s)).map(|v| v as u32),
        max_ma: r("bMaxPower").and_then(|s| leading_num(&s)).map(|v| v as u32).filter(|v| *v > 0),
        self_powered: attrs & 0x40 != 0,
        children,
    })
}

fn position(sys: &Sys, port: &str) -> Option<String> {
    let base = format!("{TC}/{port}/physical_location");
    sys.read(format!("{base}/horizontal_position"))
        .filter(|h| h != "unknown")
        .or_else(|| sys.read(format!("{base}/panel")).filter(|p| matches!(p.as_str(), "left" | "right" | "front" | "back")))
}

fn input_of(sys: &Sys, port: &str, index: usize) -> Option<PowerIn> {
    let dev = link_name(sys, format!("{TC}/{port}/device"))?;
    let psy = sys.list(PS).into_iter().find(|n| {
        n.strip_prefix("ucsi-source-psy-").and_then(|r| r.strip_prefix(dev.as_str())).and_then(|num| num.parse::<usize>().ok()) == Some(index + 1)
    })?;
    let num = |k: &str| sys.read_num::<f64>(format!("{PS}/{psy}/{k}"));
    if num("online")? < 1.0 {
        return None;
    }
    let voltage_v = (num("voltage_now").filter(|v| *v > 0.0).or_else(|| num("voltage_max"))? / 1e6) as f32;
    if voltage_v <= 0.0 {
        return None;
    }
    Some(PowerIn {
        voltage_v,
        current_a: num("current_now").map(|a| (a / 1e6) as f32),
        current_max_a: num("current_max").map(|a| (a / 1e6) as f32).filter(|a| *a > 0.0),
    })
}

fn source_caps_w(sys: &Sys, pd: &str) -> Option<f32> {
    let caps = format!("{pd}/source-capabilities");
    sys.list(&caps)
        .into_iter()
        .filter_map(|pdo| {
            let r = |k: &str| sys.read(format!("{caps}/{pdo}/{k}")).and_then(|v| leading_num(&v));
            r("maximum_power").map(|mw| mw / 1000.0).or_else(|| {
                let mv = r("voltage").or_else(|| r("maximum_voltage"))?;
                Some(mv * r("maximum_current")? / 1e6)
            })
        })
        .reduce(f32::max)
}

pub fn ports(sys: &Sys) -> Vec<UsbcPort> {
    let all = sys.list(TC);
    let names: Vec<String> = all.iter().filter(|n| n.starts_with("port") && !n.contains('-') && !n.contains('.')).cloned().collect();
    let mut by_dev: std::collections::HashMap<String, usize> = Default::default();
    names
        .into_iter()
        .map(|name| {
            let dir = format!("{TC}/{name}");
            let dev = link_name(sys, format!("{dir}/device")).unwrap_or_default();
            let index = {
                let c = by_dev.entry(dev).or_insert(0);
                *c += 1;
                *c - 1
            };
            let partner_dir = format!("{TC}/{name}-partner");
            let partner = sys.path(&partner_dir).exists();
            let devices = sys
                .list(&dir)
                .into_iter()
                .filter(|l| l.starts_with("usb") && l.contains("port"))
                .filter_map(|l| usb_device(sys, &format!("{dir}/{l}/device")))
                .fold(Vec::<UsbDevice>::new(), |mut v, d| {
                    if !v.contains(&d) {
                        v.push(d);
                    }
                    v
                });
            let mode = match sys.read(format!("{dir}/power_operation_mode")).as_deref() {
                Some("default") => OpMode::Default,
                Some("1.5A") => OpMode::Current1_5,
                Some("3.0A") => OpMode::Current3_0,
                Some("usb_power_delivery") => OpMode::PowerDelivery,
                _ => OpMode::Unknown,
            };
            UsbcPort {
                position: position(sys, &name),
                mode,
                power_role: sys.read(format!("{dir}/power_role")).and_then(|s| selected(&s)),
                data_role: sys.read(format!("{dir}/data_role")).and_then(|s| selected(&s)),
                partner,
                devices,
                input: input_of(sys, &name, index),
                partner_source_w: partner.then(|| source_caps_w(sys, &format!("{partner_dir}/usb_power_delivery"))).flatten(),
                name,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn write(dir: &Path, kv: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        for (k, v) in kv {
            if let Some(p) = Path::new(k).parent() {
                fs::create_dir_all(dir.join(p)).unwrap();
            }
            fs::write(dir.join(k), v).unwrap();
        }
    }

    fn laptop() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let ucsi = r.join("sys/devices/platform/USBC000:00");
        let p0 = ucsi.join("typec/port0");
        let p1 = ucsi.join("typec/port1");
        for (p, h) in [(&p0, "left"), (&p1, "right")] {
            write(
                p,
                &[
                    ("power_role", "source [sink]"),
                    ("data_role", "[host] device"),
                    ("power_operation_mode", "default"),
                    ("physical_location/horizontal_position", h),
                ],
            );
            symlink(&ucsi, p.join("device")).unwrap();
        }
        let hub = r.join("sys/devices/pci0000:80/0000:80:14.0/usb3");
        let port = hub.join("3-0:1.0/usb3-port1");
        let phone = hub.join("3-1");
        write(&port, &[("state", "configured")]);
        write(
            &phone,
            &[("devnum", "7"), ("product", "Redmi Note 14 Pro"), ("manufacturer", "Xiaomi"), ("speed", "480"), ("bMaxPower", "0mA"), ("bmAttributes", "c0"), ("maxchild", "0")],
        );
        symlink(&phone, port.join("device")).unwrap();
        symlink(&port, p0.join("usb3-port1")).unwrap();
        let tc = r.join("sys/class/typec");
        fs::create_dir_all(&tc).unwrap();
        symlink(&p0, tc.join("port0")).unwrap();
        symlink(&p1, tc.join("port1")).unwrap();
        let ps = r.join("sys/class/power_supply");
        for (n, online) in [("ucsi-source-psy-USBC000:001", "0"), ("ucsi-source-psy-USBC000:002", "1")] {
            let dir = ucsi.join("power_supply").join(n);
            write(&dir, &[("online", online), ("voltage_now", if online == "1" { "20000000" } else { "0" }), ("current_now", "0"), ("current_max", if online == "1" { "3250000" } else { "0" })]);
            fs::create_dir_all(&ps).unwrap();
            symlink(&dir, ps.join(n)).unwrap();
            symlink(&ucsi, dir.join("device")).unwrap();
        }
        d
    }

    #[test]
    fn phone_on_left_charger_on_right() {
        let d = laptop();
        let v = ports(&Sys::at(d.path()));
        assert_eq!(v.len(), 2);
        let l = &v[0];
        assert_eq!(l.position.as_deref(), Some("left"));
        assert_eq!(l.power_role.as_deref(), Some("sink"));
        assert!(l.connected() && l.supplying());
        assert_eq!(l.devices[0].name().as_deref(), Some("Redmi Note 14 Pro"));
        assert!(l.devices[0].self_powered);
        assert_eq!(l.devices[0].max_ma, None);
        assert_eq!(l.out_limit_w(), Some(2.5));
        assert!(l.input.is_none());
        let r = &v[1];
        assert!(r.connected() && !r.supplying());
        let i = r.input.unwrap();
        assert_eq!(i.voltage_v, 20.0);
        assert_eq!(i.watts(), Some(65.0));
    }

    #[test]
    fn partner_pd_caps() {
        let d = laptop();
        let pdo = d.path().join("sys/class/typec/port1-partner/usb_power_delivery/source-capabilities/2:fixed_supply");
        write(&pdo, &[("voltage", "9000mV"), ("maximum_current", "3000mA")]);
        let v = ports(&Sys::at(d.path()));
        assert!(v[1].partner);
        assert_eq!(v[1].partner_source_w, Some(27.0));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn no_typec() {
        let d = tempfile::tempdir().unwrap();
        assert!(ports(&Sys::at(d.path())).is_empty());
    }
}
