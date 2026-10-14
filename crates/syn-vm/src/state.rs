//! Состояние окна: список машин, их работа (опрос раз в секунду), открытые экраны,
//! мастер создания, уведомления. Долгие действия (запуск, снимки, копии) идут в
//! фоновых потоках и возвращаются через сигналы.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::prelude::*;
use syngui::widgets::LiveFrame;

use crate::model::{self, Vm};
use crate::qmp::{self, Snapshot};
use crate::qemu;
use crate::vnc::{Note, Session};

/// Значки Material Icons.
pub mod icons {
    pub const PLAY: &str = "\u{E037}";
    pub const PAUSE: &str = "\u{E034}";
    pub const STOP: &str = "\u{E047}";
    pub const POWER: &str = "\u{E8AC}";
    pub const RESTART: &str = "\u{F053}";
    pub const ADD: &str = "\u{E145}";
    pub const COMPUTER: &str = "\u{E30A}";
    pub const MONITOR: &str = "\u{E30C}";
    pub const CAMERA: &str = "\u{E412}";
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const FULLSCREEN_EXIT: &str = "\u{E5D1}";
    pub const KEYBOARD: &str = "\u{E312}";
    pub const PASTE: &str = "\u{E14F}";
    pub const DELETE: &str = "\u{E872}";
    pub const COPY: &str = "\u{E14D}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const DISC: &str = "\u{E019}";
    pub const STORAGE: &str = "\u{E1DB}";
    pub const MEMORY: &str = "\u{E322}";
    pub const CPU: &str = "\u{E30D}";
    pub const LAN: &str = "\u{EB2F}";
    pub const EJECT: &str = "\u{E8FB}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const BACK: &str = "\u{E5C4}";
    pub const MORE: &str = "\u{E5D4}";
    pub const CHECK: &str = "\u{E5CA}";
    pub const HISTORY: &str = "\u{E889}";
    pub const RESTORE: &str = "\u{E8B3}";
    pub const INFO: &str = "\u{E88E}";
    pub const WARNING: &str = "\u{E002}";
    pub const ERROR: &str = "\u{E000}";
    pub const TERMINAL: &str = "\u{EB8E}";
    pub const SCHEDULE: &str = "\u{E8B5}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const SECURITY: &str = "\u{E32A}";
    pub const FIT: &str = "\u{EA10}";
    pub const ANDROID: &str = "\u{E859}";
    pub const WINDOW: &str = "\u{F088}";
    pub const APPS: &str = "\u{E5C3}";
    pub const BOLT: &str = "\u{EA0B}";
    pub const FOLDER_OPEN: &str = "\u{E2C8}";
    pub const CHECK_CIRCLE: &str = "\u{E86C}";
    pub const TUNE: &str = "\u{E429}";
    pub const DASHBOARD: &str = "\u{E871}";
    pub const EXPAND: &str = "\u{E5CF}";
    pub const COLLAPSE: &str = "\u{E5CE}";
    pub const DNS: &str = "\u{E875}";
    pub const SPARK: &str = "\u{E65F}";
    pub const DEVICES: &str = "\u{E1B1}";
    pub const SYNC: &str = "\u{E627}";
    pub const LAYERS: &str = "\u{E53B}";
    pub const HOURGLASS: &str = "\u{E88B}";
}

/// Состояние машины.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Power {
    #[default]
    Off,
    Running,
    Paused,
    /// Процесс есть, QMP ещё не отвечает (только запустилась или занята).
    Busy,
}

impl Power {
    pub fn on(self) -> bool {
        self != Power::Off
    }

    pub fn title(self) -> &'static str {
        match self {
            Power::Off => n_!("Выключена"),
            Power::Running => n_!("Работает"),
            Power::Paused => n_!("Пауза"),
            Power::Busy => n_!("Запускается"),
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            Power::Off => "off",
            Power::Running => "on",
            Power::Paused => "paused",
            Power::Busy => "busy",
        }
    }
}

