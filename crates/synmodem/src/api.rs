//! Протокол `synmodemd`: unix-сокет [`SOCKET`], по одному JSON-запросу в строке, ответ — одна строка JSON.
//! Запрос `Subscribe` превращает соединение в поток событий ([`Event`], по строке на событие).
//! Состояние сети видно всем; SMS, звонки и управление — root и группам `wheel`/`network`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const SOCKET: &str = "/run/synmodem/synmodemd.sock";

/// Состояние модема и сети.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Status {
    /// Службы модема доступны (модем запущен).
    pub present: bool,
    /// Радио включено (`false` — режим полёта).
    pub radio: bool,
    pub imei: String,
    pub sim: Sim,
    pub registration: Registration,
    /// Имя сети, в которой зарегистрирован (или на которой стоит без регистрации).
    pub operator: String,
    /// MCC/MNC текущей сети («401/02»).
    pub plmn: String,
    /// Технология: «5G», «LTE», «3G», «2G» или пусто.
    pub technology: String,
    pub roaming: bool,
    /// Полоски сигнала 0..=4 (`None` — нет сети).
    pub bars: Option<u8>,
    /// Уровень сигнала, дБм (RSRP для LTE/5G, RSSI для 2G/3G).
    pub dbm: Option<i32>,
    /// Причина отказа сети в регистрации (3GPP 24.008), если была.
    pub reject_cause: Option<u8>,
    pub unread_sms: u32,
    pub calls: Vec<Call>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sim {
    pub state: SimState,
    /// Встроенная SIM (eUICC).
    pub esim: bool,
    /// Оператор SIM (домашняя сеть).
    pub home_operator: String,
    pub pin_retries: Option<u8>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SimState {
    #[default]
    Unknown,
    Absent,
    /// Карта есть, приложение USIM ещё не готово.
    Initializing,
    PinRequired,
    PukRequired,
    Blocked,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Registration {
    #[default]
    Unknown,
    NotRegistered,
    Searching,
    /// Стоит на соте без регистрации: только экстренные вызовы.
    Limited,
    Denied,
    Home,
    Roaming,
}

impl Registration {
    pub fn registered(self) -> bool {
        matches!(self, Self::Home | Self::Roaming)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CallState {
    /// Исходящий: набор (до гудков).
    Dialing,
    /// Исходящий: у абонента звонит.
    Alerting,
    /// Входящий звонит.
    Incoming,
    /// Второй входящий во время разговора.
    Waiting,
    Active,
    Held,
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    pub id: u8,
    pub number: String,
    pub state: CallState,
    pub incoming: bool,
    /// Начало разговора (секунды UNIX), пока не ответили — `None`.
    pub answered_at: Option<i64>,
    pub started_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SmsStatus {
    Received,
    Sending,
    Sent,
    Failed,
}

/// SMS в хранилище демона (части длинного сообщения уже склеены).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sms {
    pub id: u64,
    pub number: String,
    pub text: String,
    /// Секунды UNIX (входящее — время SMS-центра, исходящее — отправки).
    pub time: i64,
    pub incoming: bool,
    pub read: bool,
    pub status: SmsStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CallKind {
    Incoming,
    Outgoing,
    Missed,
    Rejected,
}

/// Запись журнала звонков.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallRecord {
    pub id: u64,
    pub number: String,
    pub kind: CallKind,
    pub time: i64,
    /// Длительность разговора, секунды.
    pub duration: u32,
    /// Пропущенный ещё не просмотрен.
    pub new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    Status,
    /// Радио вкл/выкл (режим полёта), запоминается.
    SetRadio { on: bool },
    SmsList,
    SmsSend { number: String, text: String },
    /// Отметить прочитанными все сообщения переписки с номером.
    SmsRead { number: String },
    SmsDelete { ids: Vec<u64> },
    Dial { number: String },
    Answer { id: u8 },
    Hangup { id: u8 },
    /// Тоновый сигнал в разговоре.
    Dtmf { id: u8, digit: char },
    CallLog,
    /// Пропущенные просмотрены.
    CallLogSeen,
    CallLogDelete { ids: Vec<u64> },
    /// Поток событий до закрытия соединения.
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    Ok,
    Error { message: String },
    Status { status: Status },
    SmsList { messages: Vec<Sms> },
    Sms { message: Sms },
    CallId { id: u8 },
    CallLog { calls: Vec<CallRecord> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Status { status: Status },
    /// Новое входящее или изменилось исходящее (статус отправки).
    Sms { message: Sms },
    /// Хранилище SMS изменилось целиком (удаление, прочтение).
    SmsChanged,
    CallLog,
}

/// Один запрос к демону.
pub fn request(req: &Request) -> Result<Response> {
    let s = UnixStream::connect(SOCKET).with_context(|| format!("synmodemd не запущен ({SOCKET})"))?;
    s.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut w = s.try_clone()?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    let mut resp = String::new();
    BufReader::new(s).read_line(&mut resp)?;
    if resp.is_empty() {
        bail!("synmodemd закрыл соединение");
    }
    let r: Response = serde_json::from_str(&resp)?;
    if let Response::Error { message } = r {
        bail!("{message}");
    }
    Ok(r)
}

pub fn status() -> Result<Status> {
    match request(&Request::Status)? {
        Response::Status { status } => Ok(status),
        r => bail!("неожиданный ответ {r:?}"),
    }
}

pub fn sms_list() -> Result<Vec<Sms>> {
    match request(&Request::SmsList)? {
        Response::SmsList { messages } => Ok(messages),
        r => bail!("неожиданный ответ {r:?}"),
    }
}

pub fn call_log() -> Result<Vec<CallRecord>> {
    match request(&Request::CallLog)? {
        Response::CallLog { calls } => Ok(calls),
        r => bail!("неожиданный ответ {r:?}"),
    }
}

/// Подписка на события: `f` вызывается на каждое событие, пока возвращает `true` и демон жив.
pub fn subscribe(mut f: impl FnMut(Event) -> bool) -> Result<()> {
    let s = UnixStream::connect(SOCKET).with_context(|| format!("synmodemd не запущен ({SOCKET})"))?;
    let mut w = s.try_clone()?;
    let mut line = serde_json::to_string(&Request::Subscribe)?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    for l in BufReader::new(s).lines() {
        let l = l?;
        match serde_json::from_str::<Event>(&l) {
            Ok(ev) => {
                if !f(ev) {
                    break;
                }
            }
            Err(_) => {
                if let Ok(Response::Error { message }) = serde_json::from_str::<Response>(&l) {
                    bail!("{message}");
                }
            }
        }
    }
    Ok(())
}
