//! События хуков Claude Code (Stop, Notification) → уведомление пользователю:
//! имя сеанса — по его номеру (`claude agents --json`), текст — последний ответ
//! Claude из транскрипта или сообщение уведомления. События телефонного
//! сеанса пропускаются: его ответы и так уходят в мессенджер.

use std::sync::Arc;

use crate::hub::Hub;

pub async fn handle(hub: Arc<Hub>, ev: serde_json::Value) {
    let get = |k: &str| ev.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let session = get("session_id");
    if hub.claude.status().session_id.as_deref() == Some(session.as_str()) {
        return;
    }
    let body = match get("hook_event_name").as_str() {
        "Stop" => {
            let last = if ev.get("last_assistant_message").and_then(|v| v.as_str()).is_some() { get("last_assistant_message") } else { last_assistant_text(&get("transcript_path")) };
            let t = if last.trim().is_empty() { "Готово.".to_string() } else { last };
            format!("✅ {}", trim_chars(&t, 700))
        }
        "Notification" => format!("❓ {}", get("message")),
        _ => return,
    };
    let program = hub.config().claude.program;
    let name = session_name(&program, &session).await.unwrap_or_else(|| {
        std::path::Path::new(&get("cwd")).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Claude".into())
    });
    hub.notify(&name, &body, "claude", "").await;
}

/// Имя сеанса по его номеру.
async fn session_name(program: &str, id: &str) -> Option<String> {
    let out = tokio::process::Command::new(crate::claude::resolve_program(program)).args(["agents", "--json"]).output().await.ok()?;
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    v.iter().find(|a| a.get("sessionId").and_then(|s| s.as_str()) == Some(id)).and_then(|a| a.get("name")?.as_str().map(str::to_string))
}

/// Последний текст Claude из транскрипта (JSONL).
pub fn last_assistant_text(path: &str) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(parts) = v.pointer("/message/content").and_then(|c| c.as_array()) else { continue };
        let t: Vec<&str> = parts.iter().filter(|p| p.get("type").and_then(|x| x.as_str()) == Some("text")).filter_map(|p| p.get("text")?.as_str()).collect();
        if !t.is_empty() {
            return t.join("\n");
        }
    }
    String::new()
}

fn trim_chars(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!("{}…", s.chars().take(n).collect::<String>().trim_end())
}

#[cfg(test)]
mod tests {
    #[test]
    fn transcript_tail() {
        let p = std::env::temp_dir().join(format!("courier-tr-{}.jsonl", std::process::id()));
        std::fs::write(&p, "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"раньше\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"итог\"}]}}\n{\"type\":\"user\"}\n").unwrap();
        assert_eq!(super::last_assistant_text(p.to_str().unwrap()), "итог");
        let _ = std::fs::remove_file(p);
        assert_eq!(super::trim_chars("абвгд", 3), "абв…");
    }
}
