//! Оборудование для «Параметров → Оборудование»: разделы (процессор,
//! память, аккумулятор, графика, дисплей, датчики, температуры, хранилище,
//! сеть, USB, ввод, звук, камеры, светодиоды) с устройствами и их
//! свойствами. Только чтение sysfs/procfs — работает и без udev, как на
//! телефоне с Android-ядром.

use crate::{backlight, battery, cpu, gpu, memory, thermal, Sys};

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub name: String,
    pub props: Vec<(String, String)>,
}

impl Device {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), props: Vec::new() }
    }

    fn prop(mut self, k: &str, v: impl Into<String>) -> Self {
        let v = v.into();
        if !v.trim().is_empty() {
            self.props.push((k.to_string(), v));
        }
        self
    }

    fn opt(self, k: &str, v: Option<impl Into<String>>) -> Self {
        match v {
            Some(v) => self.prop(k, v),
            None => self,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    /// Глиф Material Icons.
    pub icon: &'static str,
    pub devices: Vec<Device>,
}

/// Все разделы; пустые не попадают.
pub fn collect(sys: &Sys) -> Vec<Section> {
    let mut out = vec![
        section("cpu", "Процессор", "\u{E30D}", cpu_devices(sys)),
        section("memory", "Память", "\u{E322}", memory_devices(sys)),
        section("battery", "Аккумулятор", "\u{E1A4}", battery_devices(sys)),
        section("gpu", "Графика", "\u{E30A}", gpu_devices(sys)),
        section("display", "Дисплей", "\u{E30C}", display_devices(sys)),
        section("sensors", "Датчики", "\u{E51E}", iio_devices(sys)),
        section("thermal", "Температуры", "\u{E1FF}", thermal_devices(sys)),
        section("storage", "Хранилище", "\u{E1DB}", storage_devices(sys)),
        section("network", "Сеть", "\u{E80D}", net_devices(sys)),
        section("usb", "USB", "\u{E1E0}", usb_devices(sys)),
        section("input", "Устройства ввода", "\u{E312}", input_devices(sys)),
        section("sound", "Звук", "\u{E050}", sound_devices(sys)),
        section("camera", "Камеры", "\u{E3AF}", camera_devices(sys)),
        section("leds", "Светодиоды", "\u{E0F0}", led_devices(sys)),
    ];
    out.retain(|s| !s.devices.is_empty());
    out
}

fn section(id: &'static str, title: &'static str, icon: &'static str, devices: Vec<Device>) -> Section {
    Section { id, title, icon, devices }
}

/// Текстовый отчёт (для «Копировать отчёт»).
pub fn report(sections: &[Section]) -> String {
    let mut s = String::new();
    for sec in sections {
        s += &format!("## {}\n", sec.title);
        for d in &sec.devices {
            s += &format!("- {}\n", d.name);
            for (k, v) in &d.props {
                s += &format!("    {k}: {v}\n");
            }
        }
        s.push('\n');
    }
    s
}

fn mhz(v: Option<u32>) -> Option<String> {
    v.map(|m| format!("{m} МГц"))
}

fn cpu_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    let clusters = cpu::clusters(sys);
    let cores = sys
        .read("/sys/devices/system/cpu/possible")
        .map(|s| cpu::parse_cpu_list(&s).len())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| sys.read("/proc/cpuinfo").map(|s| s.lines().filter(|l| l.starts_with("processor")).count()).unwrap_or(0));
    let arch = std::env::consts::ARCH;
    v.push(
        Device::new(cpu::model(sys).unwrap_or_else(|| "Процессор".into()))
            .opt("Устройство", cpu::device_model(sys))
            .prop("Архитектура", arch)
            .prop("Ядер", cores.to_string()),
    );
    // Кластеры имеют смысл, если их меньше ядер (big.LITTLE).
    if clusters.len() > 1 && clusters.len() < cores {
        for (i, c) in clusters.iter().enumerate() {
            let range = match (c.cpus.first(), c.cpus.last()) {
                (Some(a), Some(b)) if a != b => format!("{a}–{b}"),
                (Some(a), _) => a.to_string(),
                _ => String::new(),
            };
            v.push(
                Device::new(format!("Кластер {} ({} ядер)", i + 1, c.cpus.len()))
                    .prop("Ядра", range)
                    .opt("Частота", c.min_mhz.zip(c.max_mhz).map(|(a, b)| format!("{a}–{b} МГц")))
                    .opt("Регулятор", c.governor.clone()),
            );
        }
    } else if let Some(c) = clusters.first() {
        v[0] = v[0].clone().opt("Частота", c.min_mhz.zip(c.max_mhz).map(|(a, b)| format!("{a}–{b} МГц"))).opt("Регулятор", c.governor.clone());
    }
    v
}

