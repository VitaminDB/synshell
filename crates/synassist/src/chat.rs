use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use synaptixd::ToolCall;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
    Summary,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ToolRun {
    pub name: String,
    pub summary: String,
    pub ok: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ChatMsg {
    pub role: Role,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub time: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub covers: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl ChatMsg {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, text: text.into(), thinking: String::new(), tool_calls: Vec::new(), tool: None, files: Vec::new(), error: None, time: now(), compacted: false, covers: 0 }
    }

    pub fn assistant(text: impl Into<String>, thinking: String, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: Role::Assistant, text: text.into(), thinking, tool_calls, ..Self::user("") }
    }

    pub fn tool(name: &str, summary: String, ok: bool, text: String, files: Vec<PathBuf>) -> Self {
        Self { role: Role::Tool, text, tool: Some(ToolRun { name: name.into(), summary, ok }), files, ..Self::user("") }
    }

    pub fn summary(text: String, covers: usize) -> Self {
        Self { role: Role::Summary, text, covers, ..Self::user("") }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Chat {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub messages: Vec<ChatMsg>,
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub context: usize,
    /// Глубина размышлений (`low`, `medium`, `high`…; пусто — умолчание модели).
    #[serde(default)]
    pub reasoning: String,
    /// Последний вызов модели.
    #[serde(default)]
    pub last: Usage,
    /// Сгенерировано за весь чат, токенов.
    #[serde(default)]
    pub total_tokens: usize,
}

/// Расход одного вызова модели.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Usage {
    /// Токенов промпта.
    pub prompt: usize,
    /// Токенов ответа.
    pub tokens: usize,
    /// Промпта взято из KV-кэша.
    pub cached: usize,
    pub prefill_ms: u64,
    pub decode_ms: u64,
}

impl Usage {
    /// Скорость генерации, токенов в секунду.
    pub fn speed(&self) -> f32 {
        if self.decode_ms == 0 {
            0.0
        } else {
            self.tokens as f32 * 1000.0 / self.decode_ms as f32
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub updated: i64,
    pub preview: String,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Chat {
    pub fn new() -> Self {
        let t = now();
        Self { id: format!("{t:x}"), title: String::new(), created: t, updated: t, ..Default::default() }
    }

    pub fn display_title(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        self.messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| title_from(&m.text))
            .unwrap_or_else(|| "Новый чат".to_string())
    }
}

pub fn title_from(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut t: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        t.push('…');
    }
    if t.is_empty() {
        "Новый чат".into()
    } else {
        t
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("synassist")
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open() -> Self {
        Self::at(data_dir().join("chats"))
    }

    pub fn at(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }

    fn path(&self, id: &str) -> PathBuf {
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        self.dir.join(format!("{safe}.json"))
    }

    pub fn save(&self, chat: &Chat) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(&chat.id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(chat).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn load(&self, id: &str) -> Option<Chat> {
        std::fs::read(self.path(id)).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub fn delete(&self, id: &str) -> std::io::Result<()> {
        std::fs::remove_file(self.path(id))
    }

    pub fn list(&self) -> Vec<ChatSummary> {
        let mut out: Vec<ChatSummary> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .filter_map(|e| std::fs::read(e.path()).ok())
                    .filter_map(|b| serde_json::from_slice::<Chat>(&b).ok())
                    .map(|c| ChatSummary {
                        preview: c.messages.iter().rev().find(|m| m.role == Role::Assistant && !m.text.is_empty() && !m.compacted).map(|m| title_from(&m.text)).unwrap_or_default(),
                        title: c.display_title(),
                        updated: c.updated,
                        id: c.id,
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.updated.cmp(&a.updated));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        let mut c = Chat::new();
        c.messages.push(ChatMsg::user("Какая погода завтра в Алматы? Подробно, пожалуйста"));
        c.messages.push(ChatMsg::assistant("Завтра солнечно.", String::new(), Vec::new()));
        store.save(&c).unwrap();
        let back = store.load(&c.id).unwrap();
        assert_eq!(back, c);
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert!(list[0].title.starts_with("Какая погода"));
        assert_eq!(list[0].preview, "Завтра солнечно.");
        store.delete(&c.id).unwrap();
        assert!(store.list().is_empty());
    }
}
