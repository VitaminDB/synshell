//! Домашний экран телефона — рабочий стол: одна непрозрачная поверхность
//! под окнами (слой Bottom, на CPU-композиторе без полноэкранного
//! смешивания), в ней обои и страницы, листаемые пальцем:
//!
//! - на каждом — виджеты и значки (`synshell_ui::desk`, `[[widget]]`):
//!   по умолчанию первый стол — «Сводка» (`[mobile] resources_page`: часы,
//!   процессор, память, графика, питание, сеть, запущенные приложения),
//!   остальные — сетка значков (`[mobile] home_apps` или все) при
//!   `[wallpaper] desktop_icons`; виджеты добавляются, двигаются и
//!   настраиваются удержанием на рабочем столе.
//!
//! Страницы — рабочие столы (`[workspaces] count`): листание пальцем
//! переключает стол в композиторе, смена стола извне листает страницы;
//! столы — точки вверху, касание — переход.
//!
//! Удержание на пустом месте — меню рабочего стола (добавить панель или
//! док, режим окон, обои, параметры), на значке — меню приложения.
//! Свайп вверх — меню запуска, вниз — шторка.

use std::cell::Cell;
use synshell_ui::ui::rx;
use synshell_ui::ShellCtx;
use syngui::prelude::*;
use syngui::widgets::{Carousel, PanAxis, SwipeDirection};
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static PAGE: Cell<Option<RwSignal<usize>>> = const { Cell::new(None) };
    static BUILT: Cell<Option<(u64, u64, usize)>> = const { Cell::new(None) };
}

fn page_signal() -> RwSignal<usize> {
    PAGE.with(|p| match p.get() {
        Some(s) => s,
        None => {
            let s = use_signal(0usize);
            p.set(Some(s));
            s
        }
    })
}

/// Активный рабочий стол (с 0).
fn active_workspace(ctx: &ShellCtx) -> usize {
    ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index as usize).unwrap_or(0)
}

/// Жест «домой»: страница домашнего экрана — активного стола.
pub fn show_apps() {
    let ctx = ShellCtx::get();
    page_signal().set(active_workspace(&ctx));
}

pub fn install(ctx: ShellCtx) {
    let page = page_signal();
    page.set(active_workspace(&ctx));
    // Страница = рабочий стол: стол сменили извне (панель, клавиши,
    // композитор) — домашний экран листается следом.
    create_effect(move || {
        let ws = ctx.workspaces.get();
        if let Some(a) = ws.iter().find(|w| w.active) {
            if page.get_untracked() != a.index as usize {
                page.set(a.index as usize);
            }
        }
    });
    // Ресурсы снимаются, только пока их страница на экране: домашний экран
    // не закрыт окном (страница 0 режима страниц или окон не видно).
    create_effect(move || {
        let p = page.get();
        let wins = ctx.windows.get();
        let mobile = ctx.mobile.get();
        // смена конфига (resources_page включили на лету) — пересчитать: ctx.cfg() не отслеживается
        let _ = ctx.generation.get();
        let home_shown = match &mobile {
            Some(m) if m.mode == synshell_common::action::MobileMode::Pages => m.page.is_none(),
            _ => !wins.iter().any(|w| !w.minimized),
        };
        let _ = ctx.config.get();
        synshell_ui::desk::data::set_visible(home_shown && synshell_ui::desk::has_data(&ctx, p));
    });
    create_effect(move || {
        let outputs = syngui_layer::outputs().get();
        let generation = ctx.generation.get();
        let apps = ctx.apps_rev.get();
        // Пересобрать при смене конфига (страницы, приложения), списка
        // приложений и выводов.
        let key = (generation, apps, outputs.len());
        if BUILT.with(|b| b.get()) == Some(key) {
            return;
        }
        BUILT.with(|b| b.set(Some(key)));
        if let Some(id) = SURFACE.with(|s| s.take()) {
            syngui_layer::close_surface(id);
        }
        let Some(out) = outputs.first().map(|o| o.name.clone()) else { return };
        let spec = SurfaceSpec {
            namespace: "syndesktop-home".into(),
            layer: Layer::Bottom,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            margin: [0; 4],
            // 0 — не заходить под панели, которые резервируют место.
            exclusive_zone: 0,
            keyboard: KeyboardInteractivity::None,
            output: Some(out.clone()),
            auto_size: false,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        };
        let out_name = out.clone();
        let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), out.clone())));
        synshell_ui::desk::accept_drops(id, out_name);
        SURFACE.with(|s| s.set(Some(id)));
    });
}

