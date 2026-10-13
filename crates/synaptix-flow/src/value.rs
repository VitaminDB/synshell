//! Значения на портах: текст, число, файл, список (пакет).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Val {
    #[default]
    Empty,
    Text(String),
    Number(f64),
    File(PathBuf),
    List(Vec<Val>),
}

impl Val {
    pub fn is_empty(&self) -> bool {
        match self {
            Val::Empty => true,
            Val::Text(s) => s.trim().is_empty(),
            Val::List(l) => l.is_empty(),
            _ => false,
        }
    }

    /// Строкой: текст, число, путь; список — элементы через перевод строки.
    pub fn text(&self) -> String {
        match self {
            Val::Empty => String::new(),
            Val::Text(s) => s.clone(),
            Val::Number(n) => fmt_num(*n),
            Val::File(p) => p.display().to_string(),
            Val::List(l) => l.iter().map(Val::text).collect::<Vec<_>>().join("\n"),
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Val::Number(n) => Some(*n),
            Val::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// Первый файл (у списка — первого элемента-файла).
    pub fn file(&self) -> Option<&Path> {
        match self {
            Val::File(p) => Some(p),
            Val::Text(s) if !s.trim().is_empty() && Path::new(s.trim()).is_absolute() => Some(Path::new(s.trim())),
            Val::List(l) => l.iter().find_map(Val::file),
            _ => None,
        }
    }

    /// Все файлы (одиночный — список из одного).
    pub fn files(&self) -> Vec<PathBuf> {
        match self {
            Val::List(l) => l.iter().flat_map(Val::files).collect(),
            other => other.file().map(|p| vec![p.to_path_buf()]).unwrap_or_default(),
        }
    }

    /// Элементы пакета (не список — один элемент).
    pub fn items(&self) -> Vec<Val> {
        match self {
            Val::List(l) => l.clone(),
            Val::Empty => Vec::new(),
            other => vec![other.clone()],
        }
    }

    /// Из JSON-значения поля: строка → текст (абсолютный путь к существующему файлу — файл).
    pub fn from_json(v: &Json) -> Val {
        match v {
            Json::Null => Val::Empty,
            Json::Bool(b) => Val::Text(b.to_string()),
            Json::Number(n) => n.as_f64().map(Val::Number).unwrap_or(Val::Empty),
            Json::String(s) => Val::Text(s.clone()),
            Json::Array(a) => Val::List(a.iter().map(Val::from_json).collect()),
            Json::Object(_) => Val::Text(v.to_string()),
        }
    }
}

pub(crate) fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}
