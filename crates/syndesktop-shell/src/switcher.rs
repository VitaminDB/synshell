//! Список окон при Alt+Tab. Переключает окна сам композитор (окно уже в
//! фокусе); оболочка лишь показывает ненавязчивую ленту на overlay без
//! клавиатуры: `window-switcher <выбранный id> <id1,id2,…>` —
//! показать/обновить, `window-switcher-end` — убрать.

use std::cell::Cell;
use syndesktop_common::ipc::WindowOp;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::ui::InputArea;

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
}

/// Разбор аргументов команды `window-switcher`.
pub fn command(ctx: ShellCtx, arg: &str) {
    let mut it = arg.split_whitespace();
    match it.next() {
        Some("next") => crate::actions::run(syndesktop_common::Action::FocusNext),
        Some("prev") => crate::actions::run(syndesktop_common::Action::FocusPrev),
        Some(sel) => {
            let Ok(sel) = sel.parse::<u64>() else { return };
            let ids: Vec<u64> = it.next().unwrap_or("").split(',').filter_map(|s| s.trim().parse().ok()).collect();
            ctx.switcher.set(Some((sel, ids)));
        }
        None => {}
    }
}

pub fn end(ctx: ShellCtx) {
    ctx.switcher.set(None);
}

pub fn install(ctx: ShellCtx) {
    create_effect(move || {
        let on = ctx.switcher.get().is_some();
        let cur = SURFACE.with(|s| s.get());
        match (on, cur) {
            (true, None) => {
                let id = syngui_layer::create_surface(
                    SurfaceSpec {
                        namespace: "syndesktop-switcher".into(),
                        layer: Layer::Overlay,
                        anchor: Anchor::empty(),
                        size: (0, 0),
                        margin: [0; 4],
                        exclusive_zone: -1,
                        keyboard: KeyboardInteractivity::None,
                        output: crate::manager::focused_output(&ctx),
                        auto_size: true,
                        clear_color: [0.0; 4],
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

fn view(ctx: ShellCtx) -> impl Widget {
    DecoratedBox::new()
        .child(move || {
            let Some((sel, ids)) = ctx.switcher.get() else { return Flex::new() };
            let windows = ctx.windows.get();
            let mut flex = Flex::new().direction(FlexDirection::Row).gap(8.0);
            for id in ids.iter().take(12) {
                let Some(w) = windows.iter().find(|w| w.id == *id) else { continue };
                let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
                if let Some(p) = crate::xdg::window_icon(&w.app_id) {
                    col = col.child(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("switch-icon"));
                }
                let title = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
                col = col.child(Text::new(title).max_lines(2).class("switch-title"));
                let wid = w.id;
                flex = flex.child(
                    InputArea::new(DecoratedBox::new().child(col).class(if *id == sel { "switch-card switch-card-selected" } else { "switch-card" }))
                        .on_click(move |b, _, _| {
                            if b == MouseButton::Left {
                                crate::actions::window_op(wid, WindowOp::Activate);
                            }
                        }),
                );
            }
            flex
        })
        .class("switcher")
}
