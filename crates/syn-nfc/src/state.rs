//! Связь со службой `synnfcd` (подписка: события и режим) и сохранённые метки
//! (`~/.local/share/synshell/nfc/tags.json`).

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use syngui::prelude::*;
use synnfc::api::{self, Event, Mode, Request, Status, Subscription, Tag};
use synnfc::ndef::Record;

/// Сохранённая метка.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub name: String,
    pub records: Vec<Record>,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub kind: String,
    /// Когда сохранена, секунды от эпохи.
    #[serde(default)]
    pub time: i64,
}

fn store_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/share"));
    base.join("synshell/nfc/tags.json")
}

pub fn load_saved() -> Vec<Saved> {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_saved(list: &[Saved]) {
    let p = store_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(list).unwrap_or_default()) {
        tracing::warn!("метки не сохранены: {e}");
    }
}

/// Что сейчас делает экран.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Чтение меток.
    Read,
    /// Ждём метку для записи (номер сохранённой или новое содержимое).
    Write(Vec<Record>, String),
    /// Телефон изображает метку.
    Emulate(Vec<Record>, String),
}

#[derive(Clone, Copy)]
pub struct St {
    pub tab: RwSignal<usize>,
    /// Служба доступна; текст ошибки, если нет.
    pub service: RwSignal<std::result::Result<Status, String>>,
    pub last: RwSignal<Option<Tag>>,
    pub saved: RwSignal<Vec<Saved>>,
    pub action: RwSignal<Action>,
    /// Короткое сообщение внизу.
    pub toast: RwSignal<String>,
}

/// Отправитель запросов в службу (поток подписки держит соединение).
static SENDER: Mutex<Option<api::Sender>> = Mutex::new(None);

impl St {
    pub fn new() -> Self {
        St {
            tab: use_signal(0),
            service: use_signal(Err(String::new())),
            last: use_signal(None),
            saved: use_signal(load_saved()),
            action: use_signal(Action::Read),
            toast: use_signal(String::new()),
        }
    }

    /// Режим службы по действию экрана.
    pub fn apply(&self) {
        let req = match self.action.get_untracked() {
            Action::Read => Request::Read,
            Action::Write(r, _) => Request::Write { records: r },
            Action::Emulate(r, _) => Request::Emulate { records: r },
        };
        if let Some(s) = SENDER.lock().unwrap().as_mut() {
            if let Err(e) = s.send(&req) {
                tracing::warn!("NFC: {e:#}");
            }
        }
    }

    pub fn set_action(&self, a: Action) {
        self.action.set(a);
        self.apply();
    }

    pub fn toast(&self, msg: String) {
        self.toast.set(msg.clone());
        let t = self.toast;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            run_on_main_thread(move || {
                if t.get_untracked() == msg {
                    t.set(String::new());
                }
            });
        });
    }

    pub fn save(&self, s: Saved) {
        let mut list = self.saved.get_untracked();
        list.insert(0, s);
        save_saved(&list);
        self.saved.set(list);
    }

    pub fn remove(&self, i: usize) {
        let mut list = self.saved.get_untracked();
        if i < list.len() {
            list.remove(i);
            save_saved(&list);
            self.saved.set(list);
        }
    }
}

/// Подписка на службу в фоне; оборвалась — переподключиться через 3 с.
/// `SYN_NFC_DEMO=1` — без службы: контроллер и прочитанная метка для проверки.
pub fn start(st: St) {
    if std::env::var_os("SYN_NFC_DEMO").is_some() {
        st.service.set(Ok(Status { present: true, device: "/dev/nq-nci".into(), controller: "NCI 2.0 · NXP · 01.C0.EF".into(), mode: Mode::Read, error: String::new() }));
        st.last.set(Some(Tag {
            uid: "04A1B2C3D4E5F6".into(),
            tech: "NFC-A".into(),
            protocol: "T2T".into(),
            kind: "NTAG215".into(),
            atqa: "4400".into(),
            sak: "00".into(),
            ndef: Some(vec![Record::Uri { uri: "https://github.com/VitaminDB/synshell".into() }, Record::Text { text: "Метка у входной двери".into(), lang: "ru".into() }]),
            capacity: 504,
            writable: true,
            ..Default::default()
        }));
        return;
    }
    let svc = st.service;
    let last = st.last;
    let action = st.action;
    let st2 = st;
    std::thread::spawn(move || {
        loop {
            match Subscription::open() {
                Ok(mut sub) => {
                    *SENDER.lock().unwrap() = sub.sender().ok();
                    run_on_main_thread(move || st2.apply());
                    loop {
                        match sub.next_event() {
                            Ok(Event::Status(s)) => run_on_main_thread(move || svc.set(Ok(s))),
                            Ok(Event::Tag(t)) => run_on_main_thread(move || last.set(Some(t))),
                            Ok(Event::Written { uid, ok, error }) => run_on_main_thread(move || {
                                if ok {
                                    st2.toast(t!("Записано на метку {uid}", uid = uid));
                                    action.set(Action::Read);
                                    st2.apply();
                                } else {
                                    st2.toast(t!("Не записано: {error}", error = error));
                                }
                            }),
                            Ok(Event::EmulationRead) => run_on_main_thread(move || st2.toast(t!("Метку прочитали"))),
                            Err(e) => {
                                tracing::info!("synnfcd: {e:#}");
                                break;
                            }
                        }
                    }
                    *SENDER.lock().unwrap() = None;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    run_on_main_thread(move || svc.set(Err(msg)));
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}
