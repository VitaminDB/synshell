//! synnetd — несколько подключений к интернету одновременно (Wi-Fi, USB-модем,
//! проводная сеть, мобильная сеть).
//!
//! Режимы:
//! * «основной канал» — весь трафик идёт через один канал (по умолчанию тот,
//!   у кого меньше метрика, или выбранный), остальные — запасные;
//! * «оба канала» — маршрут по умолчанию через все каналы сразу (multipath):
//!   разные соединения расходятся по разным каналам, торрент или менеджер
//!   загрузок с сотнями соединений использует все каналы.
//!
//! Для каждой программы можно выбрать своё: «только основной» или «оба» —
//! независимо от общего режима. Программа (и всё, что она запускает)
//! переносится в под-cgroup своего же сеанса, BPF-программа этой cgroup
//! помечает каждый её сокет, и маршрут выбирается по метке уже при
//! `connect()` — адрес источника всегда от нужного канала.
//!
//! USB-модемы (телефон в режиме модема: rndis, cdc_ether, cdc_ncm, ipheth),
//! которые никто не настраивает, служба поднимает сама (свой DHCP-клиент).
//!
//! Программы спрашивают службу через сокет [`SOCKET`]: запрос и ответ — по
//! строке JSON. Этот модуль — протокол и клиент (без зависимостей демона).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Сокет службы.
pub const SOCKET: &str = "/run/synnetd.sock";

/// Общий режим.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Весь трафик — через основной канал, остальные — запасные.
    #[default]
    Primary,
    /// Все каналы одновременно.
    Both,
}

/// Выбор для программы. В настройках и протоколе — строкой: `primary`,
/// `both`, `iface:<интерфейс>`.
#[derive(Clone, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum AppPolicy {
    /// Только основной канал.
    Primary,
    /// Все каналы одновременно.
    Both,
    /// Только этот канал (интерфейс: `wlan0`, `enp128s20f0u1`). Не
    /// подключён — через основной; нет у него IPv6 (а у других есть) — IPv6
    /// программе закрыт, она идёт по IPv4 через свой канал.
    Iface(String),
}

impl AppPolicy {
    pub fn id(&self) -> String {
        match self {
            AppPolicy::Primary => "primary".into(),
            AppPolicy::Both => "both".into(),
            AppPolicy::Iface(i) => format!("iface:{i}"),
        }
    }

    pub fn from_id(s: &str) -> Option<AppPolicy> {
        match s {
            "primary" => Some(AppPolicy::Primary),
            "both" => Some(AppPolicy::Both),
            _ => s.strip_prefix("iface:").filter(|i| valid_iface(i)).map(|i| AppPolicy::Iface(i.to_string())),
        }
    }

    /// Имя под-cgroup программы: `synnet-both`, `synnet-primary`, `synnet-if-wlan0`.
    pub fn cgroup_name(&self) -> String {
        match self {
            AppPolicy::Primary => "synnet-primary".into(),
            AppPolicy::Both => "synnet-both".into(),
            AppPolicy::Iface(i) => format!("synnet-if-{i}"),
        }
    }

    pub fn from_cgroup_name(s: &str) -> Option<AppPolicy> {
        match s.strip_prefix("synnet-")? {
            "primary" => Some(AppPolicy::Primary),
            "both" => Some(AppPolicy::Both),
            x => x.strip_prefix("if-").filter(|i| valid_iface(i)).map(|i| AppPolicy::Iface(i.to_string())),
        }
    }
}

/// Имя сетевого интерфейса: 1–15 байт без «/», пробелов и «:».
pub fn valid_iface(s: &str) -> bool {
    !s.is_empty() && s.len() <= 15 && s.bytes().all(|b| b.is_ascii_graphic() && b != b'/' && b != b':')
}

/// Метка сокетов программ с выбором «только канал `iface`»: из имени, чтобы
/// не зависеть от порядка в настройках и номера интерфейса (у USB-модема он
/// меняется при каждом подключении).
pub fn iface_mark(iface: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in iface.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    0x5360_0000 | (h & 0x000f_ffff)
}

impl Serialize for AppPolicy {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.id())
    }
}

impl<'de> Deserialize<'de> for AppPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        AppPolicy::from_id(&s).ok_or_else(|| serde::de::Error::custom(format!("неизвестный выбор «{s}»")))
    }
}

