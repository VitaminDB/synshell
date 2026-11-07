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
                vm.tools_cd = true;
                vm.guest_tools = GuestTools::SynVm;
                vm.disk_bus = DiskBus::Sata;
                vm.nic = NicModel::E1000e;
                // virtio-vga: в установщике — как VGA, с драйвером VirtIO GPU (virtio-win) — любое
                // разрешение и быстрая отрисовка (std VGA — «Базовый видеоадаптер», медленно).
                vm.display = DisplayModel::Virtio;
                // Буфер обмена: канал qemu-vdagent, в госте — spice-agent из «Гостевых инструментов».
                vm.clipboard = true;
            }
            OsKind::Windows10 => {
                vm.firmware = Firmware::Uefi;
                vm.tools_cd = true;
                vm.guest_tools = GuestTools::SynVm;
                vm.disk_bus = DiskBus::Sata;
                vm.nic = NicModel::E1000e;
                // virtio-vga: в установщике — как VGA, с драйвером VirtIO GPU (virtio-win) — любое
                // разрешение и быстрая отрисовка (std VGA — «Базовый видеоадаптер», медленно).
                vm.display = DisplayModel::Virtio;
                // Буфер обмена: канал qemu-vdagent, в госте — spice-agent из «Гостевых инструментов».
                vm.clipboard = true;
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
    /// Своя видеокарта syn-gpu — единственный экран гостя (экран и 3D на GPU компьютера):
    /// у QEMU `-vga none`, рабочий стол приходит кадрами DESKTOP сокета вывода syn-gpu.
    SynGpu,
}

/// Чьи «Гостевые инструменты» у Windows-гостя.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GuestTools {
    /// virtio-win (Red Hat): драйверы virtio, qemu-ga, spice-agent — как раньше.
    #[default]
    VirtioWin,
    /// Свои: агент syn-vm и драйвер virtio-serial syn-vm-serial (+ syn-gpu); без virtio-win в госте
    /// у QEMU нет balloon, портов qemu-ga и spice, диск — «Гостевые инструменты syn-vm».
    SynVm,
}

impl GuestTools {
    pub const ALL: [GuestTools; 2] = [GuestTools::SynVm, GuestTools::VirtioWin];

    pub fn title(self) -> &'static str {
        match self {
            GuestTools::SynVm => n_!("syn-vm — свои (агент, драйвер, syn-gpu)"),
            GuestTools::VirtioWin => n_!("virtio-win (Red Hat)"),
        }
    }
}

/// Масштаб экрана гостя в окне.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScaleMode {
    /// Вписать с сохранением пропорций (поля — фоном).
    #[default]
    Fit,
    /// Растянуть на всё окно.
    Stretch,
    /// Пиксель в пиксель, по центру.
    Native,
}

impl ScaleMode {
    pub fn next(self) -> Self {
        match self {
            ScaleMode::Fit => ScaleMode::Stretch,
            ScaleMode::Stretch => ScaleMode::Native,
            ScaleMode::Native => ScaleMode::Fit,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ScaleMode::Fit => n_!("Вписать с пропорциями"),
            ScaleMode::Stretch => n_!("Растянуть"),
            ScaleMode::Native => n_!("1:1 — пиксель в пиксель"),
        }
    }
}

impl DisplayModel {
    pub const ALL: [DisplayModel; 4] = [DisplayModel::Virtio, DisplayModel::Std, DisplayModel::Qxl, DisplayModel::SynGpu];

    pub fn title(self) -> &'static str {
        match self {
            DisplayModel::Virtio => n_!("virtio-vga"),
            DisplayModel::Std => n_!("Стандартный VGA"),
            DisplayModel::Qxl => n_!("QXL"),
            DisplayModel::SynGpu => n_!("syn-gpu — своя видеокарта (экран и 3D на GPU компьютера)"),
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
    /// Слушать на всех адресах компьютера (по умолчанию — только 127.0.0.1).
    pub public: bool,
}

/// Проброшенное USB-устройство: физический порт компьютера.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct UsbPass {
    /// Порт (`3-6`, `1-2.4`): любое устройство в нём уходит гостю.
    pub port: String,
    /// Что было в порту при добавлении — для подписи, когда порт пуст.
    pub name: String,
}

