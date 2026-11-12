//! Состояние «Телефона» (живёт весь процесс: окно при показе строится заново) и работа с модемом, контактами,
//! записями. Жизненный цикл звонка: отвод звука на время звонка, автозапись с ответа, показ окна на входящий
//! (и прятание обратно, если окно было спрятано), остановка записи в конце.

use std::path::PathBuf;
use std::time::Duration;

use syn_modem::api::{self, Call, CallRecord, CallServices, CallState, Event, Request, Status};
use syn_modem::time as tm;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synshell_common::contacts::{self, Contact, Phone};

use crate::prefs::{Prefs, RecordMode};
use crate::voice;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Dial,
    Log,
    Contacts,
}

impl Tab {
    pub fn from_index(i: u8) -> Tab {
        match i {
            1 => Tab::Log,
            2 => Tab::Contacts,
            _ => Tab::Dial,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Tab::Dial => 0,
            Tab::Log => 1,
            Tab::Contacts => 2,
        }
    }
}

/// Страница поверх вкладок.
#[derive(Clone, PartialEq, Debug)]
pub enum Page {
    None,
    Settings,
    Contact(String),
    /// Редактор: контакт (`None` — новый) и номер для нового.
    Edit(Option<String>, String),
    Recordings,
}

impl Page {
    pub fn index(&self) -> usize {
        match self {
            Page::None => 0,
            Page::Settings => 1,
            Page::Contact(_) => 2,
            Page::Edit(..) => 3,
            Page::Recordings => 4,
        }
    }
}

/// Фото в редакторе контакта.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum PhotoEdit {
    #[default]
    Keep,
    /// Новый JPEG (уже квадрат 512).
    Set(Vec<u8>),
    Remove,
}

/// Черновик редактора контакта.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Draft {
    pub name: String,
    pub phones: Vec<Phone>,
    pub email: String,
    pub note: String,
    pub favorite: bool,
    pub photo: PhotoEdit,
}

/// Запись разговора на диске.
#[derive(Clone, PartialEq, Debug)]
pub struct RecFile {
    pub path: PathBuf,
    pub title: String,
    pub size: u64,
    pub time: i64,
}

/// Сеанс USSD (или код телефона вроде `*#06#`): окно поверх приложения.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Ussd {
    /// Набранный код (заголовок окна).
    pub code: String,
    /// Последний ответ сети.
    pub text: String,
    /// Ждём ответа сети.
    pub waiting: bool,
    /// Сеть ждёт ответа пользователя (меню).
    pub reply: bool,
    /// Сеанс закончен (сеть закрыла, отказ) — только «Закрыть».
    pub done: bool,
}

#[derive(Clone, Copy)]
pub struct St {
    pub tab: RwSignal<Tab>,
    pub tab_idx: RwSignal<usize>,
    pub page: RwSignal<Page>,
    pub page_idx: RwSignal<usize>,
    pub number: RwSignal<String>,
    pub log: RwSignal<Vec<CallRecord>>,
    pub status: RwSignal<Option<Status>>,
    pub toast: RwSignal<String>,
    /// Секунды UNIX, раз в секунду во время звонка.
    pub tick: RwSignal<i64>,
    pub dtmf_open: RwSignal<bool>,
    pub speaker: RwSignal<bool>,
    pub muted: RwSignal<bool>,
    /// 0 — обычный экран, 1 — звонок.
    pub mode: RwSignal<usize>,
    /// Экран звонка свёрнут (открыли контакт во время разговора) — вкладки с плашкой «вернуться».
    pub call_hidden: RwSignal<bool>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub contacts: RwSignal<Vec<Contact>>,
    pub prefs: RwSignal<Prefs>,
    pub query: RwSignal<String>,
    pub missed_only: RwSignal<bool>,
    /// Идёт запись: (путь, начало).
    pub rec: RwSignal<Option<(PathBuf, i64)>>,
    pub services: RwSignal<Option<CallServices>>,
    pub draft: RwSignal<Draft>,
    /// Перестроить список номеров редактора (добавили/убрали поле).
    pub draft_rev: RwSignal<u64>,
    /// Открытое окно USSD.
    pub ussd: RwSignal<Option<Ussd>>,
    /// Ответ в меню USSD (поле ввода; окно за ним не следит — иначе поле пересоздавалось бы на каждой букве).
    pub ussd_input: RwSignal<String>,
    pub recordings: RwSignal<Vec<RecFile>>,
    /// Окно показано входящим звонком (спрятать снова, когда звонок кончится).
    pub auto_shown: RwSignal<bool>,
    /// Снимок окна (`--screenshot`): без модема, потоков и окна.
    pub demo: bool,
}

