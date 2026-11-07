//! Гостевой агент syn-vm — общий протокол хоста и гостя.
//!
//! Канал — порт virtio-serial `org.syn.vm.agent.0` (в госте: `\\.\Global\org.syn.vm.agent.0`
//! в Windows, `/dev/virtio-ports/org.syn.vm.agent.0` в Linux; у QEMU — сокет chardev). Тот же
//! протокол — между посредником на хосте (`syn-vm --agent-broker`) и его клиентами.
//!
//! Кадр: `b"SVA1"`, `u32 len` (LE) остатка, `u32 json_len`, JSON сообщения [`Msg`], сырые
//! данные (вывод команд, куски файлов, строки журналов). Метка кадра позволяет найти начало
//! следующего кадра после обрыва посреди передачи.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Имя порта virtio-serial.
pub const PORT: &str = "org.syn.vm.agent.0";
/// Адрес хоста для агента по сети (guestfwd QEMU в сети NAT): без драйверов virtio.
pub const NET_ADDR: &str = "10.0.2.100:7777";
pub const MAGIC: [u8; 4] = *b"SVA1";
/// Версия протокола.
pub const VERSION: u32 = 1;
/// Кусок данных файла или вывода.
pub const CHUNK: usize = 64 * 1024;
/// Предел кадра (защита от мусора в канале).
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Элемент каталога (рекурсивный список для --pull каталога).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    /// Путь относительно запрошенного, разделитель `/`.
    pub path: String,
    pub dir: bool,
    pub size: u64,
}

/// Сообщение. `id` — номер операции, выбранный стороной, которая её начала (хост).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum Msg {
    // ── хост → гость ──
    /// Поздороваться: гость отвечает `Hello` (после переподключения хоста).
    Hi,
    /// Запустить программу. `user` — в интерактивном сеансе вошедшего пользователя.
    /// `stdin` — данные придут сообщениями `Stdin` (пустые — конец ввода).
    Exec { id: u64, argv: Vec<String>, user: bool, cwd: Option<String>, env: Vec<String>, stdin: bool },
    /// Кусок ввода команды (данные кадра); пустой — закрыть ввод.
    Stdin { id: u64 },
    /// Завершить команду.
    Kill { id: u64 },
    /// Записать файл: дальше `Data` с кусками и `PutEnd`; ответ — `Done` (размер и SHA-256).
    Put { id: u64, path: String, size: u64 },
    PutEnd { id: u64 },
    /// Создать каталог (с родителями) — ответ `Done`.
    Mkdir { id: u64, path: String },
    /// Прочитать файл: ответ `Stat`, `Data`…, `End` с SHA-256.
    Get { id: u64, path: String },
    /// Рекурсивный список каталога (или один файл): ответ `Entries`.
    List { id: u64, path: String },
    /// Проверка связи: ответ `Pong`.
    Ping { id: u64 },
    /// Присылать журналы гостя (`Log`).
    Logs { on: bool },
    /// Точное время хоста (UTC, мс с 1970): агент ставит его системе — часы гостя не зависят
    /// от того, как он толкует RTC (местное или UTC) и от пояса хоста.
    SetTime { unix_ms: u64 },
    /// Буфер обмена (в обе стороны): текст UTF-8 — данные кадра; `mime` — `text/plain`.
    Clip { mime: String },
    /// Разрешение экрана гостя под окно (гостю без своей видеокарты: ChangeDisplaySettings).
    Mode { width: u32, height: u32 },
    /// Общая папка хоста: Windows — `net use БУКВА: \\10.0.2.4\qemu` в сеансе пользователя
    /// (`kind` = "smb"), Linux — `mount -t virtiofs|9p hostshare ПУТЬ` (`kind` = "virtiofs"|"9p").
    /// Пустой `kind` — отключить.
    /// `source` — имя ресурса SMB (`qemu`, …) или метка 9p/virtiofs (`hostshare`, …); пусто —
    /// первая папка (`qemu` / `hostshare`).
    Share {
        kind: String,
        target: String,
        #[serde(default)]
        source: String,
    },
    /// Посредник → клиент и клиент → посредник: состояние агента.
    Status { id: u64 },
    /// Обновить себя: новый exe уже записан по `path`; агент заменяет себя и перезапускается.
    Update { id: u64, path: String },

    // ── гость → хост ──
    /// Агент на связи. `boot` — метка загрузки системы, `instance` — запуска агента:
    /// новая метка — незавершённые операции прерваны перезагрузкой или перезапуском.
    Hello { version: u32, agent: String, os: String, host: String, boot: String, instance: u64, user: Option<String> },
    /// Вошёл (или вышел) пользователь интерактивного сеанса.
    Session { user: Option<String> },
    /// Адреса гостя (без петли и link-local): после `Hello` и при смене.
    Net { ips: Vec<String> },
    Started { id: u64, pid: u32 },
    /// Вывод команды (данные кадра): `fd` 1 — stdout, 2 — stderr.
    Out { id: u64, fd: u8 },
    Exit { id: u64, code: i32 },
    Error { id: u64, msg: String },
    Done { id: u64, size: u64, sha256: String },
    Stat { id: u64, size: u64 },
    /// Кусок файла (данные кадра) — в обе стороны.
    Data { id: u64 },
    End { id: u64, sha256: String },
    Entries { id: u64, entries: Vec<Entry> },
    Pong { id: u64 },
    /// Строка журнала гостя (текст — данные кадра): `src` — debug, event:System…, journal.
    Log { src: String },

    // ── посредник → клиент ──
    /// Ответ на `Status`: агент на связи (последний `Hello`) и пользователь сеанса.
    State {
        id: u64,
        hello: Option<Box<Msg>>,
        user: Option<String>,
        #[serde(default)]
        ips: Vec<String>,
    },
}

