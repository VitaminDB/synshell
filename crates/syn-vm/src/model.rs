//! Машины на диске: `~/.local/share/syn-vm/<id>/` — `vm.toml` (настройки), диски
//! `*.qcow2`, переменные UEFI `efivars.qcow2` (qcow2 — чтобы снимки сохраняли и их), состояние TPM `tpm/`, последний кадр
//! `screen.png`. Сокеты запущенной машины — в `$XDG_RUNTIME_DIR/syn-vm/<id>/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use syngui::prelude::*;

/// Гостевая система: от неё — умолчания железа и подсказки.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OsKind {
    #[default]
    Linux,
    Windows11,
    Windows10,
    Bsd,
    Android,
    Other,
}

impl OsKind {
    pub const ALL: [OsKind; 6] = [OsKind::Linux, OsKind::Windows11, OsKind::Windows10, OsKind::Bsd, OsKind::Android, OsKind::Other];

    pub fn title(self) -> &'static str {
        match self {
            OsKind::Linux => n_!("Linux"),
            OsKind::Windows11 => n_!("Windows 11"),
            OsKind::Windows10 => n_!("Windows 10 и старше"),
            OsKind::Bsd => n_!("BSD"),
            OsKind::Android => n_!("Android x86"),
            OsKind::Other => n_!("Другая"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            OsKind::Linux => n_!("Ubuntu, Fedora, Arch, Debian и другие — virtio, UEFI"),
            OsKind::Windows11 => n_!("UEFI с Secure Boot и TPM 2.0 — как требует установщик"),
            OsKind::Windows10 => n_!("UEFI, диск SATA — драйверы virtio не нужны"),
            OsKind::Bsd => n_!("FreeBSD, OpenBSD, NetBSD"),
            OsKind::Android => n_!("Android-x86, Bliss OS"),
            OsKind::Other => n_!("Совместимое железо: BIOS, SATA, e1000"),
        }
    }

    pub fn is_windows(self) -> bool {
        matches!(self, OsKind::Windows11 | OsKind::Windows10)
    }

    /// Угадать по имени образа.
    pub fn guess(file: &str) -> Option<OsKind> {
        let f = file.to_ascii_lowercase();
        if f.contains("win11") || f.contains("windows_11") || f.contains("windows11") || f.contains("win_11") {
            Some(OsKind::Windows11)
        } else if f.contains("win") && (f.contains("10") || f.contains("7") || f.contains("8") || f.contains("server")) {
            Some(OsKind::Windows10)
        } else if ["bsd"].iter().any(|k| f.contains(k)) {
            Some(OsKind::Bsd)
        } else if ["android", "bliss"].iter().any(|k| f.contains(k)) {
            Some(OsKind::Android)
        } else if ["ubuntu", "fedora", "arch", "debian", "mint", "manjaro", "opensuse", "suse", "alpine", "kali", "pop", "nixos", "endeavour", "cachyos", "gentoo", "rocky", "alma", "centos", "elementary", "zorin", "void", "linux"]
            .iter()
            .any(|k| f.contains(k))
        {
            Some(OsKind::Linux)
        } else {
            None
        }
    }

