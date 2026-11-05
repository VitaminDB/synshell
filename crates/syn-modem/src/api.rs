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
    pub data: Data,
    /// Приёмник GNSS работает (кто-то читает местоположение).
    #[serde(default)]
    pub gnss: bool,
}

/// Местоположение от приёмника GNSS (поток `GnssWatch`, раз в секунду).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Fix {
    /// Есть свежее решение (координаты ниже — его); `false` — ещё ищет спутники.
    pub valid: bool,
    pub latitude: f64,
    pub longitude: f64,
    /// Высота над уровнем моря, м.
    pub altitude: Option<f32>,
    /// Высота над эллипсоидом WGS-84, м.
    pub altitude_ellipsoid: Option<f32>,
    /// Горизонтальная погрешность (радиус), м.
    pub accuracy: Option<f32>,
    pub vertical_accuracy: Option<f32>,
    /// Скорость, м/с.
    pub speed: Option<f32>,
    /// Курс, градусы от севера.
    pub heading: Option<f32>,
    pub hdop: Option<f32>,
    pub pdop: Option<f32>,
    pub vdop: Option<f32>,
    /// Время решения UTC, мс от эпохи.
    pub time_ms: i64,
    /// Спутники, участвующие в решении, и видимые (с сигналом).
    pub satellites_used: u32,
    pub satellites_visible: u32,
    /// Видимые спутники.
    pub satellites: Vec<Satellite>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Satellite {
    /// «GPS», «ГЛОНАСС», «Galileo», «BeiDou», «QZSS», «SBAS», «NavIC».
    pub system: String,
    pub id: u16,
    pub elevation: f32,
    pub azimuth: f32,
    /// Сигнал/шум, дБ·Гц.
    pub snr: f32,
    pub used: bool,
}

/// Мобильная передача данных.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Data {
    /// Пользователь включил передачу данных (запоминается).
    pub enabled: bool,
    pub state: DataState,
    /// Адрес IPv4 сеанса.
    pub address: String,
    /// Почему не подключились (последняя ошибка).
    pub error: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DataState {
    #[default]
    Off,
    /// Ждёт сети (включено, но нет регистрации или радио выключено).
    Waiting,
    Connecting,
    Connected,
    Error,
}


/// Сведения о модеме и SIM (раздел «Мобильная сеть → О модеме и SIM»).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Info {
    pub imei: String,
    pub imei_sv: String,
    pub meid: String,
    /// Прошивка модема (MPSS).
    pub revision: String,
    pub hw_revision: String,
    pub sw_version: String,
    /// Номер телефона с SIM (если оператор его записал).
    pub msisdn: String,
    pub iccid: String,
    pub imsi: String,
    /// Имя оператора с SIM (EF_SPN).
    pub spn: String,
    pub eid: String,
    pub slots: Vec<Slot>,
    /// Поддерживаемые диапазоны LTE и 5G NR.
    pub lte_bands: Vec<u16>,
    pub nr_bands: Vec<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    /// Физический слот (с 1).
    pub physical: u8,
    pub card: bool,
    pub active: bool,
    pub logical: u8,
    pub iccid: String,
    pub euicc: bool,
}

/// Текущая сота и радио.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub technology: String,
    pub plmn: String,
    pub lac: Option<u32>,
    pub tac: Option<u32>,
    pub cell_id: Option<u32>,
    /// «B3», «n78», «GSM 900»…
    pub band: String,
    pub channel: Option<u32>,
    pub bandwidth_mhz: Option<f32>,
    pub rssi: Option<i32>,
    pub rsrp: Option<i32>,
    pub rsrq: Option<i32>,
    pub snr: Option<f32>,
}

/// Сеть из поиска.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    pub mcc: u16,
    pub mnc: u16,
    pub name: String,
    pub technologies: Vec<String>,
    pub current: bool,
    pub forbidden: bool,
    pub home: bool,
}

/// Выбор сети и технологий.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Modes {
    /// Разрешённые технологии: «5g», «4g», «3g», «2g».
    pub allowed: Vec<String>,
    /// Ручной выбор сети: MCC/MNC; `None` — автоматически.
    pub manual: Option<(u16, u16)>,
    /// Передача данных в роуминге разрешена.
    pub data_roaming: bool,
}

/// Точка доступа (профиль WDS модема).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Apn {
    /// Номер профиля в модеме (0 — новый).
    pub index: u8,
    pub name: String,
    pub apn: String,
    pub user: String,
    pub password: String,
    /// «none», «pap», «chap», «pap-chap».
    pub auth: String,
    /// «ipv4», «ipv6», «ipv4v6».
    pub ip: String,
    /// Профиль для мобильного интернета.
    pub default: bool,
    pub roaming_disallowed: bool,
}

