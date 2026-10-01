//! QMI поверх QRTR: сообщение — заголовок (тип, транзакция, номер, длина) и TLV. Клиент службы держит свой
//! сокет QRTR и поток чтения: ответы находят ждущий запрос по номеру транзакции, индикации уходят в канал.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::qrtr::{self, Addr, Socket};

/// Номера служб QMI.
pub mod svc {
    pub const WDS: u32 = 1;
    pub const DMS: u32 = 2;
    pub const NAS: u32 = 3;
    pub const WMS: u32 = 5;
    pub const VOICE: u32 = 9;
    pub const UIM: u32 = 11;
    pub const WDA: u32 = 26;
    pub const DPM: u32 = 47;
}

const TYPE_REQUEST: u8 = 0;
const TYPE_RESPONSE: u8 = 2;
const TYPE_INDICATION: u8 = 4;

/// Сообщение QMI: номер и TLV (тип → значение; повторяющиеся типы в QMI не встречаются).
#[derive(Clone, Debug, Default)]
pub struct Message {
    pub id: u16,
    pub tlvs: Vec<(u8, Vec<u8>)>,
}

impl Message {
    pub fn new(id: u16) -> Self {
        Self { id, tlvs: Vec::new() }
    }

    pub fn tlv(mut self, t: u8, v: impl Into<Vec<u8>>) -> Self {
        self.tlvs.push((t, v.into()));
        self
    }
    pub fn u8(self, t: u8, v: u8) -> Self {
        self.tlv(t, vec![v])
    }
    pub fn u16(self, t: u8, v: u16) -> Self {
        self.tlv(t, v.to_le_bytes().to_vec())
    }
    pub fn u32(self, t: u8, v: u32) -> Self {
        self.tlv(t, v.to_le_bytes().to_vec())
    }

    pub fn get(&self, t: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(k, _)| *k == t).map(|(_, v)| v.as_slice())
    }
    pub fn reader(&self, t: u8) -> Option<Reader<'_>> {
        self.get(t).map(Reader::new)
    }

    /// Результат ответа (TLV 0x02): `Ok` или код ошибки QMI.
    pub fn result(&self) -> Result<()> {
        let r = self.get(0x02).ok_or_else(|| anyhow!("нет TLV результата"))?;
        if r.len() < 4 {
            bail!("короткий TLV результата");
        }
        let status = u16::from_le_bytes([r[0], r[1]]);
        let code = u16::from_le_bytes([r[2], r[3]]);
        if status != 0 {
            return Err(QmiError(code).into());
        }
        Ok(())
    }

    fn encode(&self, kind: u8, txn: u16) -> Vec<u8> {
        let body: usize = self.tlvs.iter().map(|(_, v)| 3 + v.len()).sum();
        let mut out = Vec::with_capacity(7 + body);
        out.push(kind);
        out.extend_from_slice(&txn.to_le_bytes());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&(body as u16).to_le_bytes());
        for (t, v) in &self.tlvs {
            out.push(*t);
            out.extend_from_slice(&(v.len() as u16).to_le_bytes());
            out.extend_from_slice(v);
        }
        out
    }

    /// Разобрать датаграмму: (тип, транзакция, сообщение).
    fn decode(buf: &[u8]) -> Option<(u8, u16, Message)> {
        if buf.len() < 7 {
            return None;
        }
        let kind = buf[0];
        let txn = u16::from_le_bytes([buf[1], buf[2]]);
        let id = u16::from_le_bytes([buf[3], buf[4]]);
        let len = u16::from_le_bytes([buf[5], buf[6]]) as usize;
        let body = buf.get(7..7 + len)?;
        let mut tlvs = Vec::new();
        let mut i = 0;
        while i + 3 <= body.len() {
            let t = body[i];
            let l = u16::from_le_bytes([body[i + 1], body[i + 2]]) as usize;
            let v = body.get(i + 3..i + 3 + l)?;
            tlvs.push((t, v.to_vec()));
            i += 3 + l;
        }
        Some((kind, txn, Message { id, tlvs }))
    }
}

/// Ошибка протокола QMI (код из TLV результата).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QmiError(pub u16);

impl QmiError {
    pub const NO_EFFECT: u16 = 0x1A;
    pub const INFO_UNAVAILABLE: u16 = 0x4A;
}

impl std::fmt::Display for QmiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.0 {
            0x01 => "MalformedMessage",
            0x03 => "Internal",
            0x0E => "CallFailed",
            0x10 => "NotProvisioned",
            0x1A => "NoEffect",
            0x25 => "UimUninitialized",
            0x29 => "DeviceNotReady",
            0x2E => "InvalidArgs",
            0x30 => "InvalidId",
            0x34 => "DeviceUnsupported",
            0x4A => "InformationUnavailable",
            0x52 => "InvalidOperation",
            0x5E => "NotSupported",
            _ => "",
        };
        write!(f, "ошибка QMI 0x{:02x} {name}", self.0)
    }
}

impl std::error::Error for QmiError {}

