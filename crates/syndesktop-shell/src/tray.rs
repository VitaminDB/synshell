//! Системный лоток (StatusNotifierItem).
//!
//! TODO: полноценный SNI-хост (org.kde.StatusNotifierWatcher через zbus,
//! значки IconPixmap/IconName, меню com.canonical.dbusmenu). Пока апплет
//! пустой и не занимает места.

use syngui::prelude::*;

use crate::panel::PanelCtx;

pub fn applet(_pc: &PanelCtx) -> Box<dyn Widget> {
    Box::new(DecoratedBox::new().class("applet-tray"))
}
