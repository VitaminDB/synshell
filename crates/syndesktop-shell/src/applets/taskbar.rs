//! Панель задач: окна текущего стола (или всех), значки по .desktop,
//! заголовки. ЛКМ — свернуть/развернуть, СКМ — закрыть, ПКМ — меню окна,
//! колесо — перебор окон.

use syndesktop_common::config::Applet;
use syndesktop_common::ipc::{WindowInfo, WindowOp};
use syngui::input::MouseButton;
use syngui::prelude::*;

use crate::ctx::{PopupKind, ShellCtx};
use crate::panel::PanelCtx;
use crate::ui::InputArea;

fn visible(w: &WindowInfo, ws: Option<u32>, all: bool, output: &str, only_output: bool) -> bool {
    if w.skip_taskbar {
        return false;
    }
    if only_output && w.output.as_deref().is_some_and(|o| o != output) {
        return false;
    }
    all || ws.is_none() || w.sticky || Some(w.workspace) == ws
}

pub fn build(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let labels = a.bool_or("labels", !pc.vertical);
    let all = a.bool_or("all_workspaces", false);
    let only_output = a.bool_or("only_output", false);
    let max_width = a.int_or("max_width", 220) as f32;
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let windows = ctx.windows.get();
        let ws = ctx.active_workspace();
        let mut flex = Flex::new()
            .direction(if pc.vertical { FlexDirection::Column } else { FlexDirection::Row })
            .gap(3.0)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        let list: Vec<WindowInfo> =
            windows.iter().filter(|w| visible(w, ws, all, &pc.output, only_output)).cloned().collect();
        for w in list {
            flex = flex.child(task(w, labels, max_width, pc.clone()));
        }
        Box::new(
            InputArea::new(DecoratedBox::new().child(flex).class("applet-taskbar")).on_wheel(move |dy| {
                let ctx = ShellCtx::get();
                let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
                let list: Vec<WindowInfo> = ctx
                    .windows
                    .get_untracked()
                    .into_iter()
                    .filter(|w| visible(w, ws, false, "", false) && !w.minimized)
                    .collect();
                if list.is_empty() {
                    return;
                }
                let cur = list.iter().position(|w| w.focused).unwrap_or(0) as i64;
                let n = list.len() as i64;
                let next = ((cur + if dy > 0.0 { -1 } else { 1 }) % n + n) % n;
                crate::actions::window_op(list[next as usize].id, WindowOp::Activate);
            }),
        )
    }))
}

fn task(w: WindowInfo, labels: bool, max_width: f32, pc: PanelCtx) -> impl Widget {
    let mut cls = String::from("task");
    if w.focused {
        cls.push_str(" task-active");
    }
    if w.minimized {
        cls.push_str(" task-minimized");
    }
    if w.urgent {
        cls.push_str(" task-urgent");
    }
    if !labels {
        cls.push_str(" task-icon-only");
    }
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    match crate::xdg::window_icon(&w.app_id) {
        Some(p) => {
            row = row.child(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("task-icon"));
        }
        None => row = row.child(crate::ui::icon(crate::ui::mi::WINDOW).class("task-glyph")),
    }
    if labels {
        let title = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
        row = row.child(Text::new(title).max_lines(1).class("task-title"));
    }
    let id = w.id;
    InputArea::new(
        DecoratedBox::new()
            .child(crate::ui::vcenter(row))
            .class(cls)
            .style("max-width", syngui::mss::StyleValue::px(if labels { max_width } else { 48.0 })),
    )
    .pointer()
    .on_click(move |b, _, r| match b {
        MouseButton::Left => crate::actions::window_op(id, WindowOp::ToggleMinimize),
        MouseButton::Middle => crate::actions::window_op(id, WindowOp::Close),
        MouseButton::Right => ShellCtx::get().open_popup(PopupKind::WindowMenu(id), pc.anchor(r)),
        _ => {}
    })
}