fn memory_devices(sys: &Sys) -> Vec<Device> {
    let Some(m) = memory::read(sys) else { return Vec::new() };
    let mut d = Device::new("Оперативная память")
        .prop("Всего", memory::human_kb(m.total_kb))
        .prop("Доступно", memory::human_kb(m.available_kb));
    if m.swap_total_kb > 0 {
        d = d.prop("Подкачка", format!("{} из {}", memory::human_kb(m.swap_used_kb()), memory::human_kb(m.swap_total_kb)));
    }
    let mut v = vec![d];
    // zram (телефоны): сжатая подкачка в памяти.
    for z in sys.list("/sys/block").into_iter().filter(|b| b.starts_with("zram")) {
        let size = sys.read_num::<u64>(format!("/sys/block/{z}/disksize")).unwrap_or(0);
        if size > 0 {
            v.push(Device::new(z.clone()).prop("Размер", memory::human_kb(size / 1024)).opt("Сжатие", sys.read(format!("/sys/block/{z}/comp_algorithm"))));
        }
    }
    v
}

fn battery_devices(sys: &Sys) -> Vec<Device> {
    let mut v: Vec<Device> = battery::batteries(sys)
        .into_iter()
        .map(|b| {
            Device::new(b.model.clone().unwrap_or_else(|| b.name.clone()))
                .opt("Производитель", b.manufacturer.clone())
                .prop("Состояние", b.status.clone())
                .opt("Заряд", b.percent.map(|p| format!("{p}%")))
                .opt("Напряжение", b.voltage_v.map(|x| format!("{x:.2} В")))
                .opt("Ток", b.current_a.map(|x| format!("{x:.2} А")))
                .opt("Мощность", b.power_w.map(|x| format!("{x:.1} Вт")))
                .opt("Температура", b.temp_c.map(|x| format!("{x:.1} °C")))
                .opt("Здоровье", b.health.clone())
                .opt("Износ", b.wear_percent().map(|w| format!("{w}% от паспортной ёмкости")))
                .opt("Циклов", b.cycles.map(|c| c.to_string()))
                .opt("Технология", b.technology.clone())
        })
        .collect();
    // Без драйвера батареи (телефон без ADSP) — напряжение из IIO АЦП PMIC.
    if v.is_empty() {
        for dev in sys.list("/sys/bus/iio/devices") {
            let base = format!("/sys/bus/iio/devices/{dev}");
            for f in sys.list(&base) {
                if f.contains("vbat") && f.ends_with("_input") {
                    if let Some(uv) = sys.read_num::<f64>(format!("{base}/{f}")) {
                        v.push(Device::new("Аккумулятор (АЦП PMIC)").prop("Напряжение", format!("{:.3} В", uv / 1e6)).prop("Источник", f));
                    }
                }
            }
        }
    }
    v
}

fn gpu_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    if let Some(g) = gpu::read(sys) {
        v.push(
            Device::new(g.name.clone().unwrap_or_else(|| "GPU".into()))
                .opt("Частота", mhz(g.cur_mhz))
                .opt("Максимум", mhz(g.max_mhz))
                .opt("Загрузка", g.busy_percent.map(|b| format!("{b:.0}%"))),
        );
    }
    for card in sys.list("/sys/class/drm").into_iter().filter(|c| c.starts_with("card") && !c.contains('-')) {
        let uevent = sys.read(format!("/sys/class/drm/{card}/device/uevent")).unwrap_or_default();
        let driver = uevent.lines().find_map(|l| l.strip_prefix("DRIVER=")).map(String::from);
        let dev = Device::new(card.clone()).opt("Драйвер", driver).opt("PCI", uevent.lines().find_map(|l| l.strip_prefix("PCI_ID=")).map(String::from));
        v.push(dev);
    }
    v
}