impl St {
    pub fn new(prefs: Prefs, demo: bool) -> St {
        let tab = Tab::from_index(prefs.start_tab);
        St {
            tab: use_signal(tab),
            tab_idx: use_signal(tab.index()),
            page: use_signal(Page::None),
            page_idx: use_signal(0),
            number: use_signal(String::new()),
            log: use_signal(Vec::new()),
            status: use_signal(None),
            toast: use_signal(String::new()),
            tick: use_signal(tm::now()),
            dtmf_open: use_signal(false),
            speaker: use_signal(false),
            muted: use_signal(false),
            mode: use_signal(0),
            call_hidden: use_signal(false),
            window: use_signal(syngui::window::WindowState::default()),
            contacts: use_signal(Vec::new()),
            prefs: use_signal(prefs),
            query: use_signal(String::new()),
            missed_only: use_signal(false),
            rec: use_signal(None),
            services: use_signal(None),
            draft: use_signal(Draft::default()),
            draft_rev: use_signal(0),
            ussd: use_signal(None),
            ussd_input: use_signal(String::new()),
            recordings: use_signal(Vec::new()),
            auto_shown: use_signal(false),
            demo,
        }
    }

    pub fn toast(&self, s: impl Into<String>) {
        self.toast.set(s.into());
    }

    pub fn open(&self, p: Page) {
        self.page.set(p);
    }

    pub fn set_prefs(&self, f: impl FnOnce(&mut Prefs)) {
        let mut p = self.prefs.get_untracked();
        f(&mut p);
        if !self.demo {
            p.save();
        }
        self.prefs.set(p);
    }

    /// Контакт для номера (из загруженного списка).
    pub fn contact(&self, number: &str) -> Option<Contact> {
        if contacts::digits(number).len() < 3 {
            return None;
        }
        self.contacts.get().into_iter().find(|c| c.has_number(number))
    }

    pub fn contact_untracked(&self, number: &str) -> Option<Contact> {
        if contacts::digits(number).len() < 3 {
            return None;
        }
        self.contacts.get_untracked().into_iter().find(|c| c.has_number(number))
    }

    /// Звонок, который показывать главным: входящий/ожидающий, иначе первый.
    pub fn main_call(&self) -> Option<Call> {
        let s = self.status.get()?;
        s.calls.iter().find(|c| matches!(c.state, CallState::Incoming | CallState::Waiting)).or(s.calls.first()).cloned()
    }
}

