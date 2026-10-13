//! Экспорт графа во вкладку редактора узлов Synthos (уровень Б).

use serde_json::{Map, Value};

use crate::Graph;

/// Записать граф вкладкой Synthos и открыть его; текст — что сделано.
pub fn export_and_open(_g: &Graph, _params: &Map<String, Value>) -> Result<String, String> {
    Err("экспорт в Synthos ещё не готов".into())
}
