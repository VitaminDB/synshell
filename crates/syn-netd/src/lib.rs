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
    /// Без сети: новые сокеты IPv4/IPv6 не создаются (EPERM), открытые
    /// соединения обрываются (метка с правилом `prohibit`).
    Block,
}

impl AppPolicy {
    pub fn id(&self) -> String {
        match self {
            AppPolicy::Primary => "primary".into(),
            AppPolicy::Both => "both".into(),
            AppPolicy::Iface(i) => format!("iface:{i}"),
            AppPolicy::Block => "block".into(),
        }
    }

    pub fn from_id(s: &str) -> Option<AppPolicy> {
        match s {
            "primary" => Some(AppPolicy::Primary),
            "both" => Some(AppPolicy::Both),
            "block" => Some(AppPolicy::Block),
            _ => s.strip_prefix("iface:").filter(|i| valid_iface(i)).map(|i| AppPolicy::Iface(i.to_string())),
        }
    }

    /// Имя под-cgroup программы: `synnet-both`, `synnet-primary`, `synnet-if-wlan0`.
    pub fn cgroup_name(&self) -> String {
        match self {
            AppPolicy::Primary => "synnet-primary".into(),
            AppPolicy::Both => "synnet-both".into(),
            AppPolicy::Iface(i) => format!("synnet-if-{i}"),
            AppPolicy::Block => "synnet-block".into(),
        }
    }

    pub fn from_cgroup_name(s: &str) -> Option<AppPolicy> {
        match s.strip_prefix("synnet-")? {
            "primary" => Some(AppPolicy::Primary),
            "both" => Some(AppPolicy::Both),
            "block" => Some(AppPolicy::Block),
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
    /// Учёт трафика по программам и журнал соединений.
    pub traffic: TrafficConfig,
    /// Ограничение скорости программ (ключ — как в [`Config::apps`]).
    pub limits: BTreeMap<String, Limit>,
    /// Ограничение скорости каналов: имя интерфейса → общий предел для всего трафика программ через него.
    pub uplink_limits: BTreeMap<String, Limit>,
}

/// Ограничение скорости программы, КБ/с; 0 — без ограничения.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Limit {
    pub down_kbps: u32,
    pub up_kbps: u32,
}

impl Limit {
    pub fn is_none(&self) -> bool {
        self.down_kbps == 0 && self.up_kbps == 0
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { mode: Mode::Primary, primary: String::new(), apps: BTreeMap::new(), usb_dhcp: true, usb_devices: BTreeMap::new(), traffic: TrafficConfig::default(), limits: BTreeMap::new(), uplink_limits: BTreeMap::new() }
    }
}

/// Учёт трафика: расход по программам (по часам, архив по дням) и кольцевой
/// журнал соединений (кто, куда, когда, сколько).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TrafficConfig {
    pub enabled: bool,
    /// Журнал соединений: предел на диске, МБ (старые записи вытесняются).
    pub log_mb: u32,
    /// Архив расхода по программам: сколько дней хранить.
    pub days: u32,
    /// Записывать в журнал имена узлов (ответы DNS от systemd-resolved).
    pub hostnames: bool,
    /// Уведомлять, когда программа впервые выходит в сеть.
    pub notify_new: bool,
}

impl Default for TrafficConfig {
    fn default() -> Self {
        TrafficConfig { enabled: true, log_mb: 64, days: 400, hostnames: true, notify_new: true }
    }
}

/// Программа, впервые вышедшая в сеть.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NewApp {
    pub app: String,
    pub uid: u32,
    #[serde(default)]
    pub comm: String,
    /// Когда, мс Unix.
    pub at: u64,
    /// Куда соединилась первым делом.
    #[serde(default)]
    pub remote: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub host: Option<String>,
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
    /// Трафик программ сейчас и за сегодня.
    Traffic,
    /// Соединения из журнала (и открытые сейчас), новые первыми.
    Connections { query: ConnQuery },
    /// Архив расхода по дням (`from`, `to` — `ГГГГ-ММ-ДД` по местному времени, включительно).
    Usage { from: String, to: String },
    /// Очистить журнал соединений и архив (root и `wheel`).
    ClearTraffic,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<Traffic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<Conn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<DayUsage>>,
}

