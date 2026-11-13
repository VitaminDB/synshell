//! Без вопроса «освободить видеопамять» (сборка без фичи `desktop`: нейросетей нет).

use std::path::Path;

use syngui::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Vram {
    None,
    Freeing,
}

pub fn on_fallback(_st: crate::ui::St, _path: &Path, _reason: String) {}

pub fn dialog(_st: crate::ui::St) -> Box<dyn Widget> {
    Box::new(Row::new())
}