/// История для графиков: последние 120 секунд.
pub const HIST: usize = 120;

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Live {
    pub power: Power,
    /// Загрузка процессора в процентах от выделенных ядер.
    pub cpu: f32,
    pub rss: u64,
    pub disk_rate: f64,
    pub since: u64,
    pub cpu_hist: Vec<f32>,
    pub mem_hist: Vec<f32>,
}

/// Вкладки машины.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Screen,
    Overview,
    Settings,
    Snapshots,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Screen, Tab::Overview, Tab::Settings, Tab::Snapshots];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Screen => n_!("Экран"),
            Tab::Overview => n_!("Обзор"),
            Tab::Settings => n_!("Настройки"),
            Tab::Snapshots => n_!("Снимки"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::Screen => icons::MONITOR,
            Tab::Overview => icons::DASHBOARD,
            Tab::Settings => icons::TUNE,
            Tab::Snapshots => icons::HISTORY,
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        Some(match s {
            "screen" | "console" | "display" => Tab::Screen,
            "overview" | "info" => Tab::Overview,
            "settings" | "config" => Tab::Settings,
            "snapshots" | "snapshot" => Tab::Snapshots,
            _ => return None,
        })
    }
}

/// Откуда ставить систему в мастере.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    #[default]
    Iso,
    Disk,
    Empty,
}

/// Черновик новой машины.
#[derive(Clone, PartialEq, Debug)]
pub struct Draft {
    pub step: usize,
    pub source: Source,
    /// Образ установки или существующий диск.
    pub path: String,
    pub vm: Vm,
    /// Имя задал пользователь — не подменять по образу.
    pub named: bool,
    pub start: bool,
    pub advanced: bool,
}

impl Default for Draft {
    fn default() -> Self {
        let mut vm = model::OsKind::Linux.preset();
        vm.name = String::new();
        Draft { step: 0, source: Source::Iso, path: String::new(), vm, named: false, start: true, advanced: false }
    }
}

/// Подтверждение опасного действия.
#[derive(Clone, PartialEq, Debug)]
pub enum Confirm {
    Delete(String),
    ForceOff(String),
    Restore(String, String),
    DeleteSnapshot(String, String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub seq: u64,
}

#[derive(Clone, Copy)]
pub struct St {
    pub vms: RwSignal<Vec<Vm>>,
    pub live: RwSignal<HashMap<String, Live>>,
    /// Открытая машина; `None` — библиотека.
    pub sel: RwSignal<Option<String>>,
    pub tab: RwSignal<Tab>,
    pub draft: RwSignal<Option<Draft>>,
    /// Шаг мастера (`None` — закрыт): раскладка мастера зависит только от него,
    /// поля черновика правятся без пересборки (не теряют фокус).
    pub wiz: RwSignal<Option<usize>>,
    pub confirm: RwSignal<Option<Confirm>>,
    pub toast: RwSignal<Option<Toast>>,
    pub query: RwSignal<String>,
    /// Машины, над которыми идёт действие (запуск, снимок, копия): id → подпись.
    pub busy: RwSignal<HashMap<String, String>>,
    /// Новый кадр экрана открытой машины.
    pub frame_tick: RwSignal<u64>,
    /// Новые миниатюры.
    pub thumb_tick: RwSignal<u64>,
    pub snapshots: RwSignal<Vec<Snapshot>>,
    pub snap_name: RwSignal<String>,
    /// Экран подключён (у открытой машины).
    pub connected: RwSignal<bool>,
    /// Размер экрана гостя.
    pub guest_size: RwSignal<(u32, u32)>,
    pub fullscreen: RwSignal<bool>,
    /// Меню «ещё» на панели экрана.
    pub keys_menu: RwSignal<bool>,
    /// Показать полную командную строку QEMU.
    pub show_cmd: RwSignal<bool>,
    /// Размер области экрана (для разрешения гостя).
    pub view_size: RwSignal<(u32, u32)>,
}

impl St {
    pub fn new() -> Self {
        St {
            vms: use_signal(Vec::new()),
            live: use_signal(HashMap::new()),
            sel: use_signal(None),
            tab: use_signal(Tab::Overview),
            draft: use_signal(None),
            wiz: use_signal(None),
            confirm: use_signal(None),
            toast: use_signal(None),
            query: use_signal(String::new()),
            busy: use_signal(HashMap::new()),
            frame_tick: use_signal(0),
            thumb_tick: use_signal(0),
            snapshots: use_signal(Vec::new()),
            snap_name: use_signal(String::new()),
            connected: use_signal(false),
            guest_size: use_signal((0, 0)),
            fullscreen: use_signal(false),
            keys_menu: use_signal(false),
            show_cmd: use_signal(false),
            view_size: use_signal((0, 0)),
        }
    }

