//! Интерфейс «Паролей»: создание хранилища, разблокировка, список и
//! запись, редактор с генератором, крупный просмотр пароля, настройки.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use synshell_common::link::{self, DeviceKind, Event};
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::{GestureDetector, MultilineTextEdit};

use crate::sync::{PeerState, PeerSync};
use crate::vault::{self, Entry, Envelope, Session};
use crate::Prefs;

type W = Box<dyn Widget>;

mod ic {
    pub const LOCK: &str = "\u{E897}";
    pub const LOCK_OPEN: &str = "\u{E898}";
    pub const KEY: &str = "\u{E0DA}";
    pub const SHOW: &str = "\u{E8F4}";
    pub const HIDE: &str = "\u{E8F5}";
    pub const COPY: &str = "\u{E14D}";
    pub const PERSON: &str = "\u{E7FD}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const ADD: &str = "\u{E145}";
    pub const STAR: &str = "\u{E838}";
    pub const STAR_OFF: &str = "\u{E83A}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const DELETE: &str = "\u{E872}";
    pub const BACK: &str = "\u{E5C4}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const SYNC: &str = "\u{E627}";
    pub const SYNC_PROBLEM: &str = "\u{E629}";
    pub const PHONE: &str = "\u{E32C}";
    pub const LAPTOP: &str = "\u{E31E}";
    pub const COMPUTER: &str = "\u{E30A}";
    pub const TABLET: &str = "\u{E32F}";
    pub const OPEN: &str = "\u{E89E}";
    pub const ZOOM: &str = "\u{E56B}";
    pub const WEB: &str = "\u{E80B}";
    pub const NOTES: &str = "\u{E26C}";
    pub const REFRESH: &str = "\u{E5D5}";
    pub const REMOVE: &str = "\u{E15B}";
    pub const CHECK: &str = "\u{E5CA}";
    pub const SHIELD: &str = "\u{E32A}";
    pub const DEVICES: &str = "\u{E1B1}";
    pub const QUESTION: &str = "\u{E8FD}";
}

/// Открытое хранилище (между потоками: синхронизация и наблюдение за файлом).
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// Последнее действие пользователя, мс (автоблокировка).
static LAST_ACTIVE: AtomicI64 = AtomicI64::new(0);
/// Очередное копирование — отложенная очистка буфера проверяет, что оно последнее.
static CLIP_GEN: AtomicU64 = AtomicU64::new(0);
/// Время изменения файла хранилища, которое мы уже знаем.
static KNOWN_MTIME: Mutex<Option<SystemTime>> = Mutex::new(None);
/// Хранилища на соединённых устройствах (экран создания: «взять оттуда»).
static REMOTE: Mutex<Vec<Envelope>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Setup,
    Locked,
    Open,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Favorites,
    Recent,
}

#[derive(Clone, Copy)]
struct St {
    phase: RwSignal<Phase>,
    entries: RwSignal<Vec<Entry>>,
    selected: RwSignal<Option<String>>,
    /// Черновик редактора (новая или изменяемая запись).
    draft: RwSignal<Option<Entry>>,
    /// Пересобрать поля редактора (генератор подставил пароль).
    draft_ver: RwSignal<u64>,
    query: RwSignal<String>,
    filter: RwSignal<Filter>,
    reveal: RwSignal<bool>,
    /// Крупный просмотр пароля записи.
    big: RwSignal<Option<String>>,
    toast: RwSignal<String>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    /// Растёт с каждой ошибкой — поле встряхивается заново.
    shake: RwSignal<u64>,
    pw: RwSignal<String>,
    pw2: RwSignal<String>,
    pw_show: RwSignal<bool>,
    peers: RwSignal<Vec<PeerSync>>,
    linked: RwSignal<Vec<(String, DeviceKind)>>,
    syncing: RwSignal<bool>,
    last_sync: RwSignal<Option<i64>>,
    /// Хранилища на устройствах (имя, вид) — индексы в `REMOTE`.
    remote: RwSignal<Vec<(String, DeviceKind)>>,
    /// Выбрано хранилище с устройства — разблокировка заберёт его.
    adopt: RwSignal<Option<usize>>,
    settings: RwSignal<bool>,
    confirm_delete: RwSignal<bool>,
    gen_open: RwSignal<bool>,
    prefs: RwSignal<Prefs>,
    /// Смена мастер-пароля в настройках.
    chg_open: RwSignal<bool>,
    chg_old: RwSignal<String>,
    chg_new: RwSignal<String>,
    chg_new2: RwSignal<String>,
    /// Секретный вопрос: выбранный из готовых (последний — свой), свой текст, ответ.
    q_pick: RwSignal<usize>,
    q_custom: RwSignal<String>,
    q_answer: RwSignal<String>,
    /// Задаётся вопрос в настройках.
    q_open: RwSignal<bool>,
    /// Вопрос хранилища — виден на экране входа без мастер-пароля.
    lock_question: RwSignal<Option<String>>,
    /// Вход по ответу на вопрос с новым мастер-паролем.
    recovering: RwSignal<bool>,
    /// `[link] clipboard` — общий буфер обмена.
    shared_clip: bool,
}

fn touch() {
    LAST_ACTIVE.store(vault::now_ms(), Ordering::Relaxed);
}

pub fn root() -> W {
    let (cfg, _) = synshell_common::Config::load();
    let has_vault = vault::vault_path().exists();
    let st = St {
        phase: use_signal(if has_vault { Phase::Locked } else { Phase::Setup }),
        entries: use_signal(Vec::new()),
        selected: use_signal(None),
        draft: use_signal(None),
        draft_ver: use_signal(0),
        query: use_signal(String::new()),
        filter: use_signal(Filter::All),
        reveal: use_signal(false),
        big: use_signal(None),
        toast: use_signal(String::new()),
        busy: use_signal(false),
        error: use_signal(None),
        shake: use_signal(0),
        pw: use_signal(String::new()),
        pw2: use_signal(String::new()),
        pw_show: use_signal(false),
        peers: use_signal(Vec::new()),
        linked: use_signal(Vec::new()),
        syncing: use_signal(false),
        last_sync: use_signal(None),
        remote: use_signal(Vec::new()),
        adopt: use_signal(None),
        settings: use_signal(false),
        confirm_delete: use_signal(false),
        gen_open: use_signal(false),
        prefs: use_signal(Prefs::load()),
        chg_open: use_signal(false),
        chg_old: use_signal(String::new()),
        chg_new: use_signal(String::new()),
        chg_new2: use_signal(String::new()),
        q_pick: use_signal(0),
        q_custom: use_signal(String::new()),
        q_answer: use_signal(String::new()),
        q_open: use_signal(false),
        lock_question: use_signal(None),
        recovering: use_signal(false),
        shared_clip: cfg.link.enabled && cfg.link.clipboard,
    };
    touch();
    watch_link(st);
    ticker(st);
    if !has_vault {
        find_remote(st);
    } else {
        load_question(st);
    }
    build_root(st)
}

// ─── Данные ─────────────────────────────────────────────────────────────────

fn sorted(s: &Session) -> Vec<Entry> {
    let mut v: Vec<Entry> = s.data.live().cloned().collect();
    v.sort_by_key(|e| e.title.to_lowercase());
    v
}

/// Список из открытого хранилища в интерфейс.
fn refresh(st: St) {
    let v = SESSION.lock().unwrap().as_ref().map(sorted).unwrap_or_default();
    if let Some(id) = st.selected.get_untracked() {
        if !v.iter().any(|e| e.id == id) {
            st.selected.set(None);
        }
    }
    st.entries.set(v);
}

/// Записать хранилище на диск.
fn persist(s: &Session) -> anyhow::Result<()> {
    let path = vault::vault_path();
    s.seal()?.write(&path)?;
    *KNOWN_MTIME.lock().unwrap() = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    Ok(())
}

/// Изменить записи, сохранить и разослать.
fn mutate(st: St, f: impl FnOnce(&mut Session)) {
    touch();
    let res = {
        let mut g = SESSION.lock().unwrap();
        let Some(s) = g.as_mut() else { return };
        f(s);
        persist(s)
    };
    if let Err(e) = res {
        st.toast.set(t!("Не сохранено: {e}", e = format!("{:#}", e)));
    }
    refresh(st);
    sync_now(st, false);
}

fn sync_now(st: St, manual: bool) {
    if st.syncing.get_untracked() || st.phase.get_untracked() != Phase::Open {
        return;
    }
    st.syncing.set(true);
    std::thread::spawn(move || {
        // Синхронизация ходит по сети — на копии, чтобы не держать хранилище.
        let copy = SESSION.lock().unwrap().clone();
        let Some(mut copy) = copy else {
            run_on_main_thread(move || st.syncing.set(false));
            return;
        };
        let (peers, changed) = crate::sync::sync_all(&mut copy);
        let mut err = None;
        {
            let mut g = SESSION.lock().unwrap();
            if let Some(s) = g.as_mut() {
                s.absorb_keys(&copy);
                if changed && s.merge(&copy.data) {
                    if let Err(e) = persist(s) {
                        err = Some(format!("{e:#}"));
                    }
                }
            }
        }
        run_on_main_thread(move || {
            st.syncing.set(false);
            st.linked.set(peers.iter().map(|p| (p.name.clone(), p.kind)).collect());
            if manual {
                let msg = if peers.is_empty() {
                    t!("Нет соединённых устройств").to_string()
                } else if let Some(p) = peers.iter().find(|p| p.state == PeerState::OtherPassword) {
                    t!("На «{name}» хранилище с другим мастер-паролем", name = p.name)
                } else {
                    t!("Синхронизировано: {v}", v = names(&peers.iter().map(|p| p.name.clone()).collect::<Vec<_>>()))
                };
                st.toast.set(msg);
            }
            if let Some(e) = err {
                st.toast.set(t!("Не сохранено: {e}", e = e));
            }
            st.peers.set(peers);
            st.last_sync.set(Some(vault::now_ms()));
            refresh(st);
        });
    });
}