/// Услуги вызовов.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallServices {
    /// Ожидание вызова (`None` — сеть не ответила).
    pub waiting: Option<bool>,
    /// Скрывать свой номер: «network» (по умолчанию сети), «hide», «show».
    pub clir: String,
    pub forwards: Vec<Forward>,
    /// Почему сеть не дала настройки (если не дала).
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Forward {
    /// «always», «busy», «no-reply», «unreachable».
    pub reason: String,
    pub active: bool,
    pub number: String,
    pub timer: Option<u8>,
}

/// Расход мобильного трафика.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    /// Месяц «2026-10».
    pub month: String,
    pub rx: u64,
    pub tx: u64,
    /// С начала подсчёта (сброс вручную).
    pub total_rx: u64,
    pub total_tx: u64,
    pub since: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sim {
    pub state: SimState,
    /// Встроенная SIM (eUICC).
    pub esim: bool,
    /// Оператор SIM (домашняя сеть).
    pub home_operator: String,
    pub pin_retries: Option<u8>,
    /// Запрос PIN при включении (`None` — неизвестно).
    pub pin_enabled: Option<bool>,
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
    /// Включить приёмник GNSS на время соединения: оно становится потоком [`Fix`] (по строке в секунду).
    GnssWatch,
    /// Радио вкл/выкл (режим полёта), запоминается.
    SetRadio { on: bool },
    /// Мобильная передача данных вкл/выкл, запоминается.
    SetData { on: bool },
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
    Info,
    Cell,
    EsimProfiles,
    EsimEnable { iccid: String },
    EsimDisable { iccid: String },
    EsimDelete { iccid: String },
    EsimNickname { iccid: String, name: String },
    /// Загрузить профиль по коду активации из QR оператора (`LPA:1$сервер$код`), при
    /// необходимости — с кодом подтверждения; ход — событиями `EsimProgress` (до нескольких минут).
    EsimDownload { code: String, confirmation: String },
    /// Поиск сетей (до нескольких минут).
    NetworkScan,
    /// Регистрация: `None` — автоматически, иначе MCC/MNC.
    NetworkSelect { plmn: Option<(u16, u16)> },
    Modes,
    SetModes { allowed: Vec<String> },
    SetDataRoaming { on: bool },
    ApnList,
    ApnSave { apn: Apn },
    ApnDelete { index: u8 },
    /// PIN: включить/выключить запрос, сменить, ввести, разблокировать PUK.
    PinEnable { on: bool, pin: String },
    PinChange { old: String, new: String },
    PinVerify { pin: String },
    PinUnblock { puk: String, new: String },
    Smsc,
    SetSmsc { number: String },
    /// USSD-запрос («*100#»); ответ — событием `Ussd`.
    Ussd { code: String },
    UssdReply { text: String },
    UssdCancel,
    CallServices,
    SetCallWaiting { on: bool },
    SetClir { mode: String },
    /// Переадресация: пустой номер — выключить.
    SetForward { reason: String, number: String, timer: Option<u8> },
    Usage,
    UsageReset,
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
    Info { info: Info },
    Cell { cell: Cell },
    EsimProfiles { profiles: Vec<crate::euicc::Profile> },
    Networks { networks: Vec<Network> },
    Modes { modes: Modes },
    ApnList { apns: Vec<Apn> },
    Text { text: String },
    CallServices { services: CallServices },
    Usage { usage: Usage },
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
    /// Ответ сети на USSD; `reply` — сеть ждёт ответа пользователя.
    Ussd { text: String, reply: bool, done: bool },
    /// Ход загрузки профиля eSIM: шаг; `done` — закончена (`error` пуст — успешно).
    EsimProgress { step: String, done: bool, error: String },
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

/// Загрузить профиль eSIM (ждёт до 10 минут; ход — событиями `EsimProgress` подписки).
pub fn esim_download(code: &str, confirmation: &str) -> Result<()> {
    let s = UnixStream::connect(SOCKET).with_context(|| format!("synmodemd не запущен ({SOCKET})"))?;
    s.set_read_timeout(Some(Duration::from_secs(600)))?;
    let mut w = s.try_clone()?;
    let mut line = serde_json::to_string(&Request::EsimDownload { code: code.to_string(), confirmation: confirmation.to_string() })?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    let mut resp = String::new();
    BufReader::new(s).read_line(&mut resp)?;
    if resp.is_empty() {
        bail!("synmodemd закрыл соединение");
    }
    match serde_json::from_str::<Response>(&resp)? {
        Response::Error { message } => bail!("{message}"),
        _ => Ok(()),
    }
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

/// Поток местоположения: приёмник работает, пока `f` возвращает `true`.
pub fn gnss_watch(mut f: impl FnMut(Fix) -> bool) -> Result<()> {
    let s = UnixStream::connect(SOCKET).with_context(|| format!("synmodemd не запущен ({SOCKET})"))?;
    let mut w = s.try_clone()?;
    let mut line = serde_json::to_string(&Request::GnssWatch)?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    for l in BufReader::new(s).lines() {
        let l = l?;
        match serde_json::from_str::<Fix>(&l) {
            Ok(fix) => {
                if !f(fix) {
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