/// Звуковая карта гостя.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SoundCard {
    /// BIOS — Sound Blaster 16 с AdLib (DOS, Windows 9x), иначе Intel HDA.
    #[default]
    Auto,
    Hda,
    Sb16,
    Ac97,
}

impl SoundCard {
    pub const ALL: [SoundCard; 4] = [SoundCard::Auto, SoundCard::Hda, SoundCard::Sb16, SoundCard::Ac97];

    pub fn title(self) -> &'static str {
        match self {
            SoundCard::Auto => n_!("Авто"),
            SoundCard::Hda => n_!("Intel HDA"),
            SoundCard::Sb16 => n_!("Sound Blaster 16"),
            SoundCard::Ac97 => n_!("AC97"),
        }
    }

    pub fn resolve(self, fw: Firmware) -> SoundCard {
        match self {
            SoundCard::Auto if fw == Firmware::Bios => SoundCard::Sb16,
            SoundCard::Auto => SoundCard::Hda,
            c => c,
        }
    }
}

/// Как гость видит общую папку.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShareMode {
    /// Linux и BSD — 9p, Windows — virtiofs.
    #[default]
    Auto,
    #[serde(rename = "9p")]
    P9,
    Virtiofs,
    /// Samba QEMU в сети NAT (`\\10.0.2.4\qemu`), нужен smbd на компьютере.
    Smb,
}

impl ShareMode {
    pub const ALL: [ShareMode; 4] = [ShareMode::Auto, ShareMode::P9, ShareMode::Virtiofs, ShareMode::Smb];

    pub fn title(self) -> &'static str {
        match self {
            ShareMode::Auto => n_!("Авто"),
            ShareMode::P9 => n_!("9p"),
            ShareMode::Virtiofs => n_!("virtiofs"),
            ShareMode::Smb => n_!("SMB"),
        }
    }

    /// Что выбрать для системы.
    pub fn resolve(self, os: OsKind) -> ShareMode {
        match self {
            ShareMode::Auto if os.is_windows() => ShareMode::Virtiofs,
            ShareMode::Auto => ShareMode::P9,
            m => m,
        }
    }
}

