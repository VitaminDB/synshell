//! Автономный режим ассистента (`[assistant.autonomy]`, выключен по
//! умолчанию): раз в `interval_min` минут, если компьютер простаивает,
//! ассистент берёт одну открытую задачу со своей доски (`board`) в основном
//! хранилище заметок и выполняет её.
//!
//! Простой — это всё сразу:
//! - нет ввода `idle_min` минут, ничто не держит экран (видео, игра —
//!   idle-inhibit) и нет окна во весь экран;
//! - видеокарта загружена не больше `max_gpu_percent`;
//! - служба нейросетей ничего не исполняет;
//! - не запущена ни одна программа из `busy_apps`.
//!
//! Задача — карточка не в колонке «Готово», без срока или со сроком не
//! позже сегодня. Перед запуском (`ask`: `always`; `new` — у задачи, которую
//! ещё не разрешали; `never`) приходит уведомление «Выполнить?» с кнопками
//! «Выполнить», «Не сейчас» (до завтра), «Не делать» (метка `не-делать` на
//! карточке). Действия с подтверждением спрашиваются уведомлением, без
//! ответа за 10 минут — отказ. Итог дописывается в карточку, она уходит в
//! «Готово» (повторяющаяся порождает следующую); разговор сохраняется в
//! чатах ассистента — уведомление о результате открывает его.
//!
//! Отметки (разрешённые задачи, отложенные, неудачи) —
//! `~/.local/state/synassist/autonomy.json`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use synassist::chat::{Chat, Store};
use synshell_common::config::Autonomy;
use synshell_common::ipc::{Client, Request, Response};
use syngui::async_runtime::run_on_main_thread;
use syngui::t;

use crate::ShellCtx;

/// Метка карточки: ассистент её не трогает.
const SKIP_TAG: &str = "не-делать";

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Задачи, которые пользователь уже разрешил (`ask = "new"`).
    approved: Vec<String>,
    /// Отложено до (unix-секунды).
    snoozed: BTreeMap<String, i64>,
    /// Неудачные попытки: задача не берётся сутки.
    failed: BTreeMap<String, i64>,
}

fn state_path() -> std::path::PathBuf {
    synshell_common::paths::state_home().join("synassist/autonomy.json")
}

fn load() -> State {
    std::fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(s: &State) {
    let p = state_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_string_pretty(s).unwrap_or_default());
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Идёт задача (одна за раз).
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    if ctx.is_phone() {
        return;
    }
    std::thread::Builder::new()
        .name("autonomy".into())
        .spawn(move || {
            let mut last = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let cfg = synshell_common::Config::load().0.assistant.autonomy;
                if !cfg.enabled || RUNNING.load(Ordering::Relaxed) {
                    continue;
                }
                if last.elapsed() < Duration::from_secs(cfg.interval_min.max(1) as u64 * 60) {
                    continue;
                }
                last = Instant::now();
                match quiet(&cfg) {
                    Ok(()) => tick(ctx, &cfg),
                    Err(why) => log::debug!("автономный режим: такт пропущен — {why}"),
                }
            }
        })
        .ok();
}

/// Простаивает ли компьютер. `Err` — почему нет.
fn quiet(cfg: &Autonomy) -> Result<(), String> {
    let idle = match Client::connect().and_then(|mut c| c.request(&Request::Idle)) {
        Ok(Response::Idle { idle }) => idle,
        Ok(_) | Err(_) => return Err("композитор не ответил о простое".into()),
    };
    if idle.inhibited {
        return Err("экран удерживает программа".into());
    }
    if idle.fullscreen && !idle.locked {
        return Err("окно во весь экран".into());
    }
    if !idle.locked && idle.seconds < cfg.idle_min as u64 * 60 {
        return Err(format!("ввод был {} с назад", idle.seconds));
    }
    let gpus = synsystem::gpu::devices(&synsystem::Sys::host());
    if let Some(g) = gpus.iter().filter(|g| !g.asleep).find(|g| g.busy_percent.is_some_and(|b| b > cfg.max_gpu_percent as f32)) {
        return Err(format!("{} загружена на {:.0}%", g.name, g.busy_percent.unwrap_or(0.0)));
    }
    if let Ok(m) = synaptixd::Client::connect().and_then(|mut c| c.metrics(false)) {
        if !m.running.is_empty() {
            return Err("служба нейросетей занята".into());
        }
    }
    if !cfg.busy_apps.is_empty() {
        if let Some(app) = running_app(&cfg.busy_apps) {
            return Err(format!("запущена {app}"));
        }
    }
    Ok(())
}

