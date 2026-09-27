//! Пейджер столов: кнопка на стол, активный подсвечен, точки — окна.

use syndesktop_common::action::WorkspaceTarget;
use syndesktop_common::config::Applet;
use syndesktop_common::Action;
use syngui::input::MouseButton;
use syngui::prelude::*;

use crate::ctx::ShellCtx;
use crate::panel::PanelCtx;
use crate::ui::InputArea;

pub fn build(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let show_names = a.bool_or("names", true);
    let hide_empty = a.bool_or("hide_empty", false);
    // Единственный стол переключать некуда — пейджер не показывается.
    let show_single = a.bool_or("show_single", false);
    let vertical = pc.vertical;
    Box::new(crate::ui::rx(move || {
        let wss = ctx.workspaces.get();
        let cfg = ctx.config.get();
        // Без композитора — столы из конфига, все неактивны.
        let list: Vec<(u32, String, bool, u32, bool)> = if wss.is_empty() {
            (0..cfg.workspaces.count).map(|i| (i, cfg.workspaces.name(i), i == 0, 0, false)).collect()
        } else {
            wss.iter().map(|w| (w.index, w.name.clone(), w.active, w.windows, w.urgent)).collect()
        };
        if list.len() <= 1 && !show_single {
            return Box::new(DecoratedBox::new()) as Box<dyn Widget>;
        }
        let mut flex = Flex::new()
            .direction(if vertical { FlexDirection::Column } else { FlexDirection::Row })
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        for (idx, name, active, windows, urgent) in list {
            if hide_empty && windows == 0 && !active {
                continue;
            }
            let mut cls = String::from("ws");
            if active {
                cls.push_str(" ws-active");
            }
            if windows > 0 {
                cls.push_str(" ws-occupied");
            }
            if urgent {
                cls.push_str(" ws-urgent");
            }
            let label = if show_names { name } else { String::new() };
            let mut col = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if !label.is_empty() {
                col = col.child(Text::new(label).class("ws-label"));
            }
            let dots = windows.min(4);
            if dots > 0 {
                let mut r = Row::new().gap(2.0);
                for _ in 0..dots {
                    r = r.child(DecoratedBox::new().class("ws-dot"));
                }
                col = col.child(r);
            }
            flex = flex.child(
                InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(col)).class(cls)).pointer().buttons(&[MouseButton::Left]).on_click(move |b, _, _| {
                    if b == MouseButton::Left {
                        crate::actions::run(Action::Workspace(WorkspaceTarget::Index(idx + 1)));
                    }
                }),
            );
        }
        Box::new(
            InputArea::new(DecoratedBox::new().child(flex).class("applet-workspaces")).on_wheel(|dy| {
                crate::actions::run(Action::Workspace(if dy > 0.0 { WorkspaceTarget::Prev } else { WorkspaceTarget::Next }));
            }),
        )
    }))
}