/// Устройство компьютера, отданное гостю (кроме USB по порту — оно в `usb`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HostDev {
    /// USB по VID:PID — запасной вариант к порту: устройство найдётся в любом гнезде.
    UsbId { vendor: String, product: String, #[serde(default)] name: String },
    /// PCI через VFIO (видеокарта, NVMe, сетевая, звук): только при выключенной машине.
    Pci { addr: String, #[serde(default)] name: String },
    /// Клавиатура, мышь, геймпад напрямую (evdev); переключение хост↔гость — оба Ctrl.
    Input { path: String, #[serde(default)] name: String },
    /// Целый диск или раздел компьютера.
    Block { path: String, #[serde(default)] readonly: bool, #[serde(default)] name: String },
    /// Последовательный порт (`/dev/ttyUSB0`, `/dev/ttyACM0`) — гостю как USB-UART.
    Serial { path: String, #[serde(default)] name: String },
}

impl HostDev {
    /// Тип и идентификатор для CLI: `usb-id:18d1:4ee7`, `pci:0000:01:00.0`, `input:/dev/input/…`.
    pub fn spec(&self) -> String {
        match self {
            HostDev::UsbId { vendor, product, .. } => format!("usb-id:{vendor}:{product}"),
            HostDev::Pci { addr, .. } => format!("pci:{addr}"),
            HostDev::Input { path, .. } => format!("input:{path}"),
            HostDev::Block { path, readonly, .. } => format!("{}:{path}", if *readonly { "block-ro" } else { "block" }),
            HostDev::Serial { path, .. } => format!("serial:{path}"),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            HostDev::UsbId { name, .. } | HostDev::Pci { name, .. } | HostDev::Input { name, .. } | HostDev::Block { name, .. } | HostDev::Serial { name, .. } => name,
        }
    }

    /// Одно и то же устройство (без учёта подписи и режима).
    pub fn same(&self, o: &HostDev) -> bool {
        match (self, o) {
            (HostDev::UsbId { vendor: a, product: b, .. }, HostDev::UsbId { vendor: c, product: d, .. }) => a == c && b == d,
            (HostDev::Pci { addr: a, .. }, HostDev::Pci { addr: b, .. }) => a == b,
            (HostDev::Input { path: a, .. }, HostDev::Input { path: b, .. }) | (HostDev::Block { path: a, .. }, HostDev::Block { path: b, .. }) | (HostDev::Serial { path: a, .. }, HostDev::Serial { path: b, .. }) => a == b,
            _ => false,
        }
    }

    /// Можно ли подключить на ходу.
    pub fn hotplug(&self) -> bool {
        !matches!(self, HostDev::Pci { .. })
    }

    /// Идентификатор в QEMU.
    pub fn qemu_id(&self) -> String {
        let h = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            match self {
                HostDev::Block { path, .. } => path.hash(&mut h),
                _ => self.spec().hash(&mut h),
            }
            h.finish() & 0xffff_ffff
        };
        let k = match self {
            HostDev::UsbId { .. } => "usbid",
            HostDev::Pci { .. } => "pci",
            HostDev::Input { .. } => "input",
            HostDev::Block { .. } => "blk",
            HostDev::Serial { .. } => "ser",
        };
        format!("hd-{k}-{h:08x}")
    }
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
    /// Как кадр гостя ложится в окно, когда разрешение не под окно.
    pub scale: ScaleMode,
    /// 3D-ускорение: паравиртуальная видеокарта syn-gpu (сервер vfio-user, проект
    /// syn-virt) — Windows с её драйвером; кадры игр — слоем поверх экрана.
    pub gpu3d: bool,
    /// Доп. ключи сервера syn-gpu (`--heap-gib 4 --gpu nvidia`).
    pub gpu3d_args: String,
    pub sound: bool,
    /// Звуковая карта (`auto` — по прошивке).
    pub sound_card: SoundCard,
    /// Общий буфер обмена (qemu-vdagent, в госте — spice-vdagent).
    pub clipboard: bool,
    /// Общая папка хоста (9p, метка `hostshare`).
    pub shared_dir: String,
    /// Ещё общие папки (первая — `shared_dir`).
    pub shares: Vec<String>,
    /// USB-устройства компьютера, отданные гостю (по портам).
    pub usb: Vec<UsbPass>,
    /// Прочие устройства компьютера: PCI, ввод, диски, последовательные порты, USB по VID:PID.
    pub devices: Vec<HostDev>,
    /// Как гость видит общую папку.
    pub share_mode: ShareMode,
    /// Канал гостевого агента (qemu-ga): команды, файлы, выключение без сети.
    pub guest_agent: bool,
    /// Второй привод с «Гостевыми инструментами» (virtio-win: драйверы, агент, spice-vdagent).
    pub tools_cd: bool,
    /// Windows: свои «Гостевые инструменты» syn-vm или virtio-win.
    pub guest_tools: GuestTools,
    /// TPM компьютера вместо эмулятора (passthrough `/dev/tpmrm0`… — нужен доступ на запись).
    pub tpm_host: bool,
    /// Устройства PipeWire для звука гостя (пусто — по умолчанию).
    pub audio_out: String,
    pub audio_in: String,
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
            scale: ScaleMode::Fit,
            gpu3d: false,
            gpu3d_args: String::new(),
            sound: true,
            sound_card: SoundCard::Auto,
            clipboard: true,
            shared_dir: String::new(),
            shares: Vec::new(),
            usb: Vec::new(),
            devices: Vec::new(),
            share_mode: ShareMode::Auto,
            guest_agent: true,
            tools_cd: false,
            guest_tools: GuestTools::VirtioWin,
            tpm_host: false,
            audio_out: String::new(),
            audio_in: String::new(),
            color: String::new(),
            extra_args: String::new(),
        }
    }
}

/// Цвета карточек по кругу.
pub const PALETTE: [&str; 8] = ["#4f9dff", "#2ec27e", "#b77cff", "#ff9f43", "#22c3e6", "#f06292", "#a3d65c", "#ff6b5b"];

impl Vm {
    /// «Оптимизировать» (и уже созданной машины; видео и драйверы — для Windows): что поменять в настройках
    /// (применяется при следующем запуске) и что сделать вручную. Пусто — всё уже так.
    pub fn optimized(&self) -> (Vm, Vec<String>, Vec<String>) {
        let mut v = self.clone();
        let mut changes = Vec::new();
        let mut advice = Vec::new();
        // Свои инструменты: virtio-win в госте нет — ни spice-agent, ни драйвера VirtIO GPU.
        if v.syn_tools() {
            if !v.guest_agent {
                changes.push(t!("Гостевой агент: включить"));
                v.guest_agent = true;
            }
            if !v.tools_cd {
                changes.push(t!("Диск «Гостевые инструменты»: вставлять при запуске"));
                v.tools_cd = true;
            }
            if v.gpu_on() && v.firmware == Firmware::UefiSecure {
                changes.push(t!("Прошивка: UEFI без Secure Boot — драйверу syn-gpu нужна тестовая подпись"));
                v.firmware = Firmware::Uefi;
            }
            if v.firmware == Firmware::UefiSecure {
                changes.push(t!("Прошивка: UEFI без Secure Boot — драйверам syn-vm нужна тестовая подпись"));
                v.firmware = Firmware::Uefi;
            }
            if v.display != DisplayModel::SynGpu && crate::qemu::syn_gpu_bin().is_some() {
                advice.push(t!("Видео: «syn-gpu» — своя видеокарта (экран и 3D на GPU компьютера) после установки «Гостевых инструментов syn-vm»; virtio-vga без virtio-win — базовый адаптер, медленно."));
            }
            if v.disk_bus != DiskBus::Sata || v.nic != NicModel::E1000e {
                advice.push(t!("Без virtio-win диску и сети нужны SATA и e1000e (драйверы есть в Windows)."));
            }
            return (v, changes, advice);
        }
        if !v.clipboard {
            changes.push(t!("Общий буфер обмена: включить (канал qemu-vdagent; в госте — служба spice-agent или spice-vdagent из «Гостевых инструментов»)"));
            v.clipboard = true;
        }
        if !self.os.is_windows() {
            if !v.guest_agent {
                changes.push(t!("Гостевой агент: включить"));
                v.guest_agent = true;
            }
            return (v, changes, advice);
        }
        if v.gpu_on() && v.firmware == Firmware::UefiSecure {
            changes.push(t!("Прошивка: UEFI без Secure Boot — драйверу syn-gpu нужна тестовая подпись"));
            v.firmware = Firmware::Uefi;
        }
        if !matches!(v.display, DisplayModel::Virtio | DisplayModel::SynGpu) {
            changes.push(t!("Видео: {from} → virtio-vga (с драйвером VirtIO GPU из «Гостевых инструментов» — любое разрешение, разрешение подстраивается под окно, быстрая отрисовка)", from = syngui::i18n::t(v.display.title())));
            v.display = DisplayModel::Virtio;
        }
        if !v.guest_agent {
            changes.push(t!("Гостевой агент: включить"));
            v.guest_agent = true;
        }
        if !v.tools_cd {
            changes.push(t!("Диск «Гостевые инструменты»: вставлять при запуске"));
            v.tools_cd = true;
        }
        if v.disk_bus == DiskBus::Sata {
            advice.push(t!("Диск SATA медленнее virtio. Перевести на virtio можно после установки «Гостевых инструментов» (драйвер viostor): сделайте снимок, поменяйте шину диска в настройках и запустите машину; не загрузится — верните снимок."));
        }
        if v.nic == NicModel::E1000e {
            advice.push(t!("Сеть e1000e можно заменить на virtio после установки «Гостевых инструментов» (драйвер NetKVM)."));
        }
        (v, changes, advice)
    }

    /// Способ общей папки: «Авто» — Linux/BSD 9p, Windows — virtiofs (virtio-win) или SMB
    /// (свои инструменты: драйвера virtiofs нет, папку подключает агент syn-vm). Нет папки — None.
    pub fn share(&self) -> Option<ShareMode> {
        if self.share_dirs().is_empty() {
            return None;
        }
        Some(self.share_kind())
    }

    /// Каким способом делиться папками (и без выбранных папок — для подсказки в настройках).
    pub fn share_kind(&self) -> ShareMode {
        match self.share_mode {
            ShareMode::Auto if self.syn_tools() => ShareMode::Smb,
            m => m.resolve(self.os),
        }
    }

    /// Общие папки, что есть на диске: `shared_dir`, затем `shares` (без повторов).
    pub fn share_dirs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in std::iter::once(&self.shared_dir).chain(self.shares.iter()) {
            if !d.is_empty() && std::path::Path::new(d).is_dir() && !out.contains(d) {
                out.push(d.clone());
            }
        }
        out
    }

    /// Имена общих папок для гостя: SMB — имя ресурса (первая — `qemu`, как у QEMU), 9p/virtiofs —
    /// метка (`hostshare`, `hostshare1`…). Порядок — как у [`Vm::share_dirs`].
    pub fn share_names(&self, smb: bool) -> Vec<String> {
        let mut used: Vec<String> = Vec::new();
        self.share_dirs()
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let name = if !smb {
                    if i == 0 { "hostshare".to_string() } else { format!("hostshare{i}") }
                } else if i == 0 {
                    "qemu".to_string()
                } else {
                    let base: String = std::path::Path::new(d).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(60).collect();
                    let base = if base.is_empty() || base.eq_ignore_ascii_case("qemu") { format!("share{i}") } else { base };
                    let mut n = base.clone();
                    let mut k = 2;
                    while used.iter().any(|u| u.eq_ignore_ascii_case(&n)) {
                        n = format!("{base}{k}");
                        k += 1;
                    }
                    n
                };
                used.push(name.clone());
                name
            })
            .collect()
    }

    /// Windows-гость со своими «Гостевыми инструментами» syn-vm (без virtio-win).
    pub fn syn_tools(&self) -> bool {
        self.os.is_windows() && self.guest_tools == GuestTools::SynVm
    }

    /// Видеокарта syn-gpu подключена: «3D-ускорение» или syn-gpu — основная видеокарта.
    pub fn gpu_on(&self) -> bool {
        self.gpu3d || self.display == DisplayModel::SynGpu
    }

    /// Значение ключа сервера syn-gpu из `gpu3d_args` (`--gpu nvidia` → `Some("nvidia")`).
    pub fn gpu_arg(&self, key: &str) -> Option<String> {
        let t: Vec<&str> = self.gpu3d_args.split_whitespace().collect();
        t.iter().position(|a| *a == key).and_then(|i| t.get(i + 1)).map(|v| v.to_string())
    }

    /// Поставить (`Some`) или убрать (`None`) ключ сервера syn-gpu; прочие ключи — как были.
    pub fn set_gpu_arg(&mut self, key: &str, val: Option<&str>) {
        let t: Vec<&str> = self.gpu3d_args.split_whitespace().collect();
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        while i < t.len() {
            if t[i] == key {
                i += 2;
                continue;
            }
            out.push(t[i].to_string());
            i += 1;
        }
        if let Some(v) = val {
            out.push(key.to_string());
            out.push(v.to_string());
        }
        self.gpu3d_args = out.join(" ");
    }

    pub fn dir(&self) -> PathBuf {
        root().join(&self.id)
    }

    pub fn run_dir(&self) -> PathBuf {
        run_root().join(&self.id)
    }

    pub fn qmp_path(&self) -> PathBuf {
        self.run_dir().join("qmp.sock")
    }

    /// Второй монитор QMP — для ввода в режиме захвата (монитор принимает одно соединение
    /// за раз, а захват держит своё всё время).
    pub fn qmp_input_path(&self) -> PathBuf {
        self.run_dir().join("qmp-input.sock")
    }

    pub fn vnc_path(&self) -> PathBuf {
        self.run_dir().join("vnc.sock")
    }

    pub fn qga_path(&self) -> PathBuf {
        self.run_dir().join("qga.sock")
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
    fn gpu_args() {
        let mut v = Vm { gpu3d_args: "--heap-gib 4 --gpu nvidia --log x".into(), ..Default::default() };
        assert_eq!(v.gpu_arg("--gpu").as_deref(), Some("nvidia"));
        v.set_gpu_arg("--gpu", Some("intel"));
        assert_eq!(v.gpu3d_args, "--heap-gib 4 --log x --gpu intel");
        v.set_gpu_arg("--heap-gib", None);
        assert_eq!(v.gpu3d_args, "--log x --gpu intel");
        assert_eq!(v.gpu_arg("--heap-gib"), None);
    }

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
        vm.forwards.push(PortForward { udp: false, host: 2222, guest: 22, public: false });
        let s = toml::to_string_pretty(&vm).unwrap();
        let back: Vm = toml::from_str(&s).unwrap();
        assert_eq!(vm, back);
        // Старый файл без новых полей читается с умолчаниями.
        let v: Vm = toml::from_str("name = \"x\"\ncpus = 3\n").unwrap();
        assert_eq!(v.cpus, 3);
        assert!(v.auto_resize);
    }
}
