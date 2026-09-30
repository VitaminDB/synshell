//! Локальный протокол демона synlink (связь оболочек телефона и десктопа).
//!
//! JSON-строки по Unix-сокету [`socket_path`], как у IPC композитора:
//! одна строка-[`Request`] → одна строка-[`Response`]. После
//! `Request::Subscribe` приходят строки-[`Event`]. Запросы-потоки
//! (`Exec`, `Tcp`, `Screen`) после ответа `Ok` переводят соединение в
//! двоичный режим — см. описание каждого.
//!
//! Устройство в запросах — `id`, имя или `local` (эта машина: снимки,
//! ввод и команды без сети — удобно для отладки самого себя).

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("SYNLINK_SOCKET") {
        return PathBuf::from(p);
    }
    crate::paths::runtime_dir().join("synlink.sock")
}

/// Каталог точек монтирования файлов устройств.
pub fn mount_root() -> PathBuf {
    crate::paths::runtime_dir().join("synlink")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    #[default]
    Desktop,
    Laptop,
    Phone,
    Tablet,
}

impl DeviceKind {
    pub fn is_touch(self) -> bool {
        matches!(self, DeviceKind::Phone | DeviceKind::Tablet)
    }
    /// Подпись по-русски.
    pub fn title(self) -> &'static str {
        match self {
            DeviceKind::Desktop => "Компьютер",
            DeviceKind::Laptop => "Ноутбук",
            DeviceKind::Phone => "Телефон",
            DeviceKind::Tablet => "Планшет",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    #[default]
    Wifi,
    Usb,
}

impl Transport {
    pub fn title(self) -> &'static str {
        match self {
            Transport::Wifi => "Wi-Fi",
            Transport::Usb => "USB",
        }
    }
}

/// Эта машина.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SelfInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Разрешено ли находить эту машину в сети (Wi-Fi); по USB — всегда.
    pub discoverable: bool,
}

/// Состояние кабеля USB.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsbInfo {
    /// Кабель подключён и хост опознал устройство (у телефона — гаджет
    /// в состоянии `configured`; у компьютера — поднят сетевой интерфейс
    /// USB-устройства).
    pub cable: bool,
    /// Интерфейс USB-сети (`usb0`, `enp…u1`) и его адрес.
    pub interface: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Известное или замеченное рядом устройство.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Спарено (ключ закреплён).
    pub paired: bool,
    /// Соединение установлено.
    pub connected: bool,
    /// Чем соединено сейчас (или чем замечено).
    pub transport: Option<Transport>,
    pub address: Option<String>,
    /// Задержка до устройства, мс.
    pub rtt_ms: Option<f32>,
    pub battery: Option<Battery>,
    /// Пользователь и домашний каталог на устройстве.
    pub user: Option<String>,
    pub home: Option<String>,
    /// Где смонтированы файлы устройства (если смонтированы).
    pub mount: Option<String>,
    /// Имя для `ssh <host>` (см. `synlink ssh-config`).
    pub ssh_host: Option<String>,
    /// Последнее соединение, секунды UNIX.
    pub last_seen: Option<i64>,
}

impl PeerInfo {
    /// Файлы устройства: домашний каталог внутри точки монтирования.
    pub fn files_path(&self) -> Option<String> {
        let m = self.mount.as_ref()?;
        Some(match &self.home {
            Some(h) => format!("{}{}", m.trim_end_matches('/'), h),
            None => m.clone(),
        })
    }
}

/// Запрос на спаривание, ждущий решения пользователя.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PairPrompt {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub transport: Transport,
    /// Код для сверки на обоих экранах (по Wi-Fi); по USB — пусто.
    pub code: Option<String>,
    /// Мы начали спаривание и ждём ответа с той стороны.
    pub outgoing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Status {
    pub me: SelfInfo,
    pub usb: UsbInfo,
    pub peers: Vec<PeerInfo>,
    pub prompts: Vec<PairPrompt>,
}

impl Status {
    pub fn connected(&self) -> impl Iterator<Item = &PeerInfo> {
        self.peers.iter().filter(|p| p.connected)
    }
}

