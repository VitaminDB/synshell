//! Оборудование для «Параметров → Оборудование»: разделы (процессор,
//! память, аккумулятор, графика, дисплей, датчики, температуры, хранилище,
//! сеть, USB, ввод, звук, камеры, светодиоды) с устройствами и их
//! свойствами. Только чтение sysfs/procfs — работает и без udev, как на
//! телефоне с Android-ядром.

use crate::{backlight, battery, cpu, gpu, memory, thermal, Sys};
use synshell_tr::t;
use synshell_tr::n_;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub name: String,
    pub props: Vec<(String, String)>,
    /// Неисправность (устройство есть, но не работает) — показывается
    /// предупреждением.
    pub fault: Option<String>,
}

impl Device {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), props: Vec::new(), fault: None }
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
        section("cpu", n_!("Процессор"), "\u{E30D}", cpu_devices(sys)),
        section("memory", n_!("Память"), "\u{E322}", memory_devices(sys)),
        section("battery", n_!("Аккумулятор"), "\u{E1A4}", battery_devices(sys)),
        section("gpu", n_!("Графика"), "\u{E30A}", gpu_devices(sys)),
        section("display", n_!("Дисплей"), "\u{E30C}", display_devices(sys)),
        section("sensors", n_!("Датчики"), "\u{E51E}", iio_devices(sys)),
        section("thermal", n_!("Температуры"), "\u{E1FF}", thermal_devices(sys)),
        section("storage", n_!("Хранилище"), "\u{E1DB}", storage_devices(sys)),
        section("network", n_!("Сеть"), "\u{E80D}", net_devices(sys)),
        section("usb", "USB", "\u{E1E0}", usb_devices(sys)),
        section("input", n_!("Устройства ввода"), "\u{E312}", input_devices(sys)),
        section("sound", n_!("Звук"), "\u{E050}", sound_devices(sys)),
        section("camera", n_!("Камеры"), "\u{E3AF}", camera_devices(sys)),
        section("leds", n_!("Светодиоды"), "\u{E0F0}", led_devices(sys)),
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
            if let Some(f) = &d.fault {
                s += &t!("    НЕИСПРАВНОСТЬ: {f}\n", f = f);
            }
            for (k, v) in &d.props {
                s += &format!("    {k}: {v}\n");
            }
        }
        s.push('\n');
    }
    s
}

/// Состояния sysfs по-русски (DRM-коннектор, сеть, USB-контроллер); незнакомое — как есть.
fn ru_state(v: &str) -> String {
    match v.trim() {
        "enabled" => t!("да"),
        "disabled" => t!("нет"),
        "up" => t!("подключено"),
        "down" => t!("отключено"),
        "dormant" => t!("ожидание"),
        "unknown" | "UNKNOWN" => t!("неизвестно"),
        "configured" => t!("подключён к компьютеру"),
        "not attached" => t!("не подключён"),
        "attached" | "powered" | "default" | "addressed" => t!("подключается"),
        "suspended" => t!("сон"),
        "high-speed" => t!("USB 2.0 (480 Мбит/с)"),
        "full-speed" => t!("USB 1.1 (12 Мбит/с)"),
        "low-speed" => t!("USB 1.0 (1,5 Мбит/с)"),
        "super-speed" => t!("USB 3 (5 Гбит/с)"),
        "super-speed-plus" => t!("USB 3 (10 Гбит/с)"),
        o => return o.to_string(),
    }
    .to_string()
}

fn mhz(v: Option<u32>) -> Option<String> {
    v.map(|m| t!("{m} МГц", m = m))
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
        Device::new(cpu::model(sys).unwrap_or_else(|| t!("Процессор").into()))
            .opt(&t!("Устройство"), cpu::device_model(sys))
            .prop(&t!("Архитектура"), arch)
            .prop(&t!("Ядер"), cores.to_string()),
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
                Device::new(t!("Кластер {v} ({n} ядер)", v = i + 1, n = c.cpus.len()))
                    .prop(&t!("Ядра"), range)
                    .opt(&t!("Частота"), c.min_mhz.zip(c.max_mhz).map(|(a, b)| t!("{a}–{b} МГц", a = a, b = b)))
                    .opt(&t!("Регулятор"), c.governor.clone()),
            );
        }
    } else if let Some(c) = clusters.first() {
        v[0] = v[0].clone().opt(&t!("Частота"), c.min_mhz.zip(c.max_mhz).map(|(a, b)| t!("{a}–{b} МГц", a = a, b = b))).opt(&t!("Регулятор"), c.governor.clone());
    }
    v
}