fn running_app(names: &[String]) -> Option<String> {
    let dir = std::fs::read_dir("/proc").ok()?;
    for e in dir.flatten() {
        if !e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        let comm = comm.trim();
        if let Some(n) = names.iter().find(|n| n.trim().eq_ignore_ascii_case(comm)) {
            return Some(n.clone());
        }
    }
    None
}

/// Задача с доски ассистента.
#[derive(Clone, Debug)]
struct Task {
    board: String,
    card: String,
    title: String,
    md: String,
    done_column: Option<String>,
}

/// Первая открытая задача, которую можно брать сейчас.
fn next_task(cfg: &Autonomy, st: &State) -> Option<Task> {
    let data = syn_notes_proto::desk(None).ok()?;
    let want = cfg.board.trim().to_lowercase();
    let today = today_iso();
    let now = now_secs();
    for b in data.get("boards")?.as_array()? {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if s(b, "page_title").to_lowercase() != want && s(b, "id") != want {
            continue;
        }
        let cols = b.get("columns")?.as_array()?;
        let done_column = cols.iter().find(|c| c.get("done").and_then(Value::as_bool) == Some(true)).map(|c| s(c, "id"));
        for c in cols.iter().filter(|c| c.get("done").and_then(Value::as_bool) != Some(true)) {
            for card in c.get("cards").and_then(Value::as_array).cloned().unwrap_or_default() {
                let id = s(&card, "id");
                let tags: Vec<String> = card.get("tags").and_then(Value::as_array).map(|t| t.iter().filter_map(|x| x.as_str().map(str::to_lowercase)).collect()).unwrap_or_default();
                if tags.iter().any(|t| t == SKIP_TAG) {
                    continue;
                }
                let due = s(&card, "due");
                if !due.is_empty() && due.as_str() > today.as_str() {
                    continue;
                }
                if st.snoozed.get(&id).is_some_and(|t| *t > now) || st.failed.get(&id).is_some_and(|t| now - t < 86_400) {
                    continue;
                }
                return Some(Task { board: s(b, "id"), card: id, title: s(&card, "title"), md: s(&card, "md"), done_column });
            }
        }
    }
    None
}