/// Уведомление устройства (история демона).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RemoteNotification {
    pub device: String,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: u8,
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    Status,
    /// Перейти в режим событий: сразу `Event::Status`, дальше по изменениям.
    Subscribe,
    /// Начать спаривание с замеченным устройством.
    Pair { device: String },
    /// Ответ на запрос спаривания.
    PairReply { device: String, accept: bool },
    /// Забыть устройство (ключи ssh тоже удаляются).
    Unpair { device: String },
    /// Разорвать соединение (спаривание остаётся).
    Disconnect { device: String },
    /// Имя этой машины и видимость.
    Configure { name: Option<String>, discoverable: Option<bool> },
    /// Снимок экрана → PNG в `path` (по умолчанию во временный файл);
    /// `max_size` — уменьшить так, чтобы длинная сторона не превышала.
    Screenshot { device: String, output: Option<String>, path: Option<String>, max_size: Option<u32> },
    /// Удалённый ввод (см. `ipc::InputEvent`).
    Input { device: String, output: Option<String>, events: Vec<crate::ipc::InputEvent> },
    /// Запрос композитору устройства (окна, выводы, действия) — как
    /// `synwm msg`; ответ — `Response::Wm`.
    Wm { device: String, wm: crate::ipc::Request },
    /// Выполнить команду. `stream = false`: дождаться и вернуть
    /// `Response::Exec`. `stream = true`: после `Ok` — кадры [`ExecFrame`]
    /// (u32 длина + JSON) в обе стороны; `pty` — терминал с размером.
    Exec {
        device: String,
        argv: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        stream: bool,
        #[serde(default)]
        pty: Option<[u16; 2]>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Туннель TCP до `127.0.0.1:port` устройства (`port` 0 — его sshd):
    /// после `Ok` — сырые байты в обе стороны.
    Tcp { device: String, port: u16 },
    /// Трансляция экрана: после `Ok` демон шлёт кадры [`ScreenPacket`]
    /// (u32 длина + данные), клиент подтверждает каждый `ScreenAck`
    /// (u32 длина + JSON).
    ///
    /// `video` — клиент умеет декодировать видео (`ScreenHeader::video`):
    /// кодек и битрейт выбирает демон по `[link] screen_codec/screen_bitrate`.
    Screen {
        device: String,
        output: Option<String>,
        #[serde(default)]
        cursor: bool,
        #[serde(default)]
        video: bool,
    },
    /// Смонтировать или отмонтировать файлы устройства.
    Mount { device: String, mount: bool },
    /// Уведомления устройств (последние).
    Notifications { device: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    Ok,
    Error { message: String },
    Status { status: Status },
    Screenshot { path: String, width: u32, height: u32, output: String },
    Wm { wm: crate::ipc::Response },
    Exec { code: i32, stdout: String, stderr: String },
    Mount { path: Option<String> },
    Notifications { notifications: Vec<RemoteNotification> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Status { status: Status },
    /// Устройство соединилось (для всплывающего сообщения оболочки).
    Connected { device: PeerInfo },
    Disconnected { device: PeerInfo },
    /// Новый запрос спаривания — оболочке показать диалог.
    PairPrompt { prompt: PairPrompt },
    /// Спаривание завершилось (успешно или нет).
    Paired { device: String, name: String, ok: bool, message: Option<String> },
}

/// Кадр потока команды (`Request::Exec { stream: true }`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum ExecFrame {
    Stdin { data: Vec<u8> },
    /// Конец ввода.
    Eof,
    Resize { cols: u16, rows: u16 },
    Stdout { data: Vec<u8> },
    Stderr { data: Vec<u8> },
    Exit { code: i32 },
    Error { message: String },
}

/// Подтверждение кадра трансляции.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenAck {
    pub seq: u64,
    /// Следующий видеокадр — ключевой (декодер создан заново или ошибся).
    #[serde(default)]
    pub key: bool,
}

/// Заголовок кадра трансляции (JSON в начале пакета, дальше — данные
/// прямоугольников подряд, каждый сжат zstd).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScreenHeader {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub seq: u64,
    /// [x, y, w, h, длина сжатых данных].
    pub rects: Vec<[u32; 5]>,
    pub pointer: Option<[f64; 2]>,
    /// Видеопакет — после данных прямоугольников. Накладывать сначала видео
    /// (только в `video.rects`), потом прямоугольники без потерь.
    #[serde(default)]
    pub video: Option<ScreenVideo>,
    /// Видео не будет (у той стороны нет кодера) — почему.
    #[serde(default)]
    pub video_error: Option<String>,
}

/// Видеопакет кадра трансляции.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScreenVideo {
    /// `h264` или `hevc`.
    pub codec: String,
    pub key: bool,
    pub len: u32,
    /// [x, y, w, h] — где кадр изменился.
    pub rects: Vec<[u32; 4]>,
}

/// Синхронный клиент (оболочка, настройки, CLI).
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> std::io::Result<Self> {
        let stream = UnixStream::connect(socket_path())?;
        let writer = stream.try_clone()?;
        Ok(Self { reader: BufReader::new(stream), writer })
    }

    pub fn request(&mut self, req: &Request) -> std::io::Result<Response> {
        let mut line = serde_json::to_string(req).map_err(std::io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        let mut resp = String::new();
        if self.reader.read_line(&mut resp)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "synlink закрыл соединение"));
        }
        serde_json::from_str(&resp).map_err(std::io::Error::other)
    }

    /// Разобрать соединение после ответа `Ok` на запрос-поток.
    pub fn into_parts(self) -> (BufReader<UnixStream>, UnixStream) {
        (self.reader, self.writer)
    }

    /// Подписаться на события; итератор блокирует на чтении.
    pub fn subscribe(mut self) -> std::io::Result<Events> {
        match self.request(&Request::Subscribe)? {
            Response::Ok => Ok(Events { reader: self.reader }),
            Response::Error { message } => Err(std::io::Error::other(message)),
            other => Err(std::io::Error::other(format!("неожиданный ответ: {other:?}"))),
        }
    }
}

/// Один запрос с ответом.
pub fn request(req: &Request) -> std::io::Result<Response> {
    Client::connect()?.request(req)
}

/// Состояние демона с тайм-аутом (для кода в главном потоке UI: демон
/// занят или завис — не ждать). `None` — демона нет или не ответил.
pub fn status_quick(timeout: std::time::Duration) -> Option<Status> {
    let stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let writer = stream.try_clone().ok()?;
    let mut c = Client { reader: BufReader::new(stream), writer };
    match c.request(&Request::Status).ok()? {
        Response::Status { status } => Some(status),
        _ => None,
    }
}

pub struct Events {
    reader: BufReader<UnixStream>,
}

impl Iterator for Events {
    type Item = std::io::Result<Event>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) if line.trim().is_empty() => continue,
                Ok(_) => return Some(serde_json::from_str(&line).map_err(std::io::Error::other)),
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Записать кадр: u32 (big-endian) длина + данные.
pub fn write_frame(w: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(data)?;
    w.flush()
}

/// Прочитать кадр; `None` — конец потока.
pub fn read_frame(r: &mut impl std::io::Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > 256 << 20 {
        return Err(std::io::Error::other("слишком большой кадр"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}
