//! Устройства компьютера для гостя, кроме USB по порту (`usb.rs`): PCI через VFIO,
//! клавиатуры/мыши/геймпады напрямую (evdev), целые диски и разделы, последовательные
//! порты, USB по VID:PID, устройства звука PipeWire. Здесь — списки с причинами, почему
//! нельзя, аргументы QEMU, подключение на ходу через QMP и выдача доступа (pkexec).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::model::{HostDev, Vm};

/// Строка списка устройств: что это, можно ли отдать гостю и почему нет.
#[derive(Clone, Debug, PartialEq)]
pub struct Avail {
    pub dev: HostDev,
    /// Раздел: `pci`, `input`, `block`, `serial`, `usb-id`.
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    /// Почему отдать нельзя (пусто — можно).
    pub blocked: String,
    /// Предупреждение (можно, но осторожно).
    pub warn: String,
    /// Нужен доступ (pkexec спросит пароль при подключении).
    pub needs_access: bool,
}

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn access(p: &Path, mode: i32) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

// ─── PCI ────────────────────────────────────────────────────────────────

const PCI: &str = "/sys/bus/pci/devices";

#[derive(Clone, Debug, PartialEq)]
pub struct PciDev {
    pub addr: String,
    pub class: u32,
    pub vendor: String,
    pub device: String,
    pub driver: String,
    pub group: Option<u32>,
    pub name: String,
}

pub fn iommu_on() -> bool {
    std::fs::read_dir("/sys/kernel/iommu_groups").map(|mut d| d.next().is_some()).unwrap_or(false)
}

fn pci_info(addr: &str) -> Option<PciDev> {
    let d = Path::new(PCI).join(addr);
    if !d.exists() {
        return None;
    }
    let class = u32::from_str_radix(read(d.join("class")).trim_start_matches("0x"), 16).unwrap_or(0);
    let driver = std::fs::read_link(d.join("driver")).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())).unwrap_or_default();
    let group = std::fs::read_link(d.join("iommu_group")).ok().and_then(|p| p.file_name()?.to_str()?.parse().ok());
    let vendor = read(d.join("vendor")).trim_start_matches("0x").to_string();
    let device = read(d.join("device")).trim_start_matches("0x").to_string();
    let name = pci_name(addr).unwrap_or_else(|| format!("{} {vendor}:{device}", class_name(class)));
    Some(PciDev { addr: addr.to_string(), class, vendor, device, driver, group, name })
}

/// Имя из `lspci` (база pci.ids), если он есть.
fn pci_name(addr: &str) -> Option<String> {
    let o = std::process::Command::new("lspci").args(["-s", addr, "-mm"]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    // 02:00.0 "VGA compatible controller" "NVIDIA Corporation" "GB203M …" -ra1 …
    let f: Vec<&str> = s.split('"').collect();
    let (maker, model) = (f.get(3)?.trim(), f.get(5)?.trim());
    let maker = maker.split([',', '(']).next().unwrap_or(maker).replace(" Corporation", "").replace(" Electronics Co Ltd", "").replace(" Technology Company", "").trim().to_string();
    Some(format!("{maker} {model}"))
}

fn class_name(class: u32) -> &'static str {
    match class >> 8 {
        0x0300 | 0x0302 | 0x0380 => "Видеокарта",
        0x0108 => "NVMe",
        0x0106 => "SATA",
        0x0200 => "Ethernet",
        0x0280 => "Wi-Fi",
        0x0403 | 0x0401 => "Звук",
        0x0c03 => "USB-контроллер",
        0x0604 => "Мост PCI",
        _ => "PCI",
    }
}

pub fn pci_list() -> Vec<PciDev> {
    let Ok(rd) = std::fs::read_dir(PCI) else { return vec![] };
    let mut v: Vec<PciDev> = rd.flatten().filter_map(|e| pci_info(&e.file_name().to_string_lossy())).collect();
    v.sort_by(|a, b| a.addr.cmp(&b.addr));
    v
}

/// Все устройства группы IOMMU.
pub fn group_members(group: u32) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(format!("/sys/kernel/iommu_groups/{group}/devices")) else { return vec![] };
    let mut v: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// Что из группы уходит гостю вместе с устройством: все функции, кроме мостов.
pub fn pci_bundle(addr: &str) -> Vec<String> {
    let Some(d) = pci_info(addr) else { return vec![addr.to_string()] };
    let Some(g) = d.group else { return vec![addr.to_string()] };
    group_members(g).into_iter().filter(|m| pci_info(m).is_some_and(|p| p.class >> 8 != 0x0604)).collect()
}

