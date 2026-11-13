//! Без панели фильтров (сборка без фичи `desktop`: DLSS 5 и нейросетей нет).

use syngui::input::{Key, Modifiers};
use syngui::prelude::*;

use crate::ui::St;

/// Клавиша сравнения не перехватывается: [`on_key`] её не обрабатывает.
pub const COMPARE_KEY: Key = Key::B;

pub fn button(_st: St) -> Box<dyn Widget> {
    Box::new(Row::new())
}

pub fn on_key(_st: St, _key: Key, _mods: Modifiers) -> bool {
    false
}

pub fn panel(_st: St) -> Box<dyn Widget> {
    Box::new(Row::new())
}

pub fn compare_handle(_st: St) -> Box<dyn Widget> {
    Box::new(Row::new())
}