    /// Машина по id (главный поток).
    pub fn vm(&self, id: &str) -> Option<Vm> {
        self.vms.get_untracked().into_iter().find(|v| v.id == id)
    }

    pub fn selected(&self) -> Option<Vm> {
        let id = self.sel.get_untracked()?;
        self.vm(&id)
    }

    pub fn power(&self, id: &str) -> Power {
        self.live.get_untracked().get(id).map(|l| l.power).unwrap_or_default()
    }
}

// ─── Общие данные для фоновых потоков ───────────────────────────────────

fn mirror() -> &'static Mutex<Vec<Vm>> {
    static M: OnceLock<Mutex<Vec<Vm>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Vec::new()))
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    static S: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn thumbs() -> &'static Mutex<HashMap<String, Arc<LiveFrame>>> {
    static T: OnceLock<Mutex<HashMap<String, Arc<LiveFrame>>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Перечитать машины с диска.
pub fn reload(st: St) {
    let all = model::load_all();
    *mirror().lock().unwrap() = all.clone();
    if let Some(id) = st.sel.get_untracked() {
        if !all.iter().any(|v| v.id == id) {
            st.sel.set(None);
        }
    }
    st.vms.set(all);
}

/// Сохранить изменённую машину и обновить список.
pub fn save_vm(st: St, vm: Vm) {
    if let Err(e) = vm.save() {
        toast_err(st, e.to_string());
        return;
    }
    {
        let mut m = mirror().lock().unwrap();
        if let Some(x) = m.iter_mut().find(|v| v.id == vm.id) {
            *x = vm.clone();
        }
    }
    let mut all = st.vms.get_untracked();
    if let Some(x) = all.iter_mut().find(|v| v.id == vm.id) {
        *x = vm;
    }
    st.vms.set(all);
}

/// Изменить открытую машину.
pub fn edit(st: St, f: impl FnOnce(&mut Vm)) {
    if let Some(mut vm) = st.selected() {
        let before = vm.clone();
        f(&mut vm);
        if vm != before {
            save_vm(st, vm);
        }
    }
}

pub fn toast(st: St, text: String) {
    show_toast(st, text, false);
}

pub fn toast_err(st: St, text: String) {
    show_toast(st, text, true);
}

fn show_toast(st: St, text: String, error: bool) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    st.toast.set(Some(Toast { text, error, seq }));
    let ms = if error { 7000 } else { 3500 };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        run_on_main_thread(move || {
            if st.toast.get_untracked().is_some_and(|t| t.seq == seq) {
                st.toast.set(None);
            }
        });
    });
}

fn set_busy(st: St, id: &str, label: Option<String>) {
    let id = id.to_string();
    run_on_main_thread(move || {
        let mut b = st.busy.get_untracked();
        match label {
            Some(l) => b.insert(id, l),
            None => b.remove(&id),
        };
        st.busy.set(b);
    });
}

