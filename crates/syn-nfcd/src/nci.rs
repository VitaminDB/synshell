//! NCI (NFC Controller Interface, NCI 1.x/2.0) поверх символьного устройства драйвера
//! (`/dev/nq-nci` — NXP nfc_i2c, synmobile docs/25): пакет пишется `write()` целиком,
//! читается заголовок (3 байта) и затем полезная нагрузка; `read()` ждёт прерывания
//! чипа, поэтому читает отдельный поток без таймеров (прерванное сигналом чтение
//! оставляет драйвер в плохом состоянии). Питание — ioctl `NFC_SET_PWR`.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use synshell_tr::t;

/// Пакет NCI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Rsp { gid: u8, oid: u8, payload: Vec<u8> },
    Ntf { gid: u8, oid: u8, payload: Vec<u8> },
    Data { conn: u8, payload: Vec<u8> },
}

/// Сегмент: заголовок разбирается, PBF — ещё будут сегменты.
fn split(raw: &[u8]) -> Option<(u8, bool, u8, u8, &[u8])> {
    if raw.len() < 3 {
        return None;
    }
    let mt = raw[0] >> 5;
    let pbf = raw[0] & 0x10 != 0;
    let len = raw[2] as usize;
    let payload = raw.get(3..3 + len)?;
    Some((mt, pbf, raw[0] & 0x0F, raw[1] & 0x3F, payload))
}

/// Сборка пакетов из сегментов.
#[derive(Default)]
struct Reassembly {
    buf: Option<(u8, u8, u8, Vec<u8>)>,
}

impl Reassembly {
    fn push(&mut self, raw: &[u8]) -> Option<Packet> {
        let (mt, pbf, a, b, payload) = split(raw)?;
        let (mt, a, b, mut acc) = match self.buf.take() {
            Some((m, x, y, acc)) if m == mt => (m, x, y, acc),
            _ => (mt, a, b, Vec::new()),
        };
        acc.extend_from_slice(payload);
        if pbf {
            self.buf = Some((mt, a, b, acc));
            return None;
        }
        Some(match mt {
            0 => Packet::Data { conn: a, payload: acc },
            2 => Packet::Rsp { gid: a, oid: b, payload: acc },
            3 => Packet::Ntf { gid: a, oid: b, payload: acc },
            _ => return None,
        })
    }
}

/// Сторона устройства: запись пакета и питание. Чтение — каналом (отдельный поток).
pub trait Link: Send {
    fn write(&mut self, pkt: &[u8]) -> Result<()>;
    fn power(&mut self, on: bool) -> Result<()>;
}

/// Символьное устройство драйвера.
pub struct DevLink {
    file: File,
}

/// `_IOW(0xE9, 0x01, u32)` — ioctl питания nfc_i2c (NXP nfcandroid_platform_drivers, QTI).
const NFC_SET_PWR: libc::c_ulong = 0x4004_E901;

impl Link for DevLink {
    fn write(&mut self, pkt: &[u8]) -> Result<()> {
        self.file.write_all(pkt).context(t!("запись в NFC"))
    }

