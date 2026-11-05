//! Метки: разбор активации (RF_INTF_ACTIVATED_NTF), чтение и запись NDEF —
//! Type 2 (NTAG, MIFARE Ultralight: команды READ/WRITE через Frame-интерфейс) и
//! Type 4 (ISO-DEP: APDU приложения NDEF D2760000850101). FeliCa, ISO 15693 и
//! MIFARE Classic — только идентификатор и вид.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::api::Tag;
use crate::nci::Nci;
use crate::ndef::{self, Record};
use synshell_tr::t;

// Протоколы RF (NCI)
pub const PROTO_T1T: u8 = 0x01;
pub const PROTO_T2T: u8 = 0x02;
pub const PROTO_T3T: u8 = 0x03;
pub const PROTO_ISO_DEP: u8 = 0x04;
pub const PROTO_T5T: u8 = 0x06;
/// Проприетарный протокол NXP: MIFARE Classic.
pub const PROTO_MIFARE: u8 = 0x80;

/// Активация метки или считывателя.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub discovery_id: u8,
    pub interface: u8,
    pub protocol: u8,
    /// Технология и режим: 0x00 опрос NFC-A, 0x01 B, 0x02 F, 0x06 V; 0x80+ — прослушивание.
    pub mode: u8,
    pub max_payload: u8,
    pub credits: u8,
    pub params: Vec<u8>,
    pub activation: Vec<u8>,
}

impl Activation {
    /// Разобрать RF_INTF_ACTIVATED_NTF.
    pub fn parse(p: &[u8]) -> Option<Activation> {
        let plen = *p.get(6)? as usize;
        let params = p.get(7..7 + plen)?.to_vec();
        let rest = p.get(7 + plen..)?;
        // технология обмена, скорости приёма и передачи, длина параметров активации
        let alen = *rest.get(3).unwrap_or(&0) as usize;
        let activation = rest.get(4..4 + alen).unwrap_or(&[]).to_vec();
        Some(Activation { discovery_id: p[0], interface: p[1], protocol: p[2], mode: p[3], max_payload: p[4], credits: p[5], params, activation })
    }

    pub fn listen(&self) -> bool {
        self.mode & 0x80 != 0
    }

    /// Идентификатор метки и технология.
    pub fn uid(&self) -> (Vec<u8>, &'static str) {
        let p = &self.params;
        match self.mode {
            // NFC-A: SENS_RES(2), длина NFCID1, NFCID1, длина SEL_RES, SEL_RES
            0x00 => {
                let n = *p.get(2).unwrap_or(&0) as usize;
                (p.get(3..3 + n).unwrap_or(&[]).to_vec(), "NFC-A")
            }
            // NFC-B: SENSB_RES — NFCID0 с 1-го байта (4 байта)
            0x01 => (p.get(2..6).or_else(|| p.get(1..5)).unwrap_or(&[]).to_vec(), "NFC-B"),
            // NFC-F: скорость, длина SENSF_RES, SENSF_RES (код 01 + NFCID2 8 байт)
            0x02 => (p.get(3..11).unwrap_or(&[]).to_vec(), "NFC-F"),
            // NFC-V: флаги, DSFID, UID (8 байт, младший первым)
            0x06 => {
                let mut u = p.get(2..10).unwrap_or(&[]).to_vec();
                u.reverse();
                (u, "NFC-V")
            }
            _ => (vec![], "?"),
        }
    }

    /// SENS_RES (ATQA) и SEL_RES (SAK) NFC-A.
    pub fn atqa_sak(&self) -> Option<([u8; 2], u8)> {
        if self.mode != 0x00 {
            return None;
        }
        let p = &self.params;
        let n = *p.get(2)? as usize;
        let sak = *p.get(3 + n + 1)?;
        Some(([*p.first()?, *p.get(1)?], sak))
    }
}

