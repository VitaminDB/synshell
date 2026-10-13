use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;
use synaptixd::{Event, Generated, Message, Params};

use crate::chat::{Chat, ChatMsg, Role};
use crate::compact::{self, Scope};
use crate::tools::{Danger, Tool};

pub trait Backend: Send + Sync {
    fn chat(&self, model: &str, messages: Vec<Message>, tools: Vec<Value>, params: &Params, on_event: &mut dyn FnMut(Event)) -> Result<Generated, String>;
}

pub struct DaemonBackend;

impl Backend for DaemonBackend {
    fn chat(&self, model: &str, messages: Vec<Message>, tools: Vec<Value>, params: &Params, on_event: &mut dyn FnMut(Event)) -> Result<Generated, String> {
        let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
        c.chat(model, messages, tools, params, |ev| on_event(ev)).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    Text(String),
    Thinking(String),
    ToolText(String),
    ToolStart { name: String, summary: String },
    Message(ChatMsg),
    Compacted(Vec<ChatMsg>),
    Done,
}

pub trait Confirm: Send + Sync {
    fn confirm(&self, tool: &str, summary: &str) -> bool;
}

impl<F: Fn(&str, &str) -> bool + Send + Sync> Confirm for F {
    fn confirm(&self, tool: &str, summary: &str) -> bool {
        self(tool, summary)
    }
}

pub struct Agent {
    pub backend: Arc<dyn Backend>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub max_steps: usize,
    pub auto_approve: bool,
    pub compact_percent: u32,
    pub subagent: bool,
    pub subagent_steps: usize,
    pub system: Option<String>,
    /// Каталог памяти (см. `memory`); `None` — без памяти.
    pub memory: Option<std::path::PathBuf>,
}

const TOOL_RESULT_LIMIT: usize = 12_000;
const SUBAGENT: &str = "subagent";
const LIMIT_PROMPT: &str = "Лимит шагов исчерпан. Больше не вызывай инструменты. Коротко подведи итог: что удалось сделать и узнать, что осталось нерешённым. Без вступления.";
const SUBAGENT_PROMPT: &str = "Ты — субагент: выполняешь одну узкую подзадачу инструментами и возвращаешь только краткий итог в 1–3 абзаца, без вступления. Истории основного разговора у тебя нет — опирайся только на текст задачи.";

impl Agent {
    pub fn new(backend: Arc<dyn Backend>, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self { backend, tools, max_steps: 24, auto_approve: false, compact_percent: 85, subagent: true, subagent_steps: 16, system: None, memory: None }
    }

    pub fn configured(backend: Arc<dyn Backend>, disabled: &[String], auto_approve: bool, autocompact: bool) -> Self {
        let mut a = Self::new(backend, crate::builtin::enabled(disabled));
        a.auto_approve = auto_approve;
        a.subagent = !disabled.iter().any(|d| d == SUBAGENT);
        if !autocompact {
            a.compact_percent = 0;
        }
        a
    }

    /// Память в каталоге `dir`: раздел в системном промпте и инструмент `memory`.
    pub fn with_memory(mut self, dir: std::path::PathBuf) -> Self {
        self.tools.push(Arc::new(crate::memory::MemoryTool { dir: dir.clone() }));
        self.memory = Some(dir);
        self
    }

    fn system_prompt(&self) -> String {
        let mut s = self.system.clone().unwrap_or_else(|| crate::prompt::build(&self.tools));
        if let Some(dir) = &self.memory {
            s.push_str(&crate::memory::prompt_section(dir));
        }
        s
    }

    pub fn history(&self, chat: &Chat) -> Vec<Message> {
        let mut system = self.system_prompt();
        let summaries: Vec<&str> = chat.messages.iter().filter(|m| m.role == Role::Summary && !m.compacted).map(|m| m.text.trim()).collect();
        if !summaries.is_empty() {
            system.push_str("\n\nСводка более ранней части разговора (исходные сообщения скрыты):\n");
            system.push_str(&summaries.join("\n\n"));
        }
        let mut out = vec![Message::system(system)];
        for m in chat.messages.iter().filter(|m| !m.compacted) {
            match m.role {
                Role::User => {
                    let (extra, media) = crate::attach::describe(&m.files);
                    let mut text = m.text.clone();
                    if !extra.is_empty() {
                        text.push_str("\n\n[Вложения пользователя]");
                        text.push_str(&extra);
                    }
                    let mut msg = Message::user(text);
                    msg.media = media;
                    out.push(msg);
                }
                Role::Assistant => {
                    let mut msg = Message::assistant(m.text.clone());
                    msg.tool_calls = m.tool_calls.clone();
                    out.push(msg);
                }
                Role::Tool => {
                    let name = m.tool.as_ref().map(|t| t.name.as_str()).unwrap_or("tool");
                    let mut msg = Message::tool(name, crate::tools::truncate(&m.text, TOOL_RESULT_LIMIT));
                    // Картинки результата (снимок экрана, сгенерированное) модель видит.
                    msg.media = crate::attach::media_of(&m.files);
                    out.push(msg);
                }
                Role::Summary => {}
            }
        }
        out
    }