/// Файл хранилища изменили снаружи (синхронизация с другого устройства) — влить.
fn reload_from_disk(st: St) {
    let path = vault::vault_path();
    let m = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    if m.is_none() || *KNOWN_MTIME.lock().unwrap() == m {
        return;
    }
    *KNOWN_MTIME.lock().unwrap() = m;
    let Ok(Some(env)) = Envelope::read(&path) else { return };
    let mut changed = false;
    {
        let mut g = SESSION.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if let Ok(Some(d)) = s.open_other(&env) {
                changed = s.merge(&d);
                // Пришло с паролем, сменённым там, — у нас свой ключ: перешифровать.
                if changed || !s.same_keys(&env) {
                    let _ = persist(s);
                }
            }
        }
    }
    if changed {
        run_on_main_thread(move || refresh(st));
    }
}

/// Автоблокировка, файл хранилища, синхронизация раз в полминуты.
fn ticker(st: St) {
    std::thread::spawn(move || {
        let mut n = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            n += 1;
            let open = SESSION.lock().unwrap().is_some();
            if !open {
                continue;
            }
            reload_from_disk(st);
            run_on_main_thread(move || {
                let mins = st.prefs.get_untracked().autolock_min;
                let idle = vault::now_ms() - LAST_ACTIVE.load(Ordering::Relaxed);
                if mins > 0 && idle > mins as i64 * 60_000 && st.phase.get_untracked() == Phase::Open {
                    lock(st);
                    st.toast.set(t!("Заблокировано: долго без действий").into());
                } else if n % 15 == 0 {
                    sync_now(st, false);
                }
            });
        }
    });
}

/// События synlink: устройство соединилось — синхронизироваться.
fn watch_link(st: St) {
    std::thread::spawn(move || loop {
        if let Ok(events) = link::Client::connect().and_then(|c| c.subscribe()) {
            for ev in events {
                match ev {
                    Ok(Event::Connected { .. }) | Ok(Event::Disconnected { .. }) => run_on_main_thread(move || {
                        let linked = crate::sync::connected().into_iter().map(|p| (p.name, p.kind)).collect();
                        st.linked.set(linked);
                        match st.phase.get_untracked() {
                            Phase::Open => sync_now(st, false),
                            Phase::Setup => find_remote(st),
                            Phase::Locked => {}
                        }
                    }),
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    });
    std::thread::spawn(move || {
        let linked: Vec<(String, DeviceKind)> = crate::sync::connected().into_iter().map(|p| (p.name, p.kind)).collect();
        run_on_main_thread(move || st.linked.set(linked));
    });
}

/// Есть ли хранилища на соединённых устройствах (для первого запуска).
fn find_remote(st: St) {
    std::thread::spawn(move || {
        let mut found = Vec::new();
        let mut envs = Vec::new();
        for p in crate::sync::connected() {
            if let Some(env) = crate::sync::peer_envelope(&p) {
                found.push((p.name.clone(), p.kind));
                envs.push(env);
            }
        }
        *REMOTE.lock().unwrap() = envs;
        run_on_main_thread(move || st.remote.set(found));
    });
}

fn fail(st: St, msg: impl Into<String>) {
    st.error.set(Some(msg.into()));
    st.shake.update(|n| *n += 1);
}

fn unlock(st: St) {
    if st.busy.get_untracked() {
        return;
    }
    let pw = st.pw.get_untracked();
    if pw.is_empty() {
        return fail(st, t!("Введите мастер-пароль"));
    }
    let adopt = st.adopt.get_untracked();
    st.busy.set(true);
    st.error.set(None);
    std::thread::spawn(move || {
        let env = match adopt {
            Some(i) => REMOTE.lock().unwrap().get(i).cloned().map(Ok),
            None => Envelope::read(&vault::vault_path()).transpose(),
        };
        let res = match env {
            None => Err(t!("Хранилище не найдено").to_string()),
            Some(Err(e)) => Err(format!("{e:#}")),
            Some(Ok(env)) => match Session::unlock(&env, &pw) {
                Ok(Some(s)) => {
                    // Взятое с устройства — сохранить у себя.
                    let saved = if adopt.is_some() { persist(&s).map_err(|e| format!("{e:#}")) } else { Ok(()) };
                    if adopt.is_none() {
                        *KNOWN_MTIME.lock().unwrap() = std::fs::metadata(vault::vault_path()).and_then(|m| m.modified()).ok();
                    }
                    saved.map(|_| s)
                }
                Ok(None) => Err(t!("Неверный мастер-пароль").to_string()),
                Err(e) => Err(format!("{e:#}")),
            },
        };
        run_on_main_thread(move || {
            st.busy.set(false);
            match res {
                Ok(s) => {
                    *SESSION.lock().unwrap() = Some(s);
                    opened(st);
                }
                Err(e) => fail(st, e),
            }
        });
    });
}

fn create(st: St) {
    if st.busy.get_untracked() {
        return;
    }
    let (a, b) = (st.pw.get_untracked(), st.pw2.get_untracked());
    if a.chars().count() < 8 {
        return fail(st, t!("Не короче 8 символов"));
    }
    if a != b {
        return fail(st, t!("Пароли не совпадают"));
    }
    let q = match chosen_question(st) {
        Ok(q) => q,
        Err(e) => return fail(st, e),
    };
    st.busy.set(true);
    st.error.set(None);
    std::thread::spawn(move || {
        let res = Session::create(&a)
            .and_then(|mut s| {
                if let Some((q, ans)) = &q {
                    s.set_recovery(q, ans)?;
                }
                Ok(s)
            })
            .and_then(|s| persist(&s).map(|_| s));
        run_on_main_thread(move || {
            st.busy.set(false);
            match res {
                Ok(s) => {
                    *SESSION.lock().unwrap() = Some(s);
                    opened(st);
                    st.toast.set(t!("Хранилище создано").into());
                }
                Err(e) => fail(st, format!("{e:#}")),
            }
        });
    });
}

/// Выбранный секретный вопрос и ответ; `None` — не задавать (ответ пуст).
fn chosen_question(st: St) -> std::result::Result<Option<(String, String)>, String> {
    let ans = st.q_answer.get_untracked();
    if ans.trim().is_empty() {
        return Ok(None);
    }
    let pick = st.q_pick.get_untracked();
    let q = match QUESTIONS.get(pick) {
        Some(q) => q.to_string(),
        None => st.q_custom.get_untracked().trim().to_string(),
    };
    if q.is_empty() {
        return Err(t!("Напишите свой вопрос").into());
    }
    if vault::normalize_answer(&ans).chars().count() < 3 {
        return Err(t!("Ответ — не короче 3 букв").into());
    }
    Ok(Some((q, ans)))
}

fn clear_question_form(st: St) {
    st.q_pick.set(0);
    st.q_custom.set(String::new());
    st.q_answer.set(String::new());
}

/// Секретный вопрос из файла хранилища (для «Забыли мастер-пароль?»).
fn load_question(st: St) {
    std::thread::spawn(move || {
        let q = Envelope::read(&vault::vault_path()).ok().flatten().and_then(|e| e.question());
        run_on_main_thread(move || st.lock_question.set(q));
    });
}

/// Забыли мастер-пароль: открыть ответом и задать новый.
fn recover(st: St) {
    if st.busy.get_untracked() {
        return;
    }
    let (ans, a, b) = (st.q_answer.get_untracked(), st.pw.get_untracked(), st.pw2.get_untracked());
    if ans.trim().is_empty() {
        return fail(st, t!("Введите ответ на вопрос"));
    }
    if a.chars().count() < 8 {
        return fail(st, t!("Новый пароль — не короче 8 символов"));
    }
    if a != b {
        return fail(st, t!("Новые пароли не совпадают"));
    }
    st.busy.set(true);
    st.error.set(None);
    std::thread::spawn(move || {
        let res = match Envelope::read(&vault::vault_path()) {
            Ok(Some(env)) => match Session::recover(&env, &ans, &a) {
                Ok(Some(s)) => persist(&s).map(|_| s).map_err(|e| format!("{e:#}")),
                Ok(None) => Err(t!("Ответ не подходит").to_string()),
                Err(e) => Err(format!("{e:#}")),
            },
            Ok(None) => Err(t!("Хранилище не найдено").to_string()),
            Err(e) => Err(format!("{e:#}")),
        };
        run_on_main_thread(move || {
            st.busy.set(false);
            match res {
                Ok(s) => {
                    *SESSION.lock().unwrap() = Some(s);
                    opened(st);
                    st.toast.set(t!("Новый мастер-пароль задан — на устройствах он сменится при синхронизации").into());
                }
                Err(e) => fail(st, e),
            }
        });
    });
}

fn opened(st: St) {
    touch();
    st.pw.set(String::new());
    st.pw2.set(String::new());
    st.recovering.set(false);
    clear_question_form(st);
    st.pw_show.set(false);
    st.error.set(None);
    st.adopt.set(None);
    st.phase.set(Phase::Open);
    refresh(st);
    sync_now(st, false);
}

fn lock(st: St) {
    *SESSION.lock().unwrap() = None;
    st.entries.set(Vec::new());
    st.selected.set(None);
    st.draft.set(None);
    st.big.set(None);
    st.reveal.set(false);
    st.settings.set(false);
    st.confirm_delete.set(false);
    st.chg_open.set(false);
    st.q_open.set(false);
    st.chg_old.set(String::new());
    clear_question_form(st);
    st.phase.set(Phase::Locked);
    load_question(st);
}

fn entry(st: St, id: &str) -> Option<Entry> {
    st.entries.get_untracked().into_iter().find(|e| e.id == id)
}

fn names(v: &[String]) -> String {
    v.iter().map(|n| format!("«{n}»")).collect::<Vec<_>>().join(", ")
}

/// Скопировать: пароль — с пометкой секрета и очисткой буфера через заданное время.
fn copy(st: St, id: Option<&str>, text: String, what: &str, secret: bool) {
    touch();
    if text.is_empty() {
        return;
    }
    if secret {
        syngui::clipboard::copy_rich(&text, &[("x-kde-passwordManagerHint", b"secret")]);
    } else {
        syngui::clipboard::copy(&text);
    }
    let seq = CLIP_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let clear = st.prefs.get_untracked().clear_secs;
    let mut msg = t!("{what} скопирован", what = what);
    let linked: Vec<String> = st.linked.get_untracked().into_iter().map(|(n, _)| n).collect();
    if st.shared_clip && !linked.is_empty() {
        msg.push_str(&t!(" — вставка и на {v}", v = names(&linked)));
    }
    if secret && clear > 0 {
        msg.push_str(&t!(" · очистка через {clear} с", clear = clear));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(clear as u64));
            if CLIP_GEN.load(Ordering::Relaxed) == seq && syngui::clipboard::paste().as_deref() == Some(text.as_str()) {
                use wl_clipboard_rs::copy::{clear, ClipboardType, Seat};
                let _ = clear(ClipboardType::Regular, Seat::All);
            }
        });
    }
    st.toast.set(msg);
    if let Some(id) = id {
        let id = id.to_string();
        // «Недавние»: время копирования, без отметки изменения записи.
        let mut g = SESSION.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if let Some(e) = s.data.entries.iter_mut().find(|e| e.id == id) {
                e.used = vault::now_ms();
            }
            let _ = persist(s);
        }
        drop(g);
        refresh(st);
    }
}

