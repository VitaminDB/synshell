//! Курьер: связь с системой через мессенджеры. Служба `syncourier` (служба
//! пользователя systemd) держит каналы — пока WhatsApp, дальше Telegram и
//! почта — и ведёт разговор с Claude Code: свой сеанс для телефона плюс
//! передача сообщений в живые сеансы (`/to`). Шлёт уведомления: события
//! Claude (готово, нужен ответ) и, по настройке, уведомления системы.
//!
//! Здесь — то, что нужно и службе, и клиентам (апплет оболочки, Параметры,
//! хуки Claude Code): настройки, протокол сокета и синхронный клиент.
//!
//! Протокол: сокет `$XDG_RUNTIME_DIR/syncourier.sock`, одна строка JSON
//! запроса → одна строка JSON ответа.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Сокет службы.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNCOURIER_SOCKET") {
        return PathBuf::from(p);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe_uid())));
    dir.join("syncourier.sock")
}

fn unsafe_uid() -> u32 {
    std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap_or(1000)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Каталог настроек synshell (`SYNSHELL_CONFIG_DIR`, `~/.config/synshell`, старый `~/.config/syndesktop`).
pub fn config_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNSHELL_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    let new = base.join("synshell");
    let old = base.join("syndesktop");
    if !new.exists() && old.exists() {
        old
    } else {
        new
    }
}

/// Файл настроек Курьера.
pub fn config_path() -> PathBuf {
    config_dir().join("courier.toml")
}

/// Данные службы: сессия WhatsApp (права 600), состояние сеанса Claude.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNCOURIER_DATA_DIR") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"));
    base.join("syncourier")
}

// ─────────────────────────────────────────────────────────────── настройки

/// Какие уведомления системы пересылать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyMode {
    /// Только события Claude Code (готово, нужен ответ, вызов пользователя).
    #[default]
    Claude,
    /// Все уведомления, но только когда за компьютером никого нет.
    Away,
    /// Все уведомления всегда.
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhatsappConfig {
    pub enabled: bool,
}

impl Default for WhatsappConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Включён ли разговор с Claude Code.
    pub enabled: bool,
    /// Имя телефонного сеанса (так его видят другие сеансы: `SendMessage` → «courier»).
    pub name: String,
    /// Рабочая папка сеанса.
    pub cwd: String,
    /// Модель (`opus`, `sonnet`, полное имя); пусто — по умолчанию Claude Code.
    pub model: String,
    /// Режим разрешений: `bypassPermissions` (как у живых сеансов — тогда они принимают
    /// сообщения друг друга), `acceptEdits`, `default`…
    pub permission_mode: String,
    /// Программа `claude`.
    pub program: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            name: "courier".into(),
            cwd: "~".into(),
            model: String::new(),
            permission_mode: "bypassPermissions".into(),
            program: "claude".into(),
        }
    }
}

/// Когда пересылать события Claude Code (ход закончен, нужен ответ, вызов пользователя).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeEvents {
    /// Только когда за компьютером никого нет (за компьютером хватает голоса и уведомлений).
    #[default]
    Away,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    pub mode: NotifyMode,
    /// События Claude Code.
    pub claude: ClaudeEvents,
    /// «Нет за компьютером»: экран заблокирован или столько минут без ввода.
    pub away_minutes: u32,
    /// Программы (app_name уведомления), которые не пересылать.
    pub exclude: Vec<String>,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self { mode: NotifyMode::Claude, claude: ClaudeEvents::Away, away_minutes: 5, exclude: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub whatsapp: WhatsappConfig,
    pub claude: ClaudeConfig,
    pub notify: NotifyConfig,
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = config_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }
}

/// `~/…` → полный путь.
pub fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(p),
    }
}

// ─────────────────────────────────────────────────────────────── протокол

