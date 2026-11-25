//! Команды — единственный способ менять редактор. Одна таблица для меню,
//! клавиш, палитры, CLI (`--cmd`), сокета и агента: у каждой команды есть
//! схема аргументов (из неё строится описание инструмента для модели) и
//! обработчик, работающий без окна.

pub mod audio;
mod edit;
pub mod media;
mod template;
mod project;
mod timeline;
mod keys;
mod lut;
mod graph;
mod blocks;
pub use timeline::EFFECTS;

use serde_json::{json, Map, Value};

use crate::doc::{parse_time, Id};
use crate::editor::Editor;

/// Тип аргумента команды.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Str,
    Num,
    Bool,
    /// Время: секунды числом или строка `м:сс.ммм`.
    Time,
    /// Идентификатор (`12` или `c12`, `m3`, `t1`, `e4`).
    Id,
    Ids,
    Paths,
    Choice(&'static [&'static str]),
    /// Объект с произвольными полями (параметры эффекта, стиль титра).
    Object,
    /// Список (например, куски времени [[начало, конец], …]).
    Array,
    /// Число или строка (значение ключа: число или цвет).
    Any,
}

#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub help: &'static str,
}

pub const fn arg(name: &'static str, ty: Ty, help: &'static str) -> Arg {
    Arg { name, ty, required: false, help }
}

pub const fn req(name: &'static str, ty: Ty, help: &'static str) -> Arg {
    Arg { name, ty, required: true, help }
}

/// Результат команды: текст для человека и модели, данные для программ.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub text: String,
    pub data: Value,
}

impl Out {
    pub fn text(s: impl Into<String>) -> Self {
        Self { text: s.into(), data: Value::Null }
    }
    pub fn with(s: impl Into<String>, data: Value) -> Self {
        Self { text: s.into(), data }
    }
}

pub type Res = Result<Out, String>;

pub struct Command {
    pub id: &'static str,
    pub category: &'static str,
    pub title: &'static str,
    pub shortcut: &'static str,
    pub keywords: &'static str,
    /// Что делает — для модели и подсказки.
    pub help: &'static str,
    pub args: &'static [Arg],
    /// Необратимое действие (удаление файлов, перезапись): агент спрашивает.
    pub danger: bool,
    pub run: fn(&mut Editor, &Args) -> Res,
}

impl Command {
    /// JSON Schema аргументов.
    pub fn schema(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for a in self.args {
            let mut p = match a.ty {
                Ty::Str => json!({"type": "string"}),
                Ty::Num => json!({"type": "number"}),
                Ty::Bool => json!({"type": "boolean"}),
                Ty::Time => json!({"type": ["number", "string"]}),
                Ty::Id => json!({"type": ["integer", "string"]}),
                Ty::Ids => json!({"type": "array", "items": {"type": ["integer", "string"]}}),
                Ty::Paths => json!({"type": "array", "items": {"type": "string"}}),
                Ty::Choice(v) => json!({"type": "string", "enum": v}),
                Ty::Object => json!({"type": "object"}),
                Ty::Array => json!({"type": "array"}),
                Ty::Any => json!({"type": ["number", "string"]}),
            };
            p["description"] = Value::String(a.help.to_string());
            props.insert(a.name.to_string(), p);
            if a.required {
                required.push(Value::String(a.name.to_string()));
            }
        }
        json!({"type": "object", "properties": props, "required": required})
    }

    /// Имя инструмента для модели: `clip.split` → `clip_split`.
    pub fn tool_name(&self) -> String {
        self.id.replace('.', "_")
    }
}

/// Все команды.
pub fn all() -> Vec<&'static Command> {
    let mut v: Vec<&'static Command> = Vec::new();
    v.extend(project::COMMANDS);
    v.extend(media::COMMANDS);
    v.extend(audio::COMMANDS);
    v.extend(template::COMMANDS);
    v.extend(timeline::COMMANDS);
    v.extend(keys::COMMANDS);
    v.extend(lut::COMMANDS);
    v.extend(graph::COMMANDS);
    v.extend(blocks::COMMANDS);
    v.extend(edit::COMMANDS);
    v
}

pub fn find(id: &str) -> Option<&'static Command> {
    let id = id.trim();
    all().into_iter().find(|c| c.id == id || c.tool_name() == id)
}

/// Выполнить команду по id с аргументами-JSON.
pub fn run(ed: &mut Editor, id: &str, args: &Value) -> Res {
    let c = find(id).ok_or_else(|| format!("нет команды «{id}»"))?;
    run_cmd(ed, c, args)
}

