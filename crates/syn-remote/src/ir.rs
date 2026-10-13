//! ИК-сигналы: формат файлов Flipper (`.ir`, база Flipper-IRDB) и кодирование протоколов в
//! длительности «импульс, пауза, импульс…» (мкс) для передатчика. Тайминги и раскладка битов — как
//! в прошивке Flipper Zero (lib/infrared/encoder_decoder): коды базы записаны именно в её понимании.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// Код кнопки: разобранный протокол или сырые длительности.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Code {
    Parsed { protocol: String, address: u32, command: u32 },
    Raw { frequency: u32, data: Vec<u32> },
}

/// Кнопка пульта.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub name: String,
    #[serde(flatten)]
    pub code: Code,
}

/// Посылка для передатчика: несущая и длительности, начиная с импульса.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub freq: u32,
    pub timings: Vec<u32>,
}

/// Разбор файла Flipper (`Filetype: IR signals file` или `IR library file`): блоки `name:` …,
/// `#` — разделители и комментарии. Неполные и непонятные блоки пропускаются.
pub fn parse(text: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    let mut cur: Vec<(String, String)> = Vec::new();
    let flush = |cur: &mut Vec<(String, String)>, out: &mut Vec<Signal>| {
        if let Some(s) = block(cur) {
            out.push(s);
        }
        cur.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
        if k == "name" {
            flush(&mut cur, &mut out);
        }
        cur.push((k, v));
    }
    flush(&mut cur, &mut out);
    out
}

fn block(kv: &[(String, String)]) -> Option<Signal> {
    let get = |k: &str| kv.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
    let name = get("name")?.to_string();
    let code = match get("type")? {
        "parsed" => Code::Parsed { protocol: get("protocol")?.to_string(), address: hex_le(get("address")?)?, command: hex_le(get("command")?)? },
        "raw" => {
            let data: Vec<u32> = get("data")?.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            if data.is_empty() {
                return None;
            }
            Code::Raw { frequency: get("frequency").and_then(|f| f.parse().ok()).unwrap_or(38000), data }
        }
        _ => return None,
    };
    Some(Signal { name, code })
}

/// «04 FB 00 00» → 0x0000FB04 (байты младшим вперёд, как в памяти Flipper).
fn hex_le(s: &str) -> Option<u32> {
    let mut v = 0u32;
    for (i, b) in s.split_whitespace().take(4).enumerate() {
        v |= (u8::from_str_radix(b, 16).ok()? as u32) << (8 * i);
    }
    Some(v)
}

/// Длительности с подряд идущими отрезками одного рода, сложенными вместе; пауза в начале не нужна.
#[derive(Default)]
struct Wave(Vec<u32>);

impl Wave {
    fn mark(&mut self, us: u32) {
        if self.0.len() % 2 == 1 {
            *self.0.last_mut().unwrap() += us;
        } else {
            self.0.push(us);
        }
    }
    fn space(&mut self, us: u32) {
        if self.0.is_empty() {
            return;
        }
        if self.0.len() % 2 == 0 {
            *self.0.last_mut().unwrap() += us;
        } else {
            self.0.push(us);
        }
    }
    fn len_us(&self) -> u32 {
        self.0.iter().sum()
    }
    /// Биты младшим вперёд из байтов `data`: импульс и пауза своей длины для 1 и 0.
    fn pdwm(&mut self, data: &[u8], bits: usize, t: &Pdwm) {
        for i in 0..bits {
            let one = data[i / 8] >> (i % 8) & 1 == 1;
            self.mark(if one { t.m1 } else { t.m0 });
            self.space(if one { t.s1 } else { t.s0 });
        }
    }
    fn done(mut self, freq: u32) -> Frame {
        if self.0.len() % 2 == 0 {
            self.0.pop();
        }
        Frame { freq, timings: self.0 }
    }
}

struct Pdwm {
    m1: u32,
    s1: u32,
    m0: u32,
    s0: u32,
}

const NEC: Pdwm = Pdwm { m1: 560, s1: 1690, m0: 560, s0: 560 };
const SAMSUNG: Pdwm = Pdwm { m1: 550, s1: 1650, m0: 550, s0: 550 };
const KASEIKYO: Pdwm = Pdwm { m1: 432, s1: 1296, m0: 432, s0: 432 };
const RCA: Pdwm = Pdwm { m1: 500, s1: 2000, m0: 500, s0: 1000 };
const PIONEER: Pdwm = Pdwm { m1: 500, s1: 1500, m0: 500, s0: 500 };
const SIRC: Pdwm = Pdwm { m1: 1200, s1: 600, m0: 600, s0: 600 };

/// Бит переключения RC5/RC6: меняется с каждым нажатием, чтобы приёмник отличал новое нажатие от удержания.
static TOGGLE: AtomicBool = AtomicBool::new(false);

/// Протоколы, которые умеет кодировать `encode`.
#[cfg(test)]
const PROTOCOLS: &[&str] = &["NEC", "NECext", "NEC42", "NEC42ext", "Samsung32", "RC5", "RC5X", "RC6", "SIRC", "SIRC15", "SIRC20", "Kaseikyo", "RCA", "Pioneer"];