/// Вид метки по протоколу, SAK/ATQA и версии (GET_VERSION NTAG).
fn kind(a: &Activation, version: Option<&[u8]>) -> String {
    if let Some(v) = version {
        // GET_VERSION: 00 vendor type subtype major minor storage protocol
        if v.len() >= 8 && v[1] == 0x04 {
            let name = match (v[2], v[6]) {
                (0x04, 0x0F) => "NTAG213",
                (0x04, 0x11) => "NTAG215",
                (0x04, 0x13) => "NTAG216",
                (0x04, _) => "NTAG",
                (0x03, 0x0B) => "MIFARE Ultralight EV1",
                (0x03, _) => "MIFARE Ultralight",
                _ => "NXP Type 2",
            };
            return name.into();
        }
    }
    match (a.protocol, a.atqa_sak().map(|x| x.1)) {
        (PROTO_MIFARE, Some(0x08)) | (_, Some(0x08)) if a.protocol != PROTO_ISO_DEP => "MIFARE Classic 1K".into(),
        (PROTO_MIFARE, Some(0x18)) | (_, Some(0x18)) if a.protocol != PROTO_ISO_DEP => "MIFARE Classic 4K".into(),
        (PROTO_MIFARE, _) => "MIFARE Classic".into(),
        (PROTO_T2T, _) => "Type 2".into(),
        (PROTO_ISO_DEP, Some(0x20)) | (PROTO_ISO_DEP, Some(0x28)) => t!("ISO 14443-4 (смарт-карта)").into(),
        (PROTO_ISO_DEP, _) => "ISO 14443-4".into(),
        (PROTO_T3T, _) => "FeliCa".into(),
        (PROTO_T5T, _) => "ISO 15693".into(),
        (PROTO_T1T, _) => "Type 1 (Topaz)".into(),
        _ => t!("протокол {protocol}", protocol = format!("{:02X}", a.protocol)),
    }
}

const T: Duration = Duration::from_millis(500);

/// Обмен с меткой Type 2 через Frame-интерфейс: в конце ответа — байт статуса.
fn t2t(n: &mut Nci, cmd: &[u8]) -> Result<Vec<u8>> {
    let mut r = n.transceive(cmd, T)?;
    match r.pop() {
        Some(0) => Ok(r),
        Some(s) => bail!("{}", t!("Type 2: статус {s}", s = format!("{:02X}", s))),
        None => bail!("{}", t!("Type 2: пустой ответ")),
    }
}

fn t2t_read(n: &mut Nci, page: u8) -> Result<Vec<u8>> {
    let r = t2t(n, &[0x30, page])?;
    if r.len() < 16 {
        bail!("{}", t!("Type 2: короткий ответ READ ({n} байт)", n = r.len()));
    }
    Ok(r[..16].to_vec())
}

/// TLV NDEF в области данных Type 2: (смещение начала сообщения, длина).
fn find_ndef_tlv(data: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < data.len() {
        let t = data[i];
        match t {
            0x00 => i += 1,
            0xFE => return None,
            _ => {
                let (len, hdr) = if *data.get(i + 1)? == 0xFF { (u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]) as usize, 4) } else { (data[i + 1] as usize, 2) };
                if t == 0x03 {
                    return Some((i + hdr, len));
                }
                i += hdr + len;
            }
        }
    }
    None
}

/// Прочитать метку: вид, идентификатор, NDEF (если есть), ёмкость и запись.
pub fn read(n: &mut Nci, a: &Activation) -> Tag {
    let (uid, tech) = a.uid();
    let mut tag = Tag { uid: ndef::hex(&uid), tech: tech.into(), protocol: proto_name(a.protocol).into(), ..Default::default() };
    if let Some((atqa, sak)) = a.atqa_sak() {
        tag.atqa = ndef::hex(&atqa);
        tag.sak = format!("{sak:02X}");
    }
    if a.protocol == PROTO_ISO_DEP && !a.activation.is_empty() {
        tag.ats = ndef::hex(&a.activation);
    }
    let res = match a.protocol {
        PROTO_T2T => read_t2t(n, a, &mut tag),
        PROTO_ISO_DEP => read_t4t(n, &mut tag),
        _ => {
            tag.kind = kind(a, None);
            Ok(())
        }
    };
    if let Err(e) = res {
        tag.error = e.to_string();
    }
    if tag.kind.is_empty() {
        tag.kind = kind(a, None);
    }
    tag
}

