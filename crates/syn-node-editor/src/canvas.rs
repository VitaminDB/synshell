//! Холст редактора нод: панорама и масштаб (`PanZoomViewport`, щипок и
//! колесо), сетка, провода и карточки нод, панель масштаба, палитра «Добавить
//! ноду» по правой кнопке на фоне, диалог оттенка ноды.
//!
//! Программа встраивает холст в свою страницу: [`view`] для конкретного
//! [`NodeEditorCtx`] (обычно — активного графа `EditorWorkspace`), а свои
//! панели поверх холста (панель моделей Synthos, инспектор видеоредактора)
//! передаёт в `overlays` — они кладутся между панелью масштаба и меню.

use syngui::input::MouseButton;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::PanZoomViewport;
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
use crate::types::NodeId;
use syngui::containers::Positioned;
use syngui::EventHook;
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row, Stack, ToolButton};

use crate::icons::{MI_FIT_SCREEN, MI_ZOOM_IN, MI_ZOOM_OUT};
use crate::registry::{self, registry as node_registry, NodeCategory};
use crate::state::NodeEditorCtx;
use crate::{node_view, style_dialog, wires};

/// Холст графа `ctx` с панелями программы `overlays` поверх него.
pub fn view(ctx: NodeEditorCtx, overlays: Vec<Box<dyn Widget>>) -> impl Widget {
    view_with(ctx, overlays, true)
}

/// Холст без плавающей панели масштаба: программа ставит [`zoom_controls`]
/// в свою шапку (видеоредактор).
pub fn view_bare(ctx: NodeEditorCtx, overlays: Vec<Box<dyn Widget>>) -> impl Widget {
    view_with(ctx, overlays, false)
}

fn view_with(ctx: NodeEditorCtx, overlays: Vec<Box<dyn Widget>>, zoom_toolbar: bool) -> impl Widget {
    let menu_ctx = ctx;
    let filter_ctx = ctx;
    let viewport = PanZoomViewport::new()
        .pan(ctx.pan)
        .zoom(ctx.zoom)
        .zoom_range(0.25, 4.0)
        .grid(true)
        .grid_step(40.0)
        .pan_button(MouseButton::Middle)
        // LMB-pan не активируется в 10px-зоне вокруг любой ноды — иначе
        // случайный клик возле карточки начинает pan вместо drag-handle'а
        // или ловли port'а. `world` приходит в local-content-coord
        // (canvas-local), та же система, что `node.bounds`.
        .pan_filter(move |world| {
            const INFLATE: f32 = 10.0;
            let nodes = filter_ctx.nodes.get_untracked();
            !nodes.iter().any(|n| {
                let b = n.bounds.get_untracked();
                let pos = n.pos.get_untracked();
                // bounds.origin published в position-pass-coord, а world здесь
                // — в canvas-local; используем `pos` как canvas-local-origin
                // ноды (Stack кладёт Positioned в Stack.origin + node.pos).
                let left = pos.x - INFLATE;
                let top = pos.y - INFLATE;
                let right = pos.x + b.size.width + INFLATE;
                let bottom = pos.y + b.size.height + INFLATE;
                world.x >= left && world.x <= right
                    && world.y >= top && world.y <= bottom
            })
        })
        .on_background_context_menu(move |world, screen| {
            menu_ctx.open_bg_menu(world, screen);
        })
        .child(world_layer(ctx))
        .class("node-editor-viewport");

    let bg_menu = build_bg_menu(ctx);
    let mut layers: Vec<Box<dyn Widget>> = vec![Box::new(viewport)];
    if zoom_toolbar {
        layers.push(Box::new(overlay_row(ctx, build_toolbar(ctx))));
    }
    layers.extend(overlays);
    let canvas_size = use_signal(syngui::core::Size::zero());
    layers.push(Box::new(selection_bar(ctx, canvas_size)));
    layers.push(bg_menu);
    layers.push(Box::new(style_dialog::view(ctx)));
    // слои на всю площадь холста: полоса выделения прижимается к низу
    let frame_inner = Stack::new().fit(StackFit::Expand).children(layers);

    DecoratedBox::new().child(EventHook::new().on_resize(move |s| canvas_size.set(s)).child(frame_inner)).class("ne-canvas-frame")
}