    fn schemas(&self) -> Vec<Value> {
        let mut v: Vec<Value> = self.tools.iter().map(|t| crate::tools::schema(t.as_ref())).collect();
        if self.subagent {
            v.push(subagent_schema());
        }
        v
    }

    fn find(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name)
    }

    fn over_threshold(&self, chat: &Chat) -> bool {
        self.compact_percent > 0 && chat.context > 0 && chat.prompt_tokens * 100 >= chat.context * self.compact_percent as usize
    }

    pub fn run_turn(&self, chat: &mut Chat, user_text: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
        self.run_turn_files(chat, user_text, Vec::new(), confirm, cancel, on)
    }

    pub fn run_turn_files(
        &self,
        chat: &mut Chat,
        user_text: &str,
        files: Vec<PathBuf>,
        confirm: &dyn Confirm,
        cancel: &AtomicBool,
        on: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        let mut user = ChatMsg::user(user_text);
        user.files = files;
        chat.messages.push(user.clone());
        on(AgentEvent::Message(user));
        if self.over_threshold(chat) && self.compact(chat, Scope::BetweenTurns)? {
            on(AgentEvent::Compacted(chat.messages.clone()));
        }
        self.continue_turn(chat, confirm, cancel, on)
    }

    fn generate(&self, chat: &mut Chat, history: Vec<Message>, tools: Vec<Value>, on: &mut dyn FnMut(AgentEvent)) -> Result<Generated, String> {
        let mut params = Params::new();
        if chat.thinking {
            params = params.with("thinking", true);
        }
        let out = self.backend.chat(&chat.model, history, tools, &params, &mut |ev| match ev {
            Event::Token { text } => on(AgentEvent::Text(text)),
            Event::Thinking { text } => on(AgentEvent::Thinking(text)),
            Event::ToolText { text } => on(AgentEvent::ToolText(text)),
            _ => {}
        })?;
        if out.context > 0 {
            chat.prompt_tokens = out.prompt_tokens + out.tokens;
            chat.context = out.context;
        }
        Ok(out)
    }

    fn push_reply(&self, chat: &mut Chat, out: &Generated, on: &mut dyn FnMut(AgentEvent)) -> ChatMsg {
        let reply = ChatMsg::assistant(out.text.trim().to_string(), out.thinking.trim().to_string(), out.tool_calls.clone());
        chat.messages.push(reply.clone());
        chat.updated = crate::chat::now();
        on(AgentEvent::Message(reply.clone()));
        reply
    }

    pub fn continue_turn(&self, chat: &mut Chat, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
        let schemas = self.schemas();
        let mut pending_tools = false;
        for _ in 0..self.max_steps {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let history = self.history(chat);
            let out = self.generate(chat, history, schemas.clone(), on)?;
            self.push_reply(chat, &out, on);
            pending_tools = !out.tool_calls.is_empty();
            if !pending_tools {
                break;
            }
            for call in out.tool_calls {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let model = chat.model.clone();
                let msg = self.execute(&model, &call.name, &call.arguments, confirm, cancel, on);
                chat.messages.push(msg.clone());
                on(AgentEvent::Message(msg));
            }
            if self.over_threshold(chat) && self.compact(chat, Scope::InTurn { keep_tail: 2 })? {
                on(AgentEvent::Compacted(chat.messages.clone()));
            }
        }
        if pending_tools && !cancel.load(Ordering::Relaxed) {
            let mut history = self.history(chat);
            history.push(Message::user(LIMIT_PROMPT));
            let out = self.generate(chat, history, Vec::new(), on)?;
            self.push_reply(chat, &out, on);
        }
        chat.updated = crate::chat::now();
        on(AgentEvent::Done);
        Ok(())
    }