pub fn effects(st: St) {
    create_effect(move || {
        let calls = st.status.get().is_some_and(|s| !s.calls.is_empty());
        st.mode.set(usize::from(calls && !st.call_hidden.get()));
        if !calls {
            st.call_hidden.set(false);
            st.speaker.set(false);
            st.muted.set(false);
            st.dtmf_open.set(false);
        }
    });
    create_effect(move || st.tab_idx.set(st.tab.get().index()));
    create_effect(move || st.page_idx.set(st.page.get().index()));
    // журнал открыт — пропущенные просмотрены
    create_effect(move || {
        if !st.demo && st.tab.get() == Tab::Log && st.log.get().iter().any(|c| c.new) {
            std::thread::spawn(|| {
                let _ = api::request(&Request::CallLogSeen);
            });
        }
    });
    // всплывающее сообщение — 4 с
    create_effect(move || {
        let t = st.toast.get();
        if !t.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                run_on_main_thread(move || {
                    if st.toast.get_untracked() == t {
                        st.toast.set(String::new());
                    }
                });
            });
        }
    });
    // настройки открыты — услуги оператора
    create_effect(move || {
        if st.page.get() == Page::Settings && !st.demo {
            std::thread::spawn(move || {
                if let Ok(api::Response::CallServices { services }) = api::request(&Request::CallServices) {
                    run_on_main_thread(move || st.services.set(Some(services)));
                }
            });
        }
        if st.page.get() == Page::Recordings {
            reload_recordings(st);
        }
    });
}

pub fn reload_contacts(st: St) {
    if st.demo {
        return;
    }
    std::thread::spawn(move || {
        let c = contacts::list();
        run_on_main_thread(move || st.contacts.set(c));
    });
}

pub fn reload_log(st: St) {
    std::thread::spawn(move || {
        if let Ok(l) = api::call_log() {
            run_on_main_thread(move || st.log.set(l));
        }
    });
}

pub fn reload_recordings(st: St) {
    if st.demo {
        return;
    }
    let dir = st.prefs.get_untracked().record_dir();
    std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !matches!(p.extension().and_then(|x| x.to_str()), Some("ogg" | "opus" | "wav" | "m4a" | "flac")) {
                    continue;
                }
                let Ok(md) = e.metadata() else { continue };
                let time = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                v.push(RecFile { path: p, title, size: md.len(), time });
            }
        }
        v.sort_by(|a, b| b.time.cmp(&a.time));
        run_on_main_thread(move || st.recordings.set(v));
    });
}

/// Подписка на события модема (с переподключением).
pub fn watch(st: St) {
    std::thread::spawn(move || loop {
        let r = api::subscribe(|ev| {
            match ev {
                Event::Status { status } => run_on_main_thread(move || on_status(st, status)),
                Event::CallLog => run_on_main_thread(move || reload_log(st)),
                Event::Ussd { text, reply, done, network } => run_on_main_thread(move || on_ussd(st, text, reply, done, network)),
                _ => {}
            }
            true
        });
        let msg = match r {
            Ok(()) => t!("synmodemd остановлен").to_string(),
            Err(e) => format!("{e:#}"),
        };
        run_on_main_thread(move || {
            if st.status.get_untracked().is_some() {
                st.toast(t!("Модем недоступен: {msg}", msg = msg));
            }
            st.status.set(None);
        });
        std::thread::sleep(Duration::from_secs(5));
    });
}

fn on_status(st: St, status: Status) {
    let first = st.status.get_untracked().is_none();
    let had_calls = st.status.get_untracked().is_some_and(|s| !s.calls.is_empty());
    let ringing = status.calls.iter().any(|c| matches!(c.state, CallState::Incoming | CallState::Waiting));
    let was_ringing = st.status.get_untracked().is_some_and(|s| s.calls.iter().any(|c| matches!(c.state, CallState::Incoming | CallState::Waiting)));
    let has_calls = !status.calls.is_empty();
    let active = status.calls.iter().any(|c| c.state == CallState::Active);
    st.status.set(Some(status));
    if first {
        reload_log(st);
    }
    voice::set_call(has_calls);
    // входящий — окно на экран (было спрятано — спрятать снова после звонка)
    if ringing && !was_ringing {
        if !syngui::app::main_window_visible() {
            st.auto_shown.set(true);
        }
        st.page.set(Page::None);
        st.call_hidden.set(false);
        syngui::app::show_main_window();
    }
    if active && st.prefs.get_untracked().record == RecordMode::Auto && st.rec.get_untracked().is_none() && voice::available() {
        start_recording(st);
    }
    if had_calls && !has_calls {
        stop_recording(st);
        if st.auto_shown.get_untracked() {
            st.auto_shown.set(false);
            syngui::app::hide_main_window();
        }
    }
}

