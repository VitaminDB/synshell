use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use crate::tools::bash::{self, DEFAULT_TIMEOUT_SECS};
use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct ShellTool;

impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn group(&self) -> &'static str {
        "терминал"
    }
    fn description(&self) -> String {
        format!("Выполнить команду через bash -lc от имени пользователя (рабочая папка — домашняя или cwd). Возвращает код выхода, stdout и stderr. timeout_sec — предел в секундах, по умолчанию {DEFAULT_TIMEOUT_SECS}; по истечении команда и её потомки убиваются. Долгие задачи запускай в фоне (`cmd > /tmp/log 2>&1 &`) и читай лог.")
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string"},
                "timeout_sec": {"type": "integer"}
            },
            "required": ["command"]
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Confirm
    }
    fn summary(&self, args: &Value) -> String {
        format!("$ {}", str_arg(args, "command").unwrap_or(""))
    }
    fn run(&self, args: &Value) -> ToolResult {
        self.run_ctx(args, &AtomicBool::new(false))
    }
    fn run_ctx(&self, args: &Value, cancel: &AtomicBool) -> ToolResult {
        let command = need_str(args, "command")?;
        let cwd = super::expand(str_arg(args, "cwd").unwrap_or("~"));
        let timeout = bash::timeout_secs(args.get("timeout_sec").or_else(|| args.get("timeout")));
        bash::run_blocking(command, Some(&cwd), timeout, cancel).map(ToolOutput::text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_in_cwd_and_times_out() {
        let t = ShellTool;
        let r = t.run(&json!({"command":"pwd; exit 3","cwd":"/tmp"})).unwrap().text;
        assert!(r.contains("/tmp") && r.contains("exit: 3"), "{r}");
        assert!(t.run(&json!({"command":"sleep 5","timeout_sec":1})).is_err());
    }
}
