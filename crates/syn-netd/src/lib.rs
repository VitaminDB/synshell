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

/// Выбор для программы.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppPolicy {
    /// Только основной канал.
    Primary,
    /// Все каналы одновременно.
    Both,
}

impl AppPolicy {
    pub fn id(self) -> &'static str {
        match self {
            AppPolicy::Primary => "primary",
            AppPolicy::Both => "both",
        }
    }

    pub fn from_id(s: &str) -> Option<AppPolicy> {
        match s {
            "primary" => Some(AppPolicy::Primary),
            "both" => Some(AppPolicy::Both),
            _ => None,
        }
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
}

impl Default for Config {
    fn default() -> Self {
        Config { mode: Mode::Primary, primary: String::new(), apps: BTreeMap::new(), usb_dhcp: true }
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
    fn config_roundtrip() {
        let mut c = Config::default();
        c.mode = Mode::Both;
        c.apps.insert("qbittorrent".into(), AppPolicy::Both);
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&j).unwrap(), c);
        // Пустой JSON — значения по умолчанию.
        assert_eq!(serde_json::from_str::<Config>("{}").unwrap(), Config::default());
    }
}
