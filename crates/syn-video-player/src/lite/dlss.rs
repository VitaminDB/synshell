//! Без DLSS 5 (сборка без фичи `desktop`: телефоны на Android, встраивание в другие программы):
//! тот же интерфейс модуля, фильтр никогда не включается.

use syngui::video::VideoPlayer;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
}

pub fn attach(_st: crate::ui::St, _p: &VideoPlayer) {}
pub fn poll(_st: crate::ui::St) {}
pub fn shutdown() {}
