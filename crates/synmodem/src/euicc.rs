//! eSIM (eUICC): локальные команды профилей ES10c (SGP.22) через логический канал к ISD-R — список
//! профилей, включение, выключение, удаление, имя, EID. Канал и APDU — службой UIM модема (QMI).
//! Загрузка новых профилей с SM-DP+ (ES9+, ES10b) здесь не делается.

use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::qmi::{Client, Message};

/// AID ISD-R (SGP.22).
const ISD_R: [u8; 16] = [0xA0, 0x00, 0x00, 0x05, 0x59, 0x10, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x00, 0x00, 0x01, 0x00];
const T: Duration = Duration::from_secs(20);

/// Профиль eSIM.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub iccid: String,
    pub enabled: bool,
    /// Имя, данное пользователем.
    pub nickname: String,
    /// Оператор (Service Provider Name).
    pub provider: String,
    /// Имя профиля у оператора.
    pub name: String,
    /// «рабочий», «тестовый», «служебный».
    pub class: String,
}

// ─── BER-TLV ────────────────────────────────────────────────────────────────

/// Элемент BER-TLV: тег (1–3 байта), значение.
#[derive(Debug, Clone)]
pub struct Tlv<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

/// Разобрать последовательность TLV.
pub fn parse(mut b: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let mut i = 0;
        let mut tag = u32::from(b[0]);
        i += 1;
        if b[0] & 0x1F == 0x1F {
            while i < b.len() {
                tag = tag << 8 | u32::from(b[i]);
                i += 1;
                if b[i - 1] & 0x80 == 0 {
                    break;
                }
            }
        }
        let Some(&l0) = b.get(i) else { break };
        i += 1;
        let len = if l0 & 0x80 == 0 {
            l0 as usize
        } else {
            let n = (l0 & 0x7F) as usize;
            let Some(lb) = b.get(i..i + n) else { break };
            i += n;
            lb.iter().fold(0usize, |a, x| a << 8 | *x as usize)
        };
        let Some(v) = b.get(i..i + len) else { break };
        out.push(Tlv { tag, value: v });
        b = &b[i + len..];
    }
    out
}

fn find<'a>(items: &[Tlv<'a>], tag: u32) -> Option<&'a [u8]> {
    items.iter().find(|t| t.tag == tag).map(|t| t.value)
}

/// Собрать TLV (тег 1–2 байта).
fn tlv(tag: u32, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if tag > 0xFF {
        out.push((tag >> 8) as u8);
    }
    out.push(tag as u8);
    let n = value.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend_from_slice(&[0x81, n as u8]);
    } else {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(value);
    out
}

/// ICCID: BCD с полубайтами наоборот, F — заполнитель.
pub fn iccid_text(b: &[u8]) -> String {
    let mut s = String::new();
    for x in b {
        for d in [x & 0x0F, x >> 4] {
            if d < 10 {
                s.push((b'0' + d) as char);
            }
        }
    }
    s
}

fn iccid_bytes(s: &str) -> Vec<u8> {
    let d: Vec<u8> = s.bytes().filter(u8::is_ascii_digit).map(|c| c - b'0').collect();
    d.chunks(2).map(|p| p.get(1).copied().unwrap_or(0xF) << 4 | p[0]).collect()
}

// ─── Канал к ISD-R ──────────────────────────────────────────────────────────

/// Открытый логический канал к ISD-R карты в слоте `slot` (с 1).
pub struct Channel<'a> {
    uim: &'a Client,
    slot: u8,
    id: u8,
}

impl<'a> Channel<'a> {
    pub fn open(uim: &'a Client, slot: u8) -> Result<Self> {
        let mut aid = vec![ISD_R.len() as u8];
        aid.extend_from_slice(&ISD_R);
        let r = uim.call(Message::new(0x42).u8(0x01, slot).tlv(0x10, aid), T).context("канал к eUICC (ISD-R)")?;
        let id = r.get(0x10).and_then(|b| b.first().copied()).context("нет номера канала")?;
        Ok(Self { uim, slot, id })
    }

