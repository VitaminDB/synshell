//! SMS в формате PDU (3GPP TS 23.040): разбор SMS-DELIVER/SMS-SUBMIT из памяти модема и сборка SMS-SUBMIT
//! для отправки. Кодировки — GSM 7 бит (с таблицей расширения), 8 бит, UCS-2; длинные сообщения — частями
//! с заголовком склейки (UDH 0x00/0x08). PDU модема начинается с адреса SMS-центра.

/// Основная таблица GSM 03.38.
const GSM7: [char; 128] = [
    '@', '£', '$', '¥', 'è', 'é', 'ù', 'ì', 'ò', 'Ç', '\n', 'Ø', 'ø', '\r', 'Å', 'å', //
    'Δ', '_', 'Φ', 'Γ', 'Λ', 'Ω', 'Π', 'Ψ', 'Σ', 'Θ', 'Ξ', '\u{1b}', 'Æ', 'æ', 'ß', 'É', //
    ' ', '!', '"', '#', '¤', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/', //
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?', //
    '¡', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', //
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'Ä', 'Ö', 'Ñ', 'Ü', '§', //
    '¿', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', //
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'ä', 'ö', 'ñ', 'ü', 'à',
];

/// Таблица расширения (после ESC 0x1B).
const GSM7_EXT: [(u8, char); 10] = [
    (0x0A, '\u{c}'),
    (0x14, '^'),
    (0x28, '{'),
    (0x29, '}'),
    (0x2F, '\\'),
    (0x3C, '['),
    (0x3D, '~'),
    (0x3E, ']'),
    (0x40, '|'),
    (0x65, '€'),
];

/// Септеты GSM-7 для символа (`None` — не кодируется).
fn gsm7_encode_char(c: char) -> Option<Vec<u8>> {
    if c != '\u{1b}' {
        if let Some(i) = GSM7.iter().position(|&g| g == c) {
            return Some(vec![i as u8]);
        }
    }
    GSM7_EXT.iter().find(|(_, g)| *g == c).map(|(i, _)| vec![0x1B, *i])
}

fn gsm7_septets(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for c in text.chars() {
        out.extend(gsm7_encode_char(c)?);
    }
    Some(out)
}

fn gsm7_decode(septets: &[u8]) -> String {
    let mut s = String::new();
    let mut esc = false;
    for &b in septets {
        let b = b & 0x7F;
        if esc {
            esc = false;
            s.push(GSM7_EXT.iter().find(|(i, _)| *i == b).map(|(_, c)| *c).unwrap_or(' '));
        } else if b == 0x1B {
            esc = true;
        } else {
            s.push(GSM7[b as usize]);
        }
    }
    s
}

/// Распаковать `count` септетов, пропустив `skip_bits` бит в начале (заголовок UDH с выравниванием).
fn unpack7(data: &[u8], skip_bits: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let bit = skip_bits + k * 7;
        let (byte, off) = (bit / 8, bit % 8);
        let Some(&lo) = data.get(byte) else { break };
        let hi = data.get(byte + 1).copied().unwrap_or(0);
        let v = ((u16::from(hi) << 8 | u16::from(lo)) >> off) & 0x7F;
        out.push(v as u8);
    }
    out
}

/// Упаковать септеты, начиная с бита `skip_bits` (до него — место под UDH).
fn pack7(septets: &[u8], skip_bits: usize) -> Vec<u8> {
    let total = skip_bits + septets.len() * 7;
    let mut out = vec![0u8; total.div_ceil(8)];
    for (k, &s) in septets.iter().enumerate() {
        let bit = skip_bits + k * 7;
        let (byte, off) = (bit / 8, bit % 8);
        let v = u16::from(s & 0x7F) << off;
        out[byte] |= v as u8;
        if off > 1 {
            out[byte + 1] |= (v >> 8) as u8;
        }
    }
    out
}