pub fn ticker(st: St) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        run_on_main_thread(move || {
            if st.status.get_untracked().is_some_and(|s| !s.calls.is_empty()) {
                st.tick.set(tm::now());
            }
        });
    });
}

pub fn act(st: St, req: Request) {
    if st.demo {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = api::request(&req) {
            let m = format!("{e:#}");
            run_on_main_thread(move || st.toast(m));
        }
    });
}

pub fn dial(st: St, number: String) {
    let number = number.trim().to_string();
    if number.is_empty() {
        return;
    }
    if is_ussd(&number) {
        ussd_start(st, number);
        return;
    }
    act(st, Request::Dial { number });
}

/// Код, а не номер: `*100#`, `#102#`, `*111*1#` — USSD (и коды телефона вроде `*#06#`).
pub fn is_ussd(n: &str) -> bool {
    let n = n.trim();
    n.len() >= 3 && n.ends_with('#') && (n.starts_with('*') || n.starts_with('#')) && n.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#' || c == '+')
}

/// Запрос USSD; `*#06#` — IMEI сразу, без сети.
pub fn ussd_start(st: St, code: String) {
    if code == "*#06#" {
        let imei = st.status.get_untracked().map(|s| s.imei).filter(|i| !i.is_empty()).unwrap_or_else(|| t!("неизвестен").into());
        st.ussd.set(Some(Ussd { code, text: format!("IMEI\n{imei}"), done: true, ..Default::default() }));
        return;
    }
    st.ussd.set(Some(Ussd { code: code.clone(), waiting: true, ..Default::default() }));
    if st.demo {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = api::request(&Request::Ussd { code }) {
            let m = format!("{e:#}");
            run_on_main_thread(move || on_ussd(st, m, false, true, false));
        }
    });
}

/// Ответ в меню USSD.
pub fn ussd_reply(st: St) {
    let Some(u) = st.ussd.get_untracked() else { return };
    let text = st.ussd_input.get_untracked().trim().to_string();
    if text.is_empty() || !u.reply {
        return;
    }
    st.ussd_input.set(String::new());
    hide_keyboard();
    st.ussd.set(Some(Ussd { waiting: true, reply: false, ..u }));
    std::thread::spawn(move || {
        if let Err(e) = api::request(&Request::UssdReply { text }) {
            let m = format!("{e:#}");
            run_on_main_thread(move || on_ussd(st, m, false, true, false));
        }
    });
}

/// Закрыть окно; сеанс ещё идёт — отменить его в сети.
pub fn ussd_close(st: St) {
    let open = st.ussd.get_untracked().is_some_and(|u| !u.done);
    st.ussd.set(None);
    st.ussd_input.set(String::new());
    hide_keyboard();
    if open && !st.demo {
        std::thread::spawn(|| {
            let _ = api::request(&Request::UssdCancel);
        });
    }
}