    pub fn compact(&self, chat: &mut Chat, scope: Scope) -> Result<bool, String> {
        let Some(range) = compact::find_range(&chat.messages, scope) else { return Ok(false) };
        let text = compact::serialize(&chat.messages, range.clone());
        let msgs = vec![Message::system(compact::SUMMARY_PROMPT), Message::user(text)];
        let params = Params::new().with("thinking", false).with("max_tokens", 1024i64);
        let out = self.backend.chat(&chat.model, msgs, Vec::new(), &params, &mut |_| {})?;
        let summary = out.text.trim().to_string();
        if summary.is_empty() {
            return Ok(false);
        }
        compact::apply(&mut chat.messages, range, summary);
        chat.prompt_tokens = 0;
        chat.updated = crate::chat::now();
        Ok(true)
    }

    fn tool_names(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self.tools.iter().map(|t| t.name()).collect();
        if self.subagent {
            v.push(SUBAGENT);
        }
        v
    }

    fn execute(&self, model: &str, raw_name: &str, arguments: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> ChatMsg {
        let names = self.tool_names();
        let name = crate::tools::args::canonical_name(raw_name, &names);
        if name == SUBAGENT && self.subagent {
            return self.execute_subagent(model, arguments, confirm, cancel, on);
        }
        let Some(tool) = self.find(name) else {
            return ChatMsg::tool(raw_name, raw_name.to_string(), false, format!("Нет инструмента «{raw_name}». Доступны: {}", names.join(", ")), Vec::new());
        };
        let args = match crate::tools::args::parse(arguments, &tool.parameters()) {
            Ok(v) => v,
            Err(e) => return ChatMsg::tool(name, name.to_string(), false, e, Vec::new()),
        };
        let summary = tool.summary(&args);
        on(AgentEvent::ToolStart { name: name.to_string(), summary: summary.clone() });
        if tool.danger(&args) == Danger::Confirm && !self.auto_approve && !confirm.confirm(name, &summary) {
            return ChatMsg::tool(name, summary, false, "Пользователь отклонил это действие. Не повторяй его без новой просьбы.".into(), Vec::new());
        }
        match tool.run_ctx(&args, cancel) {
            Ok(out) => ChatMsg::tool(name, summary, true, out.text, out.files),
            Err(e) => ChatMsg::tool(name, summary, false, format!("Ошибка: {e}"), Vec::new()),
        }
    }

    fn execute_subagent(&self, model: &str, arguments: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> ChatMsg {
        let args = match crate::tools::args::parse(arguments, &subagent_schema()["function"]["parameters"]) {
            Ok(v) => v,
            Err(e) => return ChatMsg::tool(SUBAGENT, SUBAGENT.into(), false, e, Vec::new()),
        };
        let Some(task) = crate::tools::str_arg(&args, "task") else {
            return ChatMsg::tool(SUBAGENT, SUBAGENT.into(), false, "нужен параметр «task»".into(), Vec::new());
        };
        let summary = format!("Субагент: {}", crate::chat::title_from(task));
        on(AgentEvent::ToolStart { name: SUBAGENT.into(), summary: summary.clone() });
        if !self.auto_approve && !confirm.confirm(SUBAGENT, &format!("Субагент выполнит задачу сам, без отдельных подтверждений:\n{task}")) {
            return ChatMsg::tool(SUBAGENT, summary, false, "Пользователь отклонил запуск субагента.".into(), Vec::new());
        }
        let allowed: Option<Vec<&str>> = args.get("tools").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect());
        let tools: Vec<Arc<dyn Tool>> = self.tools.iter().filter(|t| allowed.as_ref().is_none_or(|a| a.contains(&t.name()))).cloned().collect();
        let mut system = crate::prompt::build(&tools);
        system.push('\n');
        system.push_str(crate::tools::str_arg(&args, "system_prompt").unwrap_or(SUBAGENT_PROMPT));
        let child = Agent {
            backend: self.backend.clone(),
            tools,
            max_steps: self.subagent_steps,
            auto_approve: true,
            compact_percent: self.compact_percent,
            subagent: false,
            subagent_steps: 0,
            system: Some(system),
            memory: None,
        };
        let mut chat = Chat::new();
        chat.model = model.to_string();
        let r = child.run_turn(&mut chat, task, confirm, cancel, &mut |ev| {
            if let AgentEvent::ToolStart { summary: s, .. } = ev {
                on(AgentEvent::ToolStart { name: SUBAGENT.into(), summary: format!("↳ {s}") });
            }
        });
        let answer = chat.messages.iter().rev().find(|m| m.role == Role::Assistant && !m.text.trim().is_empty()).map(|m| m.text.clone());
        match (r, answer) {
            (Err(e), _) => ChatMsg::tool(SUBAGENT, summary, false, format!("Ошибка субагента: {e}"), Vec::new()),
            (Ok(()), Some(a)) => ChatMsg::tool(SUBAGENT, summary, true, a, Vec::new()),
            (Ok(()), None) => ChatMsg::tool(SUBAGENT, summary, false, "Субагент не вернул итог. Сформулируй задачу уже.".into(), Vec::new()),
        }
    }
}

pub fn is_text_file(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(
        ext.as_str(),
        "txt" | "md" | "markdown" | "rs" | "py" | "js" | "ts" | "json" | "toml" | "yaml" | "yml" | "xml" | "html" | "htm" | "css" | "c" | "h" | "cpp" | "hpp" | "sh" | "log" | "csv" | "ini" | "conf" | "cfg" | "sql" | "go" | "java" | "kt" | "lua" | "tex" | "svg" | "desktop" | "service"
    )
}

/// Текст реплики пользователя с описанием вложений (без медиа).
pub fn user_text(m: &ChatMsg) -> String {
    let (extra, _) = crate::attach::describe(&m.files);
    if extra.is_empty() {
        return m.text.clone();
    }
    format!("{}\n\n[Вложения пользователя]{extra}", m.text)
}

pub fn subagent_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": SUBAGENT,
            "description": "Поручить узкую подзадачу вложенному агенту, чтобы не раздувать свой контекст. Субагент сам вызывает инструменты и возвращает только краткий итог. Вызывай, когда нужно прочитать длинную страницу или несколько страниц ради 1–3 фактов, обойти файлы серией команд или когда подзадача требует 2+ вызовов инструментов. В task — только узкая самодостаточная подзадача (у субагента нет твоей истории), заканчивай просьбой вернуть итог в 1–3 абзацах. Внутри субагента инструменты выполняются без подтверждений — пользователь подтверждает только сам запуск.",
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "Узкая самодостаточная подзадача."},
                    "tools": {"type": "array", "items": {"type": "string"}, "description": "Необязательно: имена разрешённых инструментов, например [\"bash\", \"web\"]. По умолчанию — все, кроме subagent."},
                    "system_prompt": {"type": "string", "description": "Необязательная особая инструкция субагенту."}
                },
                "required": ["task"]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolOutput, ToolResult};
    use std::sync::Mutex;

    struct Echo;
    impl Tool for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn group(&self) -> &'static str {
            "test"
        }
        fn description(&self) -> String {
            "echo".into()
        }
        fn parameters(&self) -> Value {
            serde_json::json!({"type":"object","properties":{"text":{"type":"string"}}})
        }
        fn danger(&self, _: &Value) -> Danger {
            Danger::Confirm
        }
        fn summary(&self, args: &Value) -> String {
            format!("echo {}", args["text"])
        }
        fn run(&self, args: &Value) -> ToolResult {
            Ok(ToolOutput::text(args["text"].as_str().unwrap_or("").to_uppercase()))
        }
    }

    struct Script(Mutex<Vec<Generated>>, Mutex<Vec<(usize, Vec<Message>)>>);
    impl Script {
        fn new(v: Vec<Generated>) -> Self {
            Self(Mutex::new(v), Mutex::new(Vec::new()))
        }
    }
    impl Backend for Script {
        fn chat(&self, _: &str, messages: Vec<Message>, tools: Vec<Value>, _: &Params, on: &mut dyn FnMut(Event)) -> Result<Generated, String> {
            assert_eq!(messages[0].role, "system");
            self.1.lock().unwrap().push((tools.len(), messages));
            let g = self.0.lock().unwrap().remove(0);
            on(Event::Token { text: g.text.clone() });
            Ok(g)
        }
    }

    fn text(t: &str) -> Generated {
        Generated { text: t.into(), tokens: 1, ..Default::default() }
    }

    fn calls(name: &str, args: &str) -> Generated {
        Generated { tool_calls: vec![synaptixd::ToolCall { name: name.into(), arguments: args.into() }], tokens: 1, ..Default::default() }
    }

    #[test]
    fn loop_runs_tool_and_answers() {
        let script = Arc::new(Script::new(vec![calls("functions.echo", r#"{"text":" hi "}"#), text("Готово: HI")]));
        let agent = Agent::new(script.clone(), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let mut events = Vec::new();
        let yes = |_: &str, _: &str| true;
        agent.run_turn(&mut chat, "скажи hi", &yes, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
        let roles: Vec<Role> = chat.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::Assistant]);
        assert_eq!(chat.messages[2].text, "HI");
        assert_eq!(chat.messages[3].text, "Готово: HI");
        assert!(events.contains(&AgentEvent::Done));
        assert_eq!(script.1.lock().unwrap()[0].0, 2);
    }

    #[test]
    fn declined_tool_is_reported() {
        let agent = Agent::new(Arc::new(Script::new(vec![calls("echo", "{}"), text("Хорошо")])), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let no = |_: &str, _: &str| false;
        agent.run_turn(&mut chat, "x", &no, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(!chat.messages[2].tool.as_ref().unwrap().ok);
    }

    #[test]
    fn step_limit_ends_with_summary_turn() {
        let script = Arc::new(Script::new(vec![calls("echo", r#"{"text":"a"}"#), calls("echo", r#"{"text":"b"}"#), text("Итог")]));
        let mut agent = Agent::new(script.clone(), vec![Arc::new(Echo)]);
        agent.max_steps = 2;
        agent.auto_approve = true;
        let mut chat = Chat::new();
        agent.run_turn(&mut chat, "x", &|_: &str, _: &str| false, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(chat.messages.last().unwrap().text, "Итог");
        let log = script.1.lock().unwrap();
        assert_eq!(log[2].0, 0);
        assert_eq!(log[2].1.last().unwrap().content, LIMIT_PROMPT);
    }

    #[test]
    fn subagent_runs_child_loop_and_returns_summary() {
        let script = Arc::new(Script::new(vec![
            calls("subagent", r#"{"task":"узнай HI","tools":["echo"]}"#),
            calls("echo", r#"{"text":"hi"}"#),
            text("Нашёл: HI"),
            text("Субагент сказал HI"),
        ]));
        let agent = Agent::new(script.clone(), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let asked = Mutex::new(Vec::new());
        let confirm = |t: &str, _: &str| {
            asked.lock().unwrap().push(t.to_string());
            true
        };
        agent.run_turn(&mut chat, "x", &confirm, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(*asked.lock().unwrap(), ["subagent"]);
        let tool = &chat.messages[2];
        assert_eq!(tool.tool.as_ref().unwrap().name, "subagent");
        assert_eq!(tool.text, "Нашёл: HI");
        assert_eq!(chat.messages[3].text, "Субагент сказал HI");
        let log = script.1.lock().unwrap();
        assert_eq!(log[1].0, 1, "у субагента нет вложенного subagent");
        assert!(log[1].1[0].content.contains("субагент"));
    }

    #[test]
    fn compaction_folds_old_turns_into_system_prompt() {
        let full = |t: &str| Generated { text: t.into(), tokens: 10, prompt_tokens: 950, context: 1000, ..Default::default() };
        let script = Arc::new(Script::new(vec![text("a1"), full("a2"), text("СВОДКА"), text("a3")]));
        let agent = Agent::new(script.clone(), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let yes = |_: &str, _: &str| true;
        let cancel = AtomicBool::new(false);
        agent.run_turn(&mut chat, "q1", &yes, &cancel, &mut |_| {}).unwrap();
        agent.run_turn(&mut chat, "q2", &yes, &cancel, &mut |_| {}).unwrap();
        let mut compacted = false;
        agent.run_turn(&mut chat, "q3", &yes, &cancel, &mut |e| compacted |= matches!(e, AgentEvent::Compacted(_))).unwrap();
        assert!(compacted);
        let log = script.1.lock().unwrap();
        assert!(log[2].1[1].content.contains("User: q1") && log[2].1[1].content.contains("Assistant: a2"));
        let last = &log[3].1;
        assert!(last[0].content.contains("СВОДКА"));
        assert_eq!(last.iter().filter(|m| m.role == "user").map(|m| m.content.as_str()).collect::<Vec<_>>(), ["q3"]);
        assert_eq!(chat.messages.iter().filter(|m| m.compacted).count(), 4);
    }
}