    fn apdu(&self, apdu: &[u8]) -> Result<(Vec<u8>, u8, u8)> {
        let mut a = (apdu.len() as u16).to_le_bytes().to_vec();
        a.extend_from_slice(apdu);
        let r = self.uim.call(Message::new(0x3B).u8(0x01, self.slot).tlv(0x02, a).u8(0x10, self.id), T)?;
        let mut rd = r.reader(0x10).context("нет ответа APDU")?;
        let resp = rd.arr16().context("ответ APDU")?.to_vec();
        if resp.len() < 2 {
            bail!("короткий ответ APDU");
        }
        let (sw1, sw2) = (resp[resp.len() - 2], resp[resp.len() - 1]);
        Ok((resp[..resp.len() - 2].to_vec(), sw1, sw2))
    }

    /// Класс команды с номером логического канала (ETSI TS 102 221: каналы 1–3 и 4–19).
    fn cla(&self, base: u8) -> u8 {
        if self.id < 4 {
            base | self.id
        } else {
            (base & 0x80) | 0x40 | (self.id - 4)
        }
    }

    /// ES10: команда в STORE DATA (по 255 байт), ответ — через GET RESPONSE.
    pub fn es10(&self, cmd: &[u8]) -> Result<Vec<u8>> {
        let chunks: Vec<&[u8]> = cmd.chunks(255).collect();
        let mut data = Vec::new();
        let (mut sw1, mut sw2) = (0, 0);
        for (k, c) in chunks.iter().enumerate() {
            let last = k + 1 == chunks.len();
            let p1 = if last { 0x91 } else { 0x11 };
            let mut a = vec![self.cla(0x80), 0xE2, p1, k as u8, c.len() as u8];
            a.extend_from_slice(c);
            let (d, s1, s2) = self.apdu(&a)?;
            data.extend(d);
            (sw1, sw2) = (s1, s2);
        }
        while sw1 == 0x61 {
            let (d, s1, s2) = self.apdu(&[self.cla(0x00), 0xC0, 0x00, 0x00, sw2])?;
            data.extend(d);
            (sw1, sw2) = (s1, s2);
        }
        if (sw1, sw2) != (0x90, 0x00) {
            bail!("eUICC ответила {sw1:02X}{sw2:02X}");
        }
        Ok(data)
    }
}

impl Drop for Channel<'_> {
    fn drop(&mut self) {
        let _ = self.uim.call(Message::new(0x3F).u8(0x01, self.slot).u8(0x11, self.id), Duration::from_secs(5));
    }
}

// ─── ES10c ──────────────────────────────────────────────────────────────────

pub fn profiles(ch: &Channel) -> Result<Vec<Profile>> {
    let resp = ch.es10(&tlv(0xBF2D, &[]))?;
    let top = parse(&resp);
    let body = find(&top, 0xBF2D).context("нет ProfileInfoListResponse")?;
    let inner = parse(body);
    let Some(list) = find(&inner, 0xA0) else {
        bail!("eUICC: ошибка списка профилей");
    };
    let mut out = Vec::new();
    for p in parse(list).into_iter().filter(|t| t.tag == 0xE3) {
        let f = parse(p.value);
        let s = |t| find(&f, t).map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default();
        out.push(Profile {
            iccid: find(&f, 0x5A).map(iccid_text).unwrap_or_default(),
            enabled: find(&f, 0x9F70).and_then(|v| v.first()).is_some_and(|v| *v == 1),
            nickname: s(0x90),
            provider: s(0x91),
            name: s(0x92),
            class: match find(&f, 0x95).and_then(|v| v.first()) {
                Some(0) => "тестовый",
                Some(1) => "служебный",
                _ => "рабочий",
            }
            .into(),
        });
    }
    Ok(out)
}

