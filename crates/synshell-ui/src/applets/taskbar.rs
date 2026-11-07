//! Панель задач: окна текущего стола (или всех), значки по .desktop,
//! заголовки. ЛКМ — свернуть/развернуть, СКМ — закрыть, ПКМ — меню окна,
//! колесо — перебор окон.
//!
//! Кнопки занимают только место между соседними апплетами: при нехватке
//! подписи сжимаются поровну (с многоточием), потом остаются значки, а
//! окна, которым не хватило и значков, уходят под кнопку «+N» со списком
//! (`overflow = "scroll"` — вместо неё прокрутка ряда).

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

/// Зазор между кнопками окон.
const GAP: f32 = 3.0;
/// Ширина кнопки «только значок» (значок 22 + поля 6+6 + рамка; с запасом
/// на темы с рамкой в 2 px). Вдоль вертикальной панели — её высота.
const ICON_W: f32 = 38.0;
const ICON_H: f32 = 36.0;
/// Уже этого подпись не показывается: остались бы значок и «…».
const LABEL_MIN_W: f32 = 96.0;
/// Кнопка «+N» (окна, которым не хватило места).
const MORE_W: f32 = 48.0;
const MORE_H: f32 = 34.0;

/// Раскладка кнопок в отведённой длине.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fit {
    /// Сколько кнопок показать (остальные — в «+N» или прокрутке).
    pub shown: usize,
    /// Предел длины кнопки вдоль панели.
    pub per: f32,
    /// Подписи помещаются.
    pub labels: bool,
    /// Не поместились даже значки.
    pub overflow: bool,
}

/// Сколько кнопок из `n` и какой длины помещается в `avail` (вдоль
/// панели). Сначала кнопки с подписями сжимаются поровну (не длиннее
/// `max_w`), потом остаются значки, а если и их слишком много — первые
/// значки и кнопка «+N» (при `menu`; без неё — все значки, прокрутка).
pub(crate) fn fit(n: usize, avail: f32, labels: bool, max_w: f32, icon_w: f32, more_w: f32, menu: bool) -> Fit {
    let gaps = |k: usize| GAP * k.saturating_sub(1) as f32;
    if n == 0 || !avail.is_finite() {
        return Fit { shown: n, per: if labels { max_w } else { icon_w }, labels, overflow: false };
    }
    if labels {
        let per = ((avail - gaps(n)) / n as f32).min(max_w);
        if per >= LABEL_MIN_W.min(max_w) {
            return Fit { shown: n, per, labels: true, overflow: false };
        }
    }
    if icon_w * n as f32 + gaps(n) <= avail {
        return Fit { shown: n, per: icon_w, labels: false, overflow: false };
    }
    if !menu {
        return Fit { shown: n, per: icon_w, labels: false, overflow: true };
    }
    // k значков, зазоры между ними и перед «+N», сама «+N».
    let k = ((avail - more_w) / (icon_w + GAP)).floor().max(0.0) as usize;
    Fit { shown: k.min(n - 1), per: icon_w, labels: false, overflow: true }
}

/// Какие окна показать кнопками, а какие — в «+N»: первые `shown` по
/// порядку, но окно в фокусе всегда на виду (встаёт на последнее место).
pub(crate) fn split_shown<T: Clone>(list: &[T], shown: usize, focused: impl Fn(&T) -> bool) -> (Vec<T>, Vec<T>) {
    let mut vis: Vec<T> = list.iter().take(shown).cloned().collect();
    let mut rest: Vec<T> = list.iter().skip(shown).cloned().collect();
    if shown > 0 {
        if let Some(i) = rest.iter().position(&focused) {
            let f = rest.remove(i);
            let out = vis.pop().expect("shown > 0");
            vis.push(f);
            rest.insert(0, out);
        }
    }
    (vis, rest)
}