    fn power(&mut self, on: bool) -> Result<()> {
        // SAFETY: ioctl с целым аргументом на открытом дескрипторе.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), NFC_SET_PWR as _, on as libc::c_ulong) };
        if r < 0 {
            bail!("NFC_SET_PWR: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Открыть устройство: запись — в [`DevLink`], чтение — поток, сырые сегменты в канал.
pub fn open_device(path: &str) -> Result<(DevLink, Receiver<Vec<u8>>)> {
    let file = OpenOptions::new().read(true).write(true).open(path).with_context(|| path.to_string())?;
    let mut reader = file.try_clone()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("nci-read".into())
        .spawn(move || loop {
            let mut h = [0u8; 3];
            if reader.read_exact(&mut h).is_err() {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            let mut pkt = h.to_vec();
            if h[2] > 0 {
                let mut p = vec![0u8; h[2] as usize];
                if reader.read_exact(&mut p).is_err() {
                    continue;
                }
                pkt.extend_from_slice(&p);
            }
            if tx.send(pkt).is_err() {
                return;
            }
        })?;
    Ok((DevLink { file }, rx))
}

/// Сведения о контроллере после запуска.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    /// 0x10 — NCI 1.0, 0x20 — NCI 2.0.
    pub version: u8,
    pub manufacturer: u8,
    /// Прошивка (NXP: из расширения 2F 02).
    pub firmware: String,
}

/// Соединение NCI: команды, уведомления и данные.
pub struct Nci {
    link: Box<dyn Link>,
    rx: Receiver<Vec<u8>>,
    reasm: Reassembly,
    /// Пришедшее, пока ждали ответа: уведомления и данные.
    pending: VecDeque<Packet>,
    /// Кредиты статического соединения RF (conn 0) и наибольший размер сегмента данных.
    pub credits: u8,
    pub max_data: usize,
    pub info: Info,
}

impl Nci {
    pub fn new(link: Box<dyn Link>, rx: Receiver<Vec<u8>>) -> Self {
        Nci { link, rx, reasm: Reassembly::default(), pending: VecDeque::new(), credits: 1, max_data: 255, info: Info::default() }
    }

    fn send_raw(&mut self, mt: u8, a: u8, b: u8, payload: &[u8], seg: usize) -> Result<()> {
        let chunks: Vec<&[u8]> = if payload.is_empty() { vec![&[]] } else { payload.chunks(seg.clamp(1, 255)).collect() };
        for (i, c) in chunks.iter().enumerate() {
            let more = i + 1 < chunks.len();
            let mut pkt = vec![(mt << 5) | if more { 0x10 } else { 0 } | (a & 0x0F), b & 0x3F, c.len() as u8];
            pkt.extend_from_slice(c);
            tracing::trace!("NCI >> {}", crate::ndef::hex(&pkt));
            self.link.write(&pkt)?;
        }
        Ok(())
    }

    /// Следующий пакет из канала (собранный из сегментов), не дольше `timeout`.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Packet>> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let raw = match self.rx.recv_timeout(left) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => bail!("{}", t!("NFC: поток чтения закрыт")),
            };
            tracing::trace!("NCI << {}", crate::ndef::hex(&raw));
            if let Some(p) = self.reasm.push(&raw) {
                // кредиты соединения — сразу (CORE_CONN_CREDITS_NTF)
                if let Packet::Ntf { gid: 0, oid: 6, payload } = &p {
                    for e in payload.get(1..).unwrap_or(&[]).chunks(2) {
                        if e.len() == 2 && e[0] == 0 {
                            self.credits = self.credits.saturating_add(e[1]);
                        }
                    }
                    continue;
                }
                return Ok(Some(p));
            }
        }
    }

    /// Команда и её ответ (`GID`/`OID` совпадают); уведомления и данные по пути — в очередь.
    pub fn cmd(&mut self, gid: u8, oid: u8, payload: &[u8]) -> Result<Vec<u8>> {
        self.send_raw(1, gid, oid, payload, 255)?;
        let end = Instant::now() + Duration::from_millis(2000);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.recv(left)? {
                Some(Packet::Rsp { gid: g, oid: o, payload }) if g == gid && o == oid => return Ok(payload),
                Some(p) => self.pending.push_back(p),
                None => bail!("{}", t!("NFC: нет ответа на команду {gid} {oid}", gid = format!("{:02X}", gid), oid = format!("{:02X}", oid))),
            }
        }
    }

    /// Команда, ответ которой должен начинаться со статуса 0 (STATUS_OK).
    pub fn cmd_ok(&mut self, gid: u8, oid: u8, payload: &[u8]) -> Result<Vec<u8>> {
        let r = self.cmd(gid, oid, payload)?;
        match r.first() {
            Some(0) => Ok(r),
            Some(s) => Err(anyhow!("{}", t!("NFC: команда {gid} {oid} — статус {s}", gid = format!("{:02X}", gid), oid = format!("{:02X}", oid), s = format!("{:02X}", s)))),
            None => Err(anyhow!("{}", t!("NFC: пустой ответ на {gid} {oid}", gid = format!("{:02X}", gid), oid = format!("{:02X}", oid)))),
        }
    }

    /// Ждать уведомление или данные (сначала — накопившиеся).
    pub fn event(&mut self, timeout: Duration) -> Result<Option<Packet>> {
        if let Some(p) = self.pending.pop_front() {
            return Ok(Some(p));
        }
        self.recv(timeout)
    }