/// Процессы, у которых открыт файл с этим префиксом пути (`/dev/nvidia`, `/dev/dri/renderD129`).
fn users_of(prefixes: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let pid = e.file_name().to_string_lossy().into_owned();
        if !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else { continue };
        if fds.flatten().any(|f| std::fs::read_link(f.path()).is_ok_and(|l| prefixes.iter().any(|p| l.to_string_lossy().starts_with(p.as_str())))) {
            let comm = read(e.path().join("comm"));
            if !out.contains(&comm) {
                out.push(comm);
            }
        }
    }
    out
}

/// Блочные устройства на PCI-устройстве (`nvme0n1`).
fn blocks_on_pci(addr: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir("/sys/block") else { return vec![] };
    rd.flatten()
        .filter(|e| std::fs::canonicalize(e.path().join("device")).is_ok_and(|p| p.to_string_lossy().contains(&format!("/{addr}/"))))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Сетевые интерфейсы на PCI-устройстве.
fn nets_on_pci(addr: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else { return vec![] };
    rd.flatten()
        .filter(|e| std::fs::canonicalize(e.path().join("device")).is_ok_and(|p| p.file_name().is_some_and(|f| f == addr)))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Интерфейс с маршрутом по умолчанию.
fn default_route_iface() -> Option<String> {
    read("/proc/net/route").lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.get(1) == Some(&"00000000")).then(|| f[0].to_string())
    })
}

/// Почему PCI-устройство нельзя отдать гостю (пусто — можно) и предупреждение.
pub fn pci_check(addr: &str) -> (String, String) {
    if !iommu_on() {
        return ("IOMMU выключен: добавьте intel_iommu=on iommu=pt (AMD — amd_iommu=on) в параметры ядра".into(), String::new());
    }
    let Some(d) = pci_info(addr) else { return (format!("нет устройства {addr}"), String::new()) };
    let Some(g) = d.group else { return ("устройство вне групп IOMMU".into(), String::new()) };
    let mut warn = Vec::new();
    for m in pci_bundle(addr) {
        let Some(p) = pci_info(&m) else { continue };
        let what = if m == addr { String::new() } else { format!("{m} ({}) из той же группы IOMMU {g}: ", p.name) };
        if p.class >> 16 == 0x06 {
            return (format!("{what}мост или контроллер платформы — без него компьютер не работает"), String::new());
        }
        if read(Path::new(PCI).join(&m).join("boot_vga")) == "1" {
            return (format!("{what}основной видеовыход компьютера — экран погаснет"), String::new());
        }
        if matches!(p.driver.as_str(), "i915" | "xe" | "amdgpu" | "radeon") && p.class >> 8 == 0x0300 {
            // Вторая видеокарта на amdgpu/xe тоже может держать экран.
            let users = users_of(&drm_nodes(&m));
            if !users.is_empty() {
                return (format!("{what}видеокартой пользуются: {}", users.join(", ")), String::new());
            }
        }
        if p.driver == "nvidia" {
            let users = users_of(&["/dev/nvidia".to_string()]);
            if !users.is_empty() {
                return (format!("{what}видеокарта NVIDIA занята: {} — закройте их (или остановите службы) перед запуском машины", users.join(", ")), String::new());
            }
        }
        let root_dev = root_block();
        for b in blocks_on_pci(&m) {
            if root_dev.as_deref().is_some_and(|r| r == b || disk_of(r).as_deref() == Some(b.as_str())) {
                return (format!("{what}на диске {b} стоит эта система"), String::new());
            }
            if !mounted_parts(&b).is_empty() {
                return (format!("{what}разделы диска {b} подключены: {}", mounted_parts(&b).join(", ")), String::new());
            }
            warn.push(format!("диск {b} пропадёт у компьютера"));
        }
        let def = default_route_iface();
        for n in nets_on_pci(&m) {
            if def.as_deref() == Some(n.as_str()) {
                warn.push(format!("через {n} идёт интернет компьютера — он пропадёт"));
            } else {
                warn.push(format!("сеть {n} пропадёт у компьютера"));
            }
        }
        if p.class >> 8 == 0x0c03 && !warn.iter().any(|w: &String| w.starts_with("всё в этом USB")) {
            warn.push("всё в этом USB-контроллере (клавиатура, мышь, телефон) уйдёт гостю".into());
        }
        if p.class >> 8 == 0x0403 && !what.is_empty() {
            continue;
        }
        if !what.is_empty() {
            warn.push(format!("вместе с ним уйдёт {m} ({})", p.name));
        }
    }
    (String::new(), warn.join("; "))
}

fn drm_nodes(addr: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(Path::new(PCI).join(addr).join("drm")) else { return vec![] };
    rd.flatten().map(|e| format!("/dev/dri/{}", e.file_name().to_string_lossy())).collect()
}

