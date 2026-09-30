//! Протокол между демонами synlink (поверх QUIC).
//!
//! Первый двунаправленный поток соединения — управляющий: обе стороны
//! шлют [`Hello`], затем [`Ctl`] (спаривание, состояние, уведомления,
//! пинг). Каждый вызов — отдельный поток: первое сообщение [`Rpc`], дальше
//! по виду вызова (ответ, кадры, сырые байты). Сообщения — postcard с
//! длиной u32 впереди. Типы IPC композитора (у них внутренние теги serde,
//! которых postcard не умеет) передаются строками JSON.

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use synshell_common::link::DeviceKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const PROTO: u32 = 1;
pub const ALPN: &[u8] = b"synlink/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub user: String,
    pub uid: u32,
    pub home: String,
    /// Открытый ключ ssh этой машины (для authorized_keys той стороны).
    pub ssh_key: Option<String>,
    /// Ключ хоста sshd (для known_hosts той стороны).
    pub ssh_host_key: Option<String>,
    /// Принимает ли эта сторона соединения по USB-гаджету (телефон):
    /// она подтверждает спаривание по кабелю.
    pub usb_gadget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Ctl {
    Hello(Hello),
    /// Эта сторона уже доверяет той (ключ закреплён).
    Trusted,
    /// Эта сторона не знает ту и просит спаривания.
    PairRequest,
    /// Пользователь этой стороны согласился (или по USB согласие не нужно).
    PairAccept,
    PairReject(String),
    /// Состояние устройства (батарея).
    State { battery: Option<(u8, bool)> },
    Notification(Note),
    /// Забыть друг друга (unpair).
    Forget,
    Ping(u64),
    Pong(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Note {
    pub app: String,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: u8,
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Rpc {
    /// Снимок: ответ `Shot`.
    Screenshot { output: Option<String> },
    /// JSON `Vec<ipc::InputEvent>`; ответ `Done`.
    Input { output: Option<String>, events: String },
    /// JSON `ipc::Request`; ответ `Json(ipc::Response)`.
    Wm { request: String },
    /// Команда: дальше кадры `ExecIn` / `ExecOut`.
    Exec { argv: Vec<String>, cwd: Option<String>, pty: Option<(u16, u16)> },
    /// Туннель к `127.0.0.1:port` (0 — sshd): дальше сырые байты.
    Tcp { port: u16 },
    /// Трансляция: дальше `ScreenFrame` туда, `ScreenAck` обратно.
    Screen { output: Option<String>, cursor: bool },
    Fs(FsReq),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Err(String),
    Json(String),
    Shot { output: String, width: u32, height: u32, png: Vec<u8> },
    Fs(FsResp),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecIn {
    Stdin(Vec<u8>),
    Eof,
    Resize(u16, u16),
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecOut {
    Started,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(i32),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenFrame {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub seq: u64,
    /// (x, y, w, h, RGBA сжатые zstd).
    pub rects: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    pub pointer: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenAck {
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Attr {
    pub kind: FileKind,
    pub size: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: (i64, u32),
    pub mtime: (i64, u32),
    pub ctime: (i64, u32),
    pub blocks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsReq {
    Stat(String),
    ReadDir(String),
    Read { path: String, offset: u64, len: u32 },
    Write { path: String, offset: u64, data: Vec<u8> },
    Create { path: String, mode: u32, exclusive: bool },
    Mkdir { path: String, mode: u32 },
    Unlink(String),
    Rmdir(String),
    Rename { from: String, to: String, noreplace: bool },
    SetAttr { path: String, size: Option<u64>, mode: Option<u32>, atime: Option<(i64, u32)>, mtime: Option<(i64, u32)> },
    ReadLink(String),
    Symlink { target: String, path: String },
    StatFs(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsResp {
    Attr(Attr),
    Entries(Vec<(String, Attr)>),
    Data(Vec<u8>),
    Written(u32),
    Ok,
    Target(String),
    StatFs { blocks: u64, bfree: u64, bavail: u64, files: u64, ffree: u64, bsize: u32, namelen: u32 },
    /// errno.
    Err(i32),
}

/// Записать сообщение: u32 длина + postcard.
pub async fn send<T: Serialize>(w: &mut (impl AsyncWriteExt + Unpin), msg: &T) -> Result<()> {
    let data = postcard::to_stdvec(msg)?;
    w.write_all(&(data.len() as u32).to_be_bytes()).await?;
    w.write_all(&data).await?;
    Ok(())
}

/// Прочитать сообщение; `None` — поток закрыт.
pub async fn recv<T: DeserializeOwned>(r: &mut (impl AsyncReadExt + Unpin)) -> Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > 128 << 20 {
        bail!("сообщение {n} байт — слишком большое");
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await.context("обрыв сообщения")?;
    Ok(Some(postcard::from_bytes(&buf)?))
}

/// Код сверки при спаривании по Wi-Fi: 6 цифр из хэша обоих ключей
/// (в порядке возрастания, чтобы у обеих сторон он совпал).
pub fn pair_code(fp_a: &str, fp_b: &str) -> String {
    use sha2::{Digest, Sha256};
    let (x, y) = if fp_a < fp_b { (fp_a, fp_b) } else { (fp_b, fp_a) };
    let h = Sha256::digest(format!("synlink-pair:{x}:{y}").as_bytes());
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}
