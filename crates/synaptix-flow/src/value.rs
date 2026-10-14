//! Значения на портах: текст, число, файл, список (пакет) и дескриптор —
//! объект службы (модель, условие, латент), который живёт только в процессе,
//! исполняющем граф.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
    /// Объект в памяти исполнителя; наружу уходит только подпись.
    Handle(Handle),
}

/// Дескриптор объекта (модель, условие, латент). Сравнение и хэш — по
/// номеру: копии одного дескриптора равны, новый объект — новый номер.
#[derive(Clone)]
pub struct Handle {
    id: u64,
    /// Подпись: «FLUX.1 dev», «латент 1024×1024».
    pub label: String,
    data: Option<Arc<dyn Any + Send + Sync>>,
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

impl Handle {
    pub fn new<T: Any + Send + Sync>(label: impl Into<String>, data: T) -> Self {
        Self::from_arc(label, Arc::new(data))
    }

    pub fn from_arc<T: Any + Send + Sync>(label: impl Into<String>, data: Arc<T>) -> Self {
        Self { id: NEXT_HANDLE.fetch_add(1, Ordering::Relaxed), label: label.into(), data: Some(data) }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Объект нужного типа (у дескриптора, пришедшего извне, объекта нет).
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.data.clone()?.downcast::<T>().ok()
    }
}

impl PartialEq for Handle {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle#{}({})", self.id, self.label)
    }
}

impl Serialize for Handle {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.label)
    }
}

impl<'de> Deserialize<'de> for Handle {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let label = String::deserialize(d)?;
        Ok(Self { id: 0, label, data: None })
    }
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
            Val::Handle(h) => h.label.clone(),
        }
    }

    /// Дескриптор (у списка — первого элемента-дескриптора).
    pub fn handle(&self) -> Option<&Handle> {
        match self {
            Val::Handle(h) => Some(h),
            Val::List(l) => l.iter().find_map(Val::handle),
            _ => None,
        }
    }

    /// Файлы (рекурсивно) с заменой — для перевода путей между процессами.
    pub fn map_files(&self, f: &mut dyn FnMut(&Path) -> PathBuf) -> Val {
        match self {
            Val::File(p) => Val::File(f(p)),
            Val::List(l) => Val::List(l.iter().map(|v| v.map_files(f)).collect()),
            other => other.clone(),
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
