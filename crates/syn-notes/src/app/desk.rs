//! Сводка для рабочего стола (`{"cmd":"desk"}`): повестка дня, доски с
//! открытыми карточками и разделы страницы стола — из основного хранилища
//! (`[notes] storage`), без него — из показанного проекта.
//!
//! Ответ — данные, не разметка: оболочка рисует из них свои виджеты.
//!
//! ```text
//! {"ok":true,"project":"MyLife","path":"…",
//!  "agenda":[{"kind":"overdue|today|tomorrow|event|event_tomorrow","title","sub","page"}],
//!  "boards":[{"id","page","page_title","columns":[{"id","name","color","done","cards":[{"id","title","due","priority","overdue"}]}]}],
//!  "page":{"id","title","sections":[{"kind":"text","md"}|{"kind":"kanban","id"}|{"kind":"calendar"}]}}
//! ```

use serde_json::{json, Value};
use syngui::prelude::*;

use crate::notes::gantt::calendar::parse_days;
use crate::notes::reminders::{self, ReminderKind};
use crate::notes::state::{object_refs, LiveObject};
use crate::notes::NotesCtx;

/// Сводка на главном потоке. `page` — страница стола (название или id).
pub fn summary(page: Option<&str>) -> Value {
    let ctx = use_context::<NotesCtx>();
    let main = super::server::main_storage().map(|p| crate::notes::project::normalize_path(&p));
    let active = ctx.project_path.get_untracked();
    let build = |c: NotesCtx| collect(c, page);
    let out = match main {
        Some(path) if path != active => ctx.with_project(&path, build),
        _ if ctx.has_project() => Some(build(ctx)),
        _ => None,
    };
    out.unwrap_or_else(|| json!({"ok": false, "error": "no notes project"}))
}

fn collect(ctx: NotesCtx, page: Option<&str>) -> Value {
    let now = crate::time::local_now();
    let today = now.0;
    let agenda: Vec<Value> = reminders::collect(ctx, now)
        .into_iter()
        .map(|r| {
            let kind = match r.kind {
                ReminderKind::Overdue => "overdue",
                ReminderKind::DueToday => "today",
                ReminderKind::DueTomorrow => "tomorrow",
                ReminderKind::Event => "event",
                ReminderKind::EventTomorrow => "event_tomorrow",
            };
            json!({"kind": kind, "title": r.title, "sub": r.subtitle, "page": r.page})
        })
        .collect();

    let tree = ctx.tree.get_untracked();
    let mut boards = Vec::new();
    for pid in tree.all_ids() {
        for (kind, oid) in object_refs(&ctx.page_markdown(&pid)) {
            if kind != "kanban" {
                continue;
            }
            let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &oid) else { continue };
            let doc = handle.lock();
            let columns: Vec<Value> = doc
                .columns
                .iter()
                .map(|col| {
                    let cards: Vec<Value> = doc
                        .cards
                        .iter()
                        .filter(|c| c.column == col.id)
                        .map(|c| {
                            let title = if c.title.trim().is_empty() {
                                c.md.lines().next().unwrap_or_default().to_string()
                            } else {
                                c.title.clone()
                            };
                            let overdue = c.done.is_none() && c.due.as_deref().and_then(parse_days).is_some_and(|d| d < today);
                            json!({
                                "id": c.id, "title": title, "due": c.due, "md": c.md, "tags": c.tags,
                                "priority": c.priority.map(|p| format!("{p:?}").to_lowercase()),
                                "overdue": overdue,
                            })
                        })
                        .collect();
                    json!({"id": col.id, "name": col.name, "color": col.color, "done": col.done, "cards": cards})
                })
                .collect();
            boards.push(json!({
                "id": oid, "page": pid, "page_title": tree.title_of(&pid).unwrap_or_default(), "columns": columns,
            }));
        }
    }

    let page_json = page.and_then(|p| {
        let id = if tree.find(p).is_some() { Some(p.to_string()) } else { tree.resolve_title(p) }?;
        let md = crate::tool::md::plain_markdown(&ctx.page_markdown(&id));
        Some(json!({"id": id, "title": tree.title_of(&id).unwrap_or_default(), "sections": sections(&md)}))
    });

    json!({
        "ok": true,
        "project": crate::notes::project::project_title(&ctx.project_path.get_untracked()),
        "path": ctx.project_path.get_untracked().display().to_string(),
        "agenda": agenda,
        "boards": boards,
        "page": page_json,
    })
}

/// Разделы страницы для стола: текст режется по заголовкам первых двух
/// уровней, врезки объектов (`![[kanban:id]]`, календарь) — свои разделы.
fn sections(md: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Value>| {
        if !text.trim().is_empty() {
            out.push(json!({"kind": "text", "md": text.trim().to_string()}));
        }
        text.clear();
    };
    for line in md.lines() {
        let t = line.trim();
        if let Some(inner) = t.strip_prefix("![[").and_then(|r| r.split("]]").next()) {
            if let Some((kind, id)) = inner.split_once(':') {
                flush(&mut text, &mut out);
                match kind {
                    "kanban" => out.push(json!({"kind": "kanban", "id": id})),
                    "calendar" | "gantt" => out.push(json!({"kind": "calendar", "id": id})),
                    _ => {}
                }
                continue;
            }
        }
        if (t.starts_with("# ") || t.starts_with("## ")) && !text.trim().is_empty() {
            flush(&mut text, &mut out);
        }
        // Callout `> [!note] Заголовок` — обычная цитата: стол рисует markdown без расширений.
        let line = match line.find("[!") {
            Some(i) if line[..i].trim_start().starts_with('>') => match line[i..].find(']') {
                Some(j) => format!("{}{}", &line[..i], line[i + j + 1..].trim_start()),
                None => line.to_string(),
            },
            _ => line.to_string(),
        };
        text.push_str(&line);
        text.push('\n');
    }
    flush(&mut text, &mut out);
    out
}