fn today_iso() -> String {
    let t = now_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

fn tick(ctx: ShellCtx, cfg: &Autonomy) {
    let st = load();
    let Some(task) = next_task(cfg, &st) else { return };
    let ask = match cfg.ask.as_str() {
        "never" => false,
        "always" => true,
        _ => !st.approved.contains(&task.card),
    };
    if !ask {
        run(ctx, task);
        return;
    }
    run_on_main_thread(move || {
        let t = task.clone();
        crate::notifications::local_actions(
            ctx,
            "autonomy-ask",
            &t!("Ассистент: выполнить задачу?"),
            &task.title,
            "dialog-question",
            &[("default", &t!("Выполнить")), ("run", &t!("Выполнить")), ("later", &t!("Не сейчас")), ("never", &t!("Не делать"))],
            move |action| {
                let t = t.clone();
                let mut st = load();
                match action {
                    "run" | "default" => {
                        if !st.approved.contains(&t.card) {
                            st.approved.push(t.card.clone());
                        }
                        save(&st);
                        run(ctx, t);
                    }
                    "later" => {
                        st.snoozed.insert(t.card.clone(), now_secs() + 20 * 3600);
                        save(&st);
                    }
                    "never" => {
                        let args = json!({"action": "kanban", "op": "update_card", "board": t.board, "card": t.card, "tags": format!("+{SKIP_TAG}")});
                        std::thread::spawn(move || {
                            if let Err(e) = syn_notes_proto::tool(&args) {
                                log::warn!("автономный режим: метку не поставить: {e}");
                            }
                        });
                    }
                    _ => {}
                }
            },
        );
    });
}

/// Выполнить задачу агентом в фоне.
fn run(ctx: ShellCtx, task: Task) {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("autonomy-task".into())
        .spawn(move || {
            let cfg = synshell_common::Config::load().0.assistant;
            let agent = crate::assistant::build_agent(&cfg);
            let store = Store::open();
            let mut chat = Chat::new();
            chat.title = format!("{} {}", t!("Автономно:"), task.title);
            let prompt = format!(
                "{}\n\nЗадача: «{}»\nДоска: {} · карточка: {}\n\n{}",
                t!("Ты работаешь в автономном режиме, пока пользователь отошёл. Выполни задачу с доски ассистента в заметках (инструмент notes) и в конце кратко отчитайся, что сделано. Карточку сам не закрывай и не переноси — это сделает планировщик."),
                task.title,
                task.board,
                task.card,
                if task.md.trim().is_empty() { String::new() } else { format!("Описание:\n{}", task.md.trim()) },
            );
            let confirm = move |_: &str, summary: &str| -> bool { ask_permission(ctx, summary) };
            let cancel = Arc::new(AtomicBool::new(false));
            let from = chat.messages.len();
            let r = synaptixd::with_label(&t!("Ассистент · автономно"), || agent.run_turn(&mut chat, &prompt, &confirm, &cancel, &mut |_| {}));
            let _ = store.save(&chat);
            let answer = synassist::prompt::final_answer(&chat.messages[from.min(chat.messages.len())..]);
            finish(ctx, &task, &chat.id, r.map(|_| answer));
            RUNNING.store(false, Ordering::Release);
        })
        .map_err(|_| RUNNING.store(false, Ordering::Release))
        .ok();
}

/// Подтверждение опасного действия — уведомлением; без ответа 10 минут — нет.
fn ask_permission(ctx: ShellCtx, summary: &str) -> bool {
    let (tx, rx) = channel::<bool>();
    let s = summary.to_string();
    run_on_main_thread(move || {
        let tx = std::sync::Mutex::new(Some(tx));
        crate::notifications::local_actions(
            ctx,
            "autonomy-confirm",
            &t!("Ассистент просит разрешения"),
            &s,
            "dialog-warning",
            &[("allow", &t!("Разрешить")), ("deny", &t!("Запретить"))],
            move |action| {
                if let Some(tx) = tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(action == "allow");
                }
            },
        );
    });
    rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false)
}

/// Итог: дописать в карточку, закрыть её, сообщить.
fn finish(ctx: ShellCtx, task: &Task, chat_id: &str, result: Result<String, String>) {
    let stamp = today_iso();
    let (ok, text) = match &result {
        Ok(a) => (true, a.trim().to_string()),
        Err(e) => (false, e.clone()),
    };
    let report = format!(
        "{}\n\n---\n**{} {stamp}:** {}",
        task.md.trim(),
        if ok { t!("Ассистент выполнил") } else { t!("Ассистент не справился") },
        if text.is_empty() { t!("без отчёта") } else { text.clone() }
    );
    let mut update = json!({"action": "kanban", "op": "update_card", "board": task.board, "card": task.card, "md": report.trim()});
    if ok {
        if let Some(col) = &task.done_column {
            update["column"] = json!(col);
        }
    }
    if let Err(e) = syn_notes_proto::tool(&update) {
        log::warn!("автономный режим: карточку не обновить: {e}");
    }
    if !ok {
        let mut st = load();
        st.failed.insert(task.card.clone(), now_secs());
        save(&st);
    }
    let title = task.title.clone();
    let chat = chat_id.to_string();
    let summary = if ok { t!("Ассистент выполнил задачу") } else { t!("Ассистент не справился с задачей") };
    let body = format!("{title}\n{}", text.chars().take(240).collect::<String>());
    run_on_main_thread(move || {
        crate::notifications::local_actions(ctx, "autonomy-done", &summary, &body, "dialog-information", &[("default", &t!("Открыть чат")), ("chat", &t!("Открыть чат"))], move |_| {
            let _ = std::process::Command::new("synassistant").arg("--chat").arg(&chat).spawn();
        });
    });
}