/// Чтение полей TLV по порядку (little-endian, строки и массивы с префиксом длины).
pub struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.b[self.i.min(self.b.len())..]
    }
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(v)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    pub fn i8(&mut self) -> Option<i8> {
        self.u8().map(|v| v as i8)
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    /// Строка с префиксом длины `u8`.
    pub fn str8(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        self.bytes(n).map(|b| String::from_utf8_lossy(b).into_owned())
    }
    /// Массив байтов с префиксом длины `u8`.
    pub fn arr8(&mut self) -> Option<&'a [u8]> {
        let n = self.u8()? as usize;
        self.bytes(n)
    }
    /// Массив байтов с префиксом длины `u16`.
    pub fn arr16(&mut self) -> Option<&'a [u8]> {
        let n = self.u16()? as usize;
        self.bytes(n)
    }
}

type Pending = Arc<Mutex<HashMap<u16, Sender<Message>>>>;

/// Клиент одной службы QMI.
pub struct Client {
    pub service: u32,
    sock: Arc<Socket>,
    addr: Addr,
    txn: AtomicU16,
    pending: Pending,
    /// Клиент удалён — поток чтения выходит, сокет закрывается (модем закрывает сеансы клиента).
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// Индикация службы: (номер службы, сообщение). `None` в сообщении не бывает — уход службы
/// приходит как `Message { id: 0xffff }`.
pub type Indication = (u32, Message);
/// Признак ухода службы (модем остановлен или упал) в канале индикаций.
pub const SERVICE_GONE: u16 = 0xffff;

impl Client {
    /// Подключиться к службе (ждать её объявления не дольше `timeout`); индикации — в `ind`.
    pub fn connect(service: u32, timeout: Duration, ind: Sender<Indication>) -> Result<Arc<Self>> {
        let srv = qrtr::find(service, timeout)?.ok_or_else(|| anyhow!("служба QMI {service} не найдена"))?;
        let sock = Arc::new(Socket::open()?);
        // Подписка на службу имён этим же сокетом: уход службы (DEL_SERVER) увидит поток чтения
        sock.lookup(service)?;
        let pending: Pending = Arc::default();
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = Arc::new(Self {
            service,
            sock: sock.clone(),
            addr: srv.addr,
            txn: AtomicU16::new(1),
            pending: pending.clone(),
            closed: closed.clone(),
        });
        let addr = srv.addr;
        std::thread::Builder::new().name(format!("qmi-{service}")).spawn(move || {
            let mut buf = vec![0u8; 65536];
            loop {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                let (n, from) = match sock.recv(&mut buf, Some(Duration::from_secs(1))) {
                    Ok(Some(r)) => r,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                if from.port == qrtr::PORT_CTRL {
                    if let Some(qrtr::Ctrl::DelServer(s)) = qrtr::parse_ctrl(&buf[..n]) {
                        if s.addr == addr {
                            let _ = ind.send((service, Message::new(SERVICE_GONE)));
                            break;
                        }
                    }
                    continue;
                }
                if from != addr {
                    continue;
                }
                let Some((kind, txn, msg)) = Message::decode(&buf[..n]) else { continue };
                match kind {
                    TYPE_RESPONSE => {
                        if let Some(tx) = pending.lock().unwrap().remove(&txn) {
                            let _ = tx.send(msg);
                        }
                    }
                    TYPE_INDICATION => {
                        if ind.send((service, msg)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        })?;
        Ok(c)
    }

    /// Запрос и ответ (с проверкой результата).
    pub fn call(&self, msg: Message, timeout: Duration) -> Result<Message> {
        let r = self.call_raw(msg, timeout)?;
        r.result()?;
        Ok(r)
    }

    /// Запрос и ответ без проверки результата (часть данных бывает и в ответе с ошибкой).
    pub fn call_raw(&self, msg: Message, timeout: Duration) -> Result<Message> {
        let mut txn = self.txn.fetch_add(1, Ordering::Relaxed);
        if txn == 0 {
            txn = self.txn.fetch_add(1, Ordering::Relaxed);
        }
        let (tx, rx): (Sender<Message>, Receiver<Message>) = mpsc::channel();
        self.pending.lock().unwrap().insert(txn, tx);
        let id = msg.id;
        if let Err(e) = self.sock.send_to(self.addr, &msg.encode(TYPE_REQUEST, txn)) {
            self.pending.lock().unwrap().remove(&txn);
            return Err(e.into());
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => Ok(r),
            Err(_) => {
                self.pending.lock().unwrap().remove(&txn);
                bail!("служба {} не ответила на сообщение 0x{id:04x}", self.service)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let m = Message::new(0x20).u8(0x01, 7).tlv(0x10, b"abc".to_vec());
        let raw = m.encode(TYPE_REQUEST, 0x1234);
        assert_eq!(&raw[..7], &[0, 0x34, 0x12, 0x20, 0, 10, 0]);
        let (k, t, d) = Message::decode(&raw).unwrap();
        assert_eq!((k, t, d.id), (TYPE_REQUEST, 0x1234, 0x20));
        assert_eq!(d.get(0x01), Some(&[7u8][..]));
        assert_eq!(d.get(0x10), Some(&b"abc"[..]));
    }

    #[test]
    fn result_tlv() {
        let ok = Message::new(1).tlv(0x02, vec![0, 0, 0, 0]);
        assert!(ok.result().is_ok());
        let err = Message::new(1).tlv(0x02, vec![1, 0, 0x10, 0]);
        let e = err.result().unwrap_err();
        assert_eq!(e.downcast_ref::<QmiError>(), Some(&QmiError(0x10)));
    }
}