/// Подключённые файловые системы: (имя устройства в /sys/class/block, точка, maj:min).
/// Btrfs показывает в maj:min анонимное устройство, поэтому главное — источник (`/dev/…`).
fn mounts() -> Vec<(String, String, String)> {
    let mut v = Vec::new();
    for l in read("/proc/self/mountinfo").lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        let Some(sep) = f.iter().position(|x| *x == "-") else { continue };
        let src = f.get(sep + 2).copied().unwrap_or_default();
        let name = if src.starts_with("/dev/") {
            std::fs::canonicalize(src).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
        } else {
            String::new()
        };
        let mm = f.get(2).copied().unwrap_or_default().to_string();
        let name = if name.is_empty() {
            std::fs::canonicalize(format!("/sys/dev/block/{mm}")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
        } else {
            name
        };
        if !name.is_empty() {
            v.push((name, f.get(4).copied().unwrap_or_default().replace("\\040", " "), mm));
        }
    }
    for l in read("/proc/swaps").lines().skip(1) {
        if let Some(d) = l.split_whitespace().next().filter(|d| d.starts_with("/dev/")) {
            if let Some(n) = std::fs::canonicalize(d).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())) {
                v.push((n, "swap".into(), String::new()));
            }
        }
    }
    v
}

/// Блочные устройства, лежащие под `name` (dm/LUKS/LVM): сам он и все его slaves вниз.
fn underlying(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if let Ok(rd) = std::fs::read_dir(format!("/sys/class/block/{name}/slaves")) {
        for e in rd.flatten() {
            out.extend(underlying(&e.file_name().to_string_lossy()));
        }
    }
    out
}

/// Блочное устройство корня (раздел; под dm — нижний раздел).
fn root_block() -> Option<String> {
    let m = mounts();
    let (name, _, _) = m.iter().find(|(_, mp, _)| mp == "/")?;
    underlying(name).into_iter().last()
}

/// Подключённые разделы диска (или самого раздела): «раздел → точка».
fn mounted_parts(disk: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (name, mp, _) in mounts() {
        for u in underlying(&name) {
            // Раздел диска: nvme0n1p2 для nvme0n1, sdb1 для sdb — по родителю в sysfs.
            let parent = disk_of(&u).unwrap_or_default();
            if u == disk || parent == disk {
                let item = format!("{u} → {mp}");
                if !out.contains(&item) {
                    out.push(item);
                }
            }
        }
    }
    out
}

// ─── Ввод ───────────────────────────────────────────────────────────────

/// Клавиатуры, мыши, геймпады (`/dev/input/by-id/*-event-*`).
pub fn input_list() -> Vec<(PathBuf, String, &'static str)> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir("/dev/input/by-id") else { return v };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let kind = if n.ends_with("-event-kbd") {
            "клавиатура"
        } else if n.ends_with("-event-mouse") {
            "мышь"
        } else if n.ends_with("-event-joystick") {
            "геймпад"
        } else {
            continue;
        };
        let name = n.trim_start_matches("usb-").trim_start_matches("bluetooth-").split("-event").next().unwrap_or(&n).trim_end_matches("-if01").trim_end_matches("-if00").replace('_', " ");
        v.push((e.path(), name, kind));
    }
    v.sort();
    v
}

/// Клавиатура (есть автоповтор, EV_REP): ей нужен захват всех устройств и повтор.
pub fn is_keyboard(path: &str) -> bool {
    let Some(ev) = std::fs::canonicalize(path).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())) else { return false };
    let bits = u64::from_str_radix(read(format!("/sys/class/input/{ev}/device/capabilities/ev")).trim(), 16).unwrap_or(0);
    bits & (1 << 0x14) != 0
}

// ─── Диски ──────────────────────────────────────────────────────────────

/// Диски и разделы компьютера (без loop с нулевым размером, zram, dm).
pub fn block_list() -> Vec<(PathBuf, String, Vec<String>)> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/block") else { return v };
    let root = root_block();
    for e in rd.flatten() {
        let disk = e.file_name().to_string_lossy().into_owned();
        if disk.starts_with("zram") || disk.starts_with("dm-") || disk.starts_with("ram") || read(e.path().join("size")) == "0" {
            continue;
        }
        let mut items = vec![disk.clone()];
        if let Ok(sub) = std::fs::read_dir(e.path()) {
            let mut parts: Vec<String> = sub.flatten().map(|s| s.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&disk) && n != &disk).collect();
            parts.sort();
            items.extend(parts);
        }
        for it in items {
            let sys = if it == disk { e.path() } else { e.path().join(&it) };
            let bytes = read(sys.join("size")).parse::<u64>().unwrap_or(0) * 512;
            let model = read(e.path().join("device/model"));
            let mut notes = Vec::new();
            if root.as_deref().is_some_and(|r| r == it || (it == disk && disk_of(r).as_deref() == Some(disk.as_str()))) {
                notes.push("система".to_string());
            }
            let m = if it == disk { mounted_parts(&disk) } else { mounted_parts_one(&disk, &it) };
            notes.extend(m);
            let title = format!("{} — {}{}", it, human(bytes), if model.is_empty() || it != disk { String::new() } else { format!(", {model}") });
            v.push((PathBuf::from(format!("/dev/{it}")), title, notes));
        }
    }
    v.sort();
    v
}