/// Сигнал → посылка. `None` — неизвестный протокол.
pub fn encode(code: &Code) -> Option<Frame> {
    let (protocol, a, c) = match code {
        Code::Raw { frequency, data } => {
            // передатчик принимает до 1,1 с; длинные записи (с повторами) обрезаются по паузе
            let mut w = Wave::default();
            let mut total = 0u32;
            for (i, &d) in data.iter().enumerate() {
                total = total.saturating_add(d);
                if total > 1_000_000 {
                    break;
                }
                if i % 2 == 0 {
                    w.mark(d);
                } else {
                    w.space(d);
                }
            }
            return Some(w.done(*frequency));
        }
        Code::Parsed { protocol, address, command } => (protocol.as_str(), *address, *command),
    };
    let mut w = Wave::default();
    let freq = match protocol {
        "NEC" | "NECext" | "NEC42" | "NEC42ext" => {
            let (lo, hi, bits): (u32, u32, usize) = match protocol {
                "NEC" => {
                    let (a, c) = (a & 0xff, c & 0xff);
                    (a | (!a & 0xff) << 8 | c << 16 | (!c & 0xff) << 24, 0, 32)
                }
                "NECext" => (a & 0xffff | (c & 0xffff) << 16, 0, 32),
                "NEC42" => (a & 0x1fff | (!a & 0x1fff) << 13 | (c & 0x3f) << 26, (c & 0xc0) >> 6 | (!c & 0xff) << 2, 42),
                _ => (a & 0x3ff_ffff | (c & 0x3f) << 26, (c & 0xffc0) >> 6, 42),
            };
            let data: Vec<u8> = lo.to_le_bytes().into_iter().chain(hi.to_le_bytes()).collect();
            w.mark(9000);
            w.space(4500);
            w.pdwm(&data, bits, &NEC);
            w.mark(560);
            38000
        }
        "Samsung32" => {
            let (a, c) = (a & 0xff, c & 0xff);
            let v = a | a << 8 | c << 16 | (!c & 0xff) << 24;
            w.mark(4500);
            w.space(4500);
            w.pdwm(&v.to_le_bytes(), 32, &SAMSUNG);
            w.mark(550);
            38000
        }
        "RC5" | "RC5X" => {
            // Манчестер старшим вперёд: 1 — пауза→импульс. Второй старт-бит у RC5X — 0 (7-й бит команды)
            let toggle = TOGGLE.fetch_xor(true, Ordering::Relaxed) as u32;
            let field = (protocol == "RC5") as u32;
            let v = 1 << 13 | field << 12 | toggle << 11 | (a & 0x1f) << 6 | c & 0x3f;
            for i in (0..14).rev() {
                if v >> i & 1 == 1 {
                    w.space(888);
                    w.mark(888);
                } else {
                    w.mark(888);
                    w.space(888);
                }
            }
            36000
        }
        "RC6" => {
            // Режим 0: старт 1, режим 000, бит переключения двойной длины, адрес и команда по 8 бит старшим вперёд;
            // 1 — импульс→пауза
            let toggle = TOGGLE.fetch_xor(true, Ordering::Relaxed) as u32;
            w.mark(2666);
            w.space(889);
            let v: u32 = 1 << 20 | toggle << 16 | (a & 0xff) << 8 | c & 0xff;
            for i in (0..21).rev() {
                let t = if i == 16 { 888 } else { 444 };
                if v >> i & 1 == 1 {
                    w.mark(t);
                    w.space(t);
                } else {
                    w.space(t);
                    w.mark(t);
                }
            }
            36000
        }
        "SIRC" | "SIRC15" | "SIRC20" => {
            let (mask, bits) = match protocol {
                "SIRC" => (0x1f, 12),
                "SIRC15" => (0xff, 15),
                _ => (0x1fff, 20),
            };
            let v = c & 0x7f | (a & mask) << 7;
            // Sony повторяют кадр трижды, период — 45 мс; последняя пауза кадра не передаётся
            let mut frame = Wave::default();
            frame.mark(2400);
            frame.space(600);
            frame.pdwm(&v.to_le_bytes(), bits, &SIRC);
            frame.0.pop();
            let len = frame.len_us();
            for r in 0..3 {
                if r > 0 {
                    w.space(45000u32.saturating_sub(len).max(600));
                }
                for (i, &t) in frame.0.iter().enumerate() {
                    if i % 2 == 0 {
                        w.mark(t);
                    } else {
                        w.space(t);
                    }
                }
            }
            40000
        }
        "Kaseikyo" => {
            let id = (a >> 24 & 3) as u8;
            let vendor = (a >> 8 & 0xffff) as u16;
            let (g1, g2) = ((a >> 4 & 0xf) as u8, (a & 0xf) as u8);
            let mut d = [0u8; 6];
            d[0] = vendor as u8;
            d[1] = (vendor >> 8) as u8;
            let p = d[0] ^ d[1];
            let p = (p & 0xf) ^ (p >> 4);
            d[2] = p & 0xf | g1 << 4;
            d[3] = g2 & 0xf | ((c & 0xf) as u8) << 4;
            d[4] = id << 6 | (c >> 4 & 0x3f) as u8;
            d[5] = d[2] ^ d[3] ^ d[4];
            w.mark(3456);
            w.space(1728);
            w.pdwm(&d, 48, &KASEIKYO);
            w.mark(432);
            38000
        }
        "RCA" => {
            let (a, c) = (a & 0xff, c & 0xff);
            let v = a & 0xf | c << 4 | (!a & 0xf) << 12 | (!c & 0xff) << 16;
            w.mark(4000);
            w.space(4000);
            w.pdwm(&v.to_le_bytes(), 24, &RCA);
            w.mark(500);
            38000
        }
        "Pioneer" => {
            let (a, c) = (a & 0xff, c & 0xff);
            let v = a | (!a & 0xff) << 8 | c << 16 | (!c & 0xff) << 24;
            for r in 0..2 {
                if r > 0 {
                    w.space(26000);
                }
                w.mark(8500);
                w.space(4225);
                w.pdwm(&v.to_le_bytes(), 32, &PIONEER);
                w.mark(500);
            }
            40000
        }
        _ => return None,
    };
    Some(w.done(freq))
}

