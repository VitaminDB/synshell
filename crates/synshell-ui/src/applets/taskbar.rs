//! Панель задач: окна текущего стола (или всех), значки по .desktop,
//! заголовки. ЛКМ — свернуть/развернуть, СКМ — закрыть, ПКМ — меню окна,
//! колесо — перебор окон.

use synshell_common::config::Applet;
use synshell_common::ipc::{WindowInfo, WindowOp};
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
    let title_mode = TitleMode::parse(a.str_or("title", "elide"));
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
            flex = flex.child(task(w, labels, title_mode, max_width, pc.clone()));
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

/// Как показывать заголовок, который не помещается в кнопку.
#[derive(Clone, Copy, PartialEq)]
enum TitleMode {
    /// Одна строка, многоточие в конце.
    Elide,
    /// Одна строка, многоточие в середине (видно начало и конец).
    Middle,
    /// До двух строк мелким шрифтом, дальше — многоточие.
    Wrap,
}

impl TitleMode {
    fn parse(s: &str) -> Self {
        match s {
            "middle" => Self::Middle,
            "wrap" => Self::Wrap,
            _ => Self::Elide,
        }
    }
}

fn task(w: WindowInfo, labels: bool, title_mode: TitleMode, max_width: f32, pc: PanelCtx) -> impl Widget {
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
        // `grow` отдаёт заголовку остаток ширины кнопки — без этого текст
        // не знает своей ширины и не обрезается, а вылезает за кнопку.
        let text = match title_mode {
            TitleMode::Elide => Text::new(title).max_lines(1).class("task-title grow"),
            TitleMode::Middle => Text::new(title).elide(Elide::Middle).class("task-title grow"),
            TitleMode::Wrap => Text::new(title).max_lines(2).class("task-title task-title-wrap grow"),
        };
        row = row.child(text);
    }
    let id = w.id;
    let slot = std::sync::Arc::new(syngui::core::sync::Mutex::new(Rect::zero()));
    TASKS.with(|t| t.borrow_mut().insert(id, (pc.clone(), slot.clone())));
    let pc_click = pc.clone();
    let area = InputArea::new(
        DecoratedBox::new()
            .child(crate::ui::vcenter(row))
            .class(cls)
            .style("max-width", syngui::mss::StyleValue::px(if labels { max_width } else { 48.0 })),
    )
    .pointer()
    .on_click(move |b, _, r| match b {
        MouseButton::Left => crate::actions::window_op(id, WindowOp::ToggleMinimize),
        MouseButton::Middle => crate::actions::window_op(id, WindowOp::Close),
        MouseButton::Right => ShellCtx::get().open_popup(PopupKind::WindowMenu(id), pc_click.anchor(r)),
        _ => {}
    });
    syngui::widgets::EventHook::new().report_bounds(slot).child(area)
}

/// Сообщать композитору прямоугольник значка окна `id` (туда оно
/// сворачивается анимацией) — для значков дока.
pub fn track(id: u64, pc: &PanelCtx, slot: std::sync::Arc<syngui::core::sync::Mutex<Rect>>) {
    TASKS.with(|t| t.borrow_mut().insert(id, (pc.clone(), slot)));
}

thread_local! {
    /// Кнопки окон на панелях: окно → (панель, границы кнопки).
    static TASKS: std::cell::RefCell<std::collections::HashMap<u64, (PanelCtx, std::sync::Arc<syngui::core::sync::Mutex<Rect>>)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static SENT: std::cell::RefCell<std::collections::HashMap<u64, (String, [i32; 4])>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Раз в секунду сообщать композитору, где на панели кнопка каждого окна —
/// туда он «сворачивает» окно анимацией. Шлются только изменения.
pub fn start_minimize_rects() {
    syngui_layer::add_timer(std::time::Duration::from_secs(1), || {
        let ctx = ShellCtx::get();
        if !ctx.connected.get_untracked() {
            return Some(std::time::Duration::from_secs(1));
        }
        let alive: Vec<u64> = ctx.windows.get_untracked().iter().map(|w| w.id).collect();
        TASKS.with(|t| t.borrow_mut().retain(|id, _| alive.contains(id)));
        SENT.with(|s| s.borrow_mut().retain(|id, _| alive.contains(id)));
        let list: Vec<(u64, PanelCtx, Rect)> = TASKS.with(|t| {
            t.borrow().iter().map(|(id, (pc, slot))| (*id, pc.clone(), *slot.lock().unwrap_or_else(|e| e.into_inner()))).collect()
        });
        for (id, pc, r) in list {
            if r.size.width <= 0.0 {
                continue;
            }
            let Some([x, y, w, h]) = pc.anchor(r).rect else { continue };
            // Композитору — в его единицах (масштаб интерфейса).
            let z = syngui_layer::ui_zoom();
            let rect = [(x * z).round() as i32, (y * z).round() as i32, (w * z).round() as i32, (h * z).round() as i32];
            let changed = SENT.with(|s| s.borrow().get(&id) != Some(&(pc.output.clone(), rect)));
            if changed {
                SENT.with(|s| s.borrow_mut().insert(id, (pc.output.clone(), rect)));
                crate::actions::send(synshell_common::ipc::Request::SetMinimizeRect { id, output: pc.output.clone(), rect });
            }
        }
        Some(std::time::Duration::from_secs(1))
    });
}
