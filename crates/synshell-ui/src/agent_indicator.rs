//! Индикатор «ИИ управляет»: плашка сверху по центру, пока агент (synlink)
//! управляет вводом. В обычном режиме — кнопка «Забрать»; при полном
//! управлении ввод пользователя заблокирован композитором, забрать можно
//! только сочетанием (Ctrl+Alt+Esc) — плашка подсказывает его.

use std::cell::Cell;

use syngui::prelude::*;
use syngui::t;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};
use synshell_common::ipc::{AgentOp, Request};

use crate::ctx::ShellCtx;
use crate::ui::{boxed, icon};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
}

pub fn install(ctx: ShellCtx) {
    create_effect(move || {
        let a = ctx.agent.get();
        let open = SURFACE.with(|s| s.get());
        match (a.active, open) {
            (false, Some(id)) => {
                SURFACE.with(|s| s.set(None));
                syngui_layer::close_surface(id);
            }
            (true, None) => {
                let id = syngui_layer::create_surface(
                    SurfaceSpec {
                        namespace: "syndesktop-agent".into(),
                        layer: Layer::Overlay,
                        anchor: Anchor::TOP,
                        size: (0, 0),
                        auto_size: true,
                        margin: [64, 0, 0, 0],
                        exclusive_zone: -1,
                        keyboard: KeyboardInteractivity::None,
                        ..Default::default()
                    },
                    move || Box::new(boxed("agent-surface", Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(view(ctx)))),
                );
                SURFACE.with(|s| s.set(Some(id)));
            }
            _ => {}
        }
    });
}

fn view(ctx: ShellCtx) -> impl Widget {
    boxed(
        "agent-pill",
        crate::ui::rx(move || {
            let a = ctx.agent.get();
            let sub = if a.full {
                t!("Ваш ввод заблокирован · {k} — забрать управление", k = a.takeback)
            } else {
                t!("{k} — забрать управление", k = a.takeback)
            };
            let mut text = Column::new().gap(2.0).child(Text::new(t!("ИИ управляет компьютером")).class("agent-title")).child(Text::new(sub).class("agent-sub"));
            if !a.reason.is_empty() {
                text = text.child(Text::new(a.reason.clone()).max_lines(1).class("agent-sub"));
            }
            let mut row = Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(boxed("agent-dot", icon("\u{F06C}").class("agent-icon")))
                .child(text);
            if !a.full {
                row = row.child(Button::new(t!("Забрать")).class("agent-stop").on_click(|| crate::actions::send(Request::Agent { op: AgentOp::TakeBack })));
            }
            Box::new(row) as Box<dyn Widget>
        }),
    )
}