/// Действие в фоне с подписью «занято» и уведомлением об ошибке.
pub fn background(st: St, id: &str, label: String, f: impl FnOnce() -> anyhow::Result<Option<String>> + Send + 'static) {
    if st.busy.get_untracked().contains_key(id) {
        return;
    }
    set_busy(st, id, Some(label));
    let id = id.to_string();
    std::thread::spawn(move || {
        let r = f();
        set_busy(st, &id, None);
        run_on_main_thread(move || match r {
            Ok(Some(msg)) => toast(st, msg),
            Ok(None) => {}
            Err(e) => toast_err(st, e.to_string()),
        });
    });
}

// ─── Питание ────────────────────────────────────────────────────────────

pub fn start(st: St, id: &str) {
    let Some(mut vm) = st.vm(id) else { return };
    vm.last_run = model::now();
    let _ = vm.save();
    save_vm(st, vm.clone());
    let id2 = id.to_string();
    background(st, id, t!("Запуск…"), move || {
        qemu::start(&vm)?;
        let vm2 = vm.clone();
        std::thread::spawn(move || qemu::after_boot(&vm2));
        let id = id2.clone();
        run_on_main_thread(move || {
            mark_power(st, &id, Power::Busy);
            if st.sel.get_untracked().as_deref() == Some(id.as_str()) {
                st.tab.set(Tab::Screen);
                attach(st, &id);
            }
        });
        Ok(None)
    });
}

/// Мягкое выключение (ACPI): гость сам завершает работу.
pub fn shutdown(st: St, id: &str) {
    let Some(vm) = st.vm(id) else { return };
    std::thread::spawn(move || {
        if let Err(e) = qmp::powerdown(&vm) {
            run_on_main_thread(move || toast_err(st, e.to_string()));
        } else {
            run_on_main_thread(move || toast(st, t!("Машине отправлен сигнал выключения")));
        }
    });
}

pub fn force_off(st: St, id: &str) {
    let Some(vm) = st.vm(id) else { return };
    std::thread::spawn(move || {
        if qmp::quit(&vm).is_err() {
            qemu::kill(&vm);
        }
    });
}

pub fn reset(st: St, id: &str) {
    let Some(vm) = st.vm(id) else { return };
    std::thread::spawn(move || {
        if let Err(e) = qmp::reset(&vm) {
            run_on_main_thread(move || toast_err(st, e.to_string()));
        }
    });
}

pub fn toggle_pause(st: St, id: &str) {
    let Some(vm) = st.vm(id) else { return };
    let paused = st.power(id) == Power::Paused;
    mark_power(st, id, if paused { Power::Running } else { Power::Paused });
    std::thread::spawn(move || {
        let r = if paused { qmp::resume(&vm) } else { qmp::pause(&vm) };
        if let Err(e) = r {
            run_on_main_thread(move || toast_err(st, e.to_string()));
        }
    });
}

fn mark_power(st: St, id: &str, p: Power) {
    let mut live = st.live.get_untracked();
    let l = live.entry(id.to_string()).or_default();
    if l.power != p {
        l.power = p;
        st.live.set(live);
    }
}

/// Вставить образ в привод: на работающей — сразу, всегда — в настройки.
pub fn set_cdrom(st: St, id: &str, iso: String) {
    let Some(mut vm) = st.vm(id) else { return };
    vm.cdrom = iso.clone();
    save_vm(st, vm.clone());
    if st.power(id).on() {
        std::thread::spawn(move || {
            if let Err(e) = qmp::change_cdrom(&vm, &iso) {
                run_on_main_thread(move || toast_err(st, e.to_string()));
            }
        });
    }
}

// ─── Экран ──────────────────────────────────────────────────────────────

/// Сеанс экрана машины (если подключён).
pub fn session(id: &str) -> Option<Arc<Session>> {
    sessions().lock().unwrap().get(id).filter(|s| s.alive()).cloned()
}