pub fn proto_name(p: u8) -> &'static str {
    match p {
        PROTO_T1T => "T1T",
        PROTO_T2T => "T2T",
        PROTO_T3T => "T3T",
        PROTO_ISO_DEP => "ISO-DEP",
        PROTO_T5T => "T5T",
        PROTO_MIFARE => "MIFARE",
        _ => "?",
    }
}

fn read_t2t(n: &mut Nci, a: &Activation, tag: &mut Tag) -> Result<()> {
    let version = t2t(n, &[0x60]).ok();
    tag.kind = kind(a, version.as_deref());
    let head = t2t_read(n, 0)?;
    let cc = &head[12..16];
    if cc[0] != 0xE1 {
        // не отформатирована под NDEF
        tag.raw = ndef::hex(&head);
        return Ok(());
    }
    let size = cc[2] as usize * 8;
    tag.capacity = size as u32;
    tag.writable = cc[3] & 0xF0 == 0;
    let mut data = Vec::with_capacity(size);
    let mut page = 4u8;
    while data.len() < size {
        data.extend_from_slice(&t2t_read(n, page)?);
        // сообщение уже целиком — дальше не читать
        if let Some((off, len)) = find_ndef_tlv(&data) {
            if data.len() >= off + len {
                break;
            }
        }
        page = page.wrapping_add(4);
    }
    let mut dump = head.clone();
    dump.extend_from_slice(&data);
    tag.raw = ndef::hex(&dump);
    if let Some((off, len)) = find_ndef_tlv(&data) {
        if let Some(msg) = data.get(off..off + len) {
            tag.ndef = Some(ndef::parse(msg).unwrap_or_default());
        }
    }
    Ok(())
}

/// APDU и ответ с SW1 SW2; ошибка, если SW ≠ 90 00.
fn apdu(n: &mut Nci, cmd: &[u8]) -> Result<Vec<u8>> {
    let mut r = n.transceive(cmd, T)?;
    if r.len() < 2 {
        bail!("{}", t!("ISO-DEP: короткий ответ"));
    }
    let sw = r.split_off(r.len() - 2);
    if sw != [0x90, 0x00] {
        bail!("ISO-DEP: SW {:02X}{:02X}", sw[0], sw[1]);
    }
    Ok(r)
}

pub const NDEF_AID: [u8; 7] = [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01];

/// Описание файла NDEF Type 4 из CC.
struct T4File {
    id: [u8; 2],
    max: usize,
    mle: usize,
    mlc: usize,
    writable: bool,
}

fn t4t_open(n: &mut Nci) -> Result<T4File> {
    let mut sel = vec![0x00, 0xA4, 0x04, 0x00, 0x07];
    sel.extend_from_slice(&NDEF_AID);
    sel.push(0x00);
    apdu(n, &sel).map_err(|_| anyhow::anyhow!("{}", t!("нет приложения NDEF")))?;
    apdu(n, &[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x03])?;
    let cc = apdu(n, &[0x00, 0xB0, 0x00, 0x00, 0x0F])?;
    if cc.len() < 15 || cc[7] != 0x04 {
        bail!("{}", t!("ISO-DEP: неверный CC"));
    }
    let f = T4File {
        id: [cc[9], cc[10]],
        max: u16::from_be_bytes([cc[11], cc[12]]) as usize,
        mle: (u16::from_be_bytes([cc[3], cc[4]]) as usize).clamp(1, 250),
        mlc: (u16::from_be_bytes([cc[5], cc[6]]) as usize).clamp(1, 250),
        writable: cc[14] == 0x00,
    };
    apdu(n, &[0x00, 0xA4, 0x00, 0x0C, 0x02, f.id[0], f.id[1]])?;
    Ok(f)
}