fn display_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    for conn in sys.list("/sys/class/drm").into_iter().filter(|c| c.contains('-')) {
        let base = format!("/sys/class/drm/{conn}");
        let status = sys.read(format!("{base}/status")).unwrap_or_default();
        if status != "connected" {
            continue;
        }
        let modes = sys.read(format!("{base}/modes")).unwrap_or_default();
        let name = conn.split_once('-').map(|(_, n)| n.to_string()).unwrap_or(conn.clone());
        v.push(Device::new(name).prop("Режимы", modes.lines().take(4).collect::<Vec<_>>().join(", ")).opt("Включён", sys.read(format!("{base}/enabled"))));
    }
    for b in backlight::list(sys) {
        v.push(Device::new(format!("Подсветка {}", b.name)).prop("Яркость", format!("{} из {} ({:.0}%)", b.brightness, b.max, b.percent())).prop("Тип", b.kind));
    }
    v
}

/// Датчики IIO: имя и каналы со значениями (сырое × масштаб + смещение).
fn iio_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    for dev in sys.list("/sys/bus/iio/devices").into_iter().filter(|d| d.starts_with("iio:device")) {
        let base = format!("/sys/bus/iio/devices/{dev}");
        let name = sys.read(format!("{base}/name")).unwrap_or(dev.clone());
        let mut d = Device::new(pretty_iio_name(&name));
        for f in sys.list(&base) {
            if let Some(ch) = f.strip_suffix("_input") {
                // Уже в единицах: температуры — милли°C, напряжения — мкВ/мВ.
                if let Some(val) = sys.read_num::<f64>(format!("{base}/{f}")) {
                    d = d.prop(&channel_label(ch), channel_value(ch, val));
                }
            } else if let Some(ch) = f.strip_suffix("_raw") {
                let raw = sys.read_num::<f64>(format!("{base}/{f}"));
                let scale = sys.read_num::<f64>(format!("{base}/{ch}_scale")).or_else(|| scale_for(sys, &base, ch)).unwrap_or(1.0);
                let offset = sys.read_num::<f64>(format!("{base}/{ch}_offset")).unwrap_or(0.0);
                if let Some(r) = raw {
                    d = d.prop(&channel_label(ch), format!("{:.3}", (r + offset) * scale));
                }
            }
        }
        if !d.props.is_empty() {
            v.push(d);
        }
    }
    v
}

/// Общий масштаб типа канала (`in_accel_scale` для `in_accel_x`).
fn scale_for(sys: &Sys, base: &str, ch: &str) -> Option<f64> {
    let kind = ch.rsplit_once('_').map(|(k, _)| k)?;
    sys.read_num(format!("{base}/{kind}_scale"))
}

fn pretty_iio_name(n: &str) -> String {
    // «c42d000.qcom,spmi:qcom,pmk8350@0:vadc@3100» → «АЦП PMIC (vadc)».
    if n.contains("vadc") || n.contains("adc") {
        return format!("АЦП PMIC ({})", n.rsplit(':').next().unwrap_or(n));
    }
    n.to_string()
}

fn channel_label(ch: &str) -> String {
    let ch = ch.strip_prefix("in_").unwrap_or(ch);
    let (kind, rest) = ch.split_once('_').unwrap_or((ch, ""));
    let kind_ru = match kind {
        "temp" => "Температура",
        "voltage" => "Напряжение",
        "current" => "Ток",
        "accel" => "Ускорение",
        "anglvel" => "Угловая скорость",
        "magn" => "Магнитное поле",
        "illuminance" => "Освещённость",
        "proximity" => "Приближение",
        "pressure" => "Давление",
        "humidityrelative" => "Влажность",
        other => other,
    };
    if rest.is_empty() {
        kind_ru.to_string()
    } else {
        format!("{kind_ru} {rest}")
    }
}