fn mounted_parts_one(disk: &str, part: &str) -> Vec<String> {
    mounted_parts(disk).into_iter().filter(|m| m.starts_with(&format!("{part} "))).collect()
}

fn human(b: u64) -> String {
    let g = b as f64 / 1e9;
    if g >= 1000.0 {
        format!("{:.1} ТБ", g / 1000.0)
    } else if g >= 1.0 {
        format!("{g:.0} ГБ")
    } else {
        format!("{:.0} МБ", b as f64 / 1e6)
    }
}

/// Почему диск нельзя отдать (пусто — можно), предупреждение.
pub fn block_check(path: &str, readonly: bool) -> (String, String) {
    let name = Path::new(path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let real = std::fs::canonicalize(path).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())).unwrap_or(name);
    let Some(disk) = disk_of(&real) else { return (format!("{path}: не блочное устройство"), String::new()) };
    if root_block().is_some_and(|r| r == real || (real == disk && disk_of(&r).as_deref() == Some(disk.as_str()))) {
        return ("на нём стоит эта система".into(), String::new());
    }
    let m = if real == disk { mounted_parts(&disk) } else { mounted_parts_one(&disk, &real) };
    if !m.is_empty() {
        if readonly {
            return (String::new(), format!("подключён у компьютера ({}) — гость увидит его на момент чтения, без свежих записей", m.join(", ")));
        }
        return (format!("подключён у компьютера: {} — отключите или отдайте только для чтения", m.join(", ")), String::new());
    }
    (String::new(), if readonly { String::new() } else { "гость пишет прямо на диск компьютера — данные на нём может испортить неосторожная установка".into() })
}

fn disk_of(dev: &str) -> Option<String> {
    if Path::new("/sys/block").join(dev).exists() {
        return Some(dev.to_string());
    }
    let p = std::fs::canonicalize(format!("/sys/class/block/{dev}")).ok()?;
    Some(p.parent()?.file_name()?.to_string_lossy().into_owned())
}

// ─── Последовательные порты ────────────────────────────────────────────

pub fn serial_list() -> Vec<(PathBuf, String)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev/serial/by-id") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let real = std::fs::canonicalize(e.path()).map(|p| p.display().to_string()).unwrap_or_default();
            v.push((e.path(), format!("{} ({real})", n.trim_start_matches("usb-").replace('_', " "))));
        }
    }
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if (n.starts_with("ttyUSB") || n.starts_with("ttyACM")) && !v.iter().any(|(_, t)| t.ends_with(&format!("/dev/{n})"))) {
                v.push((e.path(), n));
            }
        }
    }
    v.sort();
    v
}

// ─── Всё вместе ─────────────────────────────────────────────────────────

/// Что можно отдать гостю (для настроек и `--devices`).
pub fn available() -> Vec<Avail> {
    let mut out = Vec::new();
    for p in pci_list() {
        // Мосты, платформа, USB-контроллеры с клавиатурой — не показывать вовсе.
        if p.class >> 16 == 0x06 || p.class >> 8 == 0x0500 || p.class >> 8 == 0x0c05 || p.class >> 8 == 0x0c80 || p.class >> 8 == 0x1180 {
            continue;
        }
        let (blocked, warn) = pci_check(&p.addr);
        let detail = format!("{}{}{}", p.addr, if p.driver.is_empty() { String::new() } else { format!(", драйвер {}", p.driver) }, p.group.map(|g| format!(", группа IOMMU {g}")).unwrap_or_default());
        out.push(Avail { dev: HostDev::Pci { addr: p.addr.clone(), name: p.name.clone() }, kind: "pci", title: p.name, detail, blocked, warn, needs_access: false });
    }
    for (path, name, kind) in input_list() {
        let ok = access(&path, libc::R_OK | libc::W_OK);
        let blocked = if kind == "геймпад" { "QEMU пересылает напрямую только клавиатуру и мышь — геймпад отдайте через USB (по порту или VID:PID)".to_string() } else { String::new() };
        out.push(Avail {
            dev: HostDev::Input { path: path.display().to_string(), name: name.clone() },
            kind: "input",
            title: format!("{name} — {kind}"),
            detail: "переключение компьютер ↔ гость: оба Ctrl".into(),
            blocked,
            warn: if kind == "клавиатура" { "пока устройство у гостя, компьютер его не слышит".into() } else { String::new() },
            needs_access: !ok,
        });
    }
    for (path, title, notes) in block_list() {
        let p = path.display().to_string();
        let (blocked, warn) = block_check(&p, false);
        out.push(Avail {
            dev: HostDev::Block { path: p, readonly: false, name: title.clone() },
            kind: "block",
            title,
            detail: notes.join(", "),
            blocked,
            warn,
            needs_access: !access(&path, libc::R_OK | libc::W_OK),
        });
    }
    for (path, title) in serial_list() {
        out.push(Avail {
            dev: HostDev::Serial { path: path.display().to_string(), name: title.clone() },
            kind: "serial",
            title,
            detail: "гостю — как USB-UART (FTDI)".into(),
            blocked: String::new(),
            warn: String::new(),
            needs_access: !access(&path, libc::R_OK | libc::W_OK),
        });
    }
    for d in crate::usb::list().into_iter().filter(|d| !d.hub) {
        out.push(Avail {
            dev: HostDev::UsbId { vendor: d.vendor.clone(), product: d.product.clone(), name: d.name.clone() },
            kind: "usb-id",
            title: d.name.clone(),
            detail: format!("{}:{} в порту {} — по VID:PID найдётся в любом гнезде", d.vendor, d.product, d.port),
            blocked: String::new(),
            warn: String::new(),
            needs_access: !d.writable,
        });
    }
    out
}