fn start_new(st: St) {
    touch();
    let mut e = Entry::new();
    e.password = vault::generate(&st.prefs.get_untracked().generator);
    st.confirm_delete.set(false);
    st.gen_open.set(false);
    st.draft.set(Some(e));
    st.draft_ver.update(|v| *v += 1);
}

fn start_edit(st: St, id: &str) {
    touch();
    if let Some(e) = entry(st, id) {
        st.confirm_delete.set(false);
        st.gen_open.set(false);
        st.draft.set(Some(e));
        st.draft_ver.update(|v| *v += 1);
    }
}

fn save_draft(st: St) {
    let Some(mut d) = st.draft.get_untracked() else { return };
    d.title = d.title.trim().to_string();
    if d.title.is_empty() {
        d.title = if !d.url.trim().is_empty() {
            let h = d.host();
            if h.is_empty() { d.url.trim().to_string() } else { h }
        } else if !d.username.trim().is_empty() {
            d.username.trim().to_string()
        } else {
            st.toast.set(t!("Укажите название").into());
            return;
        };
    }
    let id = d.id.clone();
    let existed = entry(st, &id).is_some();
    mutate(st, move |s| {
        d.modified = vault::now_ms();
        match s.data.entries.iter_mut().find(|e| e.id == d.id) {
            Some(e) => {
                if e.title != d.title || e.username != d.username || e.password != d.password || e.url != d.url || e.notes != d.notes || e.favorite != d.favorite {
                    *e = d;
                }
            }
            None => s.data.entries.push(d),
        }
    });
    st.draft.set(None);
    st.selected.set(Some(id));
    st.reveal.set(false);
    st.toast.set(if existed { t!("Сохранено").into() } else { t!("Запись добавлена").into() });
}

fn delete(st: St, id: String) {
    mutate(st, |s| {
        if let Some(e) = s.data.entries.iter_mut().find(|e| e.id == id) {
            e.tombstone();
        }
    });
    st.draft.set(None);
    st.selected.set(None);
    st.confirm_delete.set(false);
    st.toast.set(t!("Запись удалена").into());
}

fn toggle_fav(st: St, id: String) {
    mutate(st, |s| {
        if let Some(e) = s.data.entries.iter_mut().find(|e| e.id == id) {
            e.favorite = !e.favorite;
            e.modified = vault::now_ms();
        }
    });
}

fn change_password(st: St) {
    if st.busy.get_untracked() {
        return;
    }
    let (old, a, b) = (st.chg_old.get_untracked(), st.chg_new.get_untracked(), st.chg_new2.get_untracked());
    if old.is_empty() {
        return fail(st, t!("Введите текущий мастер-пароль или ответ на вопрос"));
    }
    if a.chars().count() < 8 {
        return fail(st, t!("Новый — не короче 8 символов"));
    }
    if a != b {
        return fail(st, t!("Новые пароли не совпадают"));
    }
    st.busy.set(true);
    std::thread::spawn(move || {
        // Проверка ответа — Argon2: на копии, не держа хранилище.
        let copy = SESSION.lock().unwrap().clone();
        let res = if !copy.is_some_and(|s| s.check_secret(&old)) {
            Err(anyhow::anyhow!("Не подходит ни мастер-пароль, ни ответ на вопрос"))
        } else {
            let mut g = SESSION.lock().unwrap();
            match g.as_mut() {
                Some(s) => s.change_password(&a).and_then(|_| persist(s)),
                None => Ok(()),
            }
        };
        run_on_main_thread(move || {
            st.busy.set(false);
            match res {
                Ok(()) => {
                    st.error.set(None);
                    st.chg_open.set(false);
                    st.chg_old.set(String::new());
                    st.chg_new.set(String::new());
                    st.chg_new2.set(String::new());
                    st.toast.set(t!("Мастер-пароль изменён — копии на устройствах обновятся при синхронизации").into());
                    sync_now(st, false);
                }
                Err(e) => fail(st, format!("{e:#}")),
            }
        });
    });
}

/// Задать (сменить) секретный вопрос — под мастер-паролем.
fn save_question(st: St) {
    if st.busy.get_untracked() {
        return;
    }
    let pw = st.chg_old.get_untracked();
    if pw.is_empty() {
        return fail(st, t!("Введите мастер-пароль"));
    }
    let q = match chosen_question(st) {
        Ok(Some(q)) => q,
        Ok(None) => return fail(st, t!("Введите ответ")),
        Err(e) => return fail(st, e),
    };
    st.busy.set(true);
    std::thread::spawn(move || {
        let copy = SESSION.lock().unwrap().clone();
        let res = if !copy.is_some_and(|s| s.check_secret(&pw)) {
            Err(t!("Мастер-пароль неверен").to_string())
        } else {
            let mut g = SESSION.lock().unwrap();
            match g.as_mut() {
                Some(s) => s.set_recovery(&q.0, &q.1).and_then(|_| persist(s)).map_err(|e| format!("{e:#}")),
                None => Ok(()),
            }
        };
        run_on_main_thread(move || {
            st.busy.set(false);
            match res {
                Ok(()) => {
                    st.error.set(None);
                    st.q_open.set(false);
                    st.chg_old.set(String::new());
                    clear_question_form(st);
                    st.toast.set(t!("Секретный вопрос сохранён").into());
                    sync_now(st, false);
                }
                Err(e) => fail(st, e),
            }
        });
    });
}

fn remove_question(st: St) {
    let res = {
        let mut g = SESSION.lock().unwrap();
        match g.as_mut() {
            Some(s) => {
                s.remove_recovery();
                persist(s)
            }
            None => Ok(()),
        }
    };
    match res {
        Ok(()) => {
            st.toast.set(t!("Секретный вопрос убран").into());
            sync_now(st, false);
        }
        Err(e) => st.toast.set(t!("Не сохранено: {e}", e = format!("{:#}", e))),
    }
    // Перерисовать настройки.
    st.prefs.update(|_| {});
}

fn set_prefs(st: St, f: impl FnOnce(&mut Prefs)) {
    touch();
    st.prefs.update(f);
    st.prefs.get_untracked().save();
}

fn go_back(st: St) -> bool {
    touch();
    if st.big.get_untracked().is_some() {
        st.big.set(None);
    } else if st.settings.get_untracked() {
        st.settings.set(false);
        st.chg_open.set(false);
        st.error.set(None);
    } else if st.confirm_delete.get_untracked() {
        st.confirm_delete.set(false);
    } else if st.draft.get_untracked().is_some() {
        st.draft.set(None);
    } else if st.selected.get_untracked().is_some() {
        st.selected.set(None);
        st.reveal.set(false);
    } else if st.adopt.get_untracked().is_some() {
        st.adopt.set(None);
        st.error.set(None);
    } else if st.recovering.get_untracked() {
        st.recovering.set(false);
        st.error.set(None);
    } else {
        return false;
    }
    true
}