/// Настройки службы (`/var/lib/synnetd/config.toml`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    /// Основной канал: имя интерфейса; пусто — автоматически (меньшая
    /// метрика маршрута, обычно проводная сеть, затем Wi-Fi).
    pub primary: String,
    /// Программы со своим выбором: имя процесса, имя исполняемого файла или
    /// полный путь (`qbittorrent`, `/usr/bin/transmission-gtk`).
    pub apps: BTreeMap<String, AppPolicy>,
    /// Поднимать USB-модемы, которые никто не настраивает (свой DHCP).
    pub usb_dhcp: bool,
    /// Решения по USB-модемам (телефонам): ключ устройства ([`UsbModem::key`]) → выбор и имя.
    /// Нет решения — служба модем не поднимает и спрашивает пользователя (уведомление оболочки).
    pub usb_devices: BTreeMap<String, UsbDecision>,
}

impl Default for Config {
    fn default() -> Self {
        Config { mode: Mode::Primary, primary: String::new(), apps: BTreeMap::new(), usb_dhcp: true, usb_devices: BTreeMap::new() }
    }
}

/// Канал в интернет.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Uplink {
    pub iface: String,
    /// `wifi`, `ethernet`, `usb`, `mobile`, `other`.
    pub kind: String,
    /// Адрес IPv4 (`192.168.10.106/24`).
    pub ipv4: Option<String>,
    pub gateway4: Option<String>,
    /// Есть маршрут IPv6 по умолчанию.
    pub ipv6: bool,
    /// Метрика маршрута по умолчанию (меньше — предпочтительнее).
    pub metric: u32,
    /// Сейчас основной канал.
    pub primary: bool,
    /// Адрес получен самой службой (USB-модем без другого хозяина).
    pub dhcp_by_synnetd: bool,
    /// Счётчики интерфейса.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Как использовать телефон, подключённый по USB.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UsbChoice {
    /// Интернет через телефон: служба поднимает модем и берёт адрес.
    Modem,
    /// Только зарядка и файлы: модем не трогать.
    No,
}

/// Запомненное решение по устройству.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UsbDecision {
    pub choice: UsbChoice,
    /// Имя устройства для показа («Apple Inc. iPhone»).
    #[serde(default)]
    pub name: String,
}

/// USB-модем (телефон в режиме модема), подключённый сейчас.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UsbModem {
    /// Устройство: `vid:pid:серийный` (у iPhone серийный — UDID), без серийного — MAC.
    pub key: String,
    pub iface: String,
    /// Производитель и модель из USB («Apple Inc. iPhone»).
    pub name: String,
    pub driver: String,
    /// Решение пользователя (`None` — ещё не спрашивали или не ответил).
    pub choice: Option<UsbChoice>,
    /// Интерфейсом управляет NetworkManager или systemd-networkd — служба его не трогает.
    pub managed_elsewhere: bool,
}

/// Сколько процессов программы сейчас идут по своему выбору.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AppRunning {
    pub app: String,
    pub policy: String,
    pub processes: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub config: Config,
    pub uplinks: Vec<Uplink>,
    /// Правила маршрутизации установлены (каналов два и больше).
    pub active: bool,
    /// Почему служба ничего не меняет (VPN, один канал…).
    pub note: Option<String>,
    pub running: Vec<AppRunning>,
    /// Может ли спросивший менять настройки (root или группа `wheel`).
    pub can_edit: bool,
    /// USB-модемы, подключённые сейчас.
    #[serde(default)]
    pub usb_modems: Vec<UsbModem>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Заменить настройки целиком; ответ — новое состояние.
    SetConfig { config: Config },
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
}

/// Установлена ли служба.
pub fn installed() -> bool {
    Path::new("/usr/bin/synnetd").exists() || Path::new("/usr/lib/systemd/system/synnetd.service").exists()
}

/// Отправить запрос службе.
pub fn request(req: &Request) -> std::io::Result<Response> {
    let path = std::env::var("SYNNETD_SOCKET").unwrap_or_else(|_| SOCKET.to_string());
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut out = String::new();
    BufReader::new(s).read_line(&mut out)?;
    let r: Response = serde_json::from_str(&out)?;
    if !r.ok {
        return Err(std::io::Error::other(r.error.unwrap_or_default()));
    }
    Ok(r)
}

pub fn status() -> std::io::Result<Status> {
    request(&Request::Status)?.status.ok_or_else(|| std::io::Error::other("пустой ответ"))
}