/// Разбор `ТИП:ИД` из CLI: `pci:02:00.0`, `input:/dev/input/by-id/…`, `block:/dev/sdb`,
/// `block-ro:/dev/sdb1`, `serial:/dev/ttyUSB0`, `usb-id:18d1:4ee7`.
pub fn parse_spec(s: &str) -> Result<HostDev> {
    let (k, v) = s.split_once(':').context("ожидается ТИП:ИД, например pci:02:00.0, block:/dev/sdb, usb-id:18d1:4ee7")?;
    Ok(match k {
        "pci" => {
            let addr = if v.matches(':').count() == 1 { format!("0000:{v}") } else { v.to_string() };
            let name = pci_info(&addr).map(|p| p.name).unwrap_or_default();
            HostDev::Pci { addr, name }
        }
        "input" => HostDev::Input { path: v.into(), name: Path::new(v).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() },
        "block" | "block-ro" | "disk" => HostDev::Block { path: v.into(), readonly: k == "block-ro", name: String::new() },
        "serial" | "tty" => HostDev::Serial { path: v.into(), name: String::new() },
        "usb-id" | "usbid" => {
            let (vid, pid) = v.split_once(':').context("usb-id:VID:PID")?;
            let name = crate::usb::list().into_iter().find(|d| d.vendor == vid && d.product == pid).map(|d| d.name).unwrap_or_default();
            HostDev::UsbId { vendor: vid.to_lowercase(), product: pid.to_lowercase(), name }
        }
        _ => bail!("неизвестный тип «{k}»: pci, input, block, block-ro, serial, usb-id (USB по порту — usb:3-6)"),
    })
}

/// Проверка перед подключением: причина отказа или Ok(предупреждение).
pub fn check(d: &HostDev) -> Result<String> {
    let (blocked, warn) = match d {
        HostDev::Pci { addr, .. } => pci_check(addr),
        HostDev::Block { path, readonly, .. } => block_check(path, *readonly),
        HostDev::Input { path, .. } | HostDev::Serial { path, .. } => {
            if !Path::new(path).exists() {
                (format!("нет {path}"), String::new())
            } else {
                (String::new(), String::new())
            }
        }
        HostDev::UsbId { .. } => (String::new(), String::new()),
    };
    if !blocked.is_empty() {
        bail!("{}: {blocked}", d.spec());
    }
    Ok(warn)
}

// ─── QEMU ───────────────────────────────────────────────────────────────

/// Аргументы QEMU для устройства при запуске.
pub fn args(d: &HostDev) -> Vec<String> {
    let id = d.qemu_id();
    let s = |x: &str| x.to_string();
    match d {
        HostDev::UsbId { vendor, product, .. } => vec![s("-device"), format!("usb-host,bus=xhci.0,vendorid=0x{vendor},productid=0x{product},id={id}")],
        HostDev::Pci { addr, .. } => {
            let mut v = Vec::new();
            for (i, m) in pci_bundle(addr).into_iter().enumerate() {
                v.push(s("-device"));
                v.push(format!("vfio-pci,host={m},id={id}-{i}"));
            }
            v
        }
        HostDev::Input { path, .. } => {
            let kbd = is_keyboard(path);
            vec![s("-object"), format!("input-linux,id={id},evdev={},repeat={},grab_all={},grab-toggle=ctrl-ctrl", path.replace(',', ",,"), if kbd { "on" } else { "off" }, if kbd { "on" } else { "off" })]
        }
        HostDev::Block { path, readonly, .. } => {
            vec![
                s("-blockdev"),
                json!({"driver": "raw", "node-name": id, "read-only": readonly, "file": {"driver": "host_device", "filename": path, "aio": "threads", "cache": {"direct": true}}}).to_string(),
                s("-device"),
                format!("virtio-blk-pci,drive={id},id={id}-dev"),
            ]
        }
        HostDev::Serial { path, .. } => vec![s("-chardev"), format!("serial,id={id}-c,path={}", path.replace(',', ",,")), s("-device"), format!("usb-serial,bus=xhci.0,chardev={id}-c,id={id}")],
    }
}

