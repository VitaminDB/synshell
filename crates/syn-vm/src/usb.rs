//! USB-устройства компьютера для проброса в гостя. Устройство привязывается к **физическому порту**
//! (`3-6`, `1-2.4` — имя в `/sys/bus/usb/devices`), а не к VID:PID: телефон, который переходит
//! Android → fastboot → recovery, меняет VID:PID и номер устройства, но остаётся в том же порту,
//! и QEMU (`usb-host,hostbus=,hostport=`) подхватывает его снова сам.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use syngui::prelude::*;

const SYS: &str = "/sys/bus/usb/devices";
/// Правило udev, которое открывает порты, выбранные для проброса, на чтение и запись.
pub const UDEV_RULES: &str = "/etc/udev/rules.d/70-syn-vm-usb.rules";

/// Устройство, подключённое к порту сейчас.
#[derive(Clone, Debug, PartialEq)]
pub struct HostDevice {
    /// Порт: `шина-путь` (`3-6`, `1-2.4`).
    pub port: String,
    pub vendor: String,
    pub product: String,
    /// «Производитель Модель» или VID:PID, если строк нет.
    pub name: String,
    /// Узел `/dev/bus/usb/…` доступен на запись — QEMU откроет его без root.
    pub writable: bool,
    /// Концентратор (их не пробрасывают).
    pub hub: bool,
}

/// Порт → (`hostbus`, `hostport`) для QEMU.
pub fn split_port(port: &str) -> Option<(u32, &str)> {
    let (bus, path) = port.split_once('-')?;
    let bus = bus.parse().ok()?;
    if path.is_empty() || !path.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some((bus, path))
}

/// Идентификатор устройства в QEMU для порта (`usb-3_6`).
pub fn qemu_id(port: &str) -> String {
    format!("usb-{}", port.replace(['-', '.'], "_"))
}

fn read(dir: &Path, f: &str) -> String {
    std::fs::read_to_string(dir.join(f)).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Все устройства, кроме корневых концентраторов, по порядку портов.
pub fn list() -> Vec<HostDevice> {
    let Ok(rd) = std::fs::read_dir(SYS) else { return vec![] };
    let mut out: Vec<HostDevice> = rd
        .flatten()
        .filter_map(|e| {
            let port = e.file_name().to_string_lossy().into_owned();
            split_port(&port)?;
            let dir = e.path();
            let vendor = read(&dir, "idVendor");
            let product = read(&dir, "idProduct");
            let maker = read(&dir, "manufacturer");
            let model = read(&dir, "product");
            let name = match (maker.is_empty(), model.is_empty()) {
                (false, false) if model.starts_with(&maker) => model,
                (false, false) => format!("{maker} {model}"),
                (true, false) => model,
                (false, true) => maker,
                (true, true) => format!("{vendor}:{product}"),
            };
            let node = dev_node(&dir);
            let writable = node.as_deref().is_some_and(writable);
            let hub = read(&dir, "bDeviceClass") == "09";
            Some(HostDevice { port, vendor, product, name, writable, hub })
        })
        .collect();
    out.sort_by(|a, b| natural(&a.port).cmp(&natural(&b.port)));
    out
}

fn natural(port: &str) -> Vec<u32> {
    port.split(['-', '.']).map(|s| s.parse().unwrap_or(0)).collect()
}

/// Устройство в порту или `None`, если порт пуст.
pub fn at(port: &str) -> Option<HostDevice> {
    list().into_iter().find(|d| d.port == port)
}

fn dev_node(dir: &Path) -> Option<PathBuf> {
    let bus: u32 = read(dir, "busnum").parse().ok()?;
    let dev: u32 = read(dir, "devnum").parse().ok()?;
    Some(PathBuf::from(format!("/dev/bus/usb/{bus:03}/{dev:03}")))
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Порты, уже открытые правилом syn-vm.
pub fn allowed_ports() -> Vec<String> {
    std::fs::read_to_string(UDEV_RULES)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split("KERNELS==\"").nth(1)?.split('"').next().map(str::to_string))
        .collect()
}

/// Открыть порты на запись для своего пользователя: правило udev через pkexec (спросит пароль
/// администратора один раз), перезагрузка правил и повторное применение к уже подключённым устройствам.
pub fn allow(ports: &[String]) -> Result<()> {
    let mut all = allowed_ports();
    for p in ports {
        if split_port(p).is_none() {
            bail!("{}", t!("Неверный порт USB: {p}", p = p));
        }
        if !all.contains(p) {
            all.push(p.clone());
        }
    }
    let mut rules = String::from("# syn-vm: порты USB, которые пробрасываются в виртуальные машины (любое устройство в этом порту)\n");
    for p in &all {
        rules.push_str(&format!("SUBSYSTEM==\"usb\", KERNELS==\"{p}\", MODE=\"0666\"\n"));
    }
    let script = format!(
        "printf '%s' '{}' > {UDEV_RULES} && chmod 644 {UDEV_RULES} && udevadm control --reload && udevadm trigger --subsystem-match=usb --action=change",
        rules.replace('\'', "")
    );
    let st = std::process::Command::new("pkexec").args(["sh", "-c", &script]).status().context("pkexec")?;
    if !st.success() {
        bail!("{}", t!("Доступ к USB не выдан (pkexec: {c})", c = st.code().unwrap_or(-1)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        assert_eq!(split_port("3-6"), Some((3, "6")));
        assert_eq!(split_port("1-2.4"), Some((1, "2.4")));
        assert_eq!(split_port("usb3"), None);
        assert_eq!(split_port("3-6:1.0"), None);
        assert_eq!(qemu_id("1-2.4"), "usb-1_2_4");
        assert!(natural("3-10") > natural("3-6"));
    }
}
