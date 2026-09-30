//! Интерфейс окна.

pub mod app;
pub mod dialogs;
pub mod items;
pub mod jobs;
pub mod phone;
pub mod sidebar;
pub mod toolbar;
pub mod view;

use syngui::prelude::*;
use syngui::widgets::*;
use syngui::IntoWidget;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

/// Коробка с классом.
pub fn bx<M>(class: &str, child: impl IntoWidget<M>) -> W {
    boxed(DecoratedBox::new().class(class.to_string()).child(child))
}

/// Кнопка-значок с подсказкой.
pub fn icon_button(glyph: &str, tip: &str, class: &str, enabled: bool, on: impl FnMut() + Send + 'static) -> W {
    let cls = if enabled { format!("icon-btn {class}") } else { format!("icon-btn disabled {class}") };
    let inner = DecoratedBox::new().class(cls).child(Icon::new(glyph).class("icon"));
    let g = if enabled { GestureDetector::new().on_click(on).child(inner) } else { GestureDetector::new().child(inner) };
    if tip.is_empty() {
        boxed(g)
    } else {
        boxed(Tooltip::new(g, tip.to_string()).delay_ms(500))
    }
}

/// Кнопка «значок + подпись» (командная панель).
pub fn text_button(glyph: &str, label: &str, class: &str, on: impl FnMut() + Send + 'static) -> W {
    boxed(
        GestureDetector::new().on_click(on).child(
            DecoratedBox::new().class(format!("cmd-btn {class}")).child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(glyph).class("icon"))
                    .child(Text::new(label).class("cmd-label")),
            ),
        ),
    )
}

/// Глифы Material Icons (набор на все этапы — часть пока не используется).
#[allow(dead_code)]
pub mod icons {
    pub const BACK: &str = "\u{e5c4}";
    pub const FORWARD: &str = "\u{e5c8}";
    pub const UP: &str = "\u{e5d8}";
    pub const REFRESH: &str = "\u{e5d5}";
    pub const HOME: &str = "\u{e88a}";
    pub const SEARCH: &str = "\u{e8b6}";
    pub const CLOSE: &str = "\u{e5cd}";
    pub const ADD: &str = "\u{e145}";
    pub const FOLDER: &str = "\u{e2c7}";
    pub const FOLDER_OPEN: &str = "\u{e2c8}";
    pub const NEW_FOLDER: &str = "\u{e2cc}";
    pub const NEW_FILE: &str = "\u{e89c}";
    pub const CUT: &str = "\u{e14e}";
    pub const COPY: &str = "\u{e14d}";
    pub const PASTE: &str = "\u{e14f}";
    pub const RENAME: &str = "\u{e9a2}";
    pub const DELETE: &str = "\u{e872}";
    pub const DELETE_FOREVER: &str = "\u{e92b}";
    pub const SORT: &str = "\u{e164}";
    pub const VIEW_DETAILS: &str = "\u{e8ee}";
    pub const VIEW_LIST: &str = "\u{e896}";
    pub const VIEW_TILES: &str = "\u{e8f0}";
    pub const VIEW_ICONS: &str = "\u{e9b0}";
    pub const MORE: &str = "\u{e5d3}";
    pub const CHEVRON: &str = "\u{e5cc}";
    pub const EXPAND: &str = "\u{e5cf}";
    pub const DROP_DOWN: &str = "\u{e5c5}";
    pub const DESKTOP: &str = "\u{e30c}";
    pub const DOWNLOAD: &str = "\u{e2c4}";
    pub const DOCUMENT: &str = "\u{e873}";
    pub const IMAGE: &str = "\u{e413}";
    pub const MUSIC: &str = "\u{e030}";
    pub const VIDEO: &str = "\u{e04a}";
    pub const FOLDER_PINNED: &str = "\u{e617}";
    pub const PIN: &str = "\u{f10d}";
    pub const DRIVE: &str = "\u{e1db}";
    pub const USB: &str = "\u{e1e0}";
    pub const NETWORK: &str = "\u{eb2f}";
    pub const PHONE: &str = "\u{e32c}";
    pub const COMPUTER: &str = "\u{e30a}";
    pub const LAPTOP: &str = "\u{e31e}";
    pub const TRASH: &str = "\u{e872}";
    pub const RESTORE: &str = "\u{e938}";
    pub const UNDO: &str = "\u{e166}";
    pub const INFO: &str = "\u{e88e}";
    pub const TERMINAL: &str = "\u{eb8e}";
    pub const OPEN: &str = "\u{e89e}";
    pub const OPEN_WITH: &str = "\u{e89f}";
    pub const CHECK: &str = "\u{e5ca}";
    pub const HIDDEN: &str = "\u{e8f4}";
    pub const SPLIT: &str = "\u{e949}";
    pub const SIDEBAR: &str = "\u{f114}";
    pub const PREVIEW: &str = "\u{f1c5}";
    pub const LINK: &str = "\u{e157}";
    pub const FILE: &str = "\u{e24d}";
    pub const SELECT_ALL: &str = "\u{e162}";
    pub const DESELECT: &str = "\u{ebb6}";
    pub const PAUSE: &str = "\u{e034}";
    pub const PLAY: &str = "\u{e037}";
    pub const ERROR: &str = "\u{e000}";
    pub const WARNING: &str = "\u{e002}";
    pub const ZOOM_IN: &str = "\u{e8ff}";
    pub const ZOOM_OUT: &str = "\u{e900}";
    pub const TAB: &str = "\u{e8d8}";
    pub const COPY_PATH: &str = "\u{e157}";
    pub const EDIT: &str = "\u{e3c9}";
    pub const MENU: &str = "\u{e5d2}";
    pub const MORE_VERT: &str = "\u{e5d4}";
    pub const CHECK_CIRCLE: &str = "\u{e86c}";
    pub const UNCHECKED: &str = "\u{e836}";
    pub const TUNE: &str = "\u{e429}";
    pub const MOVE: &str = "\u{e2bf}";
}
