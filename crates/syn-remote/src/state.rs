//! Состояние «Пульта»: экраны, мои пульты, база кодов, подбор и передача.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::prelude::*;

use crate::db::{self, Candidate, Db, Model};
use crate::ir::{self, Signal};
use crate::keys;
use crate::store::{self, Remote};

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// Мои пульты.
    Home,
    /// Пульт по id.
    Remote(String),
    /// Добавление: тип техники.
    Categories,
    /// Марки типа техники.
    Brands(String),
    /// Модели марки (выбор вручную).
    Models(String, String),
    /// Подбор (`St::pick`).
    Pick,
    /// Пульт найден (`St::found`).
    Found,
    /// База кодов и передатчик.
    Database,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DbState {
    Idle,
    /// Скачивание или распаковка: подпись и доля (если известна).
    Busy(String, Option<f32>),
    Error(String),
}

/// Подбор пульта.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub cat: String,
    pub brand: Option<String>,
    pub cands: Arc<Vec<Candidate>>,
    pub idx: usize,
    /// Проверочная кнопка: «power», «mute», «vol_up».
    pub test: &'static str,
    /// Текущий вариант уже отправлен.
    pub sent: bool,
    pub auto: bool,
    pub loading: bool,
}

/// Найденный пульт перед сохранением.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub cat: String,
    pub brand: String,
    pub models: Vec<Model>,
    /// Выбранная модель (индекс в `models`); кнопки — её.
    pub sel: usize,
    pub signals: Vec<Signal>,
    pub name: String,
}

#[derive(Clone, Copy)]
pub struct St {
    pub page: RwSignal<Page>,
    pub remotes: RwSignal<Vec<Remote>>,
    pub db: RwSignal<Option<Arc<Db>>>,
    pub db_state: RwSignal<DbState>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
    pub toast: RwSignal<Option<(u64, String)>>,
    /// Кнопка, которая сейчас «светится» (имя сигнала).
    pub flash: RwSignal<Option<String>>,
    /// Программа передатчика (`None` — нет ИК, кнопки только подсвечиваются).
    pub ir: RwSignal<Option<String>>,
    pub pick: RwSignal<Option<Pick>>,
    pub found: RwSignal<Option<Found>>,
    /// Цифровая панель пульта открыта.
    pub numpad: RwSignal<bool>,
    /// Строка поиска на экранах выбора.
    pub query: RwSignal<String>,
    pub menu_open: RwSignal<bool>,
    pub delete_open: RwSignal<bool>,
    pub rename: RwSignal<Option<String>>,
}

static FLASH: AtomicU64 = AtomicU64::new(0);
static TOAST: AtomicU64 = AtomicU64::new(0);
static HOLD: AtomicU64 = AtomicU64::new(0);
static AUTO: AtomicU64 = AtomicU64::new(0);

/// Пауза между вариантами при автоподборе.
const AUTO_STEP: Duration = Duration::from_millis(2800);

impl St {
    pub fn new() -> Self {
        St {
            page: use_signal(Page::Home),
            remotes: use_signal(store::load()),
            db: use_signal(None),
            db_state: use_signal(DbState::Idle),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            toast: use_signal(None),
            flash: use_signal(None),
            ir: use_signal(crate::tx::program().map(|p| p.display().to_string())),
            pick: use_signal(None),
            found: use_signal(None),
            numpad: use_signal(false),
            query: use_signal(String::new()),
            menu_open: use_signal(false),
            delete_open: use_signal(false),
            rename: use_signal(None),
        }
    }

    /// Фон: указатель базы и сообщения передатчика.
    pub fn start(self) {
        let st = self;
        crate::tx::on_result(move |r| {
            if let Err(e) = r {
                run_on_main_thread(move || st.toast(t!("Не отправлено: {e}", e = e)));
            }
        });
        std::thread::spawn(move || {
            let db = db::load().map(Arc::new);
            run_on_main_thread(move || st.db.set(db));
        });
    }