/// Подключить экран открытой машины (если она работает).
pub fn attach(st: St, id: &str) {
    if session(id).is_some() {
        st.connected.set(true);
        return;
    }
    let Some(vm) = st.vm(id) else { return };
    if !st.power(id).on() && qemu::pid(&vm).is_none() {
        return;
    }
    let id_s = id.to_string();
    let on_frame: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        run_on_main_thread(move || st.frame_tick.update(|t| *t = t.wrapping_add(1)));
    });
    let id_n = id_s.clone();
    let on_note = Arc::new(move |n: Note| {
        let id = id_n.clone();
        run_on_main_thread(move || on_note(st, &id, n));
    });
    let s = Session::open(vm.vnc_path(), Duration::from_secs(15), on_frame, on_note);
    sessions().lock().unwrap().insert(id_s, s);
}

fn on_note(st: St, id: &str, n: Note) {
    let current = st.sel.get_untracked().as_deref() == Some(id);
    match n {
        Note::Connected { width, height } | Note::Resized { width, height } => {
            if current {
                st.connected.set(true);
                st.guest_size.set((width, height));
                fit_guest(st);
            }
        }
        Note::Disconnected => {
            sessions().lock().unwrap().remove(id);
            if current {
                st.connected.set(false);
            }
        }
        Note::Clipboard(text) => {
            if !text.is_empty() {
                syngui::clipboard::copy(&text);
            }
        }
    }
}

/// Попросить гостя подстроить разрешение под область экрана.
pub fn fit_guest(st: St) {
    let Some(vm) = st.selected() else { return };
    if !vm.auto_resize {
        return;
    }
    let (w, h) = st.view_size.get_untracked();
    if w < 320 || h < 240 {
        return;
    }
    if let Some(s) = session(&vm.id) {
        s.request_size(w, h);
    }
}

/// Отдать гостю текст буфера обмена хоста (он вставит его своей вставкой).
pub fn offer_clipboard(id: &str) {
    if let (Some(s), Some(text)) = (session(id), syngui::clipboard::paste()) {
        if !text.is_empty() {
            s.offer_clipboard(text);
        }
    }
}

/// Миниатюра машины: экран сеанса, свежий кадр работающей или последний сохранённый.
pub fn thumb(vm: &Vm) -> Option<Arc<LiveFrame>> {
    if let Some(s) = session(&vm.id) {
        if s.frame.size().0 > 0 {
            return Some(s.frame.clone());
        }
    }
    let mut t = thumbs().lock().unwrap();
    if let Some(f) = t.get(&vm.id) {
        return (f.size().0 > 0).then(|| f.clone());
    }
    let f = LiveFrame::new();
    load_png(&f, &vm.screen_path());
    t.insert(vm.id.clone(), f.clone());
    (f.size().0 > 0).then_some(f)
}