// ─── Учёт трафика ───────────────────────────────────────────────────────────

/// Программа без своего процесса: ответы ядра, пересылка, VPN-туннель —
/// разница между счётчиками канала и тем, что пришлось на программы.
pub const APP_SYSTEM: &str = "";

/// Трафик программы. Программа — имя исполняемого файла (у интерпретаторов —
/// скрипта, у Wine — имя `.exe`), [`APP_SYSTEM`] — система.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AppTraffic {
    pub app: String,
    /// Имя процесса (`comm`) последнего соединения.
    #[serde(default)]
    pub comm: String,
    pub uid: u32,
    /// Скорость за последнюю секунду, байт/с.
    pub rx_bps: f64,
    pub tx_bps: f64,
    /// За сегодня (с полуночи по местному времени), байт.
    pub rx_today: u64,
    pub tx_today: u64,
    /// Открытых соединений сейчас.
    pub active: u32,
    /// Соединений за сегодня.
    #[serde(default)]
    pub conns_today: u32,
    /// За сегодня по каналам: интерфейс, принято, передано.
    #[serde(default)]
    pub ifaces: Vec<(String, u64, u64)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Traffic {
    pub enabled: bool,
    /// Учёт не работает (ядро без BPF и т. п.).
    #[serde(default)]
    pub error: Option<String>,
    pub apps: Vec<AppTraffic>,
    /// Журнал соединений: занято на диске, байт; предел; самая старая запись (мс Unix).
    #[serde(default)]
    pub log_bytes: u64,
    #[serde(default)]
    pub log_limit: u64,
    #[serde(default)]
    pub log_since: Option<u64>,
    /// Имена узлов приходят от systemd-resolved.
    #[serde(default)]
    pub dns: bool,
    /// Спросивший видит всё (root, `wheel`) или только свои программы.
    #[serde(default)]
    pub all_users: bool,
    /// Программы, впервые вышедшие в сеть (последние, новые в конце).
    #[serde(default)]
    pub new_apps: Vec<NewApp>,
    /// Уведомлять о них ([`TrafficConfig::notify_new`]).
    #[serde(default)]
    pub notify_new: bool,
}

/// Соединение (запись журнала или открытое сейчас).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Conn {
    /// Начало и конец, мс Unix; конца нет — соединение открыто.
    #[serde(rename = "t")]
    pub start: u64,
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    #[serde(rename = "a")]
    pub app: String,
    #[serde(rename = "c", default)]
    pub comm: String,
    #[serde(rename = "p", default)]
    pub pid: i32,
    #[serde(rename = "u", default)]
    pub uid: u32,
    /// `tcp`, `udp`, `icmp`…
    #[serde(rename = "pr")]
    pub proto: String,
    /// Удалённый адрес и порт.
    #[serde(rename = "r")]
    pub remote: String,
    #[serde(rename = "po", default)]
    pub port: u16,
    /// Имя узла (из ответа DNS), если известно.
    #[serde(rename = "h", default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Канал (интерфейс последнего пакета).
    #[serde(rename = "i", default)]
    pub iface: String,
    #[serde(default)]
    pub rx: u64,
    #[serde(default)]
    pub tx: u64,
    /// Входящее (к службе на этой машине), а не исходящее.
    #[serde(rename = "in", default, skip_serializing_if = "std::ops::Not::not")]
    pub incoming: bool,
    /// UDP-сокет без `connect`, писавший многим адресам: сколько ещё адресов
    /// (торрент, DHT) не записаны отдельно.
    #[serde(rename = "m", default, skip_serializing_if = "is_zero")]
    pub more: u32,
    /// Другие адреса того же сокета (`адрес:порт`, до 16).
    #[serde(rename = "x", default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Отбор соединений.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ConnQuery {
    /// Программа ([`AppTraffic::app`]); `None` — все.
    pub app: Option<String>,
    /// Промежуток, мс Unix.
    pub from: Option<u64>,
    pub to: Option<u64>,
    /// Подстрока в адресе, имени узла, процессе, канале.
    pub text: String,
    /// Только открытые сейчас.
    pub active_only: bool,
    pub limit: u32,
}