fn view(ctx: ShellCtx, output: String) -> impl Widget {
    let page = page_signal();
    let slide = use_signal(0u64);
    // Положение листания столов (дробное) — обои-панорама едут за пальцем.
    let scroll = use_signal(page.get_untracked() as f32);
    let cfg = ctx.cfg();
    // Страница — рабочий стол (точки столов — вверху, свои у карусели не
    // нужны): на первом — сводка ресурсов, на остальных — значки или пусто.
    // Сигнал страницы карусель читает при пересборке — отсюда `rx`.
    let count = cfg.workspaces.count.max(1) as usize;
    let wrap = cfg.workspaces.wrap;
    let out = output.clone();
    let pages = rx(move || {
        let _ = page.get();
        // перелистывание — по группе «Домашний экран» ([animations] home; «Анимации» и «Меньше движения» — через неё)
        let slide_ms = ctx.cfg().animations.group_ms("home", 350);
        let mut pages = Carousel::new()
            .page_signal(page)
            .position_signal(scroll)
            .slide_duration_ms(slide_ms)
            .show_arrows(false)
            .show_indicators(false);
        for i in 0..count {
            pages = pages.child(synshell_ui::desk::page_view(ctx, i, out.clone()));
        }
        Box::new(
            pages
                // Пролистали пальцем — стол в композиторе.
                .on_page_change(|i| {
                    let ctx = ShellCtx::get();
                    if active_workspace(&ctx) != i {
                        let t = synshell_common::action::WorkspaceTarget::Index(i as u32 + 1);
                        synshell_ui::actions::run(synshell_common::Action::Workspace(t));
                    }
                })
                // За последний стол — на первый и наоборот (`[workspaces] wrap`).
                .on_overscroll(move |d| {
                    if wrap {
                        use synshell_common::action::WorkspaceTarget;
                        let t = if d > 0 { WorkspaceTarget::Next } else { WorkspaceTarget::Prev };
                        synshell_ui::actions::run(synshell_common::Action::Workspace(t));
                    }
                })
                .class("home-pages"),
        )
    });
    let gestures = GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_swipe(|dir, _| match dir {
            SwipeDirection::Up => synshell_ui::commands::handle("launcher"),
            SwipeDirection::Down => synshell_ui::commands::handle("shade"),
            _ => {}
        })
        .on_long_press(|_| {
            let ctx = ShellCtx::get();
            synshell_ui::edit::open_at_press(&ctx, synshell_ui::ctx::PopupKind::DesktopMenu);
        })
        .on_secondary_click(|_| {
            let ctx = ShellCtx::get();
            synshell_ui::edit::open_at_press(&ctx, synshell_ui::ctx::PopupKind::DesktopMenu);
        })
        .on_click(|| ShellCtx::get().close_popup())
        .child(pages);
    Stack::new()
        .fit(StackFit::Expand)
        .child(synshell_ui::manager::wallpaper_view(output, slide, Some(scroll)))
        .child(DecoratedBox::new().class("home-scrim"))
        .child(gestures)
        .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(workspace_dots(ctx)))
}

/// Рабочие столы точками вверху домашнего экрана (домашний экран общий
/// для всех столов — видно, на каком ты); касание — переход на стол.
fn workspace_dots(ctx: ShellCtx) -> impl Widget {
    rx(move || {
        let list = ctx.workspaces.get();
        if list.len() < 2 {
            return Box::new(DecoratedBox::new());
        }
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for w in &list {
            let class = if w.active {
                "home-ws-dot home-ws-dot-on"
            } else if w.windows > 0 {
                "home-ws-dot home-ws-dot-busy"
            } else {
                "home-ws-dot"
            };
            let n = w.index + 1;
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        let t = synshell_common::action::WorkspaceTarget::Index(n);
                        synshell_ui::actions::run(synshell_common::Action::Workspace(t));
                    })
                    .child(DecoratedBox::new().child(DecoratedBox::new().class(class)).class("home-ws-hit")),
            );
        }
        Box::new(DecoratedBox::new().child(row).class("home-ws"))
    })
}