    /// Умолчания железа для новой машины.
    pub fn preset(self) -> Vm {
        let host_cpus = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        let host_mb = host_memory_mb();
        let mut vm = Vm { os: self, ..Vm::default() };
        let (cpus, mem, disk) = match self {
            OsKind::Windows11 => (4, 8192, 80),
            OsKind::Windows10 => (4, 4096, 64),
            OsKind::Linux => (4, 4096, 40),
            OsKind::Android => (4, 4096, 32),
            OsKind::Bsd => (2, 2048, 32),
            OsKind::Other => (2, 2048, 32),
        };
        vm.cpus = cpus.min(host_cpus.saturating_sub(1).max(1));
        vm.memory_mb = mem.min((host_mb / 2).max(1024) / 512 * 512);
        vm.disk_gb = disk;
        match self {
            OsKind::Windows11 => {
                vm.firmware = Firmware::UefiSecure;
                vm.tpm = true;
                vm.disk_bus = DiskBus::Sata;
                vm.nic = NicModel::E1000e;
                vm.display = DisplayModel::Std;
                vm.clipboard = false;
            }
            OsKind::Windows10 => {
                vm.firmware = Firmware::Uefi;
                vm.disk_bus = DiskBus::Sata;
                vm.nic = NicModel::E1000e;
                vm.display = DisplayModel::Std;
                vm.clipboard = false;
            }
            OsKind::Other => {
                vm.firmware = Firmware::Bios;
                vm.disk_bus = DiskBus::Sata;
                vm.nic = NicModel::E1000e;
                vm.display = DisplayModel::Std;
                vm.clipboard = false;
            }
            OsKind::Bsd => {
                vm.firmware = Firmware::Uefi;
                vm.display = DisplayModel::Std;
                vm.clipboard = false;
            }
            OsKind::Linux | OsKind::Android => {}
        }
        vm
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Firmware {
    Bios,
    #[default]
    Uefi,
    UefiSecure,
}

impl Firmware {
    pub const ALL: [Firmware; 3] = [Firmware::Uefi, Firmware::UefiSecure, Firmware::Bios];

    pub fn title(self) -> &'static str {
        match self {
            Firmware::Bios => n_!("BIOS"),
            Firmware::Uefi => n_!("UEFI"),
            Firmware::UefiSecure => n_!("UEFI + Secure Boot"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DiskBus {
    #[default]
    Virtio,
    Sata,
    Nvme,
}

impl DiskBus {
    pub const ALL: [DiskBus; 3] = [DiskBus::Virtio, DiskBus::Sata, DiskBus::Nvme];

    pub fn title(self) -> &'static str {
        match self {
            DiskBus::Virtio => n_!("virtio"),
            DiskBus::Sata => n_!("SATA"),
            DiskBus::Nvme => n_!("NVMe"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NicModel {
    #[default]
    Virtio,
    E1000e,
    Rtl8139,
}

impl NicModel {
    pub const ALL: [NicModel; 3] = [NicModel::Virtio, NicModel::E1000e, NicModel::Rtl8139];

    pub fn title(self) -> &'static str {
        match self {
            NicModel::Virtio => n_!("virtio"),
            NicModel::E1000e => n_!("Intel e1000e"),
            NicModel::Rtl8139 => n_!("Realtek 8139"),
        }
    }

    pub fn device(self) -> &'static str {
        match self {
            NicModel::Virtio => "virtio-net-pci",
            NicModel::E1000e => "e1000e",
            NicModel::Rtl8139 => "rtl8139",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NetMode {
    /// NAT QEMU: гость выходит в сеть, снаружи — только проброшенные порты.
    #[default]
    Nat,
    None,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayModel {
    #[default]
    Virtio,
    Std,
    Qxl,
}

impl DisplayModel {
    pub const ALL: [DisplayModel; 3] = [DisplayModel::Virtio, DisplayModel::Std, DisplayModel::Qxl];

    pub fn title(self) -> &'static str {
        match self {
            DisplayModel::Virtio => n_!("virtio-vga"),
            DisplayModel::Std => n_!("Стандартный VGA"),
            DisplayModel::Qxl => n_!("QXL"),
        }
    }
}

/// Проброс порта хоста в гостя (NAT).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct PortForward {
    pub udp: bool,
    pub host: u16,
    pub guest: u16,
}

/// Диск машины.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Disk {
    /// Имя файла в папке машины или абсолютный путь.
    pub file: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Vm {
    /// Имя папки (в файле не хранится).
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub os: OsKind,
    /// Unix-время создания.
    pub created: u64,
    /// Последний запуск.
    pub last_run: u64,
    pub cpus: u32,
    pub memory_mb: u32,
    /// Размер первого диска при создании, ГБ.
    pub disk_gb: u32,
    pub firmware: Firmware,
    pub tpm: bool,
    pub disk_bus: DiskBus,
    pub disks: Vec<Disk>,
    /// Образ в приводе (пусто — привод пуст).
    pub cdrom: String,
    /// Грузиться с привода раньше диска.
    pub boot_cdrom: bool,
    pub net: NetMode,
    pub nic: NicModel,
    pub forwards: Vec<PortForward>,
    pub display: DisplayModel,
    /// Разрешение гостя — под размер окна.
    pub auto_resize: bool,
    pub sound: bool,
    /// Общий буфер обмена (qemu-vdagent, в госте — spice-vdagent).
    pub clipboard: bool,
    /// Общая папка хоста (9p, метка `hostshare`).
    pub shared_dir: String,
    /// Цвет карточки.
    pub color: String,
    /// Дополнительные аргументы QEMU.
    pub extra_args: String,
}

impl Default for Vm {
    fn default() -> Self {
        Vm {
            id: String::new(),
            name: String::new(),
            os: OsKind::Linux,
            created: 0,
            last_run: 0,
            cpus: 2,
            memory_mb: 4096,
            disk_gb: 40,
            firmware: Firmware::Uefi,
            tpm: false,
            disk_bus: DiskBus::Virtio,
            disks: Vec::new(),
            cdrom: String::new(),
            boot_cdrom: false,
            net: NetMode::Nat,
            nic: NicModel::Virtio,
            forwards: Vec::new(),
            display: DisplayModel::Virtio,
            auto_resize: true,
            sound: true,
            clipboard: true,
            shared_dir: String::new(),
            color: String::new(),
            extra_args: String::new(),
        }
    }
}

/// Цвета карточек по кругу.
pub const PALETTE: [&str; 8] = ["#4f9dff", "#2ec27e", "#b77cff", "#ff9f43", "#22c3e6", "#f06292", "#a3d65c", "#ff6b5b"];

impl Vm {
    pub fn dir(&self) -> PathBuf {
        root().join(&self.id)
    }

    pub fn run_dir(&self) -> PathBuf {
        run_root().join(&self.id)
    }

    pub fn qmp_path(&self) -> PathBuf {
        self.run_dir().join("qmp.sock")
    }

    pub fn vnc_path(&self) -> PathBuf {
        self.run_dir().join("vnc.sock")
    }

    pub fn pid_path(&self) -> PathBuf {
        self.run_dir().join("qemu.pid")
    }

    pub fn screen_path(&self) -> PathBuf {
        self.dir().join("screen.png")
    }

    pub fn vars_path(&self) -> PathBuf {
        self.dir().join("efivars.qcow2")
    }

    pub fn disk_path(&self, d: &Disk) -> PathBuf {
        let p = Path::new(&d.file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir().join(p)
        }
    }

    pub fn accent(&self) -> Color {
        Color::from_hex(if self.color.is_empty() { PALETTE[0] } else { &self.color })
    }

    pub fn save(&self) -> Result<()> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir)?;
        let s = toml::to_string_pretty(self)?;
        let tmp = dir.join("vm.toml.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, dir.join("vm.toml"))?;
        Ok(())
    }

    /// Сколько места занимают диски на самом деле (qcow2 растёт по мере записи).
    pub fn used_bytes(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        self.disks.iter().filter_map(|d| std::fs::metadata(self.disk_path(d)).ok()).map(|m| m.blocks() * 512).sum()
    }
}

pub fn root() -> PathBuf {
    synshell_common::paths::data_home().join("syn-vm")
}

pub fn run_root() -> PathBuf {
    synshell_common::paths::runtime_dir().join("syn-vm")
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Все машины, новые сверху.
pub fn load_all() -> Vec<Vm> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root()) else { return out };
    for e in rd.flatten() {
        let path = e.path().join("vm.toml");
        let Ok(s) = std::fs::read_to_string(&path) else { continue };
        match toml::from_str::<Vm>(&s) {
            Ok(mut vm) => {
                vm.id = e.file_name().to_string_lossy().into_owned();
                out.push(vm);
            }
            Err(err) => tracing::warn!("{}: {err}", path.display()),
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then(a.name.cmp(&b.name)));
    out
}

/// Имя папки из имени машины: латиница, цифры, дефисы; уникальное.
fn make_id(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        s = "vm".into();
    }
    s.truncate(32);
    let mut id = s.clone();
    let mut n = 2;
    while root().join(&id).exists() {
        id = format!("{s}-{n}");
        n += 1;
    }
    id
}

/// Свободное имя «База 2», «База 3»…
pub fn unique_name(base: &str, all: &[Vm]) -> String {
    if !all.iter().any(|v| v.name == base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| !all.iter().any(|v| &v.name == n)).unwrap()
}

/// Создать машину: папка, диск qcow2, переменные UEFI.
pub fn create(mut vm: Vm, existing_disk: Option<PathBuf>) -> Result<Vm> {
    if vm.name.trim().is_empty() {
        bail!("{}", t!("Пустое имя"));
    }
    vm.id = make_id(&vm.name);
    vm.created = now();
    if vm.color.is_empty() {
        vm.color = PALETTE[(vm.created as usize / 7 + vm.name.len()) % PALETTE.len()].to_string();
    }
    let dir = vm.dir();
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let r = (|| -> Result<()> {
        match existing_disk {
            Some(p) => vm.disks = vec![Disk { file: p.to_string_lossy().into_owned() }],
            None if vm.disk_gb > 0 => {
                let file = "disk0.qcow2".to_string();
                qemu_img_create(&dir.join(&file), vm.disk_gb as u64)?;
                vm.disks = vec![Disk { file }];
            }
            None => vm.disks.clear(),
        }
        if vm.firmware != Firmware::Bios {
            let fw = crate::qemu::firmware(vm.firmware).context(t!("Не найдена прошивка UEFI (пакет edk2-ovmf)"))?;
            make_vars(&fw.vars, &vm.vars_path())?;
        }
        vm.save()
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(vm)
}

pub fn qemu_img_create(path: &Path, gb: u64) -> Result<()> {
    let out = Command::new("qemu-img")
        .args(["create", "-q", "-f", "qcow2", "-o", "lazy_refcounts=on,preallocation=metadata"])
        .arg(path)
        .arg(format!("{gb}G"))
        .output()
        .context(t!("qemu-img не найден (пакет qemu-img)"))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Переменные UEFI из шаблона — в qcow2: `savevm` требует, чтобы все записываемые
/// устройства умели снимки, и pflash в raw их бы запретил.
pub fn make_vars(template: &Path, out: &Path) -> Result<()> {
    let o = Command::new("qemu-img").args(["convert", "-q", "-f", "raw", "-O", "qcow2"]).arg(template).arg(out).output().context("qemu-img")?;
    if !o.status.success() {
        bail!("{}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(())
}

/// Увеличить диск (уменьшать qcow2 опасно — только рост).
pub fn resize_disk(path: &Path, gb: u64) -> Result<()> {
    let out = Command::new("qemu-img").arg("resize").arg(path).arg(format!("{gb}G")).output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Виртуальный размер образа, байт.
pub fn disk_virtual_size(path: &Path) -> Option<u64> {
    let out = Command::new("qemu-img").args(["info", "--output=json", "-U"]).arg(path).output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("virtual-size")?.as_u64()
}

/// Добавить чистый диск машине.
pub fn add_disk(vm: &mut Vm, gb: u64) -> Result<()> {
    let mut n = vm.disks.len();
    let file = loop {
        let f = format!("disk{n}.qcow2");
        if !vm.dir().join(&f).exists() {
            break f;
        }
        n += 1;
    };
    qemu_img_create(&vm.dir().join(&file), gb)?;
    vm.disks.push(Disk { file });
    vm.save()
}

/// Копия машины: диски — полными копиями (с поддержкой reflink на btrfs/xfs).
pub fn clone(vm: &Vm, all: &[Vm]) -> Result<Vm> {
    let mut c = vm.clone();
    c.name = unique_name(&t!("{n} (копия)", n = vm.name), all);
    c.id = make_id(&c.name);
    c.created = now();
    c.last_run = 0;
    c.color = PALETTE[(PALETTE.iter().position(|p| *p == vm.color).unwrap_or(0) + 1) % PALETTE.len()].to_string();
    let src = vm.dir();
    let dst = c.dir();
    std::fs::create_dir_all(&dst)?;
    let r = (|| -> Result<()> {
        for e in std::fs::read_dir(&src)?.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n == "vm.toml" || e.path().is_dir() && n != "tpm" {
                continue;
            }
            if e.path().is_dir() {
                let st = Command::new("cp").arg("-a").arg(e.path()).arg(dst.join(&name)).status()?;
                if !st.success() {
                    bail!("cp {n}");
                }
                continue;
            }
            let st = Command::new("cp").arg("--reflink=auto").arg("--sparse=always").arg(e.path()).arg(dst.join(&name)).status()?;
            if !st.success() {
                bail!("cp {n}");
            }
        }
        c.save()
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&dst);
        return Err(e);
    }
    Ok(c)
}

/// Удалить машину вместе с дисками в её папке (внешние диски не трогаются).
pub fn delete(vm: &Vm) -> Result<()> {
    if vm.id.is_empty() || vm.id.contains('/') || vm.id.starts_with('.') {
        bail!("bad id");
    }
    std::fs::remove_dir_all(vm.dir())?;
    let _ = std::fs::remove_dir_all(vm.run_dir());
    Ok(())
}

pub fn host_memory_mb() -> u32 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .map(|kb| (kb / 1024) as u32)
        .unwrap_or(8192)
}

pub fn host_cpus() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4)
}

/// Свободно на разделе с машинами, байт.
pub fn free_space() -> Option<u64> {
    let dir = root();
    let _ = std::fs::create_dir_all(&dir);
    let c = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_os() {
        assert_eq!(OsKind::guess("Win11_24H2_Russian_x64.iso"), Some(OsKind::Windows11));
        assert_eq!(OsKind::guess("ubuntu-24.04-desktop-amd64.iso"), Some(OsKind::Linux));
        assert_eq!(OsKind::guess("FreeBSD-14.1-RELEASE-amd64-disc1.iso"), Some(OsKind::Bsd));
        assert_eq!(OsKind::guess("archlinux-2026.10.01-x86_64.iso"), Some(OsKind::Linux));
        assert_eq!(OsKind::guess("Win10_22H2_English_x64v1.iso"), Some(OsKind::Windows10));
        assert_eq!(OsKind::guess("something.iso"), None);
    }

    #[test]
    fn toml_roundtrip() {
        let mut vm = OsKind::Windows11.preset();
        vm.name = "Test".into();
        vm.forwards.push(PortForward { udp: false, host: 2222, guest: 22 });
        let s = toml::to_string_pretty(&vm).unwrap();
        let back: Vm = toml::from_str(&s).unwrap();
        assert_eq!(vm, back);
        // Старый файл без новых полей читается с умолчаниями.
        let v: Vm = toml::from_str("name = \"x\"\ncpus = 3\n").unwrap();
        assert_eq!(v.cpus, 3);
        assert!(v.auto_resize);
    }
}
