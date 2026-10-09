//! NDEF (NFC Forum Data Exchange Format): разбор и сборка сообщений — ссылки (URI), текст,
//! MIME (визитки vCard, Wi-Fi и т. п.), Smart Poster; остальное — как есть ([`Record::Raw`]).

use serde::{Deserialize, Serialize};

/// Запись NDEF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    /// Ссылка (well-known `U`): `https://…`, `tel:…`, `mailto:…`.
    Uri { uri: String },
    /// Текст (well-known `T`) с языком (`ru`, `en`).
    Text { text: String, lang: String },
    /// MIME (TNF 2): `text/vcard`, `application/vnd.wfa.wsc` и т. п.; данные — hex.
    Mime { mime: String, data: String },
    /// Smart Poster (well-known `Sp`): ссылка и подпись.
    SmartPoster { uri: String, title: String },
    /// Прочее: TNF, тип и данные — hex.
    Raw { tnf: u8, r#type: String, id: String, payload: String },
}

/// Префиксы URI (NFC Forum URI RTD, коды 0x00…0x23).
const URI_PREFIXES: [&str; 36] = [
    "", "http://www.", "https://www.", "http://", "https://", "tel:", "mailto:", "ftp://anonymous:anonymous@", "ftp://ftp.", "ftps://",
    "sftp://", "smb://", "nfs://", "ftp://", "dav://", "news:", "telnet://", "imap:", "rtsp://", "urn:", "pop:", "sip:", "sips:", "tftp:",
    "btspp://", "btl2cap://", "btgoep://", "tcpobex://", "irdaobex://", "file://", "urn:epc:id:", "urn:epc:tag:", "urn:epc:pat:",
    "urn:epc:raw:", "urn:epc:", "urn:nfc:",
];

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != ':').collect();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn decode_uri(p: &[u8]) -> String {
    let Some((&code, rest)) = p.split_first() else { return String::new() };
    let prefix = URI_PREFIXES.get(code as usize).copied().unwrap_or("");
    format!("{prefix}{}", String::from_utf8_lossy(rest))
}

fn encode_uri(uri: &str) -> Vec<u8> {
    // самый длинный подходящий префикс
    let (code, prefix) = URI_PREFIXES.iter().enumerate().skip(1).filter(|(_, p)| uri.starts_with(*p)).max_by_key(|(_, p)| p.len()).map(|(i, p)| (i as u8, *p)).unwrap_or((0, ""));
    let mut v = vec![code];
    v.extend_from_slice(&uri.as_bytes()[prefix.len()..]);
    v
}

fn decode_text(p: &[u8]) -> (String, String) {
    let Some((&status, rest)) = p.split_first() else { return (String::new(), String::new()) };
    let lang_len = (status & 0x3F) as usize;
    let lang = String::from_utf8_lossy(rest.get(..lang_len).unwrap_or(&[])).into_owned();
    let body = rest.get(lang_len..).unwrap_or(&[]);
    let text = if status & 0x80 != 0 {
        // UTF-16 (с BOM или big-endian)
        let units: Vec<u16> = body.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        let units = if units.first() == Some(&0xFEFF) { &units[1..] } else { &units[..] };
        String::from_utf16_lossy(units)
    } else {
        String::from_utf8_lossy(body).into_owned()
    };
    (text, lang)
}

fn encode_text(text: &str, lang: &str) -> Vec<u8> {
    let lang = if lang.is_empty() { "en" } else { lang };
    let mut v = vec![lang.len().min(63) as u8];
    v.extend_from_slice(&lang.as_bytes()[..lang.len().min(63)]);
    v.extend_from_slice(text.as_bytes());
    v
}

/// Сырые записи: (TNF, тип, id, данные).
type RawRec = (u8, Vec<u8>, Vec<u8>, Vec<u8>);

fn parse_raw(msg: &[u8]) -> Option<Vec<RawRec>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < msg.len() {
        let h = msg[i];
        i += 1;
        let sr = h & 0x10 != 0;
        let il = h & 0x08 != 0;
        let tnf = h & 0x07;
        let tlen = *msg.get(i)? as usize;
        i += 1;
        let plen = if sr {
            let v = *msg.get(i)? as usize;
            i += 1;
            v
        } else {
            let v = u32::from_be_bytes(msg.get(i..i + 4)?.try_into().ok()?) as usize;
            i += 4;
            v
        };
        let ilen = if il {
            let v = *msg.get(i)? as usize;
            i += 1;
            v
        } else {
            0
        };
        let t = msg.get(i..i + tlen)?.to_vec();
        i += tlen;
        let id = msg.get(i..i + ilen)?.to_vec();
        i += ilen;
        let p = msg.get(i..i + plen)?.to_vec();
        i += plen;
        out.push((tnf, t, id, p));
        if h & 0x40 != 0 {
            break; // ME — последняя
        }
    }
    Some(out)
}

fn encode_raw(recs: &[RawRec]) -> Vec<u8> {
    let mut v = Vec::new();
    for (k, (tnf, t, id, p)) in recs.iter().enumerate() {
        let mut h = tnf & 0x07;
        if k == 0 {
            h |= 0x80; // MB
        }
        if k + 1 == recs.len() {
            h |= 0x40; // ME
        }
        let short = p.len() < 256;
        if short {
            h |= 0x10;
        }
        if !id.is_empty() {
            h |= 0x08;
        }
        v.push(h);
        v.push(t.len() as u8);
        if short {
            v.push(p.len() as u8);
        } else {
            v.extend_from_slice(&(p.len() as u32).to_be_bytes());
        }
        if !id.is_empty() {
            v.push(id.len() as u8);
        }
        v.extend_from_slice(t);
        v.extend_from_slice(id);
        v.extend_from_slice(p);
    }
    v
}