/// Состояние канала.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    /// Выключен в настройках.
    #[default]
    Off,
    /// Не привязан: нужно подключить (QR).
    Unlinked,
    /// Показан QR-код, ждём сканирования.
    Linking,
    Connecting,
    Online,
    Error,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChannelStatus {
    /// `whatsapp`, `telegram`, `email`.
    pub id: String,
    pub title: String,
    pub state: ChannelState,
    /// Учётная запись: номер телефона, @имя, адрес.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Строка QR-кода для привязки (когда `Linking`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ClaudeStatus {
    pub enabled: bool,
    pub name: String,
    pub cwd: String,
    /// Сеанс запущен.
    pub running: bool,
    /// Идёт ход (Claude думает над сообщением).
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Status {
    pub channels: Vec<ChannelStatus>,
    pub claude: ClaudeStatus,
    pub notify: NotifyMode,
    /// Последние сообщения (для апплета): кто, текст, когда (мс).
    #[serde(default)]
    pub recent: Vec<Recent>,
}

impl Status {
    pub fn channel(&self, id: &str) -> Option<&ChannelStatus> {
        self.channels.iter().find(|c| c.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Recent {
    /// `in` — от пользователя, `out` — от системы.
    pub dir: String,
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Начать привязку канала (WhatsApp — QR-код в `Status`).
    Link { channel: String },
    /// Отвязать канал (выйти из учётной записи).
    Unlink { channel: String },
    /// Уведомление: от Claude Code (хуки, `claude-call-user`) или от программ.
    /// `source` = `claude` — всегда; иначе — по режиму уведомлений.
    Notify {
        #[serde(default)]
        title: String,
        body: String,
        #[serde(default)]
        source: String,
    },
    /// Событие хука Claude Code (JSON со входа хука как есть): служба сама
    /// находит имя сеанса и последний ответ, клиент не ждёт.
    Hook { event: serde_json::Value },
    /// Отправить файл пользователю (картинка, видео, аудио, документ; `voice` — как голосовое).
    SendFile {
        path: String,
        #[serde(default)]
        caption: String,
        #[serde(default)]
        voice: bool,
    },
    /// Сказать голосом: синтез речи synaptix → голосовое сообщение.
    Say { text: String },
    /// Сообщение «как от пользователя» с этого компьютера (апплет, проверка):
    /// разбирается так же, ответ уходит в каналы и в `Status::recent`.
    Ask { text: String },
    /// Перечитать настройки.
    Reload,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

/// Запрос к службе (синхронно, с таймаутом).
pub fn request(req: &Request) -> Result<Response, String> {
    let s = UnixStream::connect(socket_path()).map_err(|e| format!("служба syncourier не запущена: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    s.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    (&s).write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    BufReader::new(&s).read_line(&mut out).map_err(|e| e.to_string())?;
    let r: Response = serde_json::from_str(&out).map_err(|e| format!("ответ службы: {e}"))?;
    match (&r.ok, &r.error) {
        (false, Some(e)) => Err(e.clone()),
        _ => Ok(r),
    }
}

/// Состояние службы.
pub fn status() -> Result<Status, String> {
    request(&Request::Status)?.status.ok_or_else(|| "нет состояния".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trip() {
        let r = Request::Notify { title: "Claude".into(), body: "готово".into(), source: "claude".into() };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"cmd\":\"notify\""));
        assert_eq!(serde_json::from_str::<Request>(&j).unwrap(), r);
        let j = r#"{"cmd":"notify","body":"x"}"#;
        assert_eq!(serde_json::from_str::<Request>(j).unwrap(), Request::Notify { title: String::new(), body: "x".into(), source: String::new() });
    }

    #[test]
    fn config_defaults_and_partial() {
        let c: Config = toml::from_str("[notify]\nmode = \"away\"\n").unwrap();
        assert_eq!(c.notify.mode, NotifyMode::Away);
        assert_eq!(c.notify.away_minutes, 5);
        assert_eq!(c.claude.name, "courier");
        assert!(c.whatsapp.enabled);
        assert_eq!(expand_home("~/x"), home().join("x"));
    }
}