    pub fn toast(&self, msg: String) {
        let id = TOAST.fetch_add(1, Ordering::Relaxed) + 1;
        self.toast.set(Some((id, msg)));
        let t = self.toast;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(3500));
            run_on_main_thread(move || {
                if t.get_untracked().is_some_and(|(i, _)| i == id) {
                    t.set(None);
                }
            });
        });
    }

    pub fn go(&self, p: Page) {
        self.query.set(String::new());
        self.menu_open.set(false);
        if !matches!(p, Page::Pick) {
            self.stop_auto();
        }
        self.page.set(p);
    }

    /// Экран выше по иерархии; `false` — уже на главном.
    pub fn back(&self) -> bool {
        let up = match self.page.get_untracked() {
            Page::Home => return false,
            Page::Remote(_) | Page::Categories | Page::Database => Page::Home,
            Page::Brands(_) => Page::Categories,
            Page::Models(c, _) => Page::Brands(c),
            Page::Pick => match self.pick.get_untracked() {
                Some(p) => Page::Brands(p.cat),
                None => Page::Categories,
            },
            Page::Found => match self.pick.get_untracked() {
                Some(_) => Page::Pick,
                None => match self.found.get_untracked() {
                    Some(f) => Page::Models(f.cat, f.brand),
                    None => Page::Categories,
                },
            },
        };
        self.go(up);
        true
    }

    // ── Передача ───────────────────────────────────────────────────────────

    /// Отправить сигнал и подсветить кнопку.
    pub fn send(&self, sig: &Signal) {
        self.light(&sig.name);
        if self.ir.get_untracked().is_none() {
            return;
        }
        match ir::encode(&sig.code) {
            Some(f) => {
                crate::tx::send(f);
            }
            None => self.toast(t!("Протокол «{p}» не поддерживается", p = ir::describe(&sig.code))),
        }
    }

    fn light(&self, name: &str) {
        let id = FLASH.fetch_add(1, Ordering::Relaxed) + 1;
        self.flash.set(Some(name.to_string()));
        let f = self.flash;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(180));
            run_on_main_thread(move || {
                if FLASH.load(Ordering::Relaxed) == id {
                    f.set(None);
                }
            });
        });
    }

    /// Нажатие кнопки пульта: сразу посылка, при удержании громкости, каналов и стрелок — повтор.
    pub fn press(&self, sig: Signal) {
        let gen = HOLD.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(&sig);
        if !keys::repeats(keys::key_of(&sig.name)) {
            return;
        }
        let st = *self;
        std::thread::spawn(move || {
            let start = Instant::now();
            std::thread::sleep(Duration::from_millis(450));
            while HOLD.load(Ordering::Relaxed) == gen && start.elapsed() < Duration::from_secs(20) {
                let s = sig.clone();
                run_on_main_thread(move || {
                    if HOLD.load(Ordering::Relaxed) == gen {
                        st.send(&s);
                    }
                });
                std::thread::sleep(Duration::from_millis(170));
            }
        });
    }

    pub fn release(&self) {
        HOLD.fetch_add(1, Ordering::Relaxed);
    }

    // ── Мои пульты ─────────────────────────────────────────────────────────

    pub fn remote(&self, id: &str) -> Option<Remote> {
        self.remotes.get_untracked().into_iter().find(|r| r.id == id)
    }

    fn update_remotes(&self, f: impl FnOnce(&mut Vec<Remote>)) {
        let mut list = self.remotes.get_untracked();
        f(&mut list);
        store::save(&list);
        self.remotes.set(list);
    }

    pub fn delete(&self, id: &str) {
        self.update_remotes(|l| l.retain(|r| r.id != id));
        self.go(Page::Home);
        self.toast(t!("Пульт удалён"));
    }

    pub fn set_name(&self, id: &str, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.update_remotes(|l| {
            if let Some(r) = l.iter_mut().find(|r| r.id == id) {
                r.name = name;
            }
        });
    }

    /// Взять кнопки другой модели с тем же кодом питания.
    pub fn switch_model(&self, id: &str, path: &str) {
        let signals = db::read(path);
        if signals.is_empty() {
            self.toast(t!("Файл пульта не найден — обновите базу"));
            return;
        }
        let model = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
        self.update_remotes(|l| {
            if let Some(r) = l.iter_mut().find(|r| r.id == id) {
                let old = std::mem::replace(&mut r.path, path.to_string());
                r.alternatives.retain(|p| p != path);
                if !old.is_empty() {
                    r.alternatives.insert(0, old);
                }
                r.model = model.clone();
                r.signals = signals;
            }
        });
        self.toast(t!("Модель: {m}", m = model));
    }

    // ── База ───────────────────────────────────────────────────────────────

    pub fn download_db(&self) {
        if matches!(self.db_state.get_untracked(), DbState::Busy(..)) {
            return;
        }
        self.db_state.set(DbState::Busy(t!("Подключение…"), None));
        let st = *self;
        std::thread::spawn(move || {
            let r = db::download(|p| {
                let s = match p {
                    db::Progress::Download(got, total) => {
                        let mb = got as f32 / 1_048_576.0;
                        match total {
                            Some(t) if t > 0 => DbState::Busy(t!("Загрузка {a} из {b} МБ", a = format!("{mb:.1}"), b = format!("{:.1}", t as f32 / 1_048_576.0)), Some(got as f32 / t as f32)),
                            _ => DbState::Busy(t!("Загрузка {a} МБ", a = format!("{mb:.1}")), None),
                        }
                    }
                    db::Progress::Unpack(n) => DbState::Busy(t!("Распаковка: {n} пультов", n = n), None),
                };
                run_on_main_thread(move || st.db_state.set(s));
            });
            let db = db::load().map(Arc::new);
            run_on_main_thread(move || match r {
                Ok(n) => {
                    st.db.set(db);
                    st.db_state.set(DbState::Idle);
                    st.toast(t!("База кодов загружена: {n} пультов", n = n));
                }
                Err(e) => st.db_state.set(DbState::Error(format!("{e:#}"))),
            });
        });
    }

    // ── Подбор ─────────────────────────────────────────────────────────────

    pub fn start_pick(&self, cat: String, brand: Option<String>) {
        let Some(db) = self.db.get_untracked() else { return };
        self.stop_auto();
        let test = self.pick.get_untracked().map(|p| p.test).unwrap_or("power");
        self.pick.set(Some(Pick { cat: cat.clone(), brand: brand.clone(), cands: Arc::new(vec![]), idx: 0, test, sent: false, auto: false, loading: true }));
        self.found.set(None);
        self.go(Page::Pick);
        let pick = self.pick;
        std::thread::spawn(move || {
            let cands = Arc::new(db::candidates(&db, &cat, brand.as_deref()));
            run_on_main_thread(move || {
                let mut p = pick.get_untracked();
                if let Some(p) = p.as_mut().filter(|p| p.cat == cat && p.brand == brand) {
                    p.cands = cands;
                    p.loading = false;
                }
                pick.set(p);
            });
        });
    }

    fn with_pick(&self, f: impl FnOnce(&mut Pick)) {
        let mut p = self.pick.get_untracked();
        if let Some(p) = p.as_mut() {
            f(p);
        }
        self.pick.set(p);
    }

    pub fn set_test(&self, key: &'static str) {
        self.with_pick(|p| p.test = key);
    }

    /// Отправить проверочный сигнал текущего варианта.
    pub fn pick_send(&self) {
        let Some(p) = self.pick.get_untracked() else { return };
        if let Some(sig) = p.cands.get(p.idx).and_then(|c| c.test(p.test)) {
            let sig = sig.clone();
            self.send(&sig);
            self.with_pick(|p| p.sent = true);
        }
    }

    /// Следующий (`+1`) или предыдущий (`-1`) вариант — и сразу отправить его.
    pub fn pick_step(&self, d: isize) {
        let Some(p) = self.pick.get_untracked() else { return };
        let n = p.cands.len() as isize;
        let i = p.idx as isize + d;
        if i < 0 {
            return;
        }
        self.with_pick(|p| {
            p.idx = i.min(n) as usize;
            p.sent = false;
        });
        if i < n {
            self.pick_send();
        }
    }

    pub fn toggle_auto(&self) {
        if self.pick.get_untracked().is_some_and(|p| p.auto) {
            self.stop_auto();
            return;
        }
        let gen = AUTO.fetch_add(1, Ordering::Relaxed) + 1;
        self.with_pick(|p| {
            p.auto = true;
            if p.idx >= p.cands.len() {
                p.idx = 0;
            }
        });
        self.pick_send();
        let st = *self;
        std::thread::spawn(move || loop {
            std::thread::sleep(AUTO_STEP);
            if AUTO.load(Ordering::Relaxed) != gen {
                return;
            }
            run_on_main_thread(move || {
                if AUTO.load(Ordering::Relaxed) != gen {
                    return;
                }
                let Some(p) = st.pick.get_untracked() else { return };
                if p.idx + 1 >= p.cands.len() {
                    st.stop_auto();
                    st.with_pick(|p| p.idx = p.cands.len());
                } else {
                    st.pick_step(1);
                }
            });
        });
    }

    pub fn stop_auto(&self) {
        AUTO.fetch_add(1, Ordering::Relaxed);
        if self.pick.get_untracked().is_some_and(|p| p.auto) {
            self.with_pick(|p| p.auto = false);
        }
    }

    /// Вариант сработал: к экрану сохранения.
    pub fn pick_yes(&self) {
        self.stop_auto();
        let Some(p) = self.pick.get_untracked() else { return };
        let Some(c) = p.cands.get(p.idx) else { return };
        let brand = p.brand.clone().unwrap_or_else(|| if c.brand.is_empty() { t!("Универсальный") } else { c.brand.clone() });
        let signals = match c.models.first() {
            Some(m) => {
                let s = db::read(&m.path);
                if s.is_empty() {
                    c.signals.clone()
                } else {
                    s
                }
            }
            None => c.signals.clone(),
        };
        self.found.set(Some(Found { name: default_name(&brand, &p.cat), cat: p.cat.clone(), brand, models: c.models.clone(), sel: 0, signals }));
        self.go(Page::Found);
    }

    /// Модель выбрана вручную.
    pub fn choose_model(&self, cat: &str, brand: &str, m: &Model) {
        self.pick.set(None);
        let signals = db::read(&m.path);
        self.found.set(Some(Found { name: default_name(brand, cat), cat: cat.into(), brand: brand.into(), models: vec![m.clone()], sel: 0, signals }));
        self.go(Page::Found);
    }

    pub fn found_select(&self, i: usize) {
        let mut f = self.found.get_untracked();
        if let Some(f) = f.as_mut() {
            if let Some(m) = f.models.get(i) {
                let s = db::read(&m.path);
                if !s.is_empty() {
                    f.signals = s;
                    f.sel = i;
                }
            }
        }
        self.found.set(f);
    }

    pub fn found_name(&self, name: &str) {
        let mut f = self.found.get_untracked();
        if let Some(f) = f.as_mut() {
            f.name = name.to_string();
        }
        self.found.set(f);
    }

    fn found_remote(&self) -> Option<Remote> {
        let f = self.found.get_untracked()?;
        let m = f.models.get(f.sel);
        Some(Remote {
            id: store::new_id(),
            name: if f.name.trim().is_empty() { default_name(&f.brand, &f.cat) } else { f.name.trim().to_string() },
            category: f.cat.clone(),
            brand: f.brand.clone(),
            model: m.map(|m| m.name.clone()).unwrap_or_default(),
            path: m.map(|m| m.path.clone()).unwrap_or_default(),
            alternatives: f.models.iter().enumerate().filter(|(i, _)| *i != f.sel).map(|(_, m)| m.path.clone()).collect(),
            signals: f.signals.clone(),
            created: db::now(),
        })
    }

    pub fn save_found(&self) {
        let Some(r) = self.found_remote() else { return };
        let id = r.id.clone();
        self.update_remotes(|l| l.push(r));
        self.pick.set(None);
        self.found.set(None);
        self.go(Page::Remote(id));
        self.toast(t!("Пульт сохранён"));
    }

    /// Для снимка экрана: найденный пульт открывается без записи на диск.
    pub fn save_found_preview(&self) {
        let Some(r) = self.found_remote() else { return };
        let id = r.id.clone();
        self.remotes.set(vec![r]);
        self.page.set(Page::Remote(id));
    }
}

/// «LG · Телевизор».
pub fn default_name(brand: &str, cat: &str) -> String {
    let kind = match cat {
        "TVs" => t!("Телевизор"),
        "ACs" => t!("Кондиционер"),
        "Projectors" => t!("Проектор"),
        "SoundBars" => t!("Саундбар"),
        "Fans" => t!("Вентилятор"),
        _ => db::category_info(cat).0,
    };
    format!("{brand} · {kind}")
}