/// Верхняя «overlay-полоса» canvas'а: zoom-toolbar слева. Run-пилюля
/// переехала в общую шапку страницы.
///
/// **Pass-through events**: возвращаем `Padding` напрямую (без DecoratedBox-
/// обёртки). Stack растянет padding на всю canvas, но прозрачные layout-
/// контейнеры (Padding/Row) не ловят события — клик/drag/zoom в пустом месте
/// уходит на viewport ниже. DecoratedBox в overlay-host раньше перехватывал
/// все события и блокировал pan/zoom — поэтому его и нет.
///
fn overlay_row(_ctx: NodeEditorCtx, toolbar: Box<dyn Widget>) -> impl Widget {
    let row = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .main_axis_alignment(MainAxisAlignment::Start)
        .children(vec![toolbar]);

    // Padding без DecoratedBox-обёртки: Padding/Row реализуют
    // `passthrough_hit_test() == true`, поэтому пустые места строки
    // пропускают события дальше в Stack (viewport получает pan/zoom/click).
    // Row.cross_axis_alignment(Start) делает строку intrinsic-высоты —
    // нижняя часть canvas остаётся свободной для drag-нод.
    Padding::only(16.0, 16.0, 16.0, 0.0).child(row)
}
/// Реактивный «мир» — Canvas wires + ноды на абсолютных позициях.
fn world_layer(ctx: NodeEditorCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Подмена контекста: внутри Reactive use_context::<NodeEditorCtx>()
        // должно вернуть именно эту вкладку. provide_context на уровне
        // Reactive children — единственное место, где это можно сделать
        // согласованно для wires::view() и node_view::view(node).
        provide_context(ctx);
        let nodes = ctx.nodes.get();
        let mut stack = Stack::new();
        stack = stack.child(wires::view());
        for node in nodes {
            stack = stack.children(vec![node_view::view(node)]);
        }
        // Без DecoratedBox-обёртки: оборачивание в DecoratedBox с прозрачным
        // фоном лишь ставило не-passthrough контейнер между Reactive и Stack —
        // в моменты `bounds=(0,0)` (rebuild) hit-test обрезался на нём, ноды
        // не получали клики после pan/scroll. Stack passthrough даёт прямой
        // путь к Positioned'ам и далее к карточкам нод.
        vec![Box::new(stack) as Box<dyn Widget>]
    })
}

fn build_toolbar(ctx: NodeEditorCtx) -> Box<dyn Widget> {
    let row = zoom_row(ctx);
    let panel = DecoratedBox::new()
        .child(Padding::symmetric(8.0, 4.0).child(row))
        .class("node-editor-toolbar");
    Box::new(panel)
}

fn zoom_row(ctx: NodeEditorCtx) -> Row {
    let ctx_in = ctx;
    let ctx_out = ctx;
    let ctx_fit = ctx;

    // DecoratedBox-host даёт Reactive фиксированные constraints (через MSS
     // width/height в `.node-editor-toolbar-zoom-host`). Без обёртки
    // Reactive.layout получает max_width=INF от Row и схлопывается до 0×0
    // — Text внутри не рисуется. См. Reactive.layout в syngui.
    let label = DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let z = ctx_in.zoom.get();
            let pct = (z * 100.0).round() as i32;
            vec![Box::new(Center::new().child(Text::new(format!("{}%", pct)).class("node-editor-toolbar-zoom")))]
        }))
        .class("node-editor-toolbar-zoom-host");

    let row = mgui! {
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                ToolButton::new(MI_ZOOM_OUT)
                    .tooltip(tr!("nodes.toolbar.zoom_out"))
                    .on_click(move || {
                        let z = (ctx_out.zoom.get_untracked() * 0.8).max(0.25);
                        ctx_out.zoom.set(z);
                    })
                    .class("node-editor-toolbar-btn"),
                label,
                ToolButton::new(MI_ZOOM_IN)
                    .tooltip(tr!("nodes.toolbar.zoom_in"))
                    .on_click(move || {
                        let z = (ctx_in.zoom.get_untracked() * 1.25).min(4.0);
                        ctx_in.zoom.set(z);
                    })
                    .class("node-editor-toolbar-btn"),
                ToolButton::new(MI_FIT_SCREEN)
                    .tooltip(tr!("nodes.toolbar.reset_view"))
                    .on_click(move || ctx_fit.reset_view())
                    .class("node-editor-toolbar-btn"),
            ]
    };
    row
}

/// Кнопки масштаба холста (−, проценты, +, весь граф) без плавающей подложки —
/// для шапки программы.
pub fn zoom_controls(ctx: NodeEditorCtx) -> impl Widget {
    zoom_row(ctx)
}