pub fn run_cmd(ed: &mut Editor, c: &Command, args: &Value) -> Res {
    let obj = match args {
        Value::Null => Map::new(),
        Value::Object(o) => o.clone(),
        _ => return Err(format!("{}: аргументы — объект JSON", c.id)),
    };
    for a in c.args {
        if a.required && obj.get(a.name).is_none_or(Value::is_null) {
            return Err(format!("{}: нужен аргумент «{}» ({})", c.id, a.name, a.help));
        }
    }
    if let Some(k) = obj.keys().find(|k| !c.args.iter().any(|a| a.name == k.as_str())) {
        let known: Vec<&str> = c.args.iter().map(|a| a.name).collect();
        return Err(format!("{}: неизвестный аргумент «{k}»; есть: {}", c.id, known.join(", ")));
    }
    (c.run)(ed, &Args { cmd: c.id, v: obj })
}

/// Аргументы с разбором типов и понятными ошибками.
pub struct Args {
    cmd: &'static str,
    v: Map<String, Value>,
}

impl Args {
    fn err(&self, name: &str, want: &str) -> String {
        format!("{}: «{name}» — {want}", self.cmd)
    }

    pub fn has(&self, name: &str) -> bool {
        self.v.get(name).is_some_and(|v| !v.is_null())
    }

    /// Значение как есть (числа и строки).
    pub fn value(&self, name: &str) -> Option<Value> {
        self.raw(name).cloned()
    }

    pub fn raw(&self, name: &str) -> Option<&Value> {
        self.v.get(name).filter(|v| !v.is_null())
    }

    pub fn str(&self, name: &str) -> Result<Option<String>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Number(n)) => Ok(Some(n.to_string())),
            Some(_) => Err(self.err(name, "строка")),
        }
    }

    pub fn num(&self, name: &str) -> Result<Option<f64>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::Number(n)) => Ok(n.as_f64()),
            Some(Value::String(s)) => s.trim().replace(',', ".").parse().map(Some).map_err(|_| self.err(name, "число")),
            Some(_) => Err(self.err(name, "число")),
        }
    }

    pub fn bool(&self, name: &str) -> Result<Option<bool>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::String(s)) if s == "true" || s == "false" => Ok(Some(s == "true")),
            Some(_) => Err(self.err(name, "true или false")),
        }
    }

    pub fn time(&self, name: &str) -> Result<Option<f64>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(v) => parse_time(v).map(Some).ok_or_else(|| self.err(name, "время: секунды или м:сс.ммм")),
        }
    }

    pub fn id(&self, name: &str) -> Result<Option<Id>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(v) => parse_id(v).map(Some).ok_or_else(|| self.err(name, "идентификатор (12 или c12)")),
        }
    }

    pub fn ids(&self, name: &str) -> Result<Option<Vec<Id>>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::Array(a)) => a.iter().map(|v| parse_id(v).ok_or_else(|| self.err(name, "список идентификаторов"))).collect::<Result<_, _>>().map(Some),
            Some(v) => parse_id(v).map(|i| Some(vec![i])).ok_or_else(|| self.err(name, "список идентификаторов")),
        }
    }

    pub fn strs(&self, name: &str) -> Result<Option<Vec<String>>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| self.err(name, "список строк"))).collect::<Result<_, _>>().map(Some),
            Some(Value::String(s)) => Ok(Some(vec![s.clone()])),
            Some(_) => Err(self.err(name, "список строк")),
        }
    }

    pub fn object(&self, name: &str) -> Result<Option<Map<String, Value>>, String> {
        match self.raw(name) {
            None => Ok(None),
            Some(Value::Object(o)) => Ok(Some(o.clone())),
            Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(o)) => Ok(Some(o)),
                _ => Err(self.err(name, "объект JSON")),
            },
            Some(_) => Err(self.err(name, "объект JSON")),
        }
    }

    pub fn need_id(&self, name: &str) -> Result<Id, String> {
        self.id(name)?.ok_or_else(|| self.err(name, "обязателен"))
    }

    pub fn need_time(&self, name: &str) -> Result<f64, String> {
        self.time(name)?.ok_or_else(|| self.err(name, "обязательно"))
    }

    pub fn need_str(&self, name: &str) -> Result<String, String> {
        self.str(name)?.ok_or_else(|| self.err(name, "обязательна"))
    }
}

/// `12`, `"12"`, `"c12"`, `"m3"` → 12.
pub fn parse_id(v: &Value) -> Option<Id> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().trim_start_matches(|c: char| c.is_ascii_alphabetic()).parse().ok(),
        _ => None,
    }
}

/// Справка по командам текстом (CLI `--commands`, навык агента).
pub fn help_text() -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let mut cat = "";
    for c in all() {
        if c.category != cat {
            cat = c.category;
            let _ = writeln!(o, "\n## {cat}");
        }
        let args: Vec<String> = c.args.iter().map(|a| if a.required { a.name.to_string() } else { format!("[{}]", a.name) }).collect();
        let _ = writeln!(o, "{} {} — {}", c.id, args.join(" "), c.help);
    }
    o
}

#[cfg(test)]
mod tests;