/// Экранную клавиатуру ответа USSD (если её открывали) — убрать.
fn hide_keyboard() {
    if let Ok(mut c) = std::process::Command::new("synkeyboard").arg("hide").spawn() {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/// Событие USSD от демона: ответ на наш запрос или сообщение, которое сеть прислала сама.
fn on_ussd(st: St, text: String, reply: bool, done: bool, network: bool) {
    match st.ussd.get_untracked() {
        Some(mut u) => {
            if !text.is_empty() {
                u.text = text;
            } else if done && u.waiting {
                u.text = t!("Сеанс завершён").into();
            }
            u.waiting = false;
            u.reply = reply && !done;
            u.done = done || (!reply && !u.text.is_empty());
            st.ussd.set(Some(u));
        }
        // сеть прислала сама — показать; ответы на чужие запросы (synmodem ussd) — не наше окно
        None if network && !text.is_empty() => {
            st.ussd.set(Some(Ussd { text, reply: reply && !done, done: done || !reply, ..Default::default() }));
            if !syngui::app::main_window_visible() {
                syngui::app::show_main_window();
            }
        }
        None => {}
    }
}

/// Файл записи: «Имя 2026-11-11 10-20-05.ogg».
fn record_path(st: St) -> PathBuf {
    let who = st
        .main_call()
        .map(|c| st.contact_untracked(&c.number).map(|x| x.title()).unwrap_or(if c.number.is_empty() { t!("Скрытый номер").into() } else { c.number.clone() }))
        .unwrap_or_default();
    let who: String = who.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    let now = tm::now();
    let l = tm::local(now);
    let name = format!("{} {:04}-{:02}-{:02} {:02}-{:02}-{:02}", who.trim(), l.year, l.month, l.day, l.hour, l.minute, now.rem_euclid(60));
    st.prefs.get_untracked().record_dir().join(format!("{}.ogg", name.trim()))
}

pub fn start_recording(st: St) {
    if st.rec.get_untracked().is_some() {
        return;
    }
    if !voice::available() {
        st.toast(t!("Запись разговоров на этом устройстве недоступна"));
        return;
    }
    let path = record_path(st);
    match voice::record_start(&path) {
        Ok(()) => st.rec.set(Some((path, tm::now()))),
        Err(e) => st.toast(t!("Запись не началась: {e}", e = e)),
    }
}

pub fn stop_recording(st: St) {
    if st.rec.get_untracked().take().is_none() && voice::recording().is_none() {
        return;
    }
    st.rec.set(None);
    std::thread::spawn(move || {
        let saved = voice::record_stop();
        run_on_main_thread(move || {
            if let Some(p) = saved {
                st.toast(t!("Запись сохранена: {name}", name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                reload_recordings(st);
            }
        });
    });
}

/// Команда голосу разговора в palaudiod (сокет управления); в фоне.
pub fn voice_cmd(st: St, cmd: String) {
    if st.demo {
        return;
    }
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let r = (|| -> std::io::Result<String> {
            let mut s = std::os::unix::net::UnixStream::connect("/run/palaudio/control")?;
            s.set_read_timeout(Some(Duration::from_secs(5)))?;
            s.write_all(format!("{cmd}\n").as_bytes())?;
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line)?;
            Ok(line)
        })();
        let err = match r {
            Ok(l) if l.contains("\"error\"") => Some(l),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(e) = err {
            run_on_main_thread(move || st.toast(t!("Звук разговора: {error}", error = e.trim())));
        }
    });
}

pub fn dtmf(st: St, c: char) {
    let Some(id) = st.status.get_untracked().and_then(|s| s.calls.iter().find(|c| c.state == CallState::Active).map(|c| c.id)) else {
        return;
    };
    act(st, Request::Dtmf { id, digit: c });
}

/// Подсказка состояния звонка.
pub fn state_text(c: &Call, now: i64) -> String {
    match c.state {
        CallState::Dialing => t!("Вызов…").into(),
        CallState::Alerting => t!("Гудки…").into(),
        CallState::Incoming => t!("Входящий вызов").into(),
        CallState::Waiting => t!("Второй вызов").into(),
        CallState::Active => tm::duration(c.answered_at.map(|a| (now - a).max(0) as u32).unwrap_or(0)),
        CallState::Held => t!("На удержании").into(),
        CallState::Ended => t!("Завершён").into(),
    }
}

/// Картинка → квадрат 512×512 JPEG (середина) для фото контакта.
pub fn photo_jpeg(path: &std::path::Path) -> std::result::Result<Vec<u8>, String> {
    let img = image::open(path).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    let s = w.min(h);
    let img = img.crop_imm((w - s) / 2, (h - s) / 2, s, s).resize_exact(512, 512, image::imageops::FilterType::Triangle).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&img).map_err(|e| e.to_string())?;
    Ok(out)
}