impl Msg {
    /// Номер операции (у сообщений без номера — None).
    pub fn id(&self) -> Option<u64> {
        use Msg::*;
        match self {
            Exec { id, .. } | Stdin { id } | Kill { id } | Put { id, .. } | PutEnd { id } | Mkdir { id, .. } | Get { id, .. } | List { id, .. } | Ping { id } | Status { id } | Update { id, .. } => Some(*id),
            Started { id, .. } | Out { id, .. } | Exit { id, .. } | Error { id, .. } | Done { id, .. } | Stat { id, .. } | Data { id } | End { id, .. } | Entries { id, .. } | Pong { id } | State { id, .. } => Some(*id),
            Hi | Logs { .. } | SetTime { .. } | Clip { .. } | Mode { .. } | Share { .. } | Hello { .. } | Session { .. } | Net { .. } | Log { .. } => None,
        }
    }

    /// То же сообщение с другим номером (посредник переводит номера клиентов в свои).
    pub fn with_id(mut self, new: u64) -> Msg {
        use Msg::*;
        match &mut self {
            Exec { id, .. } | Stdin { id } | Kill { id } | Put { id, .. } | PutEnd { id } | Mkdir { id, .. } | Get { id, .. } | List { id, .. } | Ping { id } | Status { id } | Update { id, .. } => *id = new,
            Started { id, .. } | Out { id, .. } | Exit { id, .. } | Error { id, .. } | Done { id, .. } | Stat { id, .. } | Data { id } | End { id, .. } | Entries { id, .. } | Pong { id } | State { id, .. } => *id = new,
            _ => {}
        }
        self
    }

    /// Последнее сообщение операции (после него номер свободен).
    pub fn is_final(&self) -> bool {
        matches!(self, Msg::Exit { .. } | Msg::Error { .. } | Msg::Done { .. } | Msg::End { .. } | Msg::Entries { .. } | Msg::Pong { .. } | Msg::State { .. })
    }
}

/// Кадр: сообщение и данные.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub msg: Msg,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(msg: Msg) -> Frame {
        Frame { msg, data: Vec::new() }
    }

    pub fn with_data(msg: Msg, data: Vec<u8>) -> Frame {
        Frame { msg, data }
    }

    pub fn encode(&self) -> Vec<u8> {
        let json = serde_json::to_vec(&self.msg).unwrap_or_default();
        let len = 4 + json.len() + self.data.len();
        let mut out = Vec::with_capacity(8 + len);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&self.data);
        out
    }
}

/// Записать кадр целиком.
pub fn write_frame(w: &mut impl Write, f: &Frame) -> io::Result<()> {
    w.write_all(&f.encode())?;
    w.flush()
}

/// Прочитать кадр. Мусор до метки пропускается; негодный JSON — кадр пропускается
/// (сообщение новой версии), а конец потока — `UnexpectedEof`.
pub fn read_frame(r: &mut impl Read) -> io::Result<Frame> {
    loop {
        // Найти метку.
        let mut win = [0u8; 4];
        r.read_exact(&mut win)?;
        while win != MAGIC {
            win.copy_within(1.., 0);
            r.read_exact(&mut win[3..])?;
        }
        let mut n = [0u8; 4];
        r.read_exact(&mut n)?;
        let len = u32::from_le_bytes(n) as usize;
        if !(4..=MAX_FRAME).contains(&len) {
            continue;
        }
        let mut body = vec![0u8; len];
        r.read_exact(&mut body)?;
        let jl = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
        if jl > len - 4 {
            continue;
        }
        let Ok(msg) = serde_json::from_slice::<Msg>(&body[4..4 + jl]) else { continue };
        let data = body[4 + jl..].to_vec();
        return Ok(Frame { msg, data });
    }
}

/// SHA-256 в шестнадцатеричном виде.
pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

pub use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_resync() {
        let a = Frame::with_data(Msg::Out { id: 7, fd: 1 }, b"hello".to_vec());
        let b = Frame::new(Msg::Exit { id: 7, code: 3 });
        let mut buf = b"garbage SV".to_vec();
        buf.extend(a.encode());
        buf.extend(b"xx");
        buf.extend(b.encode());
        let mut r = &buf[..];
        assert_eq!(read_frame(&mut r).unwrap(), a);
        assert_eq!(read_frame(&mut r).unwrap(), b);
        assert!(read_frame(&mut r).is_err());
    }

    #[test]
    fn ids() {
        let m = Msg::Exec { id: 1, argv: vec!["a".into()], user: false, cwd: None, env: vec![], stdin: false };
        assert_eq!(m.clone().with_id(9).id(), Some(9));
        assert!(Msg::Exit { id: 1, code: 0 }.is_final());
        assert!(!Msg::Out { id: 1, fd: 1 }.is_final());
    }
}