fn channel_value(ch: &str, v: f64) -> String {
    let ch = ch.strip_prefix("in_").unwrap_or(ch);
    if ch.starts_with("temp") {
        format!("{:.1} °C", v / 1000.0)
    } else if ch.starts_with("voltage") {
        // processed в мВ по ABI; у qcom-vadc — мкВ (большие числа).
        if v.abs() > 100_000.0 { format!("{:.3} В", v / 1e6) } else { format!("{:.3} В", v / 1000.0) }
    } else if ch.starts_with("current") {
        if v.abs() > 100_000.0 { format!("{:.3} А", v / 1e6) } else { format!("{:.0} мА", v) }
    } else {
        format!("{v}")
    }
}

fn thermal_devices(sys: &Sys) -> Vec<Device> {
    let sensors = thermal::sensors(sys);
    if sensors.is_empty() {
        return Vec::new();
    }
    // Одно «устройство» со списком: зон на телефоне десятки.
    let mut d = Device::new("Датчики температуры");
    for s in sensors {
        d = d.prop(&s.name, format!("{:.1} °C", s.celsius));
    }
    vec![d]
}

fn storage_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    for b in sys.list("/sys/block") {
        if b.starts_with("loop") || b.starts_with("ram") || b.starts_with("zram") || b.starts_with("mtdblock") || b.starts_with("dm-") {
            continue;
        }
        let base = format!("/sys/block/{b}");
        let sectors = sys.read_num::<u64>(format!("{base}/size")).unwrap_or(0);
        if sectors == 0 {
            continue;
        }
        let gb = sectors as f64 * 512.0 / 1e9;
        let rot = sys.read(format!("{base}/queue/rotational")).as_deref() == Some("1");
        let model = sys.read(format!("{base}/device/model")).unwrap_or_default();
        let vendor = sys.read(format!("{base}/device/vendor")).unwrap_or_default();
        let kind = if b.starts_with("nvme") {
            "NVMe SSD"
        } else if b.starts_with("mmcblk") {
            "eMMC/SD"
        } else if rot {
            "HDD"
        } else if vendor.contains("SAMSUNG") || b.starts_with("sd") && !rot {
            "UFS/SSD"
        } else {
            "Диск"
        };
        v.push(Device::new(format!("{b} — {}", format!("{vendor} {model}").trim())).prop("Тип", kind).prop("Объём", format!("{gb:.1} ГБ")));
    }
    v
}

fn net_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    for n in sys.list("/sys/class/net") {
        if n == "lo" {
            continue;
        }
        let base = format!("/sys/class/net/{n}");
        let wifi = sys.path(format!("{base}/wireless")).exists() || sys.path(format!("{base}/phy80211")).exists();
        let driver = std::fs::read_link(sys.path(format!("{base}/device/driver"))).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()));
        v.push(
            Device::new(n.clone())
                .prop("Тип", if wifi { "Wi-Fi" } else if n.starts_with("usb") || n.starts_with("rndis") { "USB-сеть" } else { "Сеть" })
                .opt("Состояние", sys.read(format!("{base}/operstate")))
                .opt("MAC", sys.read(format!("{base}/address")))
                .opt("Скорость", sys.read_num::<i64>(format!("{base}/speed")).filter(|s| *s > 0).map(|s| format!("{s} Мбит/с")))
                .opt("Драйвер", driver),
        );
    }
    v
}

fn usb_devices(sys: &Sys) -> Vec<Device> {
    let mut v = Vec::new();
    for d in sys.list("/sys/bus/usb/devices") {
        let base = format!("/sys/bus/usb/devices/{d}");
        let Some(product) = sys.read(format!("{base}/product")) else { continue };
        v.push(
            Device::new(product)
                .opt("Производитель", sys.read(format!("{base}/manufacturer")))
                .opt("ID", sys.read(format!("{base}/idVendor")).zip(sys.read(format!("{base}/idProduct"))).map(|(a, b)| format!("{a}:{b}")))
                .opt("Скорость", sys.read(format!("{base}/speed")).map(|s| format!("{s} Мбит/с"))),
        );
    }
    // Режим USB-контроллера телефона (device — гаджет USB-сети).
    for udc in sys.list("/sys/class/udc") {
        v.push(Device::new(format!("Контроллер {udc}")).opt("Состояние", sys.read(format!("/sys/class/udc/{udc}/state"))).opt("Скорость", sys.read(format!("/sys/class/udc/{udc}/current_speed"))));
    }
    v
}