/// Результат команды профиля (0 — успех) с понятной причиной.
fn result_code(resp: &[u8], tag: u32) -> Result<()> {
    let top = parse(resp);
    let body = find(&top, tag).context("нет ответа eUICC")?;
    let code = find(&parse(body), 0x80).and_then(|v| v.first().copied()).unwrap_or(127);
    let why = match code {
        0 => return Ok(()),
        1 => "профиль не найден",
        2 => "профиль уже в этом состоянии",
        3 => "запрещено политикой профиля",
        5 => "сначала выключите профиль",
        6 => "нельзя: это служебный профиль",
        127 => "неизвестная ошибка",
        _ => "ошибка eUICC",
    };
    bail!("{why} (код {code})")
}

fn by_iccid(iccid: &str) -> Vec<u8> {
    tlv(0x5A, &iccid_bytes(iccid))
}

pub fn enable(ch: &Channel, iccid: &str) -> Result<()> {
    // refreshFlag = TRUE: eUICC сама перезапустит сеанс карты (модем перечитает SIM)
    let body = [tlv(0xA0, &by_iccid(iccid)), tlv(0x81, &[0xFF])].concat();
    result_code(&ch.es10(&tlv(0xBF31, &body))?, 0xBF31)
}

pub fn disable(ch: &Channel, iccid: &str) -> Result<()> {
    let body = [tlv(0xA0, &by_iccid(iccid)), tlv(0x81, &[0xFF])].concat();
    result_code(&ch.es10(&tlv(0xBF32, &body))?, 0xBF32)
}

pub fn delete(ch: &Channel, iccid: &str) -> Result<()> {
    result_code(&ch.es10(&tlv(0xBF33, &by_iccid(iccid)))?, 0xBF33)
}

pub fn set_nickname(ch: &Channel, iccid: &str, name: &str) -> Result<()> {
    let body = [by_iccid(iccid), tlv(0x90, name.as_bytes())].concat();
    result_code(&ch.es10(&tlv(0xBF29, &body))?, 0xBF29)
}

pub fn eid(ch: &Channel) -> Result<String> {
    let resp = ch.es10(&tlv(0xBF3E, &tlv(0x5C, &[0x5A])))?;
    let top = parse(&resp);
    let body = find(&top, 0xBF3E).context("нет EID")?;
    let v = find(&parse(body), 0x5A).context("нет EID")?;
    Ok(v.iter().map(|b| format!("{b:02X}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ber_long_lengths_and_tags() {
        let inner = tlv(0x5A, &[0x98, 0x10]);
        let big = vec![0x11u8; 300];
        let msg = [tlv(0xBF2D, &inner), tlv(0x94, &big)].concat();
        let top = parse(&msg);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].tag, 0xBF2D);
        assert_eq!(top[1].value.len(), 300);
        assert_eq!(find(&parse(top[0].value), 0x5A), Some(&[0x98, 0x10][..]));
    }

    #[test]
    fn iccid_roundtrip() {
        let s = "8970101234567890123";
        assert_eq!(iccid_text(&iccid_bytes(s)), s);
    }

    #[test]
    fn profile_list_parse() {
        let p = [
            tlv(0x5A, &iccid_bytes("8970123")),
            tlv(0x9F70, &[1]),
            tlv(0x91, b"activ"),
            tlv(0x92, b"50126"),
            tlv(0x95, &[2]),
        ]
        .concat();
        let list = tlv(0xA0, &tlv(0xE3, &p));
        let resp = tlv(0xBF2D, &list);
        let top = parse(&resp);
        let inner = parse(find(&top, 0xBF2D).unwrap());
        let e3 = parse(find(&inner, 0xA0).unwrap());
        let f = parse(e3[0].value);
        assert_eq!(iccid_text(find(&f, 0x5A).unwrap()), "8970123");
        assert_eq!(find(&f, 0x91), Some(&b"activ"[..]));
    }
}