fn read_t4t(n: &mut Nci, tag: &mut Tag) -> Result<()> {
    tag.kind = "ISO 14443-4".into();
    let f = t4t_open(n)?;
    tag.kind = "Type 4".into();
    tag.capacity = f.max.saturating_sub(2) as u32;
    tag.writable = f.writable;
    let nlen = apdu(n, &[0x00, 0xB0, 0x00, 0x00, 0x02])?;
    let len = u16::from_be_bytes([*nlen.first().unwrap_or(&0), *nlen.get(1).unwrap_or(&0)]) as usize;
    let mut msg = Vec::with_capacity(len);
    while msg.len() < len {
        let off = 2 + msg.len();
        let chunk = (len - msg.len()).min(f.mle);
        let r = apdu(n, &[0x00, 0xB0, (off >> 8) as u8, off as u8, chunk as u8])?;
        if r.is_empty() {
            break;
        }
        msg.extend_from_slice(&r);
    }
    tag.ndef = Some(ndef::parse(&msg).unwrap_or_default());
    Ok(())
}

/// Записать сообщение NDEF на метку (Type 2 или Type 4).
pub fn write(n: &mut Nci, a: &Activation, records: &[Record]) -> Result<()> {
    let msg = ndef::encode(records);
    match a.protocol {
        PROTO_T2T => write_t2t(n, &msg),
        PROTO_ISO_DEP => write_t4t(n, &msg),
        p => bail!("{}", t!("запись на {v} не поддерживается", v = proto_name(p))),
    }
}

