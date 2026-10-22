//! Кто и чем занимает службу: программа (по процессу соединения и её ярлыку
//! `.desktop`), её работы (генерация, картинка, граф…) с ходом и скоростью,
//! история за 5 минут. Работа живёт в потоке соединения: модели, которые она
//! занимает, и токены отмечаются через [`current`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use synaptixd::{Activity, ActivityState, AppInfo, AppUsage, Generated, MetricsSample};

const RECENT: usize = 24;
const HISTORY: usize = 300;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ── Программа по процессу ───────────────────────────────────────────────────

/// Ярлыки `.desktop`: имя исполняемого файла → (ярлык, имя, значок).
static DESKTOP: LazyLock<Mutex<HashMap<String, Option<(String, String, String)>>>> = LazyLock::new(Default::default);

fn desktop_dirs(uid: u32) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    unsafe {
        let pw = libc::getpwuid(uid);
        if !pw.is_null() {
            let home = std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
            v.push(std::path::Path::new(&home).join(".local/share/applications"));
        }
    }
    v.push("/usr/local/share/applications".into());
    v.push("/usr/share/applications".into());
    v
}

/// `Name[ru]` предпочтительнее `Name` — интерфейс оболочки русский.
fn parse_desktop(text: &str) -> Option<(String, String, String)> {
    let (mut name, mut name_ru, mut icon, mut exec) = (None, None, String::new(), String::new());
    let mut main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        if !main {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            name = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("Name[ru]=") {
            name_ru = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("Icon=") {
            icon = v.to_string();
        } else if let Some(v) = line.strip_prefix("Exec=") {
            exec = v.to_string();
        }
    }
    let exe = exec.split_whitespace().find(|t| !t.contains('=') && *t != "env").unwrap_or("");
    let exe = exe.rsplit('/').next().unwrap_or(exe).to_string();
    Some((exe, name_ru.or(name)?, icon))
}

fn desktop_for(exe: &str, uid: u32) -> Option<(String, String, String)> {
    if let Some(v) = DESKTOP.lock().unwrap().get(exe) {
        return v.clone();
    }
    let mut found = None;
    'dirs: for dir in desktop_dirs(uid) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        // Ярлык с тем же именем — первым.
        files.sort_by_key(|p| p.file_stem().map(|s| s.to_string_lossy() != exe).unwrap_or(true));
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let Some((e, name, icon)) = parse_desktop(&text) else { continue };
            if e == exe {
                let id = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                found = Some((id, name, icon));
                break 'dirs;
            }
        }
    }
    DESKTOP.lock().unwrap().insert(exe.to_string(), found.clone());
    found
}

pub fn app_of(pid: i32, uid: u32) -> AppInfo {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .map(|n| n.trim_end_matches(" (deleted)").to_string())
        .or_else(|| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "?".into());
    let (id, name, icon) = desktop_for(&exe, uid).unwrap_or_else(|| (exe.clone(), exe.clone(), String::new()));
    AppInfo { pid: pid.max(0) as u32, uid, name, icon, id, label: String::new() }
}

// ── Работы ──────────────────────────────────────────────────────────────────

pub struct Live {
    pub id: u64,
    app: AppInfo,
    started: Instant,
    state: Mutex<Activity>,
    /// Начало ответа и сколько токенов было на прошлом замере истории.
    first_token: Mutex<Option<Instant>>,
    sampled_tokens: AtomicU64,
    cancel: Mutex<Vec<Box<dyn Fn() + Send>>>,
    cancelled: AtomicBool,
}

#[derive(Default)]
struct Totals {
    requests: u64,
    tokens: u64,
    last_ms: i64,
}

#[derive(Default)]
struct Registry {
    running: BTreeMap<u64, Arc<Live>>,
    recent: VecDeque<Activity>,
    history: VecDeque<MetricsSample>,
    totals: HashMap<(u32, String), (AppInfo, Totals)>,
    /// Соединения: номер → программа, открытые ею модели.
    conns: HashMap<u64, (AppInfo, Vec<String>)>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);
static REG: LazyLock<Mutex<Registry>> = LazyLock::new(Default::default);