impl Default for ConnQuery {
    fn default() -> Self {
        ConnQuery { app: None, from: None, to: None, text: String::new(), active_only: false, limit: 2000 }
    }
}

impl Conn {
    pub fn matches(&self, q: &ConnQuery) -> bool {
        if q.app.as_ref().is_some_and(|a| *a != self.app) {
            return false;
        }
        if q.active_only && self.end.is_some() {
            return false;
        }
        if q.from.is_some_and(|f| self.end.unwrap_or(u64::MAX) < f) || q.to.is_some_and(|t| self.start > t) {
            return false;
        }
        let text = q.text.trim().to_lowercase();
        if !text.is_empty() {
            let hay = format!("{} {} {} {} {} {} {}", self.remote, self.port, self.host.as_deref().unwrap_or(""), self.comm, self.iface, self.app, self.also.join(" ")).to_lowercase();
            if !text.split_whitespace().all(|w| hay.contains(w)) {
                return false;
            }
        }
        true
    }
}

/// Расход программы за день по одному каналу.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UsageRow {
    pub app: String,
    pub uid: u32,
    pub iface: String,
    pub rx: u64,
    pub tx: u64,
    /// По часам: принято + передано, байт (24 значения; в дни перехода на летнее время — как есть).
    #[serde(default)]
    pub hours: Vec<u64>,
    /// Соединений за день.
    #[serde(default)]
    pub conns: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DayUsage {
    /// `ГГГГ-ММ-ДД`.
    pub day: String,
    pub rows: Vec<UsageRow>,
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

pub fn traffic() -> std::io::Result<Traffic> {
    request(&Request::Traffic)?.traffic.ok_or_else(|| std::io::Error::other("служба без учёта трафика (обновите synnetd)"))
}

pub fn connections(query: ConnQuery) -> std::io::Result<Vec<Conn>> {
    request(&Request::Connections { query })?.connections.ok_or_else(|| std::io::Error::other("служба без учёта трафика (обновите synnetd)"))
}

pub fn usage(from: &str, to: &str) -> std::io::Result<Vec<DayUsage>> {
    request(&Request::Usage { from: from.into(), to: to.into() })?.usage.ok_or_else(|| std::io::Error::other("служба без учёта трафика (обновите synnetd)"))
}

/// Выбор для программы (`None` — как в системе).
pub fn set_app_policy(app: &str, policy: Option<AppPolicy>) -> std::io::Result<Status> {
    let mut c = status()?.config;
    match policy {
        Some(p) => {
            c.apps.insert(app.to_string(), p);
        }
        None => {
            c.apps.remove(app);
        }
    }
    set_config(c)
}

/// Ограничение скорости канала `iface` (пустое — снять).
pub fn set_uplink_limit(iface: &str, limit: Limit) -> std::io::Result<Status> {
    let mut c = status()?.config;
    if limit.is_none() {
        c.uplink_limits.remove(iface);
    } else {
        c.uplink_limits.insert(iface.to_string(), limit);
    }
    set_config(c)
}

/// Ограничение скорости программы (пустое — снять).
pub fn set_app_limit(app: &str, limit: Limit) -> std::io::Result<Status> {
    let mut c = status()?.config;
    if limit.is_none() {
        c.limits.remove(app);
    } else {
        c.limits.insert(app.to_string(), limit);
    }
    set_config(c)
}

pub fn clear_traffic() -> std::io::Result<()> {
    request(&Request::ClearTraffic).map(|_| ())
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
        for p in [AppPolicy::Primary, AppPolicy::Both, AppPolicy::Iface("wlan0".into()), AppPolicy::Block] {
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
