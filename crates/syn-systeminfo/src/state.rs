//! Страницы и состояние окна.

use syngui::core::Color;
use syngui::prelude::*;

use crate::sampler::Snap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Cpu,
    Gpu,
    Vram,
    Npu,
    Memory,
    Network,
    Disk,
    Sensors,
    Battery,
    Processes,
}

impl Page {
    pub const ALL: [Page; 11] =
        [Page::Overview, Page::Cpu, Page::Gpu, Page::Vram, Page::Npu, Page::Memory, Page::Network, Page::Disk, Page::Sensors, Page::Battery, Page::Processes];

    pub fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Cpu => "cpu",
            Page::Gpu => "gpu",
            Page::Vram => "vram",
            Page::Npu => "npu",
            Page::Memory => "memory",
            Page::Network => "network",
            Page::Disk => "disk",
            Page::Sensors => "sensors",
            Page::Battery => "battery",
            Page::Processes => "processes",
        }
    }

    /// Имя страницы из командной строки; принимает и виды виджетов рабочего стола
    /// (`memory`, `temps`, `apps`) и короткие имена (`ram`, `mem`, `gpu_mem`, `net`).
    pub fn parse(s: &str) -> Option<Page> {
        Some(match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "overview" | "summary" | "" => Page::Overview,
            "cpu" | "processor" => Page::Cpu,
            "gpu" | "graphics" => Page::Gpu,
            "vram" | "gpu_mem" | "gpu_memory" | "gpumem" => Page::Vram,
            "npu" | "ai" => Page::Npu,
            "memory" | "mem" | "ram" | "swap" => Page::Memory,
            "network" | "net" | "wifi" => Page::Network,
            "disk" | "disks" | "storage" | "io" => Page::Disk,
            "sensors" | "temps" | "temp" | "thermal" => Page::Sensors,
            "battery" | "power" => Page::Battery,
            "processes" | "procs" | "apps" | "tasks" => Page::Processes,
            _ => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => n_!("Обзор"),
            Page::Cpu => n_!("Процессор"),
            Page::Gpu => n_!("Графика"),
            Page::Vram => n_!("Видеопамять"),
            Page::Npu => n_!("Нейропроцессор"),
            Page::Memory => n_!("Память"),
            Page::Network => n_!("Сеть"),
            Page::Disk => n_!("Накопители"),
            Page::Sensors => n_!("Датчики"),
            Page::Battery => n_!("Питание"),
            Page::Processes => n_!("Процессы"),
        }
    }

    /// Короткое имя для чипа.
    pub fn short(self) -> &'static str {
        match self {
            Page::Overview => n_!("Обзор"),
            Page::Cpu => n_!("CPU"),
            Page::Gpu => n_!("GPU"),
            Page::Vram => n_!("VRAM"),
            Page::Npu => n_!("NPU"),
            Page::Memory => n_!("ОЗУ"),
            Page::Network => n_!("Сеть"),
            Page::Disk => n_!("Диски"),
            Page::Sensors => n_!("Темп."),
            Page::Battery => n_!("Батарея"),
            Page::Processes => n_!("Процессы"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Overview => icons::DASHBOARD,
            Page::Cpu => icons::CPU,
            Page::Gpu => icons::GPU,
            Page::Vram => icons::LAYERS,
            Page::Npu => icons::NPU,
            Page::Memory => icons::MEMORY,
            Page::Network => icons::NETWORK,
            Page::Disk => icons::STORAGE,
            Page::Sensors => icons::THERMO,
            Page::Battery => icons::BATTERY,
            Page::Processes => icons::LIST,
        }
    }

    /// Цвет ресурса: графики, значок, выделение.
    pub fn color(self) -> Color {
        Color::from_hex(self.hex())
    }

    pub fn hex(self) -> &'static str {
        match self {
            Page::Overview => "#8c9eff",
            Page::Cpu => "#4f9dff",
            Page::Gpu => "#2ec27e",
            Page::Vram => "#14b8a6",
            Page::Npu => "#b77cff",
            Page::Memory => "#ff9f43",
            Page::Network => "#22c3e6",
            Page::Disk => "#a3d65c",
            Page::Sensors => "#ff6b5b",
            Page::Battery => "#5fd068",
            Page::Processes => "#f06292",
        }
    }
}

/// Значки Material Icons.
pub mod icons {
    pub const DASHBOARD: &str = "\u{E871}";
    pub const CPU: &str = "\u{E30D}";
    pub const GPU: &str = "\u{E338}";
    pub const LAYERS: &str = "\u{E53B}";
    pub const NPU: &str = "\u{EA4A}";
    pub const MEMORY: &str = "\u{E322}";
    pub const NETWORK: &str = "\u{E8D5}";
    pub const STORAGE: &str = "\u{E1DB}";
    pub const THERMO: &str = "\u{E1FF}";
    pub const BATTERY: &str = "\u{E1A4}";
    pub const LIST: &str = "\u{E8EF}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const WIFI: &str = "\u{E63E}";
    pub const ETHERNET: &str = "\u{EB54}";
    pub const CELL: &str = "\u{E202}";
    pub const USB: &str = "\u{E1E0}";
    pub const VPN: &str = "\u{E0DA}";
    pub const HUB: &str = "\u{E9F4}";
    pub const INFO: &str = "\u{E88E}";
    pub const STOP: &str = "\u{EF71}";
    pub const CANCEL: &str = "\u{E5C9}";
    pub const SPEED: &str = "\u{E9E4}";
    pub const SLEEP: &str = "\u{E3A9}";
}

/// Сортировка списка процессов.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Cpu,
    Memory,
    Name,
    Pid,
}

/// Подтверждение «Убить»: окно или процесс.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    Window(u64),
    Process(i32),
}

#[derive(Clone, Copy)]
pub struct St {
    pub page: RwSignal<Page>,
    pub snap: RwSignal<Snap>,
    /// Окно графиков, с.
    pub span: RwSignal<usize>,
    pub query: RwSignal<String>,
    pub sort: RwSignal<Sort>,
    pub selected: RwSignal<Option<i32>>,
    pub kernel_threads: RwSignal<bool>,
    /// Только свои процессы.
    pub mine: RwSignal<bool>,
    pub toast: RwSignal<String>,
    pub confirm: RwSignal<Option<Confirm>>,
}

impl St {
    pub fn new(page: Page) -> Self {
        St {
            page: use_signal(page),
            snap: use_signal(Snap::default()),
            span: use_signal(60),
            query: use_signal(String::new()),
            sort: use_signal(Sort::Cpu),
            selected: use_signal(None),
            kernel_threads: use_signal(false),
            mine: use_signal(false),
            toast: use_signal(String::new()),
            confirm: use_signal(None),
        }
    }
}
