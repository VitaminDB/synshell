//! Прямое перо на E Ink: холст сообщает композитору (`Request::InkCanvas`) свою область, толщину и цвет
//! пера — synwm рисует штрих сам из событий пера, не дожидаясь кадра приложения (как сток ONYX).
//! Своё приложение рисует штрих как обычно; кадр приложения заменяет нарисованное композитором.

use std::collections::HashMap;
use std::sync::{mpsc, Mutex, OnceLock};

use synshell_common::ipc::{Client, Request};

/// Холст одного элемента: область окна (логические x, y, w, h), толщина при нажатии 0 и 1, цвет.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub rect: [f64; 4],
    pub width: [f64; 2],
    pub color: [u8; 4],
}

struct Shared {
    areas: HashMap<u64, Area>,
    sent: Option<Request>,
    tx: Option<mpsc::Sender<Request>>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { areas: HashMap::new(), sent: None, tx: None }))
}

/// Включено только на E Ink (там путь через кадр приложения медленный).
pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(synshell_common::display::is_eink)
}

/// Холст элемента `key` (`None` — пером сейчас не рисуют: другой инструмент, только просмотр).
pub fn set(key: u64, area: Option<Area>) {
    if !enabled() {
        return;
    }
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    let changed = match area {
        Some(a) => s.areas.insert(key, a) != Some(a),
        None => s.areas.remove(&key).is_some(),
    };
    if !changed {
        return;
    }
    // Толщина и цвет — от последнего холста (обычно он один); области — все.
    let any = s.areas.values().next().copied();
    let req = Request::InkCanvas {
        app_id: crate::app::APP_ID.into(),
        rects: s.areas.values().map(|a| a.rect).collect(),
        width: any.map(|a| a.width).unwrap_or_default(),
        color: any.map(|a| a.color).unwrap_or_default(),
    };
    if s.sent.as_ref() == Some(&req) {
        return;
    }
    s.sent = Some(req.clone());
    if s.tx.is_none() {
        let (tx, rx) = mpsc::channel::<Request>();
        // Своим потоком: интерфейс не ждёт композитор.
        let _ = std::thread::Builder::new().name("notes-ink".into()).spawn(move || {
            let mut client: Option<Client> = None;
            while let Ok(mut req) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    req = next;
                }
                for _ in 0..2 {
                    if client.is_none() {
                        client = Client::connect().ok();
                    }
                    match client.as_mut().map(|c| c.request(&req)) {
                        Some(Ok(_)) => break,
                        _ => client = None,
                    }
                }
            }
        });
        s.tx = Some(tx);
    }
    if let Some(tx) = &s.tx {
        let _ = tx.send(req);
    }
}