pub fn set_config(config: Config) -> std::io::Result<Status> {
    request(&Request::SetConfig { config })?.status.ok_or_else(|| std::io::Error::other("пустой ответ"))
}

/// Подходит ли процесс под правило программы: совпадает имя процесса
/// (`comm`), имя исполняемого файла, полный путь, а у интерпретаторов
/// (`python3 /usr/bin/deluge`) — имя скрипта.
pub fn program_matches(rule: &str, exe: &str, comm: &str, argv: &[String]) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    if rule.contains('/') {
        return rule == exe || argv.first().is_some_and(|a| a == rule) || argv.get(1).is_some_and(|a| a == rule);
    }
    // `comm` ядро обрезает до 15 байт.
    if rule == base(exe) || rule == comm || (comm.len() == 15 && rule.starts_with(comm)) {
        return true;
    }
    if argv.first().is_some_and(|a| base(a) == rule) {
        return true;
    }
    // Интерпретатор: первый аргумент, не начинающийся с «-».
    let interp = ["python", "python3", "perl", "ruby", "node", "java", "bash", "sh", "mono", "dotnet"];
    if interp.iter().any(|i| base(exe).starts_with(i)) {
        if let Some(script) = argv.iter().skip(1).find(|a| !a.starts_with('-')) {
            return base(script) == rule;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matches() {
        assert!(program_matches("qbittorrent", "/usr/bin/qbittorrent", "qbittorrent", &v(&["qbittorrent"])));
        assert!(program_matches("/usr/bin/qbittorrent", "/usr/bin/qbittorrent", "qbittorrent", &[]));
        assert!(program_matches("deluge", "/usr/bin/python3.13", "deluge", &v(&["/usr/bin/python3", "/usr/bin/deluge"])));
        assert!(program_matches("deluge-gtk", "/usr/bin/python3.13", "python3", &v(&["/usr/bin/python3", "-s", "/usr/bin/deluge-gtk"])));
        assert!(program_matches("transmission-gtk", "/usr/bin/transmission-gtk", "transmission-gt", &[]));
        assert!(!program_matches("firefox", "/usr/bin/qbittorrent", "qbittorrent", &v(&["qbittorrent"])));
        assert!(!program_matches("", "/usr/bin/x", "x", &[]));
    }

    #[test]
    fn policies() {
        for p in [AppPolicy::Primary, AppPolicy::Both, AppPolicy::Iface("wlan0".into())] {
            assert_eq!(AppPolicy::from_id(&p.id()), Some(p.clone()));
            assert_eq!(AppPolicy::from_cgroup_name(&p.cgroup_name()), Some(p));
        }
        assert_eq!(AppPolicy::from_id("iface:"), None);
        assert_eq!(AppPolicy::from_id("iface:a/b"), None);
        assert_eq!(AppPolicy::from_cgroup_name("synnet-weird"), None);
        // Метки каналов стабильны, различаются и не совпадают с «основной»/«все».
        assert_eq!(iface_mark("wlan0"), iface_mark("wlan0"));
        assert_ne!(iface_mark("wlan0"), iface_mark("enp128s20f0u1"));
        assert_eq!(iface_mark("usb0") & 0xfff0_0000, 0x5360_0000);
    }

    #[test]
    fn config_roundtrip() {
        let mut c = Config::default();
        c.mode = Mode::Both;
        c.apps.insert("qbittorrent".into(), AppPolicy::Both);
        c.apps.insert("firefox".into(), AppPolicy::Iface("enp128s20f0u1".into()));
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains(r#""firefox":"iface:enp128s20f0u1""#), "{j}");
        assert_eq!(serde_json::from_str::<Config>(&j).unwrap(), c);
        // Пустой JSON — значения по умолчанию.
        assert_eq!(serde_json::from_str::<Config>("{}").unwrap(), Config::default());
    }
}

/// Запомнить выбор для USB-модема (`None` — забыть: спросить снова).
pub fn set_usb_choice(key: &str, name: &str, choice: Option<UsbChoice>) -> std::io::Result<Status> {
    let mut c = status()?.config;
    match choice {
        Some(choice) => {
            c.usb_devices.insert(key.to_string(), UsbDecision { choice, name: name.to_string() });
        }
        None => {
            c.usb_devices.remove(key);
        }
    }
    set_config(c)
}
