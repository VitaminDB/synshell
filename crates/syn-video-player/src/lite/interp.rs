//! Без промежуточных кадров RIFE (сборка без фичи `desktop`).

use syngui::video::VideoPlayer;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
}

pub fn attach(_st: crate::ui::St, _p: &VideoPlayer) {}
pub fn poll(_st: crate::ui::St) {}
pub fn apply(_st: crate::ui::St) {}
