use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::tools::{need_str, str_arg, truncate, Danger, Tool, ToolOutput, ToolResult};

pub struct ShellTool;

const OUTPUT_LIMIT: usize = 12_000;

impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn group(&self) -> &'static str {
        "терминал"
    }
    fn description(&self) -> String {
        "Выполнить команду bash от имени пользователя (рабочая папка — домашняя или cwd). Вывод stdout и stderr, до 12 тыс. символов; timeout — секунды, по умолчанию 60. Для долгих задач запускай в фоне.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string"},
                "timeout": {"type": "integer"}
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
        let command = need_str(args, "command")?;
        let cwd = super::expand(str_arg(args, "cwd").unwrap_or("~"));
        let timeout = Duration::from_secs(args.get("timeout").and_then(Value::as_u64).unwrap_or(60).clamp(1, 1800));
        let mut child = Command::new("bash")
            .args(["-lc", command])
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut stdout = child.stdout.take().ok_or("нет stdout")?;
        let mut stderr = child.stderr.take().ok_or("нет stderr")?;
        let out_thread = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = stdout.read_to_end(&mut s);
            s
        });
        let err_thread = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = stderr.read_to_end(&mut s);
            s
        });
        let start = Instant::now();
        let status = loop {
            if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                break Some(st);
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let out = String::from_utf8_lossy(&out_thread.join().unwrap_or_default()).into_owned();
        let err = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
        let mut text = String::new();
        if !out.trim().is_empty() {
            text.push_str(out.trim_end());
        }
        if !err.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("[stderr]\n");
            text.push_str(err.trim_end());
        }
        match status {
            Some(st) => {
                let code = st.code().unwrap_or(-1);
                if text.is_empty() {
                    text = "(вывода нет)".into();
                }
                Ok(ToolOutput::text(format!("{}\n[код выхода {code}]", truncate(&text, OUTPUT_LIMIT))))
            }
            None => Err(format!("превышено время ({} с); вывод:\n{}", timeout.as_secs(), truncate(&text, OUTPUT_LIMIT))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_times_out() {
        let t = ShellTool;
        let r = t.run(&json!({"command":"echo привет; echo ошибка >&2; exit 3","cwd":"/tmp"})).unwrap().text;
        assert!(r.contains("привет") && r.contains("ошибка") && r.contains("код выхода 3"));
        assert!(t.run(&json!({"command":"sleep 5","timeout":1})).is_err());
    }
}
