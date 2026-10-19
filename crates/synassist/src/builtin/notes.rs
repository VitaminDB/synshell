//! Инструмент `notes` — заметки пользователя в syn-notes: страницы, канбан,
//! Гант, mindmap, календарь, графики, повестка дня и журнал.
//!
//! Исполняет хозяин заметок (`syn-notes`) в живом состоянии приложения:
//! правки видны в открытом окне сразу, отменяются там Ctrl+Z и пишутся
//! автосохранением. Здесь — только передача вызова по сокету
//! ([`syn_notes_proto`]); не запущенный хозяин поднимается в фоне.

use serde_json::Value;

use crate::tools::{str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct NotesTool;

/// Установлены ли заметки (иначе инструмента нет).
pub fn available() -> bool {
    super::which("syn-notes") || syn_notes_proto::running()
}

impl Tool for NotesTool {
    fn name(&self) -> &'static str {
        syn_notes_proto::schema::KEY_NOTES
    }
    fn group(&self) -> &'static str {
        "заметки"
    }
    fn description(&self) -> String {
        syn_notes_proto::schema::DESCRIPTION.to_string()
    }
    fn parameters(&self) -> Value {
        syn_notes_proto::schema::notes_schema()
    }
    /// Удаление страниц, досок и карточек — с подтверждением.
    fn danger(&self, args: &Value) -> Danger {
        let action = str_arg(args, "action").unwrap_or_default();
        let op = str_arg(args, "op").unwrap_or_default();
        if action == "delete" || op == "delete" || op.starts_with("delete_") || op.starts_with("remove") {
            Danger::Confirm
        } else {
            Danger::Safe
        }
    }
    fn summary(&self, args: &Value) -> String {
        let action = str_arg(args, "action").unwrap_or("?");
        let mut s = format!("заметки: {action}");
        if let Some(op) = str_arg(args, "op") {
            s.push(' ');
            s.push_str(op);
        }
        for key in ["page", "title", "card", "query"] {
            if let Some(v) = str_arg(args, key) {
                s.push_str(&format!(" «{}»", v.chars().take(60).collect::<String>()));
                break;
            }
        }
        s
    }
    fn run(&self, args: &Value) -> ToolResult {
        syn_notes_proto::tool(args).map(ToolOutput::text)
    }
    fn pooled(&self) -> bool {
        true
    }
    fn when(&self) -> String {
        syn_notes_proto::schema::SUMMARY.to_string()
    }
}