/// Подключить на ходу.
pub fn hot_attach(vm: &Vm, d: &HostDev) -> Result<()> {
    let id = d.qemu_id();
    let mut c = crate::qmp::Conn::of(vm)?;
    match d {
        HostDev::Pci { .. } => bail!("PCI-устройство подключается только при выключенной машине — запустите её снова"),
        HostDev::UsbId { vendor, product, .. } => {
            c.call("device_add", json!({"driver": "usb-host", "bus": "xhci.0", "vendorid": u32::from_str_radix(vendor, 16)?, "productid": u32::from_str_radix(product, 16)?, "id": id}))?;
        }
        HostDev::Input { path, .. } => {
            let kbd = is_keyboard(path);
            c.call("object-add", json!({"qom-type": "input-linux", "id": id, "evdev": path, "repeat": kbd, "grab_all": kbd, "grab-toggle": "ctrl-ctrl"}))?;
        }
        HostDev::Block { path, readonly, .. } => {
            let bus = free_hotplug_port(&mut c)?;
            c.call("blockdev-add", json!({"driver": "raw", "node-name": id, "read-only": readonly, "file": {"driver": "host_device", "filename": path, "aio": "threads", "cache": {"direct": true}}}))?;
            if let Err(e) = c.call("device_add", json!({"driver": "virtio-blk-pci", "drive": id, "id": format!("{id}-dev"), "bus": bus})) {
                let _ = c.call("blockdev-del", json!({"node-name": id}));
                return Err(e);
            }
        }
        HostDev::Serial { path, .. } => {
            c.call("chardev-add", json!({"id": format!("{id}-c"), "backend": {"type": "serial", "data": {"device": path}}}))?;
            if let Err(e) = c.call("device_add", json!({"driver": "usb-serial", "bus": "xhci.0", "chardev": format!("{id}-c"), "id": id})) {
                let _ = c.call("chardev-remove", json!({"id": format!("{id}-c")}));
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Отключить на ходу.
pub fn hot_detach(vm: &Vm, d: &HostDev) -> Result<()> {
    let id = d.qemu_id();
    let mut c = crate::qmp::Conn::of(vm)?;
    match d {
        HostDev::Pci { .. } => bail!("PCI-устройство отключается только при выключенной машине"),
        HostDev::UsbId { .. } => {
            c.call("device_del", json!({"id": id}))?;
        }
        HostDev::Input { .. } => {
            c.call("object-del", json!({"id": id}))?;
        }
        HostDev::Block { .. } => {
            c.call("device_del", json!({"id": format!("{id}-dev")}))?;
            // Гость отпускает устройство не сразу (ACPI-извлечение).
            if !wait_gone(&mut c, &format!("{id}-dev"), Duration::from_secs(15)) {
                bail!("гость не отпустил диск за 15 с — размонтируйте его в госте и повторите");
            }
            c.call("blockdev-del", json!({"node-name": id}))?;
        }
        HostDev::Serial { .. } => {
            c.call("device_del", json!({"id": id}))?;
            wait_gone(&mut c, &id, Duration::from_secs(5));
            c.call("chardev-remove", json!({"id": format!("{id}-c")}))?;
        }
    }
    Ok(())
}

fn wait_gone(c: &mut crate::qmp::Conn, id: &str, timeout: Duration) -> bool {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout {
        let path = format!("/machine/peripheral/{id}");
        if c.call("qom-list", json!({"path": path})).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Свободный запасной порт PCIe для горячего подключения (`hp0`…).
fn free_hotplug_port(c: &mut crate::qmp::Conn) -> Result<String> {
    let pci = c.call("query-pci", Value::Null)?;
    for bus in pci.as_array().into_iter().flatten() {
        for d in bus.get("devices").and_then(Value::as_array).into_iter().flatten() {
            let id = d.get("qdev_id").and_then(Value::as_str).unwrap_or_default();
            let empty = d.pointer("/pci_bridge/devices").and_then(Value::as_array).is_none_or(|v| v.is_empty());
            if id.starts_with("hp") && empty {
                return Ok(id.to_string());
            }
        }
    }
    bail!("нет свободного разъёма для горячего подключения диска — отдайте диск при выключенной машине")
}

/// Сколько запасных портов PCIe добавлять каждой машине.
pub const HOTPLUG_PORTS: usize = 3;

// ─── Доступ ─────────────────────────────────────────────────────────────

/// Узлы, к которым у пользователя нет доступа, для устройств машины.
pub fn missing_access(devs: &[HostDev]) -> Vec<String> {
    let mut v = Vec::new();
    for d in devs {
        match d {
            HostDev::Input { path, .. } | HostDev::Block { path, .. } | HostDev::Serial { path, .. } => {
                let real = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
                if !access(&real, libc::R_OK | libc::W_OK) {
                    v.push(real.display().to_string());
                }
            }
            HostDev::UsbId { vendor, product, .. } => {
                for u in crate::usb::list().into_iter().filter(|u| &u.vendor == vendor && &u.product == product && !u.writable) {
                    if let Some(n) = usb_node(&u.port) {
                        v.push(n);
                    }
                }
            }
            HostDev::Pci { .. } => {}
        }
    }
    v
}

fn usb_node(port: &str) -> Option<String> {
    let d = Path::new("/sys/bus/usb/devices").join(port);
    let bus: u32 = read(d.join("busnum")).parse().ok()?;
    let dev: u32 = read(d.join("devnum")).parse().ok()?;
    Some(format!("/dev/bus/usb/{bus:03}/{dev:03}"))
}

/// Выдать себе чтение и запись узлов (ACL до переподключения устройства) — pkexec.
pub fn grant(nodes: &[String]) -> Result<()> {
    if nodes.is_empty() {
        return Ok(());
    }
    for n in nodes {
        if !n.starts_with("/dev/") || n.contains(['\'', ' ', ';', '`', '$']) {
            bail!("недопустимый путь {n}");
        }
    }
    let user = std::env::var("USER").unwrap_or_else(|_| unsafe { libc::getuid() }.to_string());
    let st = std::process::Command::new("pkexec").arg("setfacl").arg("-m").arg(format!("u:{user}:rw")).args(nodes).status().context("pkexec")?;
    if !st.success() {
        bail!("доступ к {} не выдан (pkexec: {})", nodes.join(", "), st.code().unwrap_or(-1));
    }
    Ok(())
}

// ─── VFIO ───────────────────────────────────────────────────────────────

/// Адреса PCI машины (со всеми функциями групп).
pub fn vfio_addrs(vm: &Vm) -> Vec<String> {
    let mut v = Vec::new();
    for d in &vm.devices {
        if let HostDev::Pci { addr, .. } = d {
            for m in pci_bundle(addr) {
                if !v.contains(&m) {
                    v.push(m);
                }
            }
        }
    }
    v
}

/// Привязать устройства к vfio-pci (через pkexec — сам syn-vm в роли помощника) и поднять
/// предел закреплённой памяти этому процессу, чтобы QEMU мог закрепить память гостя.
pub fn vfio_prepare(vm: &Vm) -> Result<()> {
    let addrs = vfio_addrs(vm);
    if addrs.is_empty() {
        return Ok(());
    }
    for a in &addrs {
        let (blocked, _) = pci_check(a);
        if !blocked.is_empty() {
            bail!("{a}: {blocked}");
        }
    }
    let all_bound = addrs.iter().all(|a| pci_info(a).is_some_and(|p| p.driver == "vfio-pci"));
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut lim) };
    let need = (vm.memory_mb as u64 + 512) << 20;
    let groups_ok = addrs.iter().filter_map(|a| pci_info(a)?.group).all(|g| access(Path::new(&format!("/dev/vfio/{g}")), libc::R_OK | libc::W_OK));
    if all_bound && groups_ok && lim.rlim_cur >= need {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let st = std::process::Command::new("pkexec")
        .arg(exe)
        .arg("--vfio-helper")
        .arg("bind")
        .arg(std::process::id().to_string())
        .arg(need.to_string())
        .args(&addrs)
        .status()
        .context("pkexec")?;
    if !st.success() {
        bail!("не удалось привязать {} к vfio-pci (pkexec: {})", addrs.join(", "), st.code().unwrap_or(-1));
    }
    Ok(())
}

/// Вернуть устройства драйверам компьютера.
pub fn vfio_release(addrs: &[String]) -> Result<()> {
    let bound: Vec<&String> = addrs.iter().filter(|a| pci_info(a).is_some_and(|p| p.driver == "vfio-pci")).collect();
    if bound.is_empty() {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let st = std::process::Command::new("pkexec").arg(exe).arg("--vfio-helper").arg("unbind").arg("0").arg("0").args(bound).status()?;
    if !st.success() {
        bail!("pkexec: {}", st.code().unwrap_or(-1));
    }
    Ok(())
}

/// Ждать, пока QEMU машины завершится, и вернуть PCI-устройства компьютеру.
pub fn vfio_watch(vm: &Vm) {
    let addrs = vfio_addrs(vm);
    if addrs.is_empty() {
        return;
    }
    while crate::qemu::pid(vm).is_some() {
        std::thread::sleep(Duration::from_secs(2));
    }
    if let Err(e) = vfio_release(&addrs) {
        tracing::warn!("vfio: {e}");
    }
}

/// Помощник под root (`pkexec syn-vm --vfio-helper bind|unbind PID MEMLOCK АДРЕС…`).
pub fn vfio_helper(args: &[String]) -> Result<()> {
    let [op, pid, memlock, addrs @ ..] = args else { bail!("--vfio-helper bind|unbind PID MEMLOCK ADDR…") };
    let uid: u32 = std::env::var("PKEXEC_UID").ok().and_then(|s| s.parse().ok()).context("только через pkexec")?;
    for a in addrs {
        let ok = a.len() == 12 && a.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        if !ok || !Path::new(PCI).join(a).exists() {
            bail!("неверный адрес PCI {a}");
        }
    }
    let w = |p: PathBuf, v: &str| std::fs::write(&p, v).with_context(|| format!("{} ← {v}", p.display()));
    match op.as_str() {
        "bind" => {
            let _ = std::process::Command::new("modprobe").arg("vfio-pci").status();
            for a in addrs {
                let d = Path::new(PCI).join(a);
                w(d.join("driver_override"), "vfio-pci")?;
                if d.join("driver").exists() && pci_info(a).is_some_and(|p| p.driver != "vfio-pci") {
                    w(d.join("driver/unbind"), a)?;
                }
                if pci_info(a).is_some_and(|p| p.driver != "vfio-pci") {
                    w(PathBuf::from("/sys/bus/pci/drivers_probe"), a)?;
                }
                let g = pci_info(a).and_then(|p| p.group).context("группа IOMMU")?;
                let node = PathBuf::from(format!("/dev/vfio/{g}"));
                for _ in 0..50 {
                    if node.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                use std::os::unix::ffi::OsStrExt;
                let c = std::ffi::CString::new(node.as_os_str().as_bytes())?;
                unsafe { libc::chown(c.as_ptr(), uid, u32::MAX) };
            }
            let pid: i32 = pid.parse()?;
            let lim: u64 = memlock.parse()?;
            if pid > 0 && lim > 0 {
                let r = libc::rlimit { rlim_cur: lim, rlim_max: lim };
                if unsafe { libc::prlimit(pid, libc::RLIMIT_MEMLOCK, &r, std::ptr::null_mut()) } != 0 {
                    bail!("prlimit: {}", std::io::Error::last_os_error());
                }
            }
        }
        "unbind" => {
            for a in addrs {
                let d = Path::new(PCI).join(a);
                w(d.join("driver_override"), "\n")?;
                if pci_info(a).is_some_and(|p| p.driver == "vfio-pci") {
                    w(d.join("driver/unbind"), a)?;
                }
                w(PathBuf::from("/sys/bus/pci/drivers_probe"), a)?;
            }
        }
        _ => bail!("bind|unbind"),
    }
    Ok(())
}

// ─── Звук ───────────────────────────────────────────────────────────────

/// Устройства PipeWire/PulseAudio: (имя узла, описание) для вывода (`sinks`) или ввода (`sources`).
pub fn audio_nodes(sources: bool) -> Vec<(String, String)> {
    let Ok(o) = std::process::Command::new("pactl").args(["-f", "json", "list", if sources { "sources" } else { "sinks" }]).output() else { return vec![] };
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            let name = n.get("name")?.as_str()?.to_string();
            if sources && name.ends_with(".monitor") {
                return None;
            }
            Some((name, n.get("description").and_then(Value::as_str).unwrap_or_default().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_roundtrip() {
        for s in ["usb-id:18d1:4ee7", "block-ro:/dev/sdb1", "serial:/dev/ttyUSB0", "input:/dev/input/by-id/x-event-kbd"] {
            assert_eq!(parse_spec(s).unwrap().spec(), s);
        }
        assert_eq!(parse_spec("pci:02:00.0").unwrap().spec(), "pci:0000:02:00.0");
        assert!(parse_spec("nope").is_err());
        let a = parse_spec("block:/dev/sdb").unwrap();
        let b = parse_spec("block-ro:/dev/sdb").unwrap();
        assert!(a.same(&b));
        assert_eq!(a.qemu_id(), b.qemu_id());
    }
}
