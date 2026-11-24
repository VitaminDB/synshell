//! Встроенный агент: каждая команда редактора — инструмент модели (схема из
//! таблицы команд). Один ход агента — одна группа отмены. Ядро — syn-assist,
//! модели — служба synaptixd.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use syn_assist::agent::{Agent, AgentEvent, Confirm, DaemonBackend};
use syn_assist::chat::Chat;
use syn_assist::tools::{Danger, Tool, ToolOutput, ToolResult};

use crate::cmd::{self, Command};
use crate::editor::Editor;

pub type Shared = Arc<Mutex<Editor>>;

const SYSTEM: &str = "Ты — монтажёр во встроенном ИИ-видеоредакторе syn-video-editor. У тебя полный доступ к редактору через инструменты: каждый инструмент — команда программы (то же, что делает пользователь мышью). Работай сам до готового результата: импортируй медиа, ставь клипы, режь, добавляй титры и эффекты. Пользователь потом посмотрит и поправит — не спрашивай разрешения на обычные правки, всё отменяется одним шагом.\n\nПравила:\n- Перед правкой смотри состояние: project_dump (таймлайн текстом, id: m — медиа, t — дорожки, c — клипы, e — эффекты).\n- Время — секунды числом или «м:сс.ммм».\n- Проект сохраняет программа сама. project_save с path и project_new/project_open — только если пользователь прямо просит другой файл или новый проект.\n- После серии правок проверь итог через project_dump и коротко отчитайся: что сделано, где результат.\n- Если чего-то сделать нельзя (нет инструмента) — так и скажи, не выдумывай.";

/// Инструмент-обёртка над командой.
struct CmdTool {
    cmd: &'static Command,
    name: &'static str,
    ed: Shared,
}

impl Tool for CmdTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn group(&self) -> &'static str {
        "editor"
    }
    fn description(&self) -> String {
        self.cmd.help.to_string()
    }
    fn parameters(&self) -> Value {
        self.cmd.schema()
    }
    fn danger(&self, _args: &Value) -> Danger {
        if self.cmd.danger {
            Danger::Confirm
        } else {
            Danger::Safe
        }
    }
    fn summary(&self, args: &Value) -> String {
        let a = args.as_object().filter(|o| !o.is_empty()).map(|o| Value::Object(o.clone()).to_string()).unwrap_or_default();
        format!("{} {a}", self.cmd.title)
    }
    fn run(&self, args: &Value) -> ToolResult {
        let mut ed = self.ed.lock().unwrap_or_else(|e| e.into_inner());
        cmd::run_cmd(&mut ed, self.cmd, args).map(|o| ToolOutput::text(o.text))
    }
}

pub fn tools(ed: &Shared) -> Vec<Arc<dyn Tool>> {
    cmd::all()
        .into_iter()
        .map(|c| {
            let name: &'static str = Box::leak(c.tool_name().into_boxed_str());
            Arc::new(CmdTool { cmd: c, name, ed: ed.clone() }) as Arc<dyn Tool>
        })
        .collect()
}

pub fn agent(ed: &Shared) -> Agent {
    let mut a = Agent::new(Arc::new(DaemonBackend), tools(ed));
    a.system = Some(SYSTEM.to_string());
    a.subagent = false;
    a.max_steps = 60;
    a
}

/// Подтверждение необратимого без окна: CLI — отказ, если не разрешено явно.
pub struct AutoConfirm(pub bool);

impl Confirm for AutoConfirm {
    fn confirm(&self, _tool: &str, _summary: &str) -> bool {
        self.0
    }
}

/// Ход агента: весь ход — одна группа отмены «ИИ: …».
pub fn run_turn(agent: &Agent, ed: &Shared, chat: &mut Chat, text: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
    let label: String = text.chars().take(40).collect();
    ed.lock().unwrap_or_else(|e| e.into_inner()).begin_group(&format!("ИИ: {label}"));
    let r = agent.run_turn(chat, text, confirm, cancel, on);
    ed.lock().unwrap_or_else(|e| e.into_inner()).end_group();
    r
}
