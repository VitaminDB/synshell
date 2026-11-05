//! Эмуляция метки NFC Forum Type 4 (HCE): телефон отвечает считывателю как метка с
//! приложением NDEF (`D2760000850101`), файлами CC (`E103`) и NDEF (`E104`), только для
//! чтения. Так «воспроизводится» сохранённое содержимое (ссылка, текст, визитка) —
//! идентификатор чужой метки и карты MIFARE Classic повторить нельзя.

use crate::ndef::{self, Record};
use crate::tag::NDEF_AID;

const NDEF_FILE: [u8; 2] = [0xE1, 0x04];
const CC_FILE: [u8; 2] = [0xE1, 0x03];
const SW_OK: [u8; 2] = [0x90, 0x00];
const SW_NOT_FOUND: [u8; 2] = [0x6A, 0x82];
const SW_DENIED: [u8; 2] = [0x69, 0x82];
const SW_WRONG_P: [u8; 2] = [0x6B, 0x00];
const SW_INS: [u8; 2] = [0x6D, 0x00];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sel {
    None,
    App,
    Cc,
    Ndef,
}

pub struct T4Emulator {
    cc: Vec<u8>,
    ndef: Vec<u8>,
    sel: Sel,
    /// Считыватель дочитал сообщение (для уведомления «метку прочитали»).
    pub read_done: bool,
}

impl T4Emulator {
    pub fn new(records: &[Record]) -> Self {
        let msg = ndef::encode(records);
        let mut file = (msg.len() as u16).to_be_bytes().to_vec();
        file.extend_from_slice(&msg);
        let max = (file.len() as u16).max(0x0100).to_be_bytes();
        // CCLEN 15, версия 2.0, MLe 0x003B, MLc 0x0034, TLV файла NDEF: id, размер, чтение 00, запись FF (нельзя)
        let cc = vec![0x00, 0x0F, 0x20, 0x00, 0x3B, 0x00, 0x34, 0x04, 0x06, NDEF_FILE[0], NDEF_FILE[1], max[0], max[1], 0x00, 0xFF];
        T4Emulator { cc, ndef: file, sel: Sel::None, read_done: false }
    }

    /// Ответ на C-APDU считывателя.
    pub fn apdu(&mut self, c: &[u8]) -> Vec<u8> {
        if c.len() < 4 {
            return SW_INS.to_vec();
        }
        let (ins, p1, p2) = (c[1], c[2], c[3]);
        match ins {
            // SELECT
            0xA4 => {
                let lc = *c.get(4).unwrap_or(&0) as usize;
                let data = c.get(5..5 + lc).unwrap_or(&[]);
                if p1 == 0x04 {
                    if data == NDEF_AID {
                        self.sel = Sel::App;
                        return SW_OK.to_vec();
                    }
                    self.sel = Sel::None;
                    return SW_NOT_FOUND.to_vec();
                }
                if p1 == 0x00 && self.sel != Sel::None {
                    if data == CC_FILE {
                        self.sel = Sel::Cc;
                        return SW_OK.to_vec();
                    }
                    if data == NDEF_FILE {
                        self.sel = Sel::Ndef;
                        return SW_OK.to_vec();
                    }
                }
                SW_NOT_FOUND.to_vec()
            }
            // READ BINARY
            0xB0 => {
                let file = match self.sel {
                    Sel::Cc => &self.cc,
                    Sel::Ndef => &self.ndef,
                    _ => return SW_NOT_FOUND.to_vec(),
                };
                let off = u16::from_be_bytes([p1 & 0x7F, p2]) as usize;
                let le = match c.get(4) {
                    Some(0) | None => 256,
                    Some(n) => *n as usize,
                };
                if off > file.len() {
                    return SW_WRONG_P.to_vec();
                }
                let end = (off + le).min(file.len());
                let mut r = file[off..end].to_vec();
                if self.sel == Sel::Ndef && end >= self.ndef.len() {
                    self.read_done = true;
                }
                r.extend_from_slice(&SW_OK);
                r
            }
            // UPDATE BINARY — метка только для чтения
            0xD6 => SW_DENIED.to_vec(),
            _ => SW_INS.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apdu_flow() {
        let mut e = T4Emulator::new(&[Record::Uri { uri: "https://a.b".into() }]);
        assert_eq!(e.apdu(&[0x00, 0xB0, 0, 0, 2]), SW_NOT_FOUND);
        assert_eq!(e.apdu(&[0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01, 0x00]), SW_OK);
        assert_eq!(e.apdu(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x03]), SW_OK);
        let cc = e.apdu(&[0x00, 0xB0, 0, 0, 0x0F]);
        assert_eq!(cc.len(), 17);
        assert_eq!(e.apdu(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x04]), SW_OK);
        let nlen = e.apdu(&[0x00, 0xB0, 0, 0, 2]);
        let len = u16::from_be_bytes([nlen[0], nlen[1]]) as usize;
        let body = e.apdu(&[0x00, 0xB0, 0, 2, len as u8]);
        assert_eq!(&body[len..], &SW_OK);
        assert!(e.read_done);
        assert_eq!(e.apdu(&[0x00, 0xD6, 0, 0, 1, 0]), SW_DENIED);
        assert_eq!(e.apdu(&[0x00, 0xA4, 0x04, 0x00, 0x02, 0xA0, 0x00]), SW_NOT_FOUND);
    }
}