thread_local! {
    static CURRENT: RefCell<Option<Arc<Live>>> = const { RefCell::new(None) };
    /// Чей запрос обслуживает поток соединения: «программа (роль), pid».
    static REQUESTER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Запомнить, чей запрос сейчас обслуживает поток (для журнала загрузок).
pub fn set_requester(app: &AppInfo, op: &str) {
    let who = if app.label.is_empty() { app.name.clone() } else { format!("{} ({})", app.name, app.label) };
    REQUESTER.with(|r| *r.borrow_mut() = Some(format!("«{who}», pid {}, {op}", app.pid)));
}

/// Чей запрос обслуживает поток, если это поток соединения.
pub fn requester() -> Option<String> {
    REQUESTER.with(|r| r.borrow().clone())
}

/// Работа этого потока (соединения), если идёт.
pub fn current() -> Option<Arc<Live>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Работа идёт, пока жив страж; по его концу — в недавние.
pub struct Guard(Arc<Live>);

pub fn begin(app: &AppInfo, op: &str) -> Guard {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let a = Activity { id, op: op.into(), app: app.clone(), started_ms: now_ms(), ..Default::default() };
    let live = Arc::new(Live {
        id,
        app: app.clone(),
        started: Instant::now(),
        state: Mutex::new(a),
        first_token: Mutex::new(None),
        sampled_tokens: AtomicU64::new(0),
        cancel: Mutex::new(Vec::new()),
        cancelled: AtomicBool::new(false),
    });
    REG.lock().unwrap().running.insert(id, live.clone());
    CURRENT.with(|c| *c.borrow_mut() = Some(live.clone()));
    Guard(live)
}

impl Guard {
    pub fn fail(&self, e: &str) {
        let mut a = self.0.state.lock().unwrap();
        a.error = Some(e.to_string());
        a.state = ActivityState::Failed;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
        let mut a = self.0.state.lock().unwrap().clone();
        a.finished_ms = Some(now_ms());
        if self.0.cancelled.load(Ordering::Relaxed) {
            a.state = ActivityState::Cancelled;
        } else if !matches!(a.state, ActivityState::Failed | ActivityState::Cancelled) {
            a.state = ActivityState::Done;
        }
        let mut r = REG.lock().unwrap();
        r.running.remove(&self.0.id);
        let key = (a.app.pid, a.app.label.clone());
        let t = &mut r.totals.entry(key).or_insert_with(|| (a.app.clone(), Totals::default())).1;
        t.requests += 1;
        t.tokens += a.tokens as u64;
        t.last_ms = now_ms();
        // Служебные запросы без моделей (список, состояние) в недавние не идут.
        if !a.models.is_empty() {
            r.recent.push_front(a);
            r.recent.truncate(RECENT);
        }
    }
}

impl Live {
    fn with(&self, f: impl FnOnce(&mut Activity)) {
        f(&mut self.state.lock().unwrap());
    }

    /// Работа заняла модель (ждёт её загрузки или устройства).
    pub fn model(&self, id: &str) {
        self.with(|a| {
            if !a.models.iter().any(|m| m == id) {
                a.models.push(id.to_string());
            }
        });
    }

    pub fn running(&self) {
        self.with(|a| a.state = ActivityState::Running);
    }

    pub fn waiting(&self) {
        self.with(|a| a.state = ActivityState::Waiting);
    }

    pub fn token(&self) {
        let mut ft = self.first_token.lock().unwrap();
        let first = *ft.get_or_insert_with(Instant::now);
        drop(ft);
        self.with(|a| {
            a.tokens += 1;
            let secs = first.elapsed().as_secs_f32();
            if a.tokens > 1 && secs > 0.05 {
                a.tps = (a.tokens - 1) as f32 / secs;
            }
            if a.prefill_ms == 0 {
                a.prefill_ms = self.started.elapsed().as_millis() as u64;
            }
        });
    }

    pub fn progress(&self, step: usize, total: usize) {
        self.with(|a| a.step = Some((step, total)));
    }

    /// Итог хода языковой модели.
    pub fn generated(&self, g: &Generated) {
        self.with(|a| {
            a.tokens = g.tokens as u32;
            a.prompt_tokens = g.prompt_tokens as u32;
            a.cached_tokens = g.cached_tokens as u32;
            a.context = g.context as u32;
            a.prefill_ms = g.prefill_ms;
            a.decode_ms = g.decode_ms;
            if g.decode_ms > 0 && g.tokens > 1 {
                a.tps = (g.tokens - 1) as f32 * 1000.0 / g.decode_ms as f32;
            }
        });
    }

    /// Как остановить работу (флаг генерации, отмена графа).
    pub fn on_cancel(&self, f: impl Fn() + Send + 'static) {
        self.cancel.lock().unwrap().push(Box::new(f));
    }

    /// Флаг отмены генерации.
    pub fn cancel_flag(&self, flag: Arc<AtomicBool>) {
        self.on_cancel(move || flag.store(true, Ordering::SeqCst));
    }

    pub fn app(&self) -> &AppInfo {
        &self.app
    }
}

/// Остановить работу: свою — любой программе того же пользователя, чужую —
/// администратору.
pub fn cancel(id: u64, uid: u32, admin: bool) -> Result<(), String> {
    let live = REG.lock().unwrap().running.get(&id).cloned().ok_or("работа уже закончилась")?;
    if !admin && live.app.uid != uid {
        return Err("чужую работу останавливают root и группа wheel".into());
    }
    let fs = live.cancel.lock().unwrap();
    if fs.is_empty() {
        return Err("эту работу остановить нельзя".into());
    }
    live.cancelled.store(true, Ordering::SeqCst);
    for f in fs.iter() {
        f();
    }
    Ok(())
}

// ── Соединения ──────────────────────────────────────────────────────────────

pub fn connect(conn: u64, app: &AppInfo) {
    REG.lock().unwrap().conns.insert(conn, (app.clone(), Vec::new()));
}

pub fn relabel(conn: u64, app: &AppInfo) {
    if let Some(c) = REG.lock().unwrap().conns.get_mut(&conn) {
        c.0 = app.clone();
    }
}

pub fn opened(conn: u64, models: &[String]) {
    if let Some(c) = REG.lock().unwrap().conns.get_mut(&conn) {
        c.1 = models.to_vec();
    }
}

pub fn disconnect(conn: u64) {
    REG.lock().unwrap().conns.remove(&conn);
}

// ── Снимок ──────────────────────────────────────────────────────────────────

pub struct Snapshot {
    pub running: Vec<Activity>,
    pub recent: Vec<Activity>,
    pub apps: Vec<AppUsage>,
    pub history: Vec<MetricsSample>,
}

pub fn snapshot(history: bool) -> Snapshot {
    let r = REG.lock().unwrap();
    let mut running: Vec<Activity> = r.running.values().map(|l| l.state.lock().unwrap().clone()).collect();
    running.sort_by_key(|a| a.started_ms);
    let mut apps: Vec<AppUsage> = Vec::new();
    fn usage(apps: &mut Vec<AppUsage>, app: &AppInfo) -> usize {
        match apps.iter().position(|u| u.app.pid == app.pid && u.app.label == app.label) {
            Some(i) => i,
            None => {
                apps.push(AppUsage { app: app.clone(), ..Default::default() });
                apps.len() - 1
            }
        }
    }
    for (app, models) in r.conns.values() {
        let i = usage(&mut apps, app);
        apps[i].connections += 1;
        for m in models {
            if !apps[i].models.contains(m) {
                apps[i].models.push(m.clone());
            }
        }
    }
    for a in &running {
        let i = usage(&mut apps, &a.app);
        for m in &a.models {
            if !apps[i].models.contains(m) {
                apps[i].models.push(m.clone());
            }
        }
    }
    for (app, t) in r.totals.values() {
        // Ушедшие программы — пока их процесс жив.
        if !std::path::Path::new(&format!("/proc/{}", app.pid)).exists() {
            continue;
        }
        let i = usage(&mut apps, app);
        apps[i].requests = t.requests;
        apps[i].tokens = t.tokens;
        apps[i].last_ms = t.last_ms;
    }
    // Соединение без моделей и работ (запрос состояния, подписка) — не потребитель.
    apps.retain(|u| u.requests > 0 || !u.models.is_empty());
    apps.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.app.name.cmp(&b.app.name)));
    Snapshot {
        running,
        recent: r.recent.iter().cloned().collect(),
        apps,
        history: if history { r.history.iter().cloned().collect() } else { Vec::new() },
    }
}