/// PopupMenu «Add Node» — открывается через ctx.menu_open + позиция в
/// ctx.menu_world_pos. Top-level — категории, внутри каждой категории —
/// либо leaf-пункты (для нод без `subcategory`), либо submenu по
/// `subcategory` (для группировки EQ и пр.). Mixer обрабатывается
/// отдельно — его 5 пресетов (2/3/4/5 входа + Custom) живут в подменю
/// «Аудио микшеры» внутри DSP-эффектов и стартуют ноду с нужным
/// `n_inputs`.
fn build_bg_menu(ctx: NodeEditorCtx) -> Box<dyn Widget> {
    use crate::icons::MI_MERGE_TYPE;
    use crate::types::NodeKind;
    use std::collections::BTreeMap;

    let items: Vec<MenuItem> = NodeCategory::all()
        .iter()
        .filter_map(|cat| {
            // Top-level: ноды этой категории без subcategory и (для DspEffects)
            // не Mixer (он переезжает в спец-пункт).
            let mut top: Vec<MenuItem> = node_registry()
                .iter()
                .filter(|m| {
                    m.category == *cat
                        && m.subcategory.is_none()
                        && !(matches!(m.kind, NodeKind::Mixer) && *cat == NodeCategory::DspEffects)
                })
                .map(|m| MenuItem::new(format!("add:{}", m.title), crate::i18n::node_title(m)).icon(m.icon))
                .collect();

            // Submenu по subcategory.
            let mut by_sub: BTreeMap<&'static str, Vec<&'static registry::NodeKindMeta>> =
                BTreeMap::new();
            for m in node_registry().iter().filter(|m| m.category == *cat) {
                if let Some(sub) = m.subcategory {
                    by_sub.entry(sub).or_default().push(m);
                }
            }
            for (sub_key, metas) in by_sub {
                let icon = metas.first().map(|m| m.icon).unwrap_or("");
                let children: Vec<MenuItem> = metas
                    .iter()
                    .map(|m| MenuItem::new(format!("add:{}", m.title), crate::i18n::node_title(m)).icon(m.icon))
                    .collect();
                top.push(
                    MenuItem::new(format!("sub:{}:{}", cat.key(), sub_key), crate::i18n::node_subcategory_label(sub_key))
                        .icon(icon)
                        .children(children),
                );
            }

            // Спец-обработка Mixer: 5 пресетов (2/3/4/5 + Custom).
            if *cat == NodeCategory::DspEffects {
                let mixer_presets = vec![
                    MenuItem::new("add_mixer:2", trn!("nodes.menu.mixer_sources", 2)),
                    MenuItem::new("add_mixer:3", trn!("nodes.menu.mixer_sources", 3)),
                    MenuItem::new("add_mixer:4", trn!("nodes.menu.mixer_sources", 4)),
                    MenuItem::new("add_mixer:5", trn!("nodes.menu.mixer_sources", 5)),
                    MenuItem::new("add_mixer:custom", tr!("nodes.menu.mixer_custom")),
                ];
                top.push(
                    MenuItem::new("sub:mixer", tr!("nodes.menu.mixer_group"))
                        .icon(MI_MERGE_TYPE)
                        .children(mixer_presets),
                );
            }

            if top.is_empty() {
                return None;
            }
            Some(
                MenuItem::new(format!("cat:{:?}", cat), crate::i18n::node_category_label(*cat))
                    .icon(cat.icon())
                    .children(top),
            )
        })
        .collect();

    let ctx_select = ctx;
    Box::new(
        PopupMenu::new()
            .is_open(ctx.menu_open)
            .position(ctx.menu_screen_pos)
            .items(items)
            .on_select(move |action| {
                if let Some(rest) = action.strip_prefix("add:") {
                    let kind_opt = node_registry().iter().find(|m| m.title == rest).map(|m| m.kind);
                    if let Some(k) = kind_opt {
                        let world = ctx_select.menu_world_pos.get_untracked();
                        ctx_select.add_node(k, world);
                    }
                } else if let Some(rest) = action.strip_prefix("add_mixer:") {
                    let n: usize = match rest {
                        "custom" => 2,
                        s => s.parse().unwrap_or(2),
                    };
                    let world = ctx_select.menu_world_pos.get_untracked();
                    let id = ctx_select.add_node(NodeKind::Mixer, world);
                    ctx_select.set_mixer_n_inputs(id, n);
                }
                ctx_select.menu_open.set(false);
            }),
    )
}

/// Полоса над холстом при выделенных нодах: сколько выделено, действия
/// программы над выделением (например, «Объединить в блок»), «Снять».
fn selection_bar(ctx: NodeEditorCtx, size: RwSignal<syngui::core::Size>) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // только живые ноды: холст мог перезаполниться (объединение в блок, отмена)
        let nodes = ctx.nodes.get();
        let picked: Vec<NodeId> = ctx.picked.get().into_iter().filter(|id| nodes.iter().any(|n| n.id == *id)).collect();
        if picked.len() < 2 {
            return Vec::new();
        }
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("Выделено нод: {}", picked.len())).class("ne-selbar-text"));
        // встроенное: ноды ядра — в подграф одной нодой
        if crate::nodes::flow::can_group(&ctx, &picked) {
            let sel = picked.clone();
            row = row.child(Button::new("Объединить в подграф").class("ne-selbar-btn").on_click(move || {
                if let Err(e) = crate::nodes::flow::group_nodes(&ctx, &sel) {
                    tracing::warn!("подграф: {e}");
                    crate::host::notify(crate::host::Notice::Error, e.clone());
                }
            }));
        }
        if let Some(h) = crate::node_menu() {
            for (k, label) in (h.items)(&ctx, picked[0], &picked) {
                let sel = picked.clone();
                let first = picked[0];
                row = row.child(Button::new(label).class("ne-selbar-btn").on_click(move || {
                    if let Some(h) = crate::node_menu() {
                        (h.run)(&ctx, first, &sel, &k);
                    }
                }));
            }
        }
        row = row.child(Button::new("Снять выделение").class("ne-selbar-btn").on_click(move || ctx.pick_clear()));
        let bar = DecoratedBox::new().class("ne-selbar").child(row);
        // у нижнего края холста — не закрывает шапки нод
        let h = size.get().height;
        let y = if h > 160.0 { h - 96.0 } else { 12.0 };
        vec![Box::new(Positioned::new(bar).at(16.0, y))]
    })
}