/// TLV NDEF (+ терминатор) для области данных Type 2.
pub fn t2t_tlv(msg: &[u8]) -> Vec<u8> {
    let mut v = vec![0x03];
    if msg.len() < 0xFF {
        v.push(msg.len() as u8);
    } else {
        v.push(0xFF);
        v.extend_from_slice(&(msg.len() as u16).to_be_bytes());
    }
    v.extend_from_slice(msg);
    v.push(0xFE);
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

fn write_t2t(n: &mut Nci, msg: &[u8]) -> Result<()> {
    let head = t2t_read(n, 0)?;
    let cc = &head[12..16];
    if cc[0] != 0xE1 {
        bail!("{}", t!("метка не размечена под NDEF"));
    }
    if cc[3] & 0xF0 != 0 {
        bail!("{}", t!("метка защищена от записи"));
    }
    let tlv = t2t_tlv(msg);
    let size = cc[2] as usize * 8;
    if tlv.len() > size {
        bail!("{}", t!("не помещается: {n} байт из {size}", n = tlv.len(), size = size));
    }
    for (i, chunk) in tlv.chunks(4).enumerate() {
        let mut cmd = vec![0xA2, 4 + i as u8];
        cmd.extend_from_slice(chunk);
        let r = n.transceive(&cmd, T)?;
        // ACK — 4 бита 0xA (через Frame-интерфейс — байт и статус)
        if r.first().is_none_or(|b| b & 0x0F != 0x0A) {
            bail!("{}", t!("Type 2: запись страницы {v} не подтверждена", v = 4 + i));
        }
    }
    Ok(())
}

fn write_t4t(n: &mut Nci, msg: &[u8]) -> Result<()> {
    let f = t4t_open(n)?;
    if !f.writable {
        bail!("{}", t!("метка защищена от записи"));
    }
    if msg.len() + 2 > f.max {
        bail!("{}", t!("не помещается: {n} байт из {v}", n = msg.len(), v = f.max.saturating_sub(2)));
    }
    // NLEN = 0, данные, затем настоящая длина — прерванная запись не оставит битое сообщение
    apdu(n, &[0x00, 0xD6, 0x00, 0x00, 0x02, 0x00, 0x00])?;
    let mut off = 2;
    for chunk in msg.chunks(f.mlc) {
        let mut cmd = vec![0x00, 0xD6, (off >> 8) as u8, off as u8, chunk.len() as u8];
        cmd.extend_from_slice(chunk);
        apdu(n, &cmd)?;
        off += chunk.len();
    }
    let l = (msg.len() as u16).to_be_bytes();
    apdu(n, &[0x00, 0xD6, 0x00, 0x00, 0x02, l[0], l[1]])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nci::fake;

    /// NTAG213 с сообщением NDEF (ссылка) в памяти.
    fn ntag_memory(msg: &[u8]) -> Vec<u8> {
        let mut m = vec![0x04, 0x11, 0x22, 0xB7, 0x33, 0x44, 0x55, 0x66, 0x77, 0x48, 0x00, 0x00, 0xE1, 0x10, 0x12, 0x00];
        m.extend_from_slice(&t2t_tlv(msg));
        m.resize(16 + 144 + 16, 0);
        m
    }

    fn ntag_responder(mem: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> fake::Responder {
        Box::new(move |p: &[u8]| {
            if p[0] != 0x00 {
                return vec![];
            }
            let cmd = &p[3..];
            let mut out = vec![];
            match cmd[0] {
                0x60 => out.extend_from_slice(&[0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x0F, 0x03]),
                0x30 => {
                    let m = mem.lock().unwrap();
                    let o = cmd[1] as usize * 4;
                    out.extend_from_slice(&m[o..o + 16]);
                }
                0xA2 => {
                    let mut m = mem.lock().unwrap();
                    let o = cmd[1] as usize * 4;
                    m[o..o + 4].copy_from_slice(&cmd[2..6]);
                    out.push(0x0A);
                }
                _ => {}
            }
            out.push(0x00);
            let mut pkt = vec![0x00, 0x00, out.len() as u8];
            pkt.extend(out);
            vec![pkt, vec![0x60, 0x06, 0x03, 0x01, 0x00, 0x01]]
        })
    }

    fn ntag_activation() -> Activation {
        // 61 05: id, интерфейс Frame, протокол T2T, NFC-A, 255, 1 кредит, параметры A: ATQA 4400, UID 7, SAK 00
        let p = [0x01, 0x01, 0x02, 0x00, 0xFF, 0x01, 0x0C, 0x44, 0x00, 0x07, 0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        Activation::parse(&p).unwrap()
    }

    #[test]
    fn read_and_write_ntag() {
        let url = vec![Record::Uri { uri: "https://synshell.org".into() }];
        let mem = std::sync::Arc::new(std::sync::Mutex::new(ntag_memory(&ndef::encode(&url))));
        let (mut n, _, _) = fake::nci(ntag_responder(mem.clone()));
        let a = ntag_activation();
        assert_eq!(a.uid().0, vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        let tag = read(&mut n, &a);
        assert_eq!(tag.error, "");
        assert_eq!(tag.kind, "NTAG213");
        assert_eq!(tag.uid, "04112233445566");
        assert_eq!(tag.sak, "00");
        assert_eq!(tag.capacity, 144);
        assert!(tag.writable);
        assert_eq!(tag.ndef.as_deref(), Some(&url[..]));
        let text = vec![Record::Text { text: "Метка synshell".into(), lang: "ru".into() }];
        write(&mut n, &a, &text).unwrap();
        assert_eq!(read(&mut n, &a).ndef.as_deref(), Some(&text[..]));
        // не помещается
        let big = vec![Record::Text { text: "x".repeat(400), lang: "en".into() }];
        assert!(write(&mut n, &a, &big).unwrap_err().to_string().contains("не помещается"));
    }

    #[test]
    fn read_type4_via_emulator() {
        // своя эмуляция Type 4 и чтение — одним кодом с двух сторон
        let msg = vec![Record::Uri { uri: "tel:+70000000000".into() }];
        let emu = std::sync::Arc::new(std::sync::Mutex::new(crate::emu::T4Emulator::new(&msg)));
        let (mut n, _, _) = fake::nci(Box::new(move |p: &[u8]| {
            if p[0] != 0x00 {
                return vec![];
            }
            let r = emu.lock().unwrap().apdu(&p[3..]);
            let mut pkt = vec![0x00, 0x00, r.len() as u8];
            pkt.extend(r);
            vec![pkt, vec![0x60, 0x06, 0x03, 0x01, 0x00, 0x01]]
        }));
        let a = Activation { discovery_id: 1, interface: 2, protocol: PROTO_ISO_DEP, mode: 0, max_payload: 255, credits: 1, params: vec![0x04, 0x00, 0x04, 1, 2, 3, 4, 0x01, 0x20], activation: vec![0x05, 0x78] };
        let tag = read(&mut n, &a);
        assert_eq!(tag.error, "");
        assert_eq!(tag.kind, "Type 4");
        assert_eq!(tag.ndef.as_deref(), Some(&msg[..]));
        assert!(!tag.writable);
    }
}