const QUESTIONS: &[&str] = &[
    n_!("Девичья фамилия матери"),
    n_!("Кличка первого питомца"),
    n_!("Имя лучшего школьного друга"),
    n_!("Город, где познакомились родители"),
    n_!("Модель первого телефона"),
];

// ─── Каркас ─────────────────────────────────────────────────────────────────

fn build_root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let phase = st.phase.get();
        vec![Box::new(
            AnimatedSwitcher::new(phase as u64, move || -> W {
                match st.phase.get_untracked() {
                    Phase::Setup | Phase::Locked => gate(st),
                    Phase::Open => main_view(st),
                }
            })
            .fade(true)
            .scale(0.98)
            .duration_ms(220)
            .animate_size(false)
            .class("grow"),
        )]
    });
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![Box::new(
                DecoratedBox::new()
                    .child(Text::new(t).max_lines(3).class("toast-text"))
                    .class("toast"),
            )]
        }))
        .class("toast-place");
    create_effect(move || {
        let t = st.toast.get();
        if !t.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(4500));
                run_on_main_thread(move || {
                    if st.toast.get_untracked() == t {
                        st.toast.set(String::new());
                    }
                });
            });
        }
    });
    let overlays = Reactive::new(move || -> Vec<W> {
        let mut v: Vec<W> = Vec::new();
        if let Some(id) = st.big.get() {
            if let Some(e) = entry(st, &id) {
                v.push(big_view(st, e));
            }
        }
        if st.settings.get() {
            v.push(settings_view(st));
        }
        v
    });
    Box::new(
        EventHook::new()
            .on_mouse_move(|_| touch())
            .capture_shortcuts(|k, m| m.ctrl && matches!(k, syngui::input::Key::N | syngui::input::Key::L))
            .on_key_down(move |k, m| {
                touch();
                use syngui::input::Key;
                match k {
                    Key::Escape if go_back(st) => KeyReply::Handled,
                    Key::L if m.ctrl && st.phase.get_untracked() == Phase::Open => {
                        lock(st);
                        KeyReply::Handled
                    }
                    Key::N if m.ctrl && st.phase.get_untracked() == Phase::Open => {
                        start_new(st);
                        KeyReply::Handled
                    }
                    _ => KeyReply::Ignore,
                }
            })
            .child(
                GestureDetector::new().on_back(move || go_back(st)).child(
                    Stack::new()
                        .fit(StackFit::Expand)
                        .child(DecoratedBox::new().child(body).class("root"))
                        .child(overlays)
                        .child(toast),
                ),
            ),
    )
}

fn icon_btn(glyph: &str, class: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                touch();
                f()
            })
            .child(DecoratedBox::new().child(Icon::new(glyph).class("ibtn-icon")).class(&format!("ibtn {class}"))),
    )
}

fn button(glyph: &str, label: &str, class: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    let mut row = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center);
    if !glyph.is_empty() {
        row = row.child(Icon::new(glyph).class("btn-icon"));
    }
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                touch();
                f()
            })
            .child(DecoratedBox::new().child(row.child(Text::new(label.to_string()).class("btn-label"))).class(&format!("btn {class}"))),
    )
}

fn kind_icon(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Phone => ic::PHONE,
        DeviceKind::Tablet => ic::TABLET,
        DeviceKind::Laptop => ic::LAPTOP,
        DeviceKind::Desktop => ic::COMPUTER,
    }
}

/// Цвет кружка записи — по названию.
fn avatar(title: &str, size: &str) -> W {
    let letter = title.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "•".into());
    let h = title.to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32) % 8;
    Box::new(
        DecoratedBox::new()
            .child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(letter).class(&format!("av-text av-text-{size}")))
                    .class("grow"),
            )
            .class(&format!("av av-{size} av{h}")),
    )
}

/// Полоска силы пароля.
fn strength_bar(p: &str) -> W {
    let (lvl, label) = vault::strength(p);
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for i in 0..4u8 {
        let on = !p.is_empty() && i < lvl.max(1);
        let cls = if on { format!("seg seg-{lvl}") } else { "seg".to_string() };
        row = row.child(DecoratedBox::new().class(&cls));
    }
    Box::new(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(row.class("grow"))
            .child(Text::new(syngui::i18n::t(label)).class(&format!("seg-label seg-label-{lvl}"))),
    )
}

// ─── Создание и разблокировка ───────────────────────────────────────────────

fn pw_field(st: St, sig: RwSignal<String>, placeholder: &'static str, autofocus: bool, submit: fn(St)) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let shown = st.pw_show.get();
        let busy = st.busy.get();
        let f = TextField::with_text(sig.get_untracked())
            .obscure(!shown)
            .autofocus(autofocus)
            .disabled(busy)
            .placeholder(syngui::i18n::t(placeholder))
            .prefix_icon(ic::KEY)
            .suffix_icon(if shown { ic::HIDE } else { ic::SHOW })
            .on_suffix_click(move || st.pw_show.set(!st.pw_show.get_untracked()))
            .on_change(move |t| {
                touch();
                sig.set(t.to_string());
                if st.error.get_untracked().is_some() {
                    st.error.set(None);
                }
            })
            .on_submit(move |_| submit(st))
            .class("field gate-field");
        vec![Box::new(f)]
    }))
}

/// Поле ответа на секретный вопрос (видимое: ответ важно написать точно).
fn answer_field(st: St, placeholder: &'static str, submit: fn(St)) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let busy = st.busy.get();
        vec![Box::new(
            TextField::with_text(st.q_answer.get_untracked())
                .placeholder(syngui::i18n::t(placeholder))
                .prefix_icon(ic::QUESTION)
                .disabled(busy)
                .on_change(move |t| {
                    touch();
                    st.q_answer.set(t.to_string());
                    if st.error.get_untracked().is_some() {
                        st.error.set(None);
                    }
                })
                .on_submit(move |_| submit(st))
                .class("field"),
        )]
    }))
}

/// Выбор секретного вопроса: готовые, свой; и ответ.
fn question_form(st: St, submit: fn(St)) -> W {
    let chips = Reactive::new(move || -> Vec<W> {
        let pick = st.q_pick.get();
        let mut row = Flex::row().gap(6.0).wrap();
        for (i, q) in QUESTIONS.iter().copied().chain(std::iter::once(n_!("Свой вопрос"))).enumerate() {
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        touch();
                        st.q_pick.set(i)
                    })
                    .child(DecoratedBox::new().child(Text::new(syngui::i18n::t(q)).class("seg-text")).class(if pick == i { "segbtn segbtn-on" } else { "segbtn" })),
            );
        }
        vec![Box::new(row)]
    });
    let custom = Reactive::new(move || -> Vec<W> {
        if st.q_pick.get() < QUESTIONS.len() {
            return vec![];
        }
        vec![Box::new(
            TextField::with_text(st.q_custom.get_untracked())
                .placeholder(t!("Ваш вопрос"))
                .prefix_icon(ic::EDIT)
                .on_change(move |t| {
                    touch();
                    st.q_custom.set(t.to_string())
                })
                .class("field"),
        )]
    });
    Box::new(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(chips)
            .child(custom)
            .child(answer_field(st, n_!("Ответ"), submit))
            .child(Text::new(t!("Ответ откроет смену мастер-пароля, если вы его забудете. Регистр и «ё» не важны. Выберите то, чего не узнать из соцсетей.")).max_lines(4).class("set-sub")),
    )
}