pub fn build(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let labels = a.bool_or("labels", !pc.vertical);
    let all = a.bool_or("all_workspaces", false);
    let only_output = a.bool_or("only_output", false);
    let max_width = a.int_or("max_width", 220) as f32;
    let title_mode = TitleMode::parse(a.str_or("title", "elide"));
    // Окна не помещаются даже значками: «menu» — кнопка «+N» со списком,
    // «scroll» — ряд прокручивается.
    let menu = a.str_or("overflow", "menu") != "scroll";
    let pc = pc.clone();
    // Длина, отведённая панелью задач между соседними апплетами (её слот
    // растягивается, `flex-grow`): по ней кнопки сжимаются и уходят в «+N».
    // До первой раскладки неизвестна — бесконечность (как раньше).
    let avail = use_signal(f32::INFINITY);
    // Толщина слота поперёк панели: на вертикальной ряд кнопок занимает её целиком,
    // и кнопки встают по оси панели (как часы), а не к левому краю слота.
    let thick = use_signal(0.0f32);
    let vertical = pc.vertical;
    let inner = crate::ui::rx(move || {
        let windows = ctx.windows.get();
        let ws = ctx.active_workspace();
        let list: Vec<WindowInfo> =
            windows.iter().filter(|w| visible(w, ws, all, &pc.output, only_output)).cloned().collect();
        let len = avail.get();
        let f = if vertical {
            let f = fit(list.len(), len, false, ICON_H, ICON_H, MORE_H, menu);
            Fit { labels, ..f }
        } else {
            fit(list.len(), len, labels, max_width, ICON_W, MORE_W, menu)
        };
        let (shown, hidden) = split_shown(&list, f.shown, |w| w.focused);
        let mut flex = Flex::new()
            .direction(if vertical { FlexDirection::Column } else { FlexDirection::Row })
            .gap(GAP)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        let th = thick.get();
        for w in shown {
            let cap = if vertical { max_width } else { f.per };
            flex = flex.child(task(w, f.labels, title_mode, cap, pc.clone()));
        }
        if !hidden.is_empty() && menu {
            flex = flex.child(more_button(hidden.iter().map(|w| w.id).collect(), hidden.iter().any(|w| w.focused || w.urgent), pc.clone()));
        }
        let body: Box<dyn Widget> = if f.overflow && !menu && len.is_finite() {
            let (sv, dim) = if vertical { (ScrollView::new().vertical(), "height") } else { (ScrollView::new().horizontal(), "width") };
            Box::new(sv.child(flex).class("taskbar-scroll").style(dim, syngui::mss::StyleValue::px(len)))
        } else if vertical && th > 0.0 {
            Box::new(Column::new().width(th).cross_axis_alignment(CrossAxisAlignment::Center).child(flex))
        } else {
            Box::new(flex)
        };
        Box::new(
            // Обрезка — страховка до первой раскладки: кнопки не ложатся
            // поверх соседних апплетов.
            InputArea::new(DecoratedBox::new().child(body).clip(true).class("applet-taskbar")).on_wheel(move |dy| {
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
        ) as Box<dyn Widget>
    });
    Box::new(syngui::widgets::EventHook::new().fill().child(inner).on_resize(move |s| {
        let v = if vertical { s.height } else { s.width };
        if vertical && s.width > 0.0 && (thick.get_untracked() - s.width).abs() >= 1.0 {
            thick.set(s.width);
        }
        if v > 0.0 && (avail.get_untracked() - v).abs() >= 1.0 {
            avail.set(v);
        }
    }))
}

/// Кнопка «+N»: окна, которым не хватило места, — списком.
fn more_button(ids: Vec<u64>, attention: bool, pc: PanelCtx) -> impl Widget {
    let n = ids.len();
    let mut cls = String::from("task task-more");
    if attention {
        cls.push_str(" task-active");
    }
    InputArea::new(
        DecoratedBox::new()
            .child(crate::ui::vcenter(Text::new(format!("+{n}")).max_lines(1).class("task-title task-more-label")))
            .class(cls),
    )
    .pointer()
    .on_click(move |b, _, r| {
        if b == MouseButton::Left {
            let ctx = ShellCtx::get();
            let kind = PopupKind::TaskOverflow(ids.clone());
            if ctx.popup.get_untracked().is_some_and(|p| p.kind == kind) {
                ctx.close_popup();
            } else {
                ctx.open_popup(kind, pc.anchor(r));
            }
        }
    })
}

/// Список окон из «+N»: щелчок — к окну, средняя — закрыть, правая — меню окна.
pub fn overflow_view(ids: Vec<u64>) -> impl Widget {
    let ctx = ShellCtx::get();
    Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Ещё окна")).class("popup-title")).child(
        move || {
            let windows = ctx.windows.get();
            let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            for w in ids.iter().filter_map(|id| windows.iter().find(|w| w.id == *id)) {
                let title = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
                let icon: Box<dyn Widget> = match crate::xdg::window_icon(&w.app_id) {
                    Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("task-icon")),
                    None => Box::new(crate::ui::icon(crate::ui::mi::WINDOW).class("task-glyph")),
                };
                let mut cls = String::from("menu-item");
                if w.focused {
                    cls.push_str(" task-active");
                }
                let label = Text::new(title)
                    .max_lines(1)
                    .class(if w.minimized { "menu-label grow task-overflow-minimized" } else { "menu-label grow" });
                let id = w.id;
                col = col.child(
                    InputArea::new(
                        DecoratedBox::new()
                            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon).child(label))
                            .class(cls),
                    )
                    .pointer()
                    .on_click(move |b, _, _| {
                        let ctx = ShellCtx::get();
                        match b {
                            MouseButton::Left => {
                                ctx.close_popup();
                                crate::actions::window_op(id, WindowOp::Activate);
                            }
                            MouseButton::Middle => crate::actions::window_op(id, WindowOp::Close),
                            MouseButton::Right => {
                                if let Some(a) = ctx.popup.get_untracked().map(|p| p.anchor) {
                                    ctx.open_popup(PopupKind::WindowMenu(id), a);
                                }
                            }
                            _ => {}
                        }
                    }),
                );
            }
            col
        },
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn total(f: &Fit, menu_btn: bool) -> f32 {
        let k = f.shown + menu_btn as usize;
        f.per * f.shown as f32 + if menu_btn { MORE_W } else { 0.0 } + GAP * k.saturating_sub(1) as f32
    }

    #[test]
    fn roomy_bar_keeps_full_labels() {
        let f = fit(4, 900.0, true, 220.0, ICON_W, MORE_W, true);
        assert_eq!(f, Fit { shown: 4, per: 220.0, labels: true, overflow: false });
    }

    #[test]
    fn labels_shrink_evenly_then_turn_into_icons() {
        // 1280×800 по умолчанию: между пейджером и правыми апплетами ~700 px.
        let f = fit(6, 700.0, true, 220.0, ICON_W, MORE_W, true);
        assert!(f.labels && f.shown == 6 && f.per < 220.0 && f.per >= LABEL_MIN_W);
        assert!(total(&f, false) <= 700.0);
        let f = fit(12, 700.0, true, 220.0, ICON_W, MORE_W, true);
        assert_eq!((f.labels, f.shown, f.overflow), (false, 12, false));
        assert!(total(&f, false) <= 700.0);
    }

    #[test]
    fn too_many_windows_go_to_more_button() {
        for avail in [0.0, 30.0, 120.0, 330.0, 700.0] {
            for n in 1..60 {
                let f = fit(n, avail, true, 220.0, ICON_W, MORE_W, true);
                let more = f.shown < n;
                assert_eq!(more, f.overflow, "n={n} avail={avail}");
                // Правые апплеты не перекрываются: всё — в отведённой длине
                // (кроме «+N» в совсем узком месте, где не влезает и она).
                if avail >= MORE_W {
                    assert!(total(&f, more) <= avail + 0.01, "n={n} avail={avail} {f:?}");
                }
            }
        }
        // 16 окон на 700 px — значки, без «+N»; 19 — 15 значков и «+4».
        assert!(!fit(16, 700.0, true, 220.0, ICON_W, MORE_W, true).overflow);
        assert_eq!(fit(19, 700.0, true, 220.0, ICON_W, MORE_W, true).shown, 15);
        let f = fit(19, 330.0, true, 220.0, ICON_W, MORE_W, true);
        assert!(f.overflow && f.shown == 6);
    }

    #[test]
    fn scroll_mode_shows_all_icons() {
        let f = fit(30, 330.0, true, 220.0, ICON_W, MORE_W, false);
        assert_eq!((f.shown, f.labels, f.overflow), (30, false, true));
    }

    #[test]
    fn unknown_length_shows_everything() {
        assert_eq!(fit(19, f32::INFINITY, true, 220.0, ICON_W, MORE_W, true).shown, 19);
    }

    #[test]
    fn focused_window_stays_visible() {
        let list: Vec<u32> = (0..10).collect();
        let (vis, rest) = split_shown(&list, 4, |w| *w == 7);
        assert_eq!(vis, vec![0, 1, 2, 7]);
        assert_eq!(rest, vec![3, 4, 5, 6, 8, 9]);
        let (vis, rest) = split_shown(&list, 4, |w| *w == 1);
        assert_eq!((vis, rest.len()), (vec![0, 1, 2, 3], 6));
        let (vis, rest) = split_shown(&list, 0, |w| *w == 1);
        assert_eq!((vis.len(), rest.len()), (0, 10));
    }
}
