pub mod args;
pub mod bash;
pub mod web;

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Danger {
    Safe,
    Confirm,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub files: Vec<std::path::PathBuf>,
}

impl ToolOutput {
    pub fn text(s: impl Into<String>) -> Self {
        Self { text: s.into(), files: Vec::new() }
    }
}

pub type ToolResult = Result<ToolOutput, String>;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn group(&self) -> &'static str;
    fn description(&self) -> String;
    fn parameters(&self) -> Value;
    fn danger(&self, args: &Value) -> Danger;
    fn summary(&self, args: &Value) -> String;
    fn run(&self, args: &Value) -> ToolResult;
    fn run_ctx(&self, args: &Value, _cancel: &std::sync::atomic::AtomicBool) -> ToolResult {
        self.run(args)
    }
}

pub fn schema(t: &dyn Tool) -> Value {
    serde_json::json!({
        "type": "function",
        "function": { "name": t.name(), "description": t.description(), "parameters": t.parameters() }
    })
}

pub fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

pub fn need_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).ok_or_else(|| format!("нужен параметр «{key}»"))
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}\n… [обрезано: всего {} символов]", s.chars().count())
}

/// Ход выполнения инструмента (долгая генерация): `(шаг, всего, подпись)`.
/// Слушатель ставит агент на время вызова; инструменты зовут [`progress`].
type ProgressFn = Box<dyn FnMut(usize, usize, &str) + Send>;

thread_local! {
    static PROGRESS: std::cell::RefCell<Option<ProgressFn>> = const { std::cell::RefCell::new(None) };
}

/// Сообщить ход: `step` из `total` (0 из 0 — только подпись этапа).
pub fn progress(step: usize, total: usize, label: &str) {
    PROGRESS.with(|p| {
        if let Some(f) = p.borrow_mut().as_mut() {
            f(step, total, label);
        }
    });
}

/// Выполнить `f` со слушателем хода (в том же потоке).
pub fn with_progress<R>(listener: ProgressFn, f: impl FnOnce() -> R) -> R {
    let prev = PROGRESS.with(|p| p.replace(Some(listener)));
    let r = f();
    PROGRESS.with(|p| *p.borrow_mut() = prev);
    r
}