    /// Отправить данные по соединению RF (conn 0) и дождаться ответа; уведомление о
    /// деактивации по пути (метку убрали) — ошибка, само уведомление остаётся в очереди.
    pub fn transceive(&mut self, payload: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        self.send_data(payload)?;
        self.wait_data(timeout)
    }

    /// Отправить данные (с ожиданием кредита).
    pub fn send_data(&mut self, payload: &[u8]) -> Result<()> {
        let end = Instant::now() + Duration::from_millis(1000);
        while self.credits == 0 {
            let left = end.saturating_duration_since(Instant::now());
            match self.recv(left.min(Duration::from_millis(100)))? {
                Some(p) => self.pending.push_back(p),
                None if left.is_zero() => {
                    // кредит не вернули — пробуем всё равно (у некоторых контроллеров NTF не приходит)
                    tracing::debug!("NFC: нет кредита соединения, отправка без него");
                    break;
                }
                None => {}
            }
        }
        self.credits = self.credits.saturating_sub(1);
        let seg = self.max_data;
        self.send_raw(0, 0, 0, payload, seg)
    }

    /// Ждать данные по соединению RF.
    pub fn wait_data(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let end = Instant::now() + timeout;
        // данные, пришедшие раньше, — в очереди
        if let Some(i) = self.pending.iter().position(|p| matches!(p, Packet::Data { conn: 0, .. })) {
            if let Some(Packet::Data { payload, .. }) = self.pending.remove(i) {
                return Ok(payload);
            }
        }
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.recv(left)? {
                Some(Packet::Data { conn: 0, payload }) => return Ok(payload),
                // RF_DEACTIVATE_NTF или ошибка интерфейса — метки больше нет
                Some(p @ Packet::Ntf { gid: 1, oid: 6, .. }) => {
                    self.pending.push_back(p);
                    bail!("{}", t!("метка убрана"));
                }
                Some(Packet::Ntf { gid: 0, oid: 7, payload }) | Some(Packet::Ntf { gid: 1, oid: 8, payload }) if payload.first().is_some_and(|s| *s != 0) => {
                    bail!("{}", t!("ошибка RF {v}", v = format!("{:02X}", payload[0])))
                }
                Some(p) => self.pending.push_back(p),
                None => bail!("{}", t!("метка не ответила")),
            }
        }
    }

    /// Выключить и включить чип, CORE_RESET (с сохранением настроек, записанных Android) и
    /// CORE_INIT; NXP — расширение `2F 02` (версия прошивки).
    pub fn start(&mut self) -> Result<Info> {
        let _ = self.link.power(false);
        std::thread::sleep(Duration::from_millis(20));
        self.link.power(true)?;
        std::thread::sleep(Duration::from_millis(20));
        // остатки прошлой сессии
        while let Ok(Some(_)) = self.recv(Duration::from_millis(30)) {}
        self.pending.clear();
        self.reasm = Reassembly::default();
        let rsp = self.cmd_ok(0, 0, &[0x01])?;
        let mut info = Info::default();
        if rsp.len() >= 3 {
            // NCI 1.x: версия в ответе
            info.version = rsp[1];
        } else {
            // NCI 2.0: CORE_RESET_NTF: причина, тип конфигурации, версия, производитель
            let end = Instant::now() + Duration::from_millis(1000);
            while Instant::now() < end {
                match self.event(Duration::from_millis(200))? {
                    Some(Packet::Ntf { gid: 0, oid: 0, payload }) if payload.len() >= 4 => {
                        info.version = payload[2];
                        info.manufacturer = payload[3];
                        break;
                    }
                    Some(_) | None => {}
                }
            }
        }
        if info.version >= 0x20 {
            self.cmd_ok(0, 1, &[0x00, 0x00])?;
        } else {
            self.cmd_ok(0, 1, &[])?;
        }
        if info.manufacturer == 0x04 || info.version >= 0x20 {
            if let Ok(r) = self.cmd(0x0F, 0x02, &[]) {
                if r.first() == Some(&0) && r.len() >= 3 {
                    let n = r.len();
                    info.firmware = format!("{:02X}.{:02X}.{:02X}", r[n - 3], r[n - 2], r[n - 1]);
                }
            }
        }
        self.pending.clear();
        self.info = info.clone();
        Ok(info)
    }

    /// Выключить чип.
    pub fn stop(&mut self) {
        let _ = self.link.power(false);
    }
}

