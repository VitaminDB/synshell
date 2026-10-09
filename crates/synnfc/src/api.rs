//! Протокол `synnfcd`: unix-сокет [`SOCKET`], по JSON-запросу в строке, ответ — строка JSON.
//! После `Subscribe` соединение получает события ([`Event`]) и может слать запросы режима
//! (`Read`, `Write`, `Emulate`, `Stop`): режим живёт, пока живо соединение, — закрыли
//! программу, и чип выключается (батарея).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::ndef::Record;
use synshell_tr::t;

pub const SOCKET: &str = "/run/synnfc/synnfcd.sock";

/// Прочитанная метка.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// Идентификатор, hex.
    pub uid: String,
    /// «NFC-A», «NFC-B», «NFC-F», «NFC-V».
    pub tech: String,
    /// «T2T», «ISO-DEP», «T3T», «T5T», «MIFARE».
    pub protocol: String,
    /// «NTAG215», «MIFARE Classic 1K», «Type 4»…
    pub kind: String,
    #[serde(default)]
    pub atqa: String,
    #[serde(default)]
    pub sak: String,
    #[serde(default)]
    pub ats: String,
    /// Сообщение NDEF (`Some(vec![])` — размечена, но пуста; `None` — не NDEF).
    pub ndef: Option<Vec<Record>>,
    /// Ёмкость для NDEF, байт.
    #[serde(default)]
    pub capacity: u32,
    #[serde(default)]
    pub writable: bool,
    /// Память Type 2, hex (для сохранения).
    #[serde(default)]
    pub raw: String,
    /// Чтение не удалось целиком (метку убрали, неизвестная структура).
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    /// Опрос меток.
    Read,
    /// Опрос: первую поднесённую метку перезаписать.
    Write,
    /// Телефон — метка Type 4 для считывателей.
    Emulate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Контроллер найден.
    pub present: bool,
    pub device: String,
    /// «NCI 2.0 · NXP · прошивка 01.C0.EF».
    pub controller: String,
    pub mode: Mode,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Status,
    Subscribe,
    Read,
    Write { records: Vec<Record> },
    Emulate { records: Vec<Record> },
    Stop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Status(Status),
    Tag(Tag),
    /// Запись на метку `uid`: удалась или нет.
    Written { uid: String, ok: bool, error: String },
    /// Считыватель прочитал эмулируемую метку.
    EmulationRead,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub status: Option<Status>,
}

/// Один запрос — один ответ.
pub fn request(req: &Request) -> Result<Response> {
    let mut s = UnixStream::connect(SOCKET).with_context(|| t!("служба NFC не запущена ({SOCKET})", SOCKET = SOCKET))?;
    writeln!(s, "{}", serde_json::to_string(req)?)?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line)?;
    let r: Response = serde_json::from_str(&line).context(t!("ответ synnfcd"))?;
    if !r.ok {
        bail!("{}", r.error);
    }
    Ok(r)
}

/// Подписка: соединение, по которому идут события и уходят запросы режима.
pub struct Subscription {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Subscription {
    pub fn open() -> Result<Self> {
        let mut stream = UnixStream::connect(SOCKET).with_context(|| t!("служба NFC не запущена ({SOCKET})", SOCKET = SOCKET))?;
        writeln!(stream, "{}", serde_json::to_string(&Request::Subscribe)?)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Subscription { stream, reader })
    }

    /// Отправитель запросов для другого потока.
    pub fn sender(&self) -> Result<Sender> {
        Ok(Sender(self.stream.try_clone()?))
    }

    /// Следующая строка: событие или ответ на запрос (ответы пропускаются).
    pub fn next_event(&mut self) -> Result<Event> {
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                bail!("{}", t!("synnfcd закрыл соединение"));
            }
            if let Ok(e) = serde_json::from_str::<Event>(&line) {
                return Ok(e);
            }
        }
    }
}

/// Запросы режима по соединению подписки.
pub struct Sender(UnixStream);

impl Sender {
    pub fn send(&mut self, req: &Request) -> Result<()> {
        writeln!(self.0, "{}", serde_json::to_string(req)?)?;
        Ok(())
    }
}