fn gate(st: St) -> W {
    let card = Reactive::new(move || -> Vec<W> {
        let phase = st.phase.get();
        let adopt = st.adopt.get();
        let recovering = st.recovering.get() && phase == Phase::Locked;
        let setup = phase == Phase::Setup && adopt.is_none();
        let remote = st.remote.get_untracked();
        let question = st.lock_question.get();
        let (title, sub) = match (phase, adopt) {
            _ if recovering => (t!("Забыли мастер-пароль?").to_string(), t!("Ответьте на секретный вопрос и задайте новый мастер-пароль — записи останутся на месте.").to_string()),
            (_, Some(i)) => (
                t!("Хранилище с устройства").to_string(),
                t!("Введите мастер-пароль хранилища с «{v}» — оно сохранится и здесь и будет синхронизироваться.", v = remote.get(i).map(|r| r.0.as_str()).unwrap_or("")),
            ),
            (Phase::Setup, None) => (t!("Новое хранилище").to_string(), t!("Придумайте мастер-пароль. Он открывает все пароли — запомните его.").to_string()),
            _ => (t!("Пароли").to_string(), t!("Введите мастер-пароль").to_string()),
        };
        let badge = if setup {
            ic::SHIELD
        } else if recovering {
            ic::QUESTION
        } else {
            ic::LOCK
        };
        let mut col = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                DecoratedBox::new()
                    .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(badge).class("badge-icon")).class("grow"))
                    .class("badge"),
            )
            .child(Text::new(title).class("gate-title"))
            .child(Text::new(sub).max_lines(4).class("gate-sub"));
        let shake = st.shake.get();
        let mut fields = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if recovering {
            fields = fields
                .child(
                    DecoratedBox::new()
                        .child(Column::new().gap(4.0).child(Text::new(t!("Секретный вопрос")).class("fc-label")).child(Text::new(syngui::i18n::t(&question.clone().unwrap_or_default())).max_lines(3).class("q-text")))
                        .class("q-box"),
                )
                .child(answer_field(st, n_!("Ответ"), recover))
                .child(pw_field(st, st.pw, n_!("Новый мастер-пароль"), false, recover))
                .child(Reactive::new(move || -> Vec<W> { vec![strength_bar(&st.pw.get())] }))
                .child(pw_field(st, st.pw2, n_!("Повторите новый"), false, recover));
        } else if setup {
            fields = fields
                .child(pw_field(st, st.pw, n_!("Мастер-пароль"), true, create))
                .child(Reactive::new(move || -> Vec<W> { vec![strength_bar(&st.pw.get())] }))
                .child(pw_field(st, st.pw2, n_!("Повторите мастер-пароль"), false, create))
                .child(Text::new(t!("Секретный вопрос — по желанию")).class("ed-label q-head"))
                .child(question_form(st, create));
        } else {
            fields = fields.child(pw_field(st, st.pw, n_!("Мастер-пароль"), true, unlock));
        }
        let err = st.error.get();
        let cls = if err.is_some() && shake % 2 == 1 { "fields shake-a" } else if err.is_some() { "fields shake-b" } else { "fields" };
        col = col.child(DecoratedBox::new().child(fields).class(cls));
        if let Some(e) = err {
            col = col.child(Text::new(e).max_lines(2).class("gate-error"));
        }
        let busy = st.busy.get();
        let label = match (busy, setup, recovering) {
            (true, _, _) => t!("Проверка…"),
            (false, _, true) => t!("Задать пароль и открыть"),
            (false, true, _) => t!("Создать хранилище"),
            _ => t!("Открыть"),
        };
        col = col.child(button(if setup || recovering { ic::CHECK } else { ic::LOCK_OPEN }, &label, if busy { "btn-primary btn-wide btn-busy" } else { "btn-primary btn-wide" }, move || {
            if recovering {
                recover(st)
            } else if setup {
                create(st)
            } else {
                unlock(st)
            }
        }));
        if adopt.is_some() || recovering {
            col = col.child(button("", &t!("Назад"), "btn-flat btn-wide", move || {
                go_back(st);
            }));
        } else if phase == Phase::Locked && question.is_some() {
            col = col.child(button("", &t!("Забыли мастер-пароль?"), "btn-flat btn-small", move || {
                st.error.set(None);
                st.pw.set(String::new());
                st.pw2.set(String::new());
                st.q_answer.set(String::new());
                st.recovering.set(true);
            }));
        }
        vec![Box::new(col)]
    });
    // Хранилища на соединённых устройствах — при первом запуске взять оттуда.
    let remote = Reactive::new(move || -> Vec<W> {
        let r = st.remote.get();
        if st.phase.get() != Phase::Setup || st.adopt.get().is_some() || r.is_empty() {
            return vec![];
        }
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("или возьмите хранилище с устройства")).class("gate-or"));
        for (i, (name, kind)) in r.into_iter().enumerate() {
            col = col.child(
                GestureDetector::new()
                    .on_click(move || {
                        st.adopt.set(Some(i));
                        st.pw.set(String::new());
                        st.error.set(None);
                    })
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(12.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(kind_icon(kind)).class("dev-icon"))
                                    .child(
                                        Column::new()
                                            .gap(2.0)
                                            .child(Text::new(name).max_lines(1).class("dev-name"))
                                            .child(Text::new(t!("Есть хранилище паролей · через synlink")).class("dev-sub"))
                                            .class("grow"),
                                    )
                                    .child(Icon::new(ic::OPEN).class("dev-go")),
                            )
                            .class("dev-card"),
                    ),
            );
        }
        vec![Box::new(col)]
    });
    let linked = Reactive::new(move || -> Vec<W> {
        let l = st.linked.get();
        if l.is_empty() || st.phase.get() != Phase::Locked {
            return vec![];
        }
        vec![Box::new(
            Row::new()
                .gap(6.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(ic::DEVICES).class("gate-foot-icon"))
                .child(Text::new(t!("Синхронизация с {v}", v = names(&l.into_iter().map(|x| x.0).collect::<Vec<_>>()))).max_lines(1).class("gate-foot")),
        )]
    });
    Box::new(
        ScrollView::new().vertical().child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    DecoratedBox::new()
                        .child(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(card).child(remote).child(linked))
                        .class("gate-card"),
                )
                .class("gate"),
        ),
    )
}

// ─── Главный экран ──────────────────────────────────────────────────────────

fn main_view(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(760.0);
    Box::new(Reactive::new(move || -> Vec<W> { vec![if narrow.get() { phone(st) } else { desktop(st) }] }))
}

fn phone(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let screen = if st.draft.get().is_some() {
            3u64
        } else if st.selected.get().is_some() {
            2
        } else {
            1
        };
        vec![Box::new(
            AnimatedSwitcher::new(screen, move || -> W {
                if st.draft.get_untracked().is_some() {
                    editor(st, true)
                } else if let Some(id) = st.selected.get_untracked() {
                    detail(st, id, true)
                } else {
                    list_view(st, true)
                }
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(220)
            .animate_size(false)
            .class("grow"),
        )]
    }))
}

fn desktop(st: St) -> W {
    let right = Reactive::new(move || -> Vec<W> {
        let editing = st.draft.get().map(|d| d.id);
        let sel = st.selected.get();
        let key = match (&editing, &sel) {
            (Some(id), _) => format!("e{id}"),
            (None, Some(id)) => format!("d{id}"),
            _ => "none".into(),
        };
        let k = key.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
        vec![Box::new(
            AnimatedSwitcher::new(k, move || -> W {
                if st.draft.get_untracked().is_some() {
                    editor(st, false)
                } else if let Some(id) = st.selected.get_untracked() {
                    detail(st, id, false)
                } else {
                    empty_detail(st)
                }
            })
            .fade(true)
            .slide(0.0, 10.0)
            .duration_ms(180)
            .animate_size(false)
            .class("grow"),
        )]
    });
    Box::new(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().child(list_view(st, false)).class("side"))
            .child(DecoratedBox::new().child(right).class("grow main")),
    )
}

fn empty_detail(st: St) -> W {
    let n = st.entries.get_untracked().len();
    Box::new(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .gap(10.0)
            .child(Icon::new(ic::KEY).class("empty-icon"))
            .child(Text::new(if n == 0 { t!("Пока ни одного пароля") } else { t!("Выберите запись") }).class("empty-title"))
            .child(Text::new(if n == 0 { t!("Добавьте первый — пароль сгенерируется сам") } else { t!("Ctrl+N — новая запись, Ctrl+L — заблокировать") }).class("muted"))
            .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(if n == 0 { button(ic::ADD, &t!("Добавить пароль"), "btn-primary", move || start_new(st)) } else { Box::new(DecoratedBox::new()) }))
            .class("grow empty-detail"),
    )
}

/// Состояние синхронизации в шапке списка.
fn sync_chip(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let peers = st.peers.get();
        let linked = st.linked.get();
        let syncing = st.syncing.get();
        let (glyph, text, cls) = if syncing {
            (ic::SYNC, t!("Синхронизация…").to_string(), "chip chip-busy")
        } else if let Some(p) = peers.iter().find(|p| p.state == PeerState::OtherPassword) {
            (ic::SYNC_PROBLEM, t!("{name}: другой мастер-пароль", name = p.name), "chip chip-warn")
        } else if let Some(p) = peers.iter().find(|p| matches!(p.state, PeerState::Error(_))) {
            (ic::SYNC_PROBLEM, t!("{name}: нет доступа", name = p.name), "chip chip-warn")
        } else if !peers.is_empty() {
            let n: Vec<String> = peers.iter().map(|p| p.name.clone()).collect();
            (kind_icon(peers[0].kind), n.join(", "), "chip chip-ok")
        } else if !linked.is_empty() {
            (kind_icon(linked[0].1), linked.iter().map(|l| l.0.clone()).collect::<Vec<_>>().join(", "), "chip")
        } else {
            (ic::DEVICES, t!("Только здесь").to_string(), "chip")
        };
        vec![Box::new(
            GestureDetector::new()
                .on_click(move || {
                    touch();
                    sync_now(st, true)
                })
                .child(
                    DecoratedBox::new()
                        .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("chip-icon")).child(Text::new(text).max_lines(1).class("chip-text")))
                        .class(cls),
                ),
        )]
    }))
}