/// Упакованный GSM-7 (имена сетей): `spare` — неиспользованных бит в последнем октете.
pub fn gsm7_packed(data: &[u8], spare: u8) -> String {
    let septets = (data.len() * 8).saturating_sub(spare as usize) / 7;
    gsm7_decode(&unpack7(data, 0, septets))
}

/// Номер: BCD с полубайтами наоборот; `digits` — число цифр.
fn decode_address(toa: u8, data: &[u8], digits: usize) -> String {
    // Буквенно-цифровой отправитель («Kcell», «Bank») — GSM-7 упакованный
    if toa & 0x70 == 0x50 {
        let septets = digits * 4 / 7;
        return gsm7_decode(&unpack7(data, 0, septets));
    }
    let mut s = String::new();
    if toa & 0x70 == 0x10 {
        s.push('+');
    }
    for i in 0..digits {
        let b = data.get(i / 2).copied().unwrap_or(0xFF);
        let d = if i % 2 == 0 { b & 0x0F } else { b >> 4 };
        s.push(match d {
            0..=9 => (b'0' + d) as char,
            0xA => '*',
            0xB => '#',
            0xC => 'a',
            0xD => 'b',
            0xE => 'c',
            _ => continue,
        });
    }
    s
}

fn encode_address(number: &str) -> Vec<u8> {
    let intl = number.starts_with('+');
    let digits: Vec<u8> = number
        .chars()
        .filter_map(|c| match c {
            '0'..='9' => Some(c as u8 - b'0'),
            '*' => Some(0xA),
            '#' => Some(0xB),
            _ => None,
        })
        .collect();
    let mut out = vec![digits.len() as u8, if intl { 0x91 } else { 0x81 }];
    for pair in digits.chunks(2) {
        let lo = pair[0];
        let hi = pair.get(1).copied().unwrap_or(0xF);
        out.push(hi << 4 | lo);
    }
    out
}

/// Время SMS-центра (7 полуоктетов) → секунды UNIX.
fn decode_timestamp(b: &[u8]) -> Option<i64> {
    if b.len() < 7 {
        return None;
    }
    let sw = |x: u8| -> i64 { i64::from(x & 0x0F) * 10 + i64::from(x >> 4) };
    let (y, mo, d, h, mi, s) = (2000 + sw(b[0]), sw(b[1]), sw(b[2]), sw(b[3]), sw(b[4]), sw(b[5]));
    // Пояс в четвертях часа, знак — бит 3 младшего полубайта
    let tz_raw = b[6];
    let q = i64::from(tz_raw & 0x07) * 10 + i64::from(tz_raw >> 4);
    let tz = if tz_raw & 0x08 != 0 { -q } else { q } * 15 * 60;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - tz)
}

/// Дни от 1970-01-01 (алгоритм Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coding {
    Gsm7,
    Bit8,
    Ucs2,
}

fn coding_of(dcs: u8) -> Coding {
    match dcs >> 4 {
        0x0..=0x3 => match (dcs >> 2) & 3 {
            1 => Coding::Bit8,
            2 => Coding::Ucs2,
            _ => Coding::Gsm7,
        },
        0xE => Coding::Ucs2,
        0xF if dcs & 0x04 != 0 => Coding::Bit8,
        _ => Coding::Gsm7,
    }
}

/// Часть длинного сообщения: номер склейки, всего частей, номер этой части (с 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Concat {
    pub reference: u16,
    pub total: u8,
    pub seq: u8,
}

/// Разобранное SMS.
#[derive(Clone, Debug, PartialEq)]
pub struct Sms {
    /// Отправитель (входящее) или получатель (исходящее).
    pub number: String,
    pub text: String,
    /// Время SMS-центра, секунды UNIX (у исходящих нет).
    pub timestamp: Option<i64>,
    pub incoming: bool,
    pub concat: Option<Concat>,
    /// Класс 0 — «флеш», показать сразу и не хранить.
    pub flash: bool,
}