fn memory_devices(sys: &Sys) -> Vec<Device> {
    let Some(m) = memory::read(sys) else { return Vec::new() };
    let mut d = Device::new(t!("Оперативная память"))
        .prop(&t!("Всего"), memory::human_kb(m.total_kb))
        .prop(&t!("Доступно"), memory::human_kb(m.available_kb));
    if m.swap_total_kb > 0 {
        d = d.prop(&t!("Подкачка"), t!("{v} из {v2}", v = memory::human_kb(m.swap_used_kb()), v2 = memory::human_kb(m.swap_total_kb)));
    }
    let mut v = vec![d];
    // zram (телефоны): сжатая подкачка в памяти.
    for z in sys.list("/sys/block").into_iter().filter(|b| b.starts_with("zram")) {
        let size = sys.read_num::<u64>(format!("/sys/block/{z}/disksize")).unwrap_or(0);
        if size > 0 {
            v.push(Device::new(z.clone()).prop(&t!("Размер"), memory::human_kb(size / 1024)).opt(&t!("Сжатие"), sys.read(format!("/sys/block/{z}/comp_algorithm"))));
        }
    }
    v
}

fn battery_devices(sys: &Sys) -> Vec<Device> {
    let mut v: Vec<Device> = battery::batteries(sys)
        .into_iter()
        .map(|b| {
            Device::new(b.model.clone().unwrap_or_else(|| b.name.clone()))
                .opt(&t!("Производитель"), b.manufacturer.clone())
                .prop(&t!("Состояние"), b.status.clone())
                .opt(&t!("Заряд"), b.percent.map(|p| format!("{p}%")))
                .opt(&t!("Напряжение"), b.voltage_v.map(|x| t!("{x} В", x = format!("{:.2}", x))))
                .opt(&t!("Ток"), b.current_a.map(|x| t!("{x} А", x = format!("{:.2}", x))))
                .opt(&t!("Мощность"), b.power_w.map(|x| t!("{x} Вт", x = format!("{:.1}", x))))
                .opt(&t!("Температура"), b.temp_c.map(|x| format!("{x:.1} °C")))
                .opt(&t!("Здоровье"), b.health.clone())
                .opt(&t!("Износ"), b.wear_percent().map(|w| t!("{w}% от паспортной ёмкости", w = w)))
                .opt(&t!("Циклов"), b.cycles.map(|c| c.to_string()))
                .opt(&t!("Технология"), b.technology.clone())
        })
        .collect();
    // Без драйвера батареи (телефон без ADSP) — напряжение из IIO АЦП PMIC.
    if v.is_empty() {
        for dev in sys.list("/sys/bus/iio/devices") {
            let base = format!("/sys/bus/iio/devices/{dev}");
            for f in sys.list(&base) {
                if f.contains("vbat") && f.ends_with("_input") {
                    if let Some(uv) = sys.read_num::<f64>(format!("{base}/{f}")) {
                        v.push(Device::new(t!("Аккумулятор (АЦП PMIC)")).prop(&t!("Напряжение"), t!("{v} В", v = format!("{:.3}", uv / 1e6))).prop(&t!("Источник"), f));
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
                .opt(&t!("Частота"), mhz(g.cur_mhz))
                .opt(&t!("Максимум"), mhz(g.max_mhz))
                .opt(&t!("Загрузка"), g.busy_percent.map(|b| format!("{b:.0}%"))),
        );
    }
    for card in sys.list("/sys/class/drm").into_iter().filter(|c| c.starts_with("card") && !c.contains('-')) {
        let uevent = sys.read(format!("/sys/class/drm/{card}/device/uevent")).unwrap_or_default();
        let driver = uevent.lines().find_map(|l| l.strip_prefix("DRIVER=")).map(String::from);
        let dev = Device::new(card.clone()).opt(&t!("Драйвер"), driver).opt("PCI", uevent.lines().find_map(|l| l.strip_prefix("PCI_ID=")).map(String::from));
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
        v.push(Device::new(name).prop(&t!("Режимы"), modes.lines().take(4).collect::<Vec<_>>().join(", ")).opt(&t!("Включён"), sys.read(format!("{base}/enabled")).map(|v| ru_state(&v))));
    }
    for b in backlight::list(sys) {
        v.push(Device::new(t!("Подсветка {name}", name = b.name)).prop(&t!("Яркость"), t!("{brightness} из {max} ({percent}%)", brightness = b.brightness, max = b.max, percent = format!("{:.0}", b.percent()))).prop(&t!("Тип"), b.kind));
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
        return t!("АЦП PMIC ({v})", v = n.rsplit(':').next().unwrap_or(n));
    }
    n.to_string()
}

fn channel_label(ch: &str) -> String {
    let ch = ch.strip_prefix("in_").unwrap_or(ch);
    let (kind, rest) = ch.split_once('_').unwrap_or((ch, ""));
    let kind_ru = match kind {
        "temp" => t!("Температура"),
        "voltage" => t!("Напряжение"),
        "current" => t!("Ток"),
        "accel" => t!("Ускорение"),
        "anglvel" => t!("Угловая скорость"),
        "magn" => t!("Магнитное поле"),
        "illuminance" => t!("Освещённость"),
        "proximity" => t!("Приближение"),
        "pressure" => t!("Давление"),
        "humidityrelative" => t!("Влажность"),
        other => other.to_string(),
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
        if v.abs() > 100_000.0 { t!("{v} В", v = format!("{:.3}", v / 1e6)) } else { t!("{v} В", v = format!("{:.3}", v / 1000.0)) }
    } else if ch.starts_with("current") {
        if v.abs() > 100_000.0 { t!("{v} А", v = format!("{:.3}", v / 1e6)) } else { t!("{v} мА", v = format!("{:.0}", v)) }
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
    let mut d = Device::new(t!("Датчики температуры"));
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
            &t!("Диск")
        };
        v.push(Device::new(format!("{b} — {}", format!("{vendor} {model}").trim())).prop(&t!("Тип"), kind).prop(&t!("Объём"), t!("{gb} ГБ", gb = format!("{:.1}", gb))));
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
                .prop(&t!("Тип"), if wifi { "Wi-Fi".to_string() } else if n.starts_with("usb") || n.starts_with("rndis") { t!("USB-сеть") } else { t!("Сеть") })
                .opt(&t!("Состояние"), sys.read(format!("{base}/operstate")).map(|v| ru_state(&v)))
                .opt("MAC", sys.read(format!("{base}/address")))
                .opt(&t!("Скорость"), sys.read_num::<i64>(format!("{base}/speed")).filter(|s| *s > 0).map(|s| t!("{s} Мбит/с", s = s)))
                .opt(&t!("Драйвер"), driver),
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
                .opt(&t!("Производитель"), sys.read(format!("{base}/manufacturer")))
                .opt("ID", sys.read(format!("{base}/idVendor")).zip(sys.read(format!("{base}/idProduct"))).map(|(a, b)| format!("{a}:{b}")))
                .opt(&t!("Скорость"), sys.read(format!("{base}/speed")).map(|s| t!("{s} Мбит/с", s = s))),
        );
    }
    // Режим USB-контроллера телефона (device — гаджет USB-сети).
    for udc in sys.list("/sys/class/udc") {
        v.push(Device::new(t!("Контроллер {udc}", udc = udc)).opt(&t!("Состояние"), sys.read(format!("/sys/class/udc/{udc}/state")).map(|v| ru_state(&v))).opt(&t!("Скорость"), sys.read(format!("/sys/class/udc/{udc}/current_speed")).map(|v| ru_state(&v))));
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
            t!("Сенсорный экран")
        } else if name.contains("haptic") {
            t!("Вибромотор")
        } else if name.contains("pwrkey") || name.contains("resin") || name.contains("keys") {
            t!("Кнопки")
        } else if handlers.contains("mouse") {
            t!("Мышь / тачпад")
        } else if handlers.contains("kbd") {
            t!("Клавиатура")
        } else {
            t!("Ввод")
        };
        v.push(Device::new(name).prop(&t!("Тип"), kind).prop(&t!("Обработчики"), handlers));
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

/// Состояние камер от пробы платформы (`/run/camera/status.json`, служба
/// syncamd при старте): камеры HAL и модули по слотам с итогом пробы датчика.
/// Без файла — узлы video4linux.
fn camera_devices(sys: &Sys) -> Vec<Device> {
    if let Some(v) = sys.read("/run/camera/status.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        return camera_modules(&v);
    }
    sys.list("/sys/class/video4linux")
        .into_iter()
        .filter_map(|v| {
            let name = sys.read(format!("/sys/class/video4linux/{v}/name"))?;
            Some(Device::new(name).prop(&t!("Устройство"), format!("/dev/{v}")))
        })
        .collect()
}

/// Роль модуля по имени (`diting_sunny_s5khm6_wide` → основная, датчик s5khm6).
fn camera_role(name: &str) -> (String, Option<&str>) {
    let role = match name.rsplit('_').next().unwrap_or("") {
        "wide" | "main" | "rear" => t!("Основная камера"),
        "front" => t!("Фронтальная камера"),
        "ultra" | "uw" | "ultrawide" => t!("Широкоугольная камера"),
        "macro" => t!("Макрокамера"),
        "tele" | "telephoto" => t!("Телекамера"),
        "depth" => t!("Камера глубины"),
        _ => t!("Камера"),
    };
    let parts: Vec<&str> = name.split('_').collect();
    // <устройство>_<производитель модуля>_<датчик>_<роль>
    let sensor = (parts.len() >= 4).then(|| parts[parts.len() - 2]);
    (role, sensor)
}

fn camera_modules(v: &serde_json::Value) -> Vec<Device> {
    let empty = Vec::new();
    let cams = v["cameras"].as_array().unwrap_or(&empty);
    let mods = v["modules"].as_array().unwrap_or(&empty);
    // Камеры HAL: сначала физические по слотам рабочих модулей, за ними логические (мультикамера).
    let mut next_cam = cams.iter();
    let mut out = Vec::new();
    for m in mods {
        let name = m["name"].as_str().unwrap_or("");
        let (role, sensor) = camera_role(name);
        let mut d = Device::new(role).opt(&t!("Датчик"), sensor.map(str::to_uppercase)).prop(&t!("Слот"), m["slot"].to_string());
        let hex = |k: &str| m[k].as_u64().map(|x| format!("0x{x:X}"));
        match m["status"].as_str().unwrap_or("") {
            "ok" => {
                d = d.prop(&t!("Состояние"), t!("работает"));
                if let Some(c) = next_cam.next() {
                    let (w, h) = (c["width"].as_u64().unwrap_or(0), c["height"].as_u64().unwrap_or(0));
                    if w > 0 {
                        d = d.prop(&t!("Наибольший кадр"), t!("{w}×{h} ({v} Мп)", w = w, h = h, v = format!("{:.1}", (w * h) as f64 / 1e6)));
                    }
                    if c["flash"].as_bool() == Some(true) {
                        d = d.prop(&t!("Вспышка"), t!("есть"));
                    }
                    d = d.prop(&t!("Камера"), c["id"].to_string());
                }
            }
            "mismatch" => {
                let (r, e) = (hex("read_id").unwrap_or_default(), hex("expected_id").unwrap_or_default());
                d = d.prop(&t!("Состояние"), t!("не опознана"));
                d.fault = Some(t!("датчик отвечает id {r} вместо {e}: стоит другой модуль, его описания нет в прошивке", r = r, e = e));
            }
            _ => {
                d = d.prop(&t!("Состояние"), t!("неисправна"));
                d.fault = Some(t!("датчик не отвечает (нет ответа по шине камеры)").into());
            }
        }
        out.push(d);
    }
    out
}

fn led_devices(sys: &Sys) -> Vec<Device> {
    sys.list("/sys/class/leds")
        .into_iter()
        .filter(|l| !l.starts_with("mmc") && !l.contains("::"))
        .map(|l| {
            let b = sys.read(format!("/sys/class/leds/{l}/brightness")).unwrap_or_default();
            let m = sys.read(format!("/sys/class/leds/{l}/max_brightness")).unwrap_or_default();
            Device::new(l).prop(&t!("Яркость"), t!("{b} из {m}", b = b, m = m))
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

    #[test]
    fn camera_status() {
        let v: serde_json::Value = serde_json::from_str(r#"{"cameras":[{"id":0,"facing":"front","orientation":270,"width":2592,"height":1952,"flash":false},{"id":1,"facing":"back","orientation":90,"width":3264,"height":2448,"flash":true}],"modules":[{"slot":0,"name":"diting_sunny_s5khm6_wide","status":"mismatch","read_id":7025,"expected_id":6870},{"slot":1,"name":"diting_sunny_imx596_front","status":"ok"},{"slot":2,"name":"diting_sunny_s5k4h7_ultra","status":"ok"},{"slot":3,"name":"diting_ofilm_gc02m1_macro","status":"no_response","read_id":0,"expected_id":736}]}"#).unwrap();
        let d = camera_modules(&v);
        assert_eq!(d[0].name, "Основная камера");
        assert!(d[0].fault.as_deref().unwrap().contains("0x1B71 вместо 0x1AD6"));
        assert_eq!(d[1].name, "Фронтальная камера");
        assert!(d[1].fault.is_none() && d[1].props.iter().any(|(k, v)| k == "Камера" && v == "0"));
        assert!(d[2].props.iter().any(|(k, v)| k == "Вспышка" && v == "есть"));
        assert_eq!(d[3].name, "Макрокамера");
        assert!(d[3].fault.is_some());
    }
}