fn list_view(st: St, phone: bool) -> W {
    let header = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(t!("Пароли")).max_lines(1).class("title"))
        .child(DecoratedBox::new().class("grow"))
        .child(sync_chip(st))
        .child(icon_btn(ic::LOCK, "", move || lock(st)))
        .child(icon_btn(ic::SETTINGS, "", move || st.settings.set(true)))
        .class("bar");
    let search = Reactive::new(move || -> Vec<W> {
        vec![Box::new(
            TextField::with_text(st.query.get_untracked())
                .placeholder(t!("Поиск"))
                .prefix_icon(ic::SEARCH)
                .autofocus(!phone)
                .on_change(move |t| {
                    touch();
                    st.query.set(t.to_string())
                })
                .class("field search"),
        )]
    });
    let filters = Reactive::new(move || -> Vec<W> {
        let f = st.filter.get();
        let all = st.entries.get();
        let favs = all.iter().filter(|e| e.favorite).count();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for (k, label) in [(Filter::All, t!("Все · {n}", n = all.len())), (Filter::Favorites, t!("Избранное · {favs}", favs = favs)), (Filter::Recent, t!("Недавние").to_string())] {
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        touch();
                        st.filter.set(k)
                    })
                    .child(DecoratedBox::new().child(Text::new(label).class("seg-text")).class(if f == k { "segbtn segbtn-on" } else { "segbtn" })),
            );
        }
        vec![Box::new(row)]
    });
    let list = Reactive::new(move || -> Vec<W> {
        let q = st.query.get();
        let f = st.filter.get();
        let sel = st.selected.get();
        let mut v: Vec<Entry> = st.entries.get().into_iter().filter(|e| e.matches(&q)).collect();
        match f {
            Filter::All => {}
            Filter::Favorites => v.retain(|e| e.favorite),
            Filter::Recent => {
                v.retain(|e| e.used > 0);
                v.sort_by_key(|e| std::cmp::Reverse(e.used));
                v.truncate(20);
            }
        }
        if v.is_empty() {
            let (glyph, text) = if !q.trim().is_empty() {
                (ic::SEARCH, t!("Ничего не найдено"))
            } else if f == Filter::Favorites {
                (ic::STAR_OFF, t!("Отметьте записи звёздочкой"))
            } else if f == Filter::Recent {
                (ic::COPY, t!("Здесь появятся скопированные"))
            } else {
                (ic::KEY, t!("Нажмите «+», чтобы добавить пароль"))
            };
            return vec![Box::new(
                Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("empty-icon")).child(Text::new(text).class("muted")).class("empty"),
            )];
        }
        // Избранное — наверху «Всех».
        if f == Filter::All && q.trim().is_empty() {
            v.sort_by_key(|e| (!e.favorite, e.title.to_lowercase()));
        }
        v.into_iter().map(|e| row(st, e, sel.as_deref())).collect()
    });
    let fab = GestureDetector::new()
        .on_click(move || start_new(st))
        .child(DecoratedBox::new().child(Icon::new(ic::ADD).class("fab-icon")).class("fab"));
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(
                Column::new()
                    .child(header)
                    .child(DecoratedBox::new().child(Column::new().gap(10.0).child(search).child(filters)).class("tools"))
                    .child(ScrollView::new().vertical().child(Column::new().gap(2.0).child(list).class("list")).class("grow")),
            )
            .child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::End).child(fab).class("fab-place")),
    )
}

fn row(st: St, e: Entry, sel: Option<&str>) -> W {
    let on = sel == Some(e.id.as_str());
    let id = e.id.clone();
    let sub = if !e.username.is_empty() { e.username.clone() } else { e.host() };
    let (id_u, user) = (e.id.clone(), e.username.clone());
    let (id_p, pass) = (e.id.clone(), e.password.clone());
    let mut title = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(e.title.clone()).max_lines(1).class("row-title"));
    if e.favorite {
        title = title.child(Icon::new(ic::STAR).class("row-star"));
    }
    let mut actions = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !e.username.is_empty() {
        actions = actions.child(icon_btn(ic::PERSON, "ibtn-soft", move || copy(st, Some(&id_u), user.clone(), &t!("Логин"), false)));
    }
    if !e.password.is_empty() {
        actions = actions.child(icon_btn(ic::COPY, "ibtn-accent", move || copy(st, Some(&id_p), pass.clone(), &t!("Пароль"), true)));
    }
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                touch();
                st.reveal.set(false);
                st.confirm_delete.set(false);
                st.draft.set(None);
                st.selected.set(Some(id.clone()));
            })
            .child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(avatar(&e.title, "m"))
                            .child(Column::new().gap(2.0).child(title).child(Text::new(sub).max_lines(1).class("row-sub")).class("grow"))
                            .child(actions),
                    )
                    .class(if on { "row row-on" } else { "row" }),
            ),
    )
}

// ─── Запись ─────────────────────────────────────────────────────────────────

fn field_card(label: &str, glyph: &str, value: W, actions: Vec<W>) -> W {
    let mut right = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for a in actions {
        right = right.child(a);
    }
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(glyph).class("fc-icon"))
                    .child(Column::new().gap(4.0).child(Text::new(label.to_string()).class("fc-label")).child(value).class("grow"))
                    .child(right),
            )
            .class("fc"),
    )
}

fn masked(p: &str) -> String {
    "•".repeat(p.chars().count().clamp(8, 24))
}

fn detail(st: St, id: String, phone: bool) -> W {
    let Some(e) = entry(st, &id) else { return empty_detail(st) };
    let mut bar = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if phone {
        bar = bar.child(icon_btn(ic::BACK, "", move || {
            st.selected.set(None);
            st.reveal.set(false);
        }));
    }
    let (id_f, id_e) = (id.clone(), id.clone());
    let bar = bar
        .child(DecoratedBox::new().class("grow"))
        .child(icon_btn(if e.favorite { ic::STAR } else { ic::STAR_OFF }, if e.favorite { "ibtn-star" } else { "" }, move || toggle_fav(st, id_f.clone())))
        .child(icon_btn(ic::EDIT, "", move || start_edit(st, &id_e)))
        .class("bar");

    let host = e.host();
    let hero = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(avatar(&e.title, "l"))
        .child(Text::new(e.title.clone()).max_lines(2).class("hero-title"))
        .child(Text::new(if host.is_empty() { " ".to_string() } else { host.clone() }).max_lines(1).class("hero-sub"))
        .class("hero");

    // Главные действия — крупно.
    let mut main_actions = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !e.password.is_empty() {
        let (i, p) = (id.clone(), e.password.clone());
        main_actions = main_actions.child(DecoratedBox::new().child(button(ic::COPY, &t!("Копировать пароль"), "btn-primary btn-big", move || copy(st, Some(&i), p.clone(), &t!("Пароль"), true))).class("grow"));
    }
    if !e.username.is_empty() {
        let (i, u) = (id.clone(), e.username.clone());
        main_actions = main_actions.child(DecoratedBox::new().child(button(ic::PERSON, &t!("Логин"), "btn-tonal btn-big", move || copy(st, Some(&i), u.clone(), &t!("Логин"), false))).class(if e.password.is_empty() { "grow" } else { "" }));
    }

    let mut cards = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !e.username.is_empty() {
        let (i, u) = (id.clone(), e.username.clone());
        cards = cards.child(field_card(&t!("Логин"), ic::PERSON, Box::new(Text::new(e.username.clone()).max_lines(2).class("fc-value")), vec![icon_btn(ic::COPY, "", move || copy(st, Some(&i), u.clone(), &t!("Логин"), false))]));
    }
    if !e.password.is_empty() {
        let pass = e.password.clone();
        let value = Reactive::new(move || -> Vec<W> {
            let shown = st.reveal.get();
            vec![Box::new(Text::new(if shown { pass.clone() } else { masked(&pass) }).max_lines(4).class(if shown { "fc-value fc-pass" } else { "fc-value fc-dots" }))]
        });
        let (i, p) = (id.clone(), e.password.clone());
        let i_big = id.clone();
        let eye = Reactive::new(move || -> Vec<W> { vec![icon_btn(if st.reveal.get() { ic::HIDE } else { ic::SHOW }, "", move || st.reveal.set(!st.reveal.get_untracked()))] });
        cards = cards.child(field_card(
            &t!("Пароль"),
            ic::KEY,
            Box::new(Column::new().gap(8.0).child(value).child(strength_bar(&e.password))),
            vec![Box::new(eye), icon_btn(ic::ZOOM, "", move || st.big.set(Some(i_big.clone()))), icon_btn(ic::COPY, "ibtn-accent", move || copy(st, Some(&i), p.clone(), &t!("Пароль"), true))],
        ));
    }
    if !e.url.trim().is_empty() {
        let url = e.url.trim().to_string();
        let open_url = if url.contains("://") { url.clone() } else { format!("https://{url}") };
        let (i, u) = (id.clone(), url.clone());
        cards = cards.child(field_card(
            &t!("Сайт"),
            ic::WEB,
            Box::new(Text::new(url.clone()).max_lines(2).class("fc-value fc-link")),
            vec![
                icon_btn(ic::OPEN, "", move || {
                    let _ = std::process::Command::new("xdg-open").arg(&open_url).spawn();
                }),
                icon_btn(ic::COPY, "", move || copy(st, Some(&i), u.clone(), &t!("Адрес"), false)),
            ],
        ));
    }
    if !e.notes.trim().is_empty() {
        let (i, n) = (id.clone(), e.notes.clone());
        cards = cards.child(field_card(&t!("Заметки"), ic::NOTES, Box::new(Text::new(e.notes.clone()).class("fc-notes")), vec![icon_btn(ic::COPY, "", move || copy(st, Some(&i), n.clone(), &t!("Текст"), false))]));
    }
    let meta = Text::new(t!("Изменено {v} · создано {v2}", v = when(e.modified), v2 = when(e.created))).class("meta");

    Box::new(
        Column::new().child(bar).child(
            ScrollView::new()
                .vertical()
                .child(
                    Column::new()
                        .gap(18.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(hero)
                        .child(main_actions)
                        .child(cards)
                        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(meta))
                        .class("detail"),
                )
                .class("grow"),
        ),
    )
}