fn input_devices(sys: &Sys) -> Vec<Device> {
    let text = sys.read("/proc/bus/input/devices").unwrap_or_default();
    let mut v = Vec::new();
    for block in text.split("\n\n") {
        let name = block.lines().find_map(|l| l.strip_prefix("N: Name=")).map(|s| s.trim_matches('"').to_string());
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers=")).unwrap_or("").trim().to_string();
        let Some(name) = name else { continue };
        let kind = if name.contains("touch") || name == "fts" || name.contains("goodix") && !name.contains("uinput") {
            "Сенсорный экран"
        } else if name.contains("haptic") {
            "Вибромотор"
        } else if name.contains("pwrkey") || name.contains("resin") || name.contains("keys") {
            "Кнопки"
        } else if handlers.contains("mouse") {
            "Мышь / тачпад"
        } else if handlers.contains("kbd") {
            "Клавиатура"
        } else {
            "Ввод"
        };
        v.push(Device::new(name).prop("Тип", kind).prop("Обработчики", handlers));
    }
    v
}

fn sound_devices(sys: &Sys) -> Vec<Device> {
    let text = sys.read("/proc/asound/cards").unwrap_or_default();
    text.lines()
        .filter(|l| l.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|l| Device::new(l.split_once(':').map(|(_, n)| n.trim()).unwrap_or(l.trim()).to_string()))
        .collect()
}

fn camera_devices(sys: &Sys) -> Vec<Device> {
    sys.list("/sys/class/video4linux")
        .into_iter()
        .filter_map(|v| {
            let name = sys.read(format!("/sys/class/video4linux/{v}/name"))?;
            Some(Device::new(name).prop("Устройство", format!("/dev/{v}")))
        })
        .collect()
}

fn led_devices(sys: &Sys) -> Vec<Device> {
    sys.list("/sys/class/leds")
        .into_iter()
        .filter(|l| !l.starts_with("mmc") && !l.contains("::"))
        .map(|l| {
            let b = sys.read(format!("/sys/class/leds/{l}/brightness")).unwrap_or_default();
            let m = sys.read(format!("/sys/class/leds/{l}/max_brightness")).unwrap_or_default();
            Device::new(l).prop("Яркость", format!("{b} из {m}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn iio_and_input() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let iio = r.join("sys/bus/iio/devices/iio:device0");
        fs::create_dir_all(&iio).unwrap();
        fs::write(iio.join("name"), "c42d000.qcom,spmi:qcom,pmk8350@0:vadc@3100").unwrap();
        fs::write(iio.join("in_temp_pm8350_die_temp_input"), "35400").unwrap();
        fs::write(iio.join("in_voltage_pm8350b_vbat_sns_input"), "3912000").unwrap();
        fs::create_dir_all(r.join("proc/bus/input")).unwrap();
        fs::write(r.join("proc/bus/input/devices"), "I: Bus=0000\nN: Name=\"fts\"\nH: Handlers=event5\n\nN: Name=\"qcom-hv-haptics\"\nH: Handlers=event2\n").unwrap();
        let sys = Sys::at(r);
        let s = collect(&sys);
        let sensors = s.iter().find(|s| s.id == "sensors").unwrap();
        assert_eq!(sensors.devices[0].name, "АЦП PMIC (vadc@3100)");
        assert!(sensors.devices[0].props.iter().any(|(k, v)| k.starts_with("Температура") && v == "35.4 °C"));
        // Без драйвера батареи — напряжение из АЦП.
        let bat = s.iter().find(|s| s.id == "battery").unwrap();
        assert_eq!(bat.devices[0].props[0].1, "3.912 В");
        let input = s.iter().find(|s| s.id == "input").unwrap();
        assert_eq!(input.devices[0].props[0].1, "Сенсорный экран");
        assert_eq!(input.devices[1].props[0].1, "Вибромотор");
        assert!(report(&s).contains("## Датчики"));
    }
}