/// Раз в секунду — точка истории; `mem` — (RAM, видеопамять) моделей.
pub fn sampler(mem: impl Fn() -> (u64, u64) + Send + 'static) {
    std::thread::Builder::new()
        .name("metrics".into())
        .spawn(move || {
            let mut last = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let dt = last.elapsed().as_secs_f32().max(0.001);
                last = Instant::now();
                let (ram, vram) = mem();
                let mut r = REG.lock().unwrap();
                let (mut tokens, mut running, mut waiting) = (0u64, 0u32, 0u32);
                for l in r.running.values() {
                    let a = l.state.lock().unwrap();
                    let now = a.tokens as u64;
                    let before = l.sampled_tokens.swap(now, Ordering::Relaxed);
                    tokens += now.saturating_sub(before);
                    match a.state {
                        ActivityState::Waiting => waiting += 1,
                        _ => running += 1,
                    }
                }
                r.history.push_back(MetricsSample { t_ms: now_ms(), tps: tokens as f32 / dt, running, waiting, ram, vram });
                while r.history.len() > HISTORY {
                    r.history.pop_front();
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry() {
        let t = "[Desktop Entry]\nName=Synthos\nName[ru]=Синтоз\nIcon=synthos\nExec=env FOO=1 /usr/bin/synthos %U\n[Desktop Action x]\nName=Другое\n";
        assert_eq!(parse_desktop(t), Some(("synthos".into(), "Синтоз".into(), "synthos".into())));
    }

    #[test]
    fn activity_lifecycle() {
        let app = AppInfo { pid: std::process::id(), name: "t".into(), id: "t".into(), ..Default::default() };
        {
            let g = begin(&app, "generate");
            let l = current().unwrap();
            l.model("m");
            l.running();
            l.token();
            assert_eq!(snapshot(false).running.iter().find(|a| a.id == l.id).map(|a| a.tokens), Some(1));
            drop(g);
        }
        assert!(current().is_none());
        let s = snapshot(false);
        assert!(s.recent.iter().any(|a| a.op == "generate" && a.state == ActivityState::Done));
        assert!(s.apps.iter().any(|u| u.app.pid == app.pid && u.requests >= 1));
    }
}