/// «сегодня в 12:30», «3 окт. 2026».
fn when(ms: i64) -> String {
    if ms <= 0 {
        return "—".into();
    }
    let secs = ms / 1000;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    let mut now: libc::tm = unsafe { std::mem::zeroed() };
    let n = (vault::now_ms() / 1000) as libc::time_t;
    unsafe { libc::localtime_r(&n, &mut now) };
    const MON: [&str; 12] = [n_!("янв."), n_!("февр."), n_!("мар."), n_!("апр."), n_!("мая"), n_!("июня"), n_!("июля"), n_!("авг."), n_!("сент."), n_!("окт."), n_!("нояб."), n_!("дек.")];
    if tm.tm_year == now.tm_year && tm.tm_yday == now.tm_yday {
        t!("сегодня в {tm_hour}:{tm_min}", tm_hour = format!("{:02}", tm.tm_hour), tm_min = format!("{:02}", tm.tm_min))
    } else {
        t!("{day} {month} {year}", day = tm.tm_mday, month = syngui::i18n::t(MON[tm.tm_mon.clamp(0, 11) as usize]), year = tm.tm_year + 1900)
    }
}

/// Крупный просмотр: каждый символ в своей клетке с номером, классы символов цветом.
fn big_view(st: St, e: Entry) -> W {
    let mut grid = Flex::row().gap(8.0).wrap().main_axis_alignment(MainAxisAlignment::Center);
    for (i, c) in e.password.chars().enumerate() {
        let cls = if c.is_ascii_digit() {
            "bc-char bc-digit"
        } else if c.is_alphabetic() {
            if c.is_uppercase() { "bc-char bc-upper" } else { "bc-char" }
        } else {
            "bc-char bc-sym"
        };
        let shown = if c == ' ' { "␣".to_string() } else { c.to_string() };
        grid = grid.child(
            DecoratedBox::new()
                .child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(shown).class(cls)).child(Text::new((i + 1).to_string()).class("bc-num")))
                .class(if (i / 4) % 2 == 0 { "bc" } else { "bc bc-alt" }),
        );
    }
    let legend = Row::new()
        .gap(14.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .child(Text::new(t!("abc строчные")).class("lg"))
        .child(Text::new(t!("ABC заглавные")).class("lg lg-upper"))
        .child(Text::new(t!("123 цифры")).class("lg lg-digit"))
        .child(Text::new(t!("#!? символы")).class("lg lg-sym"));
    let (id, pass) = (e.id.clone(), e.password.clone());
    let card = Column::new()
        .gap(18.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(avatar(&e.title, "m"))
                .child(Column::new().gap(2.0).child(Text::new(e.title.clone()).max_lines(1).class("big-title")).child(Text::new(tn!(e.password.chars().count(), "{n} символ", "{n} символа", "{n} символов")).class("big-sub")).class("grow"))
                .child(icon_btn(ic::CLOSE, "", move || st.big.set(None))),
        )
        .child(ScrollView::new().vertical().child(grid.class("bc-grid")).class("bc-scroll"))
        .child(legend)
        .child(button(ic::COPY, &t!("Копировать пароль"), "btn-primary btn-big", move || copy(st, Some(&id), pass.clone(), &t!("Пароль"), true)));
    overlay(st, Box::new(DecoratedBox::new().child(card).class("sheet sheet-big")), move || st.big.set(None))
}

fn overlay(_st: St, content: W, close: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(GestureDetector::new().on_click(move || close()).child(DecoratedBox::new().class("scrim")))
            .child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(GestureDetector::new().on_click(touch).child(content))
                    .class("overlay-place"),
            ),
    )
}

// ─── Редактор ───────────────────────────────────────────────────────────────

fn edit_field(st: St, label: &'static str, glyph: &'static str, get: fn(&Entry) -> String, set: fn(&mut Entry, String), autofocus: bool) -> W {
    let f = Reactive::new(move || -> Vec<W> {
        st.draft_ver.get();
        let v = st.draft.get_untracked().map(|d| get(&d)).unwrap_or_default();
        vec![Box::new(
            TextField::with_text(v)
                .placeholder(syngui::i18n::t(label))
                .prefix_icon(glyph)
                .autofocus(autofocus)
                .on_change(move |t| {
                    touch();
                    let t = t.to_string();
                    st.draft.update(|d| {
                        if let Some(d) = d {
                            set(d, t)
                        }
                    });
                })
                .on_submit(move |_| save_draft(st))
                .class("field"),
        )]
    });
    Box::new(Column::new().gap(6.0).child(Text::new(label).class("ed-label")).child(f))
}

fn generator(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !st.gen_open.get() {
            return vec![];
        }
        let o = st.prefs.get().generator;
        let regen = move || {
            let p = vault::generate(&st.prefs.get_untracked().generator);
            st.draft.update(|d| {
                if let Some(d) = d {
                    d.password = p
                }
            });
            st.draft_ver.update(|v| *v += 1);
        };
        let chip = move |label: &'static str, on: bool, f: fn(&mut vault::GenOpts)| -> W {
            Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        set_prefs(st, |p| f(&mut p.generator));
                        regen();
                    })
                    .child(DecoratedBox::new().child(Text::new(syngui::i18n::t(label)).class("seg-text")).class(if on { "segbtn segbtn-on" } else { "segbtn" })),
            )
        };
        let len = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(t!("Длина")).class("ed-label grow"))
            .child(icon_btn(ic::REMOVE, "ibtn-soft", move || {
                set_prefs(st, |p| p.generator.length = p.generator.length.saturating_sub(2).max(8));
                regen();
            }))
            .child(Text::new(o.length.to_string()).class("gen-len"))
            .child(icon_btn(ic::ADD, "ibtn-soft", move || {
                set_prefs(st, |p| p.generator.length = (p.generator.length + 2).min(64));
                regen();
            }));
        let opts = Flex::row()
            .gap(6.0)
            .wrap()
            .child(chip("A–Z", o.upper, |g| g.upper = !g.upper))
            .child(chip("0–9", o.digits, |g| g.digits = !g.digits))
            .child(chip("!@#", o.symbols, |g| g.symbols = !g.symbols))
            .child(chip(n_!("Без похожих"), o.no_similar, |g| g.no_similar = !g.no_similar));
        vec![Box::new(
            DecoratedBox::new()
                .child(Column::new().gap(10.0).child(len).child(opts).child(button(ic::REFRESH, &t!("Ещё вариант"), "btn-tonal", regen)))
                .class("gen"),
        )]
    }))
}

fn editor(st: St, _phone: bool) -> W {
    let Some(d) = st.draft.get_untracked() else { return empty_detail(st) };
    let is_new = entry(st, &d.id).is_none();
    let bar = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_btn(ic::CLOSE, "", move || st.draft.set(None)))
        .child(Text::new(if is_new { t!("Новая запись") } else { t!("Изменить") }).class("bar-title grow"))
        .child(button(ic::CHECK, &t!("Сохранить"), "btn-primary", move || save_draft(st)))
        .class("bar");

    let pass = Reactive::new(move || -> Vec<W> {
        st.draft_ver.get();
        let shown = st.pw_show.get();
        let v = st.draft.get_untracked().map(|d| d.password).unwrap_or_default();
        vec![Box::new(
            TextField::with_text(v)
                .placeholder(t!("Пароль"))
                .prefix_icon(ic::KEY)
                .obscure(!shown)
                .suffix_icon(if shown { ic::HIDE } else { ic::SHOW })
                .on_suffix_click(move || st.pw_show.set(!st.pw_show.get_untracked()))
                .on_change(move |t| {
                    touch();
                    let t = t.to_string();
                    st.draft.update(|d| {
                        if let Some(d) = d {
                            d.password = t
                        }
                    });
                })
                .class("field field-pass"),
        )]
    });
    let pass_block = Column::new()
        .gap(8.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Пароль")).class("ed-label grow"))
                .child(Reactive::new(move || -> Vec<W> {
                    let open = st.gen_open.get();
                    vec![button(ic::REFRESH, &if open { t!("Скрыть генератор") } else { t!("Генератор") }, "btn-flat btn-small", move || st.gen_open.set(!st.gen_open.get_untracked()))]
                })),
        )
        .child(pass)
        .child(Reactive::new(move || -> Vec<W> { vec![strength_bar(&st.draft.get().map(|d| d.password).unwrap_or_default())] }))
        .child(generator(st));

    let notes = Reactive::new(move || -> Vec<W> {
        st.draft_ver.get();
        let v = st.draft.get_untracked().map(|d| d.notes).unwrap_or_default();
        vec![Box::new(
            MultilineTextEdit::new()
                .text(v)
                .placeholder(t!("Ответы на секретные вопросы, PIN, коды…"))
                .rows(4)
                .on_change(move |t| {
                    touch();
                    let t = t.to_string();
                    st.draft.update(|d| {
                        if let Some(d) = d {
                            d.notes = t
                        }
                    });
                })
                .class("notes"),
        )]
    });

    let id = d.id.clone();
    let danger = Reactive::new(move || -> Vec<W> {
        if is_new {
            return vec![];
        }
        let id = id.clone();
        if st.confirm_delete.get() {
            return vec![Box::new(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(10.0)
                            .child(Text::new(t!("Удалить запись? Она исчезнет и на связанных устройствах.")).max_lines(3).class("danger-text"))
                            .child(
                                Row::new()
                                    .gap(8.0)
                                    .child(DecoratedBox::new().child(button("", &t!("Отмена"), "btn-flat btn-wide", move || st.confirm_delete.set(false))).class("grow"))
                                    .child(DecoratedBox::new().child(button(ic::DELETE, &t!("Удалить"), "btn-danger btn-wide", move || delete(st, id.clone()))).class("grow")),
                            ),
                    )
                    .class("danger-box"),
            )];
        }
        vec![button(ic::DELETE, &t!("Удалить запись"), "btn-flat btn-danger-flat", move || st.confirm_delete.set(true))]
    });

    Box::new(
        Column::new().child(bar).child(
            ScrollView::new()
                .vertical()
                .child(
                    Column::new()
                        .gap(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(edit_field(st, n_!("Название"), ic::KEY, |d| d.title.clone(), |d, v| d.title = v, is_new))
                        .child(edit_field(st, n_!("Логин"), ic::PERSON, |d| d.username.clone(), |d, v| d.username = v, false))
                        .child(pass_block)
                        .child(edit_field(st, n_!("Сайт"), ic::WEB, |d| d.url.clone(), |d, v| d.url = v, false))
                        .child(Column::new().gap(6.0).child(Text::new(t!("Заметки")).class("ed-label")).child(notes))
                        .child(danger)
                        .class("editor"),
                )
                .class("grow"),
        ),
    )
}