/// Разобрать сообщение NDEF. Пустое сообщение (`D0 00 00`) — пустой список.
pub fn parse(msg: &[u8]) -> Option<Vec<Record>> {
    let raw = parse_raw(msg)?;
    Some(
        raw.into_iter()
            .filter(|(tnf, _, _, _)| *tnf != 0)
            .map(|(tnf, t, id, p)| match (tnf, t.as_slice()) {
                (1, b"U") => Record::Uri { uri: decode_uri(&p) },
                (1, b"T") => {
                    let (text, lang) = decode_text(&p);
                    Record::Text { text, lang }
                }
                (1, b"Sp") => {
                    let inner = parse_raw(&p).unwrap_or_default();
                    let uri = inner.iter().find(|r| r.0 == 1 && r.1 == b"U").map(|r| decode_uri(&r.3)).unwrap_or_default();
                    let title = inner.iter().find(|r| r.0 == 1 && r.1 == b"T").map(|r| decode_text(&r.3).0).unwrap_or_default();
                    Record::SmartPoster { uri, title }
                }
                (2, _) => Record::Mime { mime: String::from_utf8_lossy(&t).into_owned(), data: hex(&p) },
                _ => Record::Raw { tnf, r#type: hex(&t), id: hex(&id), payload: hex(&p) },
            })
            .collect(),
    )
}

/// Собрать сообщение NDEF. Пустой список — пустое сообщение (`D0 00 00`).
pub fn encode(records: &[Record]) -> Vec<u8> {
    if records.is_empty() {
        return vec![0xD0, 0x00, 0x00];
    }
    let raw: Vec<RawRec> = records
        .iter()
        .map(|r| match r {
            Record::Uri { uri } => (1, b"U".to_vec(), vec![], encode_uri(uri)),
            Record::Text { text, lang } => (1, b"T".to_vec(), vec![], encode_text(text, lang)),
            Record::SmartPoster { uri, title } => {
                let mut inner = vec![(1, b"U".to_vec(), vec![], encode_uri(uri))];
                if !title.is_empty() {
                    inner.push((1, b"T".to_vec(), vec![], encode_text(title, "")));
                }
                (1, b"Sp".to_vec(), vec![], encode_raw(&inner))
            }
            Record::Mime { mime, data } => (2, mime.as_bytes().to_vec(), vec![], unhex(data).unwrap_or_default()),
            Record::Raw { tnf, r#type, id, payload } => (*tnf, unhex(r#type).unwrap_or_default(), unhex(id).unwrap_or_default(), unhex(payload).unwrap_or_default()),
        })
        .collect();
    encode_raw(&raw)
}

impl Record {
    /// Текст MIME-записи, если это текст (vCard и т. п.).
    pub fn mime_text(&self) -> Option<String> {
        match self {
            Record::Mime { mime, data } if mime.starts_with("text/") => unhex(data).map(|b| String::from_utf8_lossy(&b).into_owned()),
            _ => None,
        }
    }

    /// Визитка vCard из полей.
    pub fn vcard(name: &str, phone: &str, email: &str) -> Record {
        let mut s = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        s.push_str(&format!("FN:{name}\r\nN:{name};;;;\r\n"));
        if !phone.is_empty() {
            s.push_str(&format!("TEL:{phone}\r\n"));
        }
        if !email.is_empty() {
            s.push_str(&format!("EMAIL:{email}\r\n"));
        }
        s.push_str("END:VCARD\r\n");
        Record::Mime { mime: "text/vcard".into(), data: hex(s.as_bytes()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_roundtrip_and_prefix() {
        let r = vec![Record::Uri { uri: "https://www.example.org/путь".into() }];
        let b = encode(&r);
        // MB|ME|SR, well-known, тип U, префикс 0x02
        assert_eq!(&b[..4], &[0xD1, 0x01, b.len() as u8 - 4, 0x55]);
        assert_eq!(b[4], 0x02);
        assert_eq!(parse(&b).unwrap(), r);
    }

    #[test]
    fn text_smart_poster_vcard() {
        let r = vec![
            Record::Text { text: "Привет".into(), lang: "ru".into() },
            Record::SmartPoster { uri: "tel:+79990000000".into(), title: "Позвонить".into() },
            Record::vcard("Иван", "+7 999", "a@b.c"),
        ];
        let b = encode(&r);
        let back = parse(&b).unwrap();
        assert_eq!(back, r);
        assert!(back[2].mime_text().unwrap().contains("FN:Иван"));
    }

    #[test]
    fn long_record_and_empty() {
        let long = "x".repeat(300);
        let r = vec![Record::Text { text: long.clone(), lang: "en".into() }];
        let b = encode(&r);
        assert_eq!(b[0] & 0x10, 0); // не короткая
        assert_eq!(parse(&b).unwrap(), r);
        assert_eq!(encode(&[]), vec![0xD0, 0, 0]);
        assert_eq!(parse(&[0xD0, 0, 0]).unwrap(), vec![]);
        assert!(parse(&[0xD1, 0x01, 0x10, 0x55]).is_none());
        // UTF-16 с BOM
        let p = [0x82u8, b'e', b'n', 0xFE, 0xFF, 0x00, 0x41];
        assert_eq!(decode_text(&p), ("A".into(), "en".into()));
    }
}
