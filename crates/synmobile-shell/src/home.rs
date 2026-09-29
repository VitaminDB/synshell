//! Домашний экран: сетка приложений из .desktop; поверх окон (overlay),
//! прячется при запуске приложения и по кнопке «Домой».

use std::cell::Cell;
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::Ctx;

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
}

pub fn install(ctx: Ctx) {
    create_effect(move || {
        let visible = ctx.home_visible.get();
        let current = SURFACE.with(|s| s.get());
        match (visible, current) {
            (true, None) => {
                let id = syngui_layer::create_surface(
                    SurfaceSpec {
                        namespace: "synmobile-home".into(),
                        layer: Layer::Overlay,
                        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                        size: (0, 0),
                        margin: [crate::statusbar::HEIGHT as i32, 0, crate::navbar::HEIGHT as i32, 0],
                        exclusive_zone: -1,
                        keyboard: KeyboardInteractivity::OnDemand,
                        ..Default::default()
                    },
                    move || Box::new(view(ctx)),
                );
                SURFACE.with(|s| s.set(Some(id)));
            }
            (false, Some(id)) => {
                SURFACE.with(|s| s.set(None));
                syngui_layer::close_surface(id);
            }
            _ => {}
        }
    });
}

fn view(ctx: Ctx) -> impl Widget {
    use syngui::widget::WidgetExt;
    let apps = synshell_common::xdg::apps();
    let mut grid = Grid::new(4).gap(6.0);
    for app in apps.iter().filter(|a| !a.no_display) {
        let cmd = app.command();
        let terminal = app.terminal;
        let name = app.name.clone();
        let icon = synshell_common::xdg::lookup_icon(&app.icon);
        let icon_widget: Box<dyn Widget> = match icon {
            Some(path) => Box::new(Image::new(path.display().to_string()).class("app-icon")),
            None => Box::new(Icon::new(crate::navbar::mi::APPS).class("app-icon")),
        };
        let launch = move || {
            let term = ctx.config.get_untracked().general.terminal.clone();
            if terminal {
                crate::spawn(&format!("{term} -e {cmd}"));
            } else {
                crate::spawn(&cmd);
            }
            ctx.home_visible.set(false);
        };
        let tile = GestureDetector::new()
            .on_click(launch)
            .child(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon_widget)
                            .child(Text::new(name).class("app-name")),
                    )
                    .class("app-tile"),
            );
        grid = grid.child(tile);
    }
    DecoratedBox::new()
        .child(
            Column::new()
                .child(Text::new("Приложения").class("home-title"))
                .child(ScrollView::new().vertical().child(grid).class("grow")),
        )
        .class("home")
}