// ─── Настройки ──────────────────────────────────────────────────────────────

fn settings_view(st: St) -> W {
    let choice = move |title: String, sub: String, opts: Vec<(u32, String)>, cur: u32, f: fn(&mut Prefs, u32)| -> W {
        let mut row = Flex::row().gap(6.0).wrap();
        for (v, label) in opts {
            row = row.child(
                GestureDetector::new()
                    .on_click(move || set_prefs(st, |p| f(p, v)))
                    .child(DecoratedBox::new().child(Text::new(label).class("seg-text")).class(if cur == v { "segbtn segbtn-on" } else { "segbtn" })),
            );
        }
        Box::new(Column::new().gap(8.0).child(Text::new(title).class("set-title")).child(Text::new(sub).max_lines(2).class("set-sub")).child(row))
    };
    let body = Reactive::new(move || -> Vec<W> {
        let p = st.prefs.get();
        let mut col = Column::new()
            .gap(20.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(choice(t!("Автоблокировка"), t!("Закрыть хранилище, если долго ничего не делать"), vec![(1, t!("1 мин")), (5, t!("5 мин")), (15, t!("15 мин")), (60, t!("1 ч")), (0, t!("Никогда"))], p.autolock_min, |p, v| p.autolock_min = v))
            .child(choice(
                t!("Очистка буфера обмена"),
                t!("Скопированный пароль стирается — здесь и на связанных устройствах"),
                vec![(15, t!("15 с")), (30, t!("30 с")), (60, t!("1 мин")), (0, t!("Не очищать"))],
                p.clear_secs,
                |p, v| p.clear_secs = v,
            ));
        // Устройства.
        let peers = st.peers.get();
        let mut devs = Column::new().gap(8.0).child(Text::new(t!("Синхронизация через synlink")).class("set-title"));
        if peers.is_empty() {
            devs = devs.child(Text::new(t!("Нет соединённых устройств. Подключите телефон кабелем или спарьте по Wi-Fi — пароли синхронизируются сами, если там тот же мастер-пароль.")).max_lines(4).class("set-sub"));
        }
        for p in peers {
            let (state, cls) = match &p.state {
                PeerState::Synced => (t!("Синхронизировано").to_string(), "dev-sub"),
                PeerState::Copied => (t!("Хранилище скопировано туда").to_string(), "dev-sub"),
                PeerState::OtherPassword => (t!("Там хранилище с другим мастер-паролем").to_string(), "dev-sub dev-warn"),
                PeerState::Error(e) => (e.clone(), "dev-sub dev-warn"),
            };
            devs = devs.child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(kind_icon(p.kind)).class("dev-icon"))
                            .child(Column::new().gap(2.0).child(Text::new(p.name.clone()).max_lines(1).class("dev-name")).child(Text::new(state).max_lines(2).class(cls)).class("grow")),
                    )
                    .class("dev-card"),
            );
        }
        if !st.shared_clip {
            devs = devs.child(Text::new(t!("Общий буфер обмена выключен: Параметры → Связь с устройствами.")).max_lines(2).class("set-sub dev-warn"));
        }
        devs = devs.child(button(ic::SYNC, &t!("Синхронизировать сейчас"), "btn-tonal", move || sync_now(st, true)));
        col = col.child(devs);
        // Секретный вопрос.
        let question = SESSION.lock().unwrap().as_ref().and_then(|s| s.question());
        let mut qcol = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Секретный вопрос")).class("set-title"));
        if st.q_open.get() {
            let shake = st.shake.get();
            let err = st.error.get();
            let fields = Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(question_form(st, save_question))
                .child(pw_field(st, st.chg_old, n_!("Мастер-пароль для подтверждения"), false, save_question));
            let cls = if err.is_some() && shake % 2 == 1 { "fields shake-a" } else if err.is_some() { "fields shake-b" } else { "fields" };
            qcol = qcol.child(DecoratedBox::new().child(fields).class(cls));
            if let Some(e) = err {
                qcol = qcol.child(Text::new(e).max_lines(2).class("gate-error"));
            }
            qcol = qcol.child(
                Row::new()
                    .gap(8.0)
                    .child(DecoratedBox::new().child(button("", &t!("Отмена"), "btn-flat btn-wide", move || {
                        st.q_open.set(false);
                        st.error.set(None);
                        st.chg_old.set(String::new());
                        clear_question_form(st);
                    })).class("grow"))
                    .child(DecoratedBox::new().child(button(ic::CHECK, &if st.busy.get() { t!("Шифрование…") } else { t!("Сохранить") }, "btn-primary btn-wide", move || save_question(st))).class("grow")),
            );
        } else {
            qcol = qcol.child(
                Text::new(match &question {
                    Some(q) => t!("«{q}» — если забудете мастер-пароль, ответ позволит задать новый (экран входа → «Забыли мастер-пароль?»).", q = syngui::i18n::t(q)),
                    None => t!("Не задан. Без него забытый мастер-пароль не восстановить — записи будут потеряны.").to_string(),
                })
                .max_lines(4)
                .class(if question.is_some() { "set-sub" } else { "set-sub dev-warn" }),
            );
            let mut row = Row::new().gap(8.0).child(DecoratedBox::new().child(button(ic::QUESTION, &if question.is_some() { t!("Изменить вопрос") } else { t!("Задать вопрос") }, "btn-tonal btn-wide", move || {
                st.error.set(None);
                st.chg_open.set(false);
                st.chg_old.set(String::new());
                clear_question_form(st);
                st.q_open.set(true)
            })).class("grow"));
            if question.is_some() {
                row = row.child(DecoratedBox::new().child(button(ic::DELETE, &t!("Убрать"), "btn-flat btn-wide btn-danger-flat", move || remove_question(st))).class("grow"));
            }
            qcol = qcol.child(row);
        }
        col = col.child(qcol);
        // Мастер-пароль.
        if st.chg_open.get() {
            let shake = st.shake.get();
            let err = st.error.get();
            let fields = Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(pw_field(st, st.chg_old, n_!("Текущий мастер-пароль или ответ на вопрос"), true, change_password))
                .child(pw_field(st, st.chg_new, n_!("Новый мастер-пароль"), false, change_password))
                .child(Reactive::new(move || -> Vec<W> { vec![strength_bar(&st.chg_new.get())] }))
                .child(pw_field(st, st.chg_new2, n_!("Повторите новый"), false, change_password));
            let cls = if err.is_some() && shake % 2 == 1 { "fields shake-a" } else if err.is_some() { "fields shake-b" } else { "fields" };
            let mut c = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Смена мастер-пароля")).class("set-title")).child(DecoratedBox::new().child(fields).class(cls));
            if let Some(e) = err {
                c = c.child(Text::new(e).max_lines(2).class("gate-error"));
            }
            c = c.child(
                Row::new()
                    .gap(8.0)
                    .child(DecoratedBox::new().child(button("", &t!("Отмена"), "btn-flat btn-wide", move || {
                        st.chg_open.set(false);
                        st.error.set(None);
                    })).class("grow"))
                    .child(DecoratedBox::new().child(button(ic::CHECK, &if st.busy.get() { t!("Шифрование…") } else { t!("Сменить") }, "btn-primary btn-wide", move || change_password(st))).class("grow")),
            );
            col = col.child(c);
        } else {
            col = col.child(
                Row::new()
                    .gap(8.0)
                    .child(DecoratedBox::new().child(button(ic::KEY, &t!("Сменить мастер-пароль"), "btn-tonal btn-wide", move || {
                        st.error.set(None);
                        st.q_open.set(false);
                        st.chg_old.set(String::new());
                        st.chg_open.set(true)
                    })).class("grow"))
                    .child(DecoratedBox::new().child(button(ic::LOCK, &t!("Заблокировать"), "btn-tonal btn-wide", move || lock(st))).class("grow")),
            );
        }
        vec![Box::new(col)]
    });
    let card = Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Настройки")).class("bar-title grow"))
                .child(icon_btn(ic::CLOSE, "", move || {
                    st.settings.set(false);
                    st.chg_open.set(false);
                    st.error.set(None);
                })),
        )
        .child(ScrollView::new().vertical().child(body).class("set-scroll"));
    overlay(st, Box::new(DecoratedBox::new().child(card).class("sheet")), move || {
        st.settings.set(false);
        st.chg_open.set(false);
    })
}