/// Разобрать PDU из памяти модема (с адресом SMS-центра в начале).
pub fn decode(pdu: &[u8]) -> Option<Sms> {
    let smsc_len = *pdu.first()? as usize;
    let p = pdu.get(1 + smsc_len..)?;
    let first = *p.first()?;
    let mti = first & 0x03;
    let udhi = first & 0x40 != 0;
    let mut i = 1;
    let incoming = match mti {
        0 => true,
        1 => {
            i += 1; // TP-MR
            false
        }
        _ => return None,
    };
    let digits = *p.get(i)? as usize;
    let toa = *p.get(i + 1)?;
    let alen = digits.div_ceil(2);
    let number = decode_address(toa, p.get(i + 2..i + 2 + alen)?, digits);
    i += 2 + alen;
    i += 1; // TP-PID
    let dcs = *p.get(i)?;
    i += 1;
    let timestamp = if incoming {
        let t = decode_timestamp(p.get(i..i + 7)?);
        i += 7;
        t
    } else {
        // TP-VP: нет / относительный (1 октет) / абсолютный или расширенный (7)
        i += match (first >> 3) & 3 {
            0 => 0,
            2 => 1,
            _ => 7,
        };
        None
    };
    let udl = *p.get(i)? as usize;
    let ud = p.get(i + 1..)?;
    let coding = coding_of(dcs);
    let mut concat = None;
    let mut udh_len = 0;
    if udhi {
        let hl = *ud.first()? as usize;
        udh_len = hl + 1;
        let h = ud.get(1..1 + hl)?;
        let mut k = 0;
        while k + 2 <= h.len() {
            let (iei, l) = (h[k], h[k + 1] as usize);
            let v = h.get(k + 2..k + 2 + l)?;
            match (iei, l) {
                (0x00, 3) => concat = Some(Concat { reference: u16::from(v[0]), total: v[1], seq: v[2] }),
                (0x08, 4) => {
                    concat = Some(Concat { reference: u16::from_be_bytes([v[0], v[1]]), total: v[2], seq: v[3] })
                }
                _ => {}
            }
            k += 2 + l;
        }
    }
    let text = match coding {
        Coding::Gsm7 => {
            // UDL — в септетах, включая заголовок с выравниванием до септета
            let skip = (udh_len * 8).div_ceil(7);
            gsm7_decode(&unpack7(ud, skip * 7, udl.saturating_sub(skip)))
        }
        Coding::Ucs2 => {
            let body = ud.get(udh_len..udl.min(ud.len()))?;
            let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        Coding::Bit8 => {
            let body = ud.get(udh_len..udl.min(ud.len()))?;
            String::from_utf8_lossy(body).into_owned()
        }
    };
    let flash = dcs & 0xC0 == 0 && dcs & 0x10 != 0 && dcs & 0x03 == 0 || dcs & 0xF3 == 0xF0;
    Some(Sms { number, text, timestamp, incoming, concat, flash })
}

/// Собрать SMS-SUBMIT (одно или несколько PDU, с адресом SMS-центра «по умолчанию»).
/// `reference` — номер склейки для длинного сообщения.
pub fn encode_submit(number: &str, text: &str, reference: u8) -> Vec<Vec<u8>> {
    let addr = encode_address(number);
    let build = |dcs: u8, udh: Option<[u8; 6]>, udl: u8, ud: &[u8]| -> Vec<u8> {
        let mut p = vec![0x00, if udh.is_some() { 0x41 } else { 0x01 }, 0x00];
        p.extend_from_slice(&addr);
        p.extend_from_slice(&[0x00, dcs, udl]);
        p.extend_from_slice(ud);
        p
    };
    let udh = |total: usize, seq: usize| [0x05, 0x00, 0x03, reference, total as u8, seq as u8];
    if let Some(septets) = gsm7_septets(text) {
        if septets.len() <= 160 {
            return vec![build(0x00, None, septets.len() as u8, &pack7(&septets, 0))];
        }
        // 153 септета на часть; ESC с продолжением не разрывать
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut rest = &septets[..];
        while !rest.is_empty() {
            let mut n = rest.len().min(153);
            if n < rest.len() && rest[n - 1] == 0x1B {
                n -= 1;
            }
            parts.push(&rest[..n]);
            rest = &rest[n..];
        }
        let total = parts.len();
        return parts
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let h = udh(total, k + 1);
                // Заголовок 6 октетов = 48 бит, выравнивание до 49 (7 септетов)
                let mut ud = pack7(s, 49);
                ud[..6].copy_from_slice(&h);
                build(0x00, Some(h), (7 + s.len()) as u8, &ud)
            })
            .collect();
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let be = |u: &[u16]| u.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    if units.len() <= 70 {
        let ud = be(&units);
        return vec![build(0x08, None, ud.len() as u8, &ud)];
    }
    let mut parts: Vec<&[u16]> = Vec::new();
    let mut rest = &units[..];
    while !rest.is_empty() {
        let mut n = rest.len().min(67);
        // Суррогатную пару не разрывать
        if n < rest.len() && (0xD800..0xDC00).contains(&rest[n - 1]) {
            n -= 1;
        }
        parts.push(&rest[..n]);
        rest = &rest[n..];
    }
    let total = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(k, u)| {
            let h = udh(total, k + 1);
            let mut ud = h.to_vec();
            ud.extend(be(u));
            build(0x08, Some(h), ud.len() as u8, &ud)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn deliver_gsm7() {
        // Известный пример (diafaan): буквенный отправитель «diafaan», текст «diafaan.com»
        let pdu = hex("0791448720003023240DD0E474D81C0EBB010000111011315214000BE474D81C0EBB5DE3771B");
        let s = decode(&pdu).unwrap();
        assert!(s.incoming);
        assert_eq!(s.text, "diafaan.com");
        assert_eq!(s.number, "diafaan");
    }

    #[test]
    fn deliver_ucs2_russian() {
        // От +77011234567, UCS-2 «Привет»
        let pdu = hex("07919712690080F2040B919707214365F70008621010311500210C041F04400438043204350442");
        let s = decode(&pdu).unwrap();
        assert_eq!(s.number, "+79701234567");
        assert_eq!(s.text, "Привет");
        assert!(s.timestamp.is_some());
    }

    #[test]
    fn timestamp_utc() {
        // 2026-01-01 00:00:00, пояс +0
        assert_eq!(decode_timestamp(&[0x62, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00]), Some(1767225600));
        // То же время в поясе +5 (20 четвертей): UTC на 5 ч раньше
        assert_eq!(decode_timestamp(&[0x62, 0x10, 0x10, 0x00, 0x00, 0x00, 0x02]), Some(1767225600 - 5 * 3600));
    }

    #[test]
    fn submit_roundtrip_gsm7() {
        let p = encode_submit("+77011234567", "Hello [world] €5", 1);
        assert_eq!(p.len(), 1);
        let s = decode(&p[0]).unwrap();
        assert!(!s.incoming);
        assert_eq!(s.number, "+77011234567");
        assert_eq!(s.text, "Hello [world] €5");
    }

    #[test]
    fn submit_long_gsm7_parts() {
        let text: String = "a".repeat(200);
        let p = encode_submit("123", &text, 7);
        assert_eq!(p.len(), 2);
        let a = decode(&p[0]).unwrap();
        let b = decode(&p[1]).unwrap();
        assert_eq!(a.concat, Some(Concat { reference: 7, total: 2, seq: 1 }));
        assert_eq!(b.concat.unwrap().seq, 2);
        assert_eq!(a.text.len() + b.text.len(), 200);
        assert_eq!(format!("{}{}", a.text, b.text), text);
    }

    #[test]
    fn submit_long_ucs2_parts() {
        let text: String = "Ж".repeat(100) + "😀";
        let p = encode_submit("8701", &text, 9);
        assert_eq!(p.len(), 2);
        let joined: String = p.iter().map(|x| decode(x).unwrap().text).collect();
        assert_eq!(joined, text);
    }
}