/// Сводка кода для человека: «NEC 04·08», «raw 38 кГц, 67 отрезков».
pub fn describe(code: &Code) -> String {
    match code {
        Code::Parsed { protocol, address, command } => format!("{protocol} {address:02X}·{command:02X}"),
        Code::Raw { frequency, data } => format!("raw {} kHz · {}", frequency / 1000, data.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(p: &str, a: u32, c: u32) -> Code {
        Code::Parsed { protocol: p.into(), address: a, command: c }
    }

    #[test]
    fn parse_flipper_file() {
        let text = "Filetype: IR signals file\nVersion: 1\n# \nname: Power\ntype: parsed\nprotocol: NEC\naddress: 04 00 00 00\ncommand: 08 00 00 00\n# \nname: Off\ntype: raw\nfrequency: 38000\nduty_cycle: 0.330000\ndata: 8445 4206 542 1566\n#\nname: Broken\ntype: parsed\n";
        let s = parse(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Signal { name: "Power".into(), code: parsed("NEC", 4, 8) });
        assert_eq!(s[1].code, Code::Raw { frequency: 38000, data: vec![8445, 4206, 542, 1566] });
        assert_eq!(hex_le("04 FB 00 00"), Some(0xFB04));
    }

    #[test]
    fn nec_lg_power() {
        // LG: адрес 0x04, команда 0x08 → 0xF708FB04 младшим вперёд
        let f = encode(&parsed("NEC", 4, 8)).unwrap();
        assert_eq!(f.freq, 38000);
        assert_eq!(f.timings.len(), 2 + 64 + 1);
        assert_eq!(&f.timings[..2], &[9000, 4500]);
        let v = 0xF708FB04u32;
        for i in 0..32 {
            assert_eq!(f.timings[2 + 2 * i], 560);
            assert_eq!(f.timings[3 + 2 * i], if v >> i & 1 == 1 { 1690 } else { 560 }, "бит {i}");
        }
        assert_eq!(*f.timings.last().unwrap(), 560);
    }

    #[test]
    fn necext_is_16bit() {
        let f = encode(&parsed("NECext", 0xFB04, 0xF708)).unwrap();
        let g = encode(&parsed("NEC", 4, 8)).unwrap();
        assert_eq!(f, g);
    }

    #[test]
    fn rc5_manchester() {
        TOGGLE.store(false, Ordering::Relaxed);
        let f = encode(&parsed("RC5", 0, 12)).unwrap();
        assert_eq!(f.freq, 36000);
        // начинается импульсом, длительности кратны 888, суммарно 14 бит минус начальная пауза
        assert!(f.timings.iter().all(|t| t % 888 == 0));
        assert_eq!(f.timings.iter().sum::<u32>() % 888, 0);
        assert_eq!(f.timings.len() % 2, 1);
    }

    #[test]
    fn rc6_starts_with_leader() {
        let f = encode(&parsed("RC6", 0, 0x0C)).unwrap();
        assert_eq!(&f.timings[..3], &[2666, 889, 444]);
    }

    #[test]
    fn sirc_three_frames() {
        let f = encode(&parsed("SIRC", 1, 0x15)).unwrap();
        assert_eq!(f.freq, 40000);
        assert_eq!(f.timings.iter().filter(|&&t| t == 2400).count(), 3);
        assert_eq!(f.timings.len() % 2, 1);
    }

    #[test]
    fn all_protocols_encode() {
        for p in PROTOCOLS {
            let f = encode(&parsed(p, 0x12, 0x34)).unwrap_or_else(|| panic!("{p}"));
            assert!(f.timings.len() % 2 == 1 && f.timings.iter().all(|&t| t > 0), "{p}");
            assert!(f.timings.iter().sum::<u32>() < 1_100_000, "{p}");
        }
    }
}
