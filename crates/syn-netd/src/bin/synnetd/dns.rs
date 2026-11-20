//! Имена узлов для журнала: ответы DNS от systemd-resolved (varlink
//! `io.systemd.Resolve.Monitor.SubscribeQueryResults`). Адрес из ответа
//! запоминается под именем, которое спрашивали (`www.youtube.com`, а не
//! конец цепочки CNAME). Программы с DNS поверх HTTPS (браузеры) мимо
//! resolved — у их соединений только адрес.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::IpAddr;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MONITOR: &str = "/run/systemd/resolve/io.systemd.Resolve.Monitor";
const CAP: usize = 32768;

#[derive(Default)]
pub struct Names {
    map: HashMap<IpAddr, String>,
    order: VecDeque<IpAddr>,
    /// Подписка resolved работает.
    pub live: bool,
}

impl Names {
    pub fn get(&self, ip: &IpAddr) -> Option<String> {
        self.map.get(ip).cloned()
    }

    fn put(&mut self, ip: IpAddr, name: String) {
        if self.map.insert(ip, name).is_none() {
            self.order.push_back(ip);
            while self.order.len() > CAP {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }
}

fn parse(v: &serde_json::Value, names: &Mutex<Names>) {
    let p = v.get("parameters").unwrap_or(v);
    let question = p.get("question").and_then(|q| q.as_array()).and_then(|q| q.first()).and_then(|q| q.get("name")).and_then(|n| n.as_str());
    let Some(answers) = p.get("answer").and_then(|a| a.as_array()) else { return };
    let mut n = names.lock().unwrap();
    for a in answers {
        let rr = a.get("rr").unwrap_or(a);
        let Some(addr) = rr.get("address").and_then(|x| x.as_array()) else { continue };
        let bytes: Vec<u8> = addr.iter().filter_map(|b| b.as_u64()).map(|b| b as u8).collect();
        let ip = match bytes.len() {
            4 => IpAddr::from([bytes[0], bytes[1], bytes[2], bytes[3]]),
            16 => IpAddr::from(<[u8; 16]>::try_from(bytes.as_slice()).unwrap()),
            _ => continue,
        };
        let name = question.or_else(|| rr.get("key").and_then(|k| k.get("name")).and_then(|n| n.as_str()));
        if let Some(name) = name {
            n.put(ip, name.trim_end_matches('.').to_string());
        }
    }
}

/// Подписаться на ответы resolved (поток; переподключается).
pub fn start(names: Arc<Mutex<Names>>) {
    let _ = std::thread::Builder::new().name("dns".into()).spawn(move || loop {
        if let Ok(mut s) = UnixStream::connect(MONITOR) {
            let req = b"{\"method\":\"io.systemd.Resolve.Monitor.SubscribeQueryResults\",\"more\":true}\0";
            if s.write_all(req).is_ok() {
                names.lock().unwrap().live = true;
                let mut r = BufReader::new(s);
                let mut buf = Vec::new();
                while r.read_until(0, &mut buf).is_ok_and(|n| n > 0) {
                    if buf.last() == Some(&0) {
                        buf.pop();
                    }
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
                        if v.get("error").is_some() {
                            tracing::debug!("resolved: {v}");
                            break;
                        }
                        parse(&v, &names);
                    }
                    buf.clear();
                }
                names.lock().unwrap().live = false;
            }
        }
        std::thread::sleep(Duration::from_secs(30));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer() {
        let n = Mutex::new(Names::default());
        let v: serde_json::Value = serde_json::from_str(r#"{"parameters":{"state":"success","question":[{"class":1,"type":1,"name":"www.youtube.com"}],"answer":[{"rr":{"key":{"class":1,"type":5,"name":"www.youtube.com"},"name":"youtube-ui.l.google.com"}},{"rr":{"key":{"class":1,"type":1,"name":"youtube-ui.l.google.com"},"address":[142,250,1,2]}}]},"continues":true}"#).unwrap();
        parse(&v, &n);
        assert_eq!(n.lock().unwrap().get(&"142.250.1.2".parse().unwrap()).as_deref(), Some("www.youtube.com"));
    }
}
