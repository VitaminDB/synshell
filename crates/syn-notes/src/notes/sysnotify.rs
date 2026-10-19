//! Напоминания заметок в системных уведомлениях (`org.freedesktop.Notifications`
//! оболочки) с кнопками: «Открыть» — страница в окне заметок, «Готово» —
//! карточка уходит в колонку «Готово» своей доски, «Через час» — то же
//! уведомление ещё раз через час.
//!
//! Шина сеанса и подписка на `ActionInvoked` живут в своём потоке; действия
//! исполняются на главном потоке в живом состоянии приложения — так же,
//! как правки пользователя.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use super::reminders::{headline, Reminder, ReminderKind};
use super::NotesCtx;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// Больше за раз — одним общим уведомлением «и ещё N».
const MAX_SEPARATE: usize = 4;

/// Что показать: проект и напоминание (или сводка «ещё N»).
struct Item {
    project: PathBuf,
    reminder: Option<Reminder>,
    summary: String,
    body: String,
}

static TX: OnceLock<Mutex<Option<Sender<Item>>>> = OnceLock::new();

/// Поток уведомлений: соединение с шиной, показ и ответы на кнопки.
fn sender() -> Option<Sender<Item>> {
    let slot = TX.get_or_init(|| Mutex::new(start()));
    slot.lock().ok().and_then(|g| g.clone())
}

fn start() -> Option<Sender<Item>> {
    let conn = Connection::session().map_err(|e| tracing::warn!("уведомления заметок: D-Bus: {e}")).ok()?;
    // Есть ли кому показывать: без сервера уведомлений — тосты окна.
    let has_server = conn
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(DEST,))
        .ok()
        .and_then(|r| r.body().deserialize::<bool>().ok())
        .unwrap_or(false);
    if !has_server {
        return None;
    }
    let shown: std::sync::Arc<Mutex<HashMap<u32, (PathBuf, Reminder)>>> = Default::default();
    listen(conn.clone(), shown.clone());
    let (tx, rx) = channel::<Item>();
    std::thread::Builder::new()
        .name("notes-notify".into())
        .spawn(move || {
            for item in rx {
                let mut actions: Vec<String> = Vec::new();
                if let Some(r) = &item.reminder {
                    if r.page.is_some() {
                        actions.extend(["default".into(), syngui::tr!("notes.notify.open")]);
                    }
                    if r.card.is_some() {
                        actions.extend(["done".into(), syngui::tr!("notes.notify.done")]);
                    }
                    actions.extend(["snooze".into(), syngui::tr!("notes.notify.snooze")]);
                } else {
                    actions.extend(["default".into(), syngui::tr!("notes.notify.open")]);
                }
                let actions: Vec<&str> = actions.iter().map(String::as_str).collect();
                let urgency: u8 = match item.reminder.as_ref().map(|r| r.kind) {
                    Some(ReminderKind::Overdue) => 2,
                    _ => 1,
                };
                let mut hints: HashMap<&str, Value> = HashMap::new();
                hints.insert("desktop-entry", Value::from("syn-notes"));
                hints.insert("urgency", Value::from(urgency));
                let reply = conn.call_method(
                    Some(DEST),
                    PATH,
                    Some(DEST),
                    "Notify",
                    &(syngui::tr!("notes.title"), 0u32, "syn-notes", item.summary.as_str(), item.body.as_str(), actions, hints, -1i32),
                );
                match reply.and_then(|r| r.body().deserialize::<u32>().map_err(Into::into)) {
                    Ok(id) => {
                        tracing::debug!("уведомление {id}: {}", item.summary);
                        let r = item.reminder.unwrap_or_else(|| summary_reminder());
                        if let Ok(mut m) = shown.lock() {
                            m.insert(id, (item.project, r));
                        }
                    }
                    Err(e) => tracing::warn!("уведомление заметок: {e}"),
                }
            }
        })
        .ok()?;
    Some(tx)
}

/// Заглушка для сводки «и ещё N»: её «Открыть» показывает окно заметок.
fn summary_reminder() -> Reminder {
    Reminder {
        key: String::new(),
        kind: ReminderKind::DueToday,
        title: String::new(),
        subtitle: String::new(),
        page: None,
        card: None,
        day: 0,
        at: 0,
    }
}

/// Ответы на кнопки: `ActionInvoked(id, ключ)` нашего уведомления.
fn listen(conn: Connection, shown: std::sync::Arc<Mutex<HashMap<u32, (PathBuf, Reminder)>>>) {
    std::thread::Builder::new()
        .name("notes-notify-actions".into())
        .spawn(move || {
            let rule = "type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'";
            let Ok(rule) = zbus::MatchRule::try_from(rule) else { return };
            let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None) else { return };
            for msg in it.flatten() {
                let Ok((id, action)) = msg.body().deserialize::<(u32, String)>() else { continue };
                let Some((project, r)) = shown.lock().ok().and_then(|mut m| m.remove(&id)) else { continue };
                run_on_main_thread(move || act(&action, project, r));
            }
        })
        .ok();
}

/// Кнопка уведомления — на главном потоке.
fn act(action: &str, project: PathBuf, r: Reminder) {
    let ctx = use_context::<NotesCtx>();
    match action {
        "done" => {
            let Some((board, card)) = r.card.clone() else { return };
            let column = ctx
                .with_project(&project, |c| match c.object("kanban", &board) {
                    Some(super::LiveObject::Kanban { handle, .. }) => handle.lock().done_column().map(|col| col.id.clone()),
                    _ => None,
                })
                .flatten();
            let Some(column) = column else {
                tracing::warn!("уведомление: у доски {board} нет колонки «Готово»");
                return;
            };
            let args = json!({
                "action": "kanban", "op": "move_card", "project": project.display().to_string(),
                "board": board, "card": card, "column": column,
            });
            if let Err(e) = crate::tool::dispatch_now(&args) {
                tracing::warn!("уведомление: «Готово» не вышло: {e}");
            }
        }
        "snooze" => {
            syngui::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                run_on_main_thread(move || {
                    show(&[(project, r)]);
                });
            });
        }
        _ => {
            if ctx.project_path.get_untracked() != project {
                ctx.switch_project(&project);
            }
            if let Some(page) = &r.page {
                ctx.activate(page);
            }
            syngui::app::show_main_window();
        }
    }
}

/// Показать напоминания уведомлениями. `false` — системных уведомлений нет.
pub fn show(batch: &[(PathBuf, Reminder)]) -> bool {
    let Some(tx) = sender() else { return false };
    let send = |p: &PathBuf, r: &Reminder| {
        let _ = tx.send(Item { project: p.clone(), reminder: Some(r.clone()), summary: headline(r), body: r.subtitle.clone() });
    };
    if batch.len() <= MAX_SEPARATE {
        batch.iter().for_each(|(p, r)| send(p, r));
    } else {
        batch.iter().take(MAX_SEPARATE - 1).for_each(|(p, r)| send(p, r));
        let rest: Vec<String> = batch.iter().skip(MAX_SEPARATE - 1).map(|(_, r)| format!("• {}", r.title)).collect();
        let _ = tx.send(Item {
            project: batch[0].0.clone(),
            reminder: None,
            summary: syngui::tr!("notes.reminders.toast.more", n = batch.len() - (MAX_SEPARATE - 1)),
            body: rest.join("\n"),
        });
    }
    true
}