#[cfg(test)]
pub mod fake {
    //! Имитация контроллера для тестов: ответы на команды и обмен с меткой задаёт замыкание.
    use super::*;
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    pub type Responder = Box<dyn FnMut(&[u8]) -> Vec<Vec<u8>> + Send>;

    pub struct FakeLink {
        pub tx: Sender<Vec<u8>>,
        pub respond: Arc<Mutex<Responder>>,
        pub sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl Link for FakeLink {
        fn write(&mut self, pkt: &[u8]) -> Result<()> {
            self.sent.lock().unwrap().push(pkt.to_vec());
            for r in (self.respond.lock().unwrap())(pkt) {
                let _ = self.tx.send(r);
            }
            Ok(())
        }
        fn power(&mut self, _on: bool) -> Result<()> {
            Ok(())
        }
    }

    /// Соединение с имитацией и журнал отправленного.
    pub fn nci(respond: Responder) -> (Nci, Arc<Mutex<Vec<Vec<u8>>>>, Sender<Vec<u8>>) {
        let (tx, rx) = mpsc::channel();
        let sent = Arc::new(Mutex::new(vec![]));
        let link = FakeLink { tx: tx.clone(), respond: Arc::new(Mutex::new(respond)), sent: sent.clone() };
        (Nci::new(Box::new(link), rx), sent, tx)
    }

    /// Ответы контроллера NCI 2.0 (NXP) на запуск.
    pub fn startup(pkt: &[u8]) -> Option<Vec<Vec<u8>>> {
        Some(match pkt {
            [0x20, 0x00, 0x01, 0x01] => vec![vec![0x40, 0x00, 0x01, 0x00], vec![0x60, 0x00, 0x0A, 0x02, 0x01, 0x20, 0x04, 0x05, 0x01, 0xA4, 0x01, 0x10, 0x68]],
            [0x20, 0x01, 0x02, 0x00, 0x00] => vec![vec![0x40, 0x01, 0x01, 0x00]],
            [0x2F, 0x02, 0x00] => vec![vec![0x4F, 0x02, 0x05, 0x00, 0x00, 0x01, 0xC0, 0xEF]],
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_nci2_nxp() {
        let (mut n, sent, _) = fake::nci(Box::new(|p| fake::startup(p).unwrap_or_default()));
        let info = n.start().unwrap();
        assert_eq!(info, Info { version: 0x20, manufacturer: 0x04, firmware: "01.C0.EF".into() });
        assert_eq!(sent.lock().unwrap().len(), 3);
    }

    #[test]
    fn segmented_data_and_credits() {
        // ответ на данные двумя сегментами, кредит возвращается уведомлением
        let (mut n, _, _) = fake::nci(Box::new(|p| {
            if p[0] == 0x00 {
                vec![vec![0x10, 0x00, 0x02, 0xAA, 0xBB], vec![0x00, 0x00, 0x01, 0xCC], vec![0x60, 0x06, 0x03, 0x01, 0x00, 0x01]]
            } else {
                vec![]
            }
        }));
        n.credits = 1;
        assert_eq!(n.transceive(&[0x30, 0x04], Duration::from_millis(200)).unwrap(), vec![0xAA, 0xBB, 0xCC]);
        // кредит пришёл после данных — учтётся при следующей отправке
        assert_eq!(n.transceive(&[0x30, 0x08], Duration::from_millis(200)).unwrap(), vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn deactivation_while_waiting() {
        let (mut n, _, _) = fake::nci(Box::new(|p| if p[0] == 0x00 { vec![vec![0x61, 0x06, 0x02, 0x03, 0x02]] } else { vec![] }));
        let e = n.transceive(&[0x30, 0x04], Duration::from_millis(200)).unwrap_err();
        assert!(e.to_string().contains("убрана"));
        assert!(matches!(n.event(Duration::ZERO).unwrap(), Some(Packet::Ntf { gid: 1, oid: 6, .. })));
    }
}
