//! Хранилище демона: SMS, журнал звонков, части длинных SMS в ожидании склейки, режим радио.
//! Один JSON-файл [`PATH`] (только root), запись через временный файл.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::api::{CallRecord, Sms};

pub const PATH: &str = "/var/lib/synmodem/state.json";

/// Часть длинного SMS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub number: String,
    pub reference: u16,
    pub total: u8,
    pub seq: u8,
    pub text: String,
    pub time: i64,
    /// Когда пришла (для сборки неполных по таймауту).
    pub received: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub sms: Vec<Sms>,
    #[serde(default)]
    pub calls: Vec<CallRecord>,
    #[serde(default)]
    pub parts: Vec<Part>,
    #[serde(default)]
    pub next_id: u64,
    /// Пользователь выключил радио (режим полёта).
    #[serde(default)]
    pub radio_off: bool,
    /// Мобильная передача данных включена (по умолчанию выключена: роуминг стоит денег).
    #[serde(default)]
    pub data_on: bool,
    /// Передача данных в роуминге разрешена.
    #[serde(default)]
    pub data_roaming: bool,
    /// Скрытие своего номера: «network», «hide», «show».
    #[serde(default)]
    pub clir: String,
    #[serde(default)]
    pub usage: crate::api::Usage,
}

impl Store {
    /// Учесть прирост трафика (`rx`, `tx` — байты с прошлого замера) в месяце и в общем счётчике.
    pub fn add_usage(&mut self, rx: u64, tx: u64, now: i64) {
        let l = crate::time::local(now);
        let month = format!("{:04}-{:02}", l.year, l.month);
        let u = &mut self.usage;
        if u.month != month {
            u.month = month;
            u.rx = 0;
            u.tx = 0;
        }
        if u.since == 0 {
            u.since = now;
        }
        u.rx += rx;
        u.tx += tx;
        u.total_rx += rx;
        u.total_tx += tx;
    }
}

/// Через сколько недостающие части перестают ждать (сообщение показывается как есть).
const PART_TIMEOUT: i64 = 24 * 3600;

impl Store {
    pub fn load() -> Self {
        std::fs::read_to_string(PATH).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Path::new(PATH);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("tmp");
        let Ok(data) = serde_json::to_vec(self) else { return };
        use std::os::unix::fs::OpenOptionsExt;
        let ok = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &data).and_then(|_| f.sync_all()));
        if ok.is_ok() {
            let _ = std::fs::rename(&tmp, p);
        } else {
            tracing::warn!("не записать {PATH}");
        }
    }

    pub fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Положить часть; если сообщение собрано — вернуть (номер, текст, время первой части).
    pub fn add_part(&mut self, part: Part) -> Option<(String, String, i64)> {
        if !self.parts.iter().any(|p| p.number == part.number && p.reference == part.reference && p.seq == part.seq) {
            self.parts.push(part.clone());
        }
        let same = |p: &Part| p.number == part.number && p.reference == part.reference && p.total == part.total;
        let have: Vec<&Part> = self.parts.iter().filter(|p| same(p)).collect();
        if have.len() < part.total as usize {
            return None;
        }
        Some(self.take_parts(&part.number, part.reference, part.total))
    }

    fn take_parts(&mut self, number: &str, reference: u16, total: u8) -> (String, String, i64) {
        let (mut mine, rest): (Vec<Part>, Vec<Part>) = std::mem::take(&mut self.parts)
            .into_iter()
            .partition(|p| p.number == number && p.reference == reference && p.total == total);
        self.parts = rest;
        mine.sort_by_key(|p| p.seq);
        let time = mine.first().map(|p| p.time).unwrap_or_default();
        // Пропущенные части отмечаются многоточием
        let mut text = String::new();
        let mut expect = 1;
        for p in &mine {
            if p.seq != expect {
                text.push('…');
            }
            text.push_str(&p.text);
            expect = p.seq + 1;
        }
        if expect <= total {
            text.push('…');
        }
        (number.to_string(), text, time)
    }

    /// Неполные сообщения, ждущие дольше таймаута.
    pub fn expired_parts(&mut self, now: i64) -> Vec<(String, String, i64)> {
        let mut keys: Vec<(String, u16, u8)> = self
            .parts
            .iter()
            .filter(|p| now - p.received > PART_TIMEOUT)
            .map(|p| (p.number.clone(), p.reference, p.total))
            .collect();
        keys.dedup();
        keys.into_iter().map(|(n, r, t)| self.take_parts(&n, r, t)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(seq: u8, text: &str) -> Part {
        Part { number: "+7701".into(), reference: 5, total: 3, seq, text: text.into(), time: 100 + seq as i64, received: 0 }
    }

    #[test]
    fn parts_join_in_order() {
        let mut s = Store::default();
        assert!(s.add_part(part(2, "b")).is_none());
        assert!(s.add_part(part(2, "b")).is_none(), "повтор части не считается");
        assert!(s.add_part(part(1, "a")).is_none());
        let (n, t, time) = s.add_part(part(3, "c")).unwrap();
        assert_eq!((n.as_str(), t.as_str(), time), ("+7701", "abc", 101));
        assert!(s.parts.is_empty());
    }

    #[test]
    fn expired_parts_marked() {
        let mut s = Store::default();
        s.add_part(part(1, "a"));
        s.add_part(part(3, "c"));
        assert!(s.expired_parts(10).is_empty());
        let got = s.expired_parts(PART_TIMEOUT + 10);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, "a…c");
    }
}
