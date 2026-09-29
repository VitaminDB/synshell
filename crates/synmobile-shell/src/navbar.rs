//! Панель навигации снизу: «Назад» (закрыть окно), «Домой», «Окна».

use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};

use crate::Ctx;

pub const HEIGHT: u32 = 56;

/// Кодпоинты Material Icons (шрифт syngui `material-icons`).
pub mod mi {
    pub const ARROW_BACK: &str = "\u{E5C4}";
    pub const HOME: &str = "\u{E88A}";
    pub const APPS: &str = "\u{E5C3}";
}

pub fn install(ctx: Ctx) {
    syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "synmobile-navbar".into(),
            layer: Layer::Top,
            anchor: Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, HEIGHT),
            exclusive_zone: HEIGHT as i32,
            keyboard: KeyboardInteractivity::None,
            ..Default::default()
        },
        move || Box::new(view(ctx)),
    );
}

fn nav(icon: &str, on: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new("").icon(icon).on_click(on).class("nav-button")
}

fn view(ctx: Ctx) -> impl Widget {
    use syngui::widget::WidgetExt;
    DecoratedBox::new()
        .child(
            Row::new()
                .gap(8.0)
                .child(nav(mi::ARROW_BACK, move || crate::action(synshell_common::Action::Close)))
                .child(nav(mi::HOME, move || ctx.home_visible.set(!ctx.home_visible.get_untracked())))
                .child(nav(mi::APPS, move || ctx.home_visible.set(true))),
        )
        .class("navbar")
}
