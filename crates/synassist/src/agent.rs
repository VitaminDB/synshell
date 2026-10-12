use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;
use synaptixd::{Event, Generated, Message, Params};

use crate::chat::{Chat, ChatMsg, Role};
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
}

const TOOL_RESULT_LIMIT: usize = 12_000;

impl Agent {
    pub fn new(backend: Arc<dyn Backend>, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self { backend, tools, max_steps: 12, auto_approve: false }
    }

    pub fn history(&self, chat: &Chat) -> Vec<Message> {
        let mut out = vec![Message::system(crate::prompt::build(&self.tools))];
        for m in &chat.messages {
            match m.role {
                Role::User => out.push(Message::user(m.text.clone())),
                Role::Assistant => {
                    let mut msg = Message::assistant(m.text.clone());
                    msg.tool_calls = m.tool_calls.clone();
                    out.push(msg);
                }
                Role::Tool => {
                    let name = m.tool.as_ref().map(|t| t.name.as_str()).unwrap_or("tool");
                    out.push(Message::tool(name, crate::tools::truncate(&m.text, TOOL_RESULT_LIMIT)));
                }
            }
        }
        out
    }

    fn find(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name)
    }

    pub fn run_turn(&self, chat: &mut Chat, user_text: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
        let user = ChatMsg::user(user_text);
        chat.messages.push(user.clone());
        on(AgentEvent::Message(user));
        self.continue_turn(chat, confirm, cancel, on)
    }

    pub fn continue_turn(&self, chat: &mut Chat, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
        let schemas: Vec<Value> = self.tools.iter().map(|t| crate::tools::schema(t.as_ref())).collect();
        let mut params = Params::new();
        if chat.thinking {
            params = params.with("thinking", true);
        }
        for _ in 0..self.max_steps {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let history = self.history(chat);
            let out = self.backend.chat(&chat.model, history, schemas.clone(), &params, &mut |ev| match ev {
                Event::Token { text } => on(AgentEvent::Text(text)),
                Event::Thinking { text } => on(AgentEvent::Thinking(text)),
                Event::ToolText { text } => on(AgentEvent::ToolText(text)),
                _ => {}
            })?;
            let reply = ChatMsg::assistant(out.text.trim().to_string(), out.thinking.trim().to_string(), out.tool_calls.clone());
            chat.messages.push(reply.clone());
            chat.updated = crate::chat::now();
            on(AgentEvent::Message(reply));
            if out.tool_calls.is_empty() {
                break;
            }
            for call in out.tool_calls {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let msg = self.execute(&call.name, &call.arguments, confirm, on);
                chat.messages.push(msg.clone());
                on(AgentEvent::Message(msg));
            }
        }
        chat.updated = crate::chat::now();
        on(AgentEvent::Done);
        Ok(())
    }

    fn execute(&self, name: &str, arguments: &str, confirm: &dyn Confirm, on: &mut dyn FnMut(AgentEvent)) -> ChatMsg {
        let Some(tool) = self.find(name) else {
            return ChatMsg::tool(name, name.to_string(), false, format!("Нет инструмента «{name}». Доступны: {}", self.names()), Vec::new());
        };
        let args: Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(_) if arguments.trim().is_empty() => Value::Object(Default::default()),
            Err(e) => return ChatMsg::tool(name, name.to_string(), false, format!("Аргументы не JSON: {e}"), Vec::new()),
        };
        let summary = tool.summary(&args);
        on(AgentEvent::ToolStart { name: name.to_string(), summary: summary.clone() });
        if tool.danger(&args) == Danger::Confirm && !self.auto_approve && !confirm.confirm(name, &summary) {
            return ChatMsg::tool(name, summary, false, "Пользователь отклонил это действие. Не повторяй его без новой просьбы.".into(), Vec::new());
        }
        match tool.run(&args) {
            Ok(out) => ChatMsg::tool(name, summary, true, out.text, out.files),
            Err(e) => ChatMsg::tool(name, summary, false, format!("Ошибка: {e}"), Vec::new()),
        }
    }

    fn names(&self) -> String {
        self.tools.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
    }
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

    struct Script(Mutex<Vec<Generated>>);
    impl Backend for Script {
        fn chat(&self, _: &str, messages: Vec<Message>, tools: Vec<Value>, _: &Params, on: &mut dyn FnMut(Event)) -> Result<Generated, String> {
            assert_eq!(tools.len(), 1);
            assert_eq!(messages[0].role, "system");
            let g = self.0.lock().unwrap().remove(0);
            on(Event::Token { text: g.text.clone() });
            Ok(g)
        }
    }

    #[test]
    fn loop_runs_tool_and_answers() {
        let script = Script(Mutex::new(vec![
            Generated { text: String::new(), tokens: 1, thinking: String::new(), tool_calls: vec![synaptixd::ToolCall { name: "echo".into(), arguments: r#"{"text":"hi"}"#.into() }] },
            Generated { text: "Готово: HI".into(), tokens: 1, thinking: String::new(), tool_calls: Vec::new() },
        ]));
        let agent = Agent::new(Arc::new(script), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let mut events = Vec::new();
        let yes = |_: &str, _: &str| true;
        agent.run_turn(&mut chat, "скажи hi", &yes, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
        let roles: Vec<Role> = chat.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::Assistant]);
        assert_eq!(chat.messages[2].text, "HI");
        assert_eq!(chat.messages[3].text, "Готово: HI");
        assert!(events.contains(&AgentEvent::Done));
    }

    #[test]
    fn declined_tool_is_reported() {
        let script = Script(Mutex::new(vec![
            Generated { text: String::new(), tokens: 1, thinking: String::new(), tool_calls: vec![synaptixd::ToolCall { name: "echo".into(), arguments: "{}".into() }] },
            Generated { text: "Хорошо".into(), tokens: 1, thinking: String::new(), tool_calls: Vec::new() },
        ]));
        let agent = Agent::new(Arc::new(script), vec![Arc::new(Echo)]);
        let mut chat = Chat::new();
        let no = |_: &str, _: &str| false;
        agent.run_turn(&mut chat, "x", &no, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(!chat.messages[2].tool.as_ref().unwrap().ok);
    }
}