fn load_png(frame: &LiveFrame, path: &std::path::Path) -> bool {
    let Ok(img) = image::open(path) else { return false };
    let img = if img.width() > 960 { img.resize(960, 960 * img.height() / img.width().max(1), image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    frame.set(rgba.width(), rgba.height(), Arc::from(rgba.into_raw()));
    true
}

/// Снимок экрана в «Изображения».
pub fn screenshot(st: St, id: &str) {
    let Some(vm) = st.vm(id) else { return };
    std::thread::spawn(move || {
        let dir = synshell_common::paths::user_dir_or_default("PICTURES").join(t!("Снимки экрана"));
        let _ = std::fs::create_dir_all(&dir);
        let stamp = chrono_like(model::now());
        let out = dir.join(format!("{} {stamp}.png", vm.name.replace('/', "-")));
        let r = qmp::screendump(&vm, &out);
        run_on_main_thread(move || match r {
            Ok(()) => toast(st, t!("Снимок сохранён: {p}", p = out.display())),
            Err(e) => toast_err(st, e.to_string()),
        });
    });
}

/// «ГГГГ-ММ-ДД ЧЧ.ММ.СС» по местному времени.
pub fn chrono_like(t: u64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as libc::time_t;
    unsafe { libc::localtime_r(&tt, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}.{:02}.{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ─── Снимки ─────────────────────────────────────────────────────────────

/// Перечитать снимки открытой машины.
pub fn load_snapshots(st: St) {
    let Some(vm) = st.selected() else { return };
    std::thread::spawn(move || {
        let on = qemu::pid(&vm).is_some();
        let list = list_snapshots(&vm, on);
        run_on_main_thread(move || {
            if st.sel.get_untracked().as_deref() == Some(vm.id.as_str()) {
                st.snapshots.set(list);
            }
        });
    });
}

fn list_snapshots(vm: &Vm, on: bool) -> Vec<Snapshot> {
    if on {
        return qmp::hmp(vm, "info snapshots").map(|s| qmp::parse_snapshots(&s)).unwrap_or_default();
    }
    let Some(d) = vm.disks.first() else { return Vec::new() };
    let out = std::process::Command::new("qemu-img").args(["snapshot", "-l", "-U"]).arg(vm.disk_path(d)).output();
    out.map(|o| qmp::parse_snapshots(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// Действие со снимком: `create`, `restore`, `delete`.
pub fn snapshot_op(st: St, op: &'static str, name: String) {
    let Some(vm) = st.selected() else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let on = qemu::pid(&vm).is_some();
    let label = match op {
        "create" => t!("Сохраняется снимок…"),
        "restore" => t!("Восстановление…"),
        _ => t!("Удаление снимка…"),
    };
    let id = vm.id.clone();
    background(st, &id, label, move || {
        if on {
            let cmd = match op {
                "create" => "savevm",
                "restore" => "loadvm",
                _ => "delvm",
            };
            qmp::hmp(&vm, &format!("{cmd} \"{}\"", name.replace('"', "")))?;
        } else {
            let flag = match op {
                "create" => "-c",
                "restore" => "-a",
                _ => "-d",
            };
            let mut files: Vec<_> = vm.disks.iter().map(|d| vm.disk_path(d)).filter(|p| p.extension().is_some_and(|e| e == "qcow2")).collect();
            if vm.vars_path().exists() {
                files.push(vm.vars_path());
            }
            for f in files {
                let o = std::process::Command::new("qemu-img").args(["snapshot", flag, &name]).arg(&f).output()?;
                if !o.status.success() {
                    let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    // У переменных UEFI снимка может не быть (сделан до них) — не ошибка.
                    if f != vm.vars_path() {
                        anyhow::bail!("{err}");
                    }
                }
            }
        }
        let list = list_snapshots(&vm, on);
        run_on_main_thread(move || {
            st.snapshots.set(list);
            st.snap_name.set(String::new());
        });
        Ok(Some(match op {
            "create" => t!("Снимок «{n}» сохранён", n = name),
            "restore" => t!("Машина возвращена к снимку «{n}»", n = name),
            _ => t!("Снимок «{n}» удалён", n = name),
        }))
    });
}

// ─── Опрос ──────────────────────────────────────────────────────────────

/// Раз в секунду: какие машины работают, нагрузка, память; раз в 3 с — миниатюры.
pub fn start_poller(st: St) {
    std::thread::Builder::new()
        .name("vm-poll".into())
        .spawn(move || {
            let tck = qemu::clk_tck();
            let mut prev: HashMap<String, (Instant, qemu::ProcStat)> = HashMap::new();
            let mut hist: HashMap<String, (Vec<f32>, Vec<f32>)> = HashMap::new();
            let mut since: HashMap<String, u64> = HashMap::new();
            let mut tick = 0u64;
            loop {
                let vms = mirror().lock().unwrap().clone();
                let mut out: HashMap<String, Live> = HashMap::new();
                let mut thumbs_changed = false;
                for vm in &vms {
                    let Some(pid) = qemu::pid(vm) else {
                        prev.remove(&vm.id);
                        hist.remove(&vm.id);
                        since.remove(&vm.id);
                        continue;
                    };
                    let power = match qmp::status(vm).as_deref() {
                        Some("running") => Power::Running,
                        Some("paused" | "suspended" | "prelaunch" | "inmigrate" | "postmigrate" | "save-vm" | "restore-vm") => Power::Paused,
                        Some(_) => Power::Running,
                        None => Power::Busy,
                    };
                    let now = Instant::now();
                    let ps = qemu::proc_stat(pid).unwrap_or_default();
                    let (cpu, disk_rate) = match prev.get(&vm.id) {
                        Some((t, p)) => {
                            let dt = now.duration_since(*t).as_secs_f64().max(0.01);
                            let cpu = (ps.ticks.saturating_sub(p.ticks) as f64 / tck / dt * 100.0 / vm.cpus.max(1) as f64) as f32;
                            let io = (ps.read + ps.write).saturating_sub(p.read + p.write) as f64 / dt;
                            (cpu.clamp(0.0, 100.0), io)
                        }
                        None => (0.0, 0.0),
                    };
                    prev.insert(vm.id.clone(), (now, ps));
                    let h = hist.entry(vm.id.clone()).or_default();
                    h.0.push(cpu);
                    h.1.push(ps.rss as f32 / (vm.memory_mb.max(1) as f32 * 1048576.0) * 100.0);
                    for v in [&mut h.0, &mut h.1] {
                        if v.len() > HIST {
                            v.drain(..v.len() - HIST);
                        }
                    }
                    let s = *since.entry(vm.id.clone()).or_insert_with(|| process_start(pid).unwrap_or_else(model::now));
                    out.insert(vm.id.clone(), Live { power, cpu, rss: ps.rss, disk_rate, since: s, cpu_hist: h.0.clone(), mem_hist: h.1.clone() });

                    // Миниатюра: последний кадр сохраняется в папке машины.
                    if tick % 3 == 0 && power == Power::Running {
                        // Временный файл — рядом: rename из /run в домашний каталог не проходит (разные ФС).
                        let tmp = vm.dir().join("screen.part.png");
                        if qmp::screendump(vm, &tmp).is_ok() && std::fs::rename(&tmp, vm.screen_path()).is_ok() {
                            let f = thumbs().lock().unwrap().entry(vm.id.clone()).or_insert_with(LiveFrame::new).clone();
                            thumbs_changed |= load_png(&f, &vm.screen_path());
                        }
                    }
                }
                tick += 1;
                run_on_main_thread(move || {
                    let old = st.live.get_untracked();
                    // Машина выключилась — закрыть экран, перечитать миниатюру.
                    for (id, l) in &old {
                        if l.power.on() && !out.contains_key(id) {
                            if let Some(s) = sessions().lock().unwrap().remove(id) {
                                s.close();
                            }
                            if let Some(mut vm) = st.vm(id) {
                                if let Some(f) = thumbs().lock().unwrap().get(id) {
                                    load_png(f, &vm.screen_path());
                                }
                                // Система встала на диск — дальше грузиться с него.
                                if vm.boot_cdrom && vm.used_bytes() > 2 << 30 {
                                    vm.boot_cdrom = false;
                                    save_vm(st, vm.clone());
                                    toast(st, t!("«{n}»: система на диске — теперь загрузка с него, образ остаётся в приводе", n = vm.name));
                                }
                            }
                            if st.sel.get_untracked().as_deref() == Some(id.as_str()) {
                                st.connected.set(false);
                            }
                        }
                    }
                    // Открыта работающая машина без экрана — подключить.
                    if let Some(id) = st.sel.get_untracked() {
                        if out.get(&id).is_some_and(|l| l.power.on()) && session(&id).is_none() {
                            attach(st, &id);
                        }
                    }
                    if old != out {
                        st.live.set(out);
                    }
                    if thumbs_changed {
                        st.thumb_tick.update(|t| *t += 1);
                    }
                });
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .ok();
}

/// Время запуска процесса (Unix).
fn process_start(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let start_ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let btime: u64 = std::fs::read_to_string("/proc/stat").ok()?.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
    Some(btime + (start_ticks as f64 / qemu::clk_tck()) as u64)
}
