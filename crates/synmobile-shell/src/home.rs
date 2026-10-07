//! Домашний экран телефона — рабочий стол: одна непрозрачная поверхность
//! под окнами (слой Bottom, на CPU-композиторе без полноэкранного
//! смешивания), в ней обои и страницы, листаемые пальцем:
//!
//! - первый стол — «Сводка» (`[mobile] resources_page`): часы, процессор,
//!   память, питание, запущенные приложения;
//! - остальные — сетка значков (`[mobile] home_apps` или все) при
//!   `[wallpaper] desktop_icons` («Значки на рабочем столе»), иначе пусто:
//!   приложения запускаются из «Пуска» и дока.
//!
//! Страницы — рабочие столы (`[workspaces] count`): листание пальцем
//! переключает стол в композиторе, смена стола извне листает страницы;
//! столы — точки вверху, касание — переход.
//!
//! Удержание на пустом месте — меню рабочего стола (добавить панель или
//! док, режим окон, обои, параметры), на значке — меню приложения.
//! Свайп вверх — меню запуска, вниз — шторка.

use std::cell::Cell;
use synshell_common::xdg;
use synshell_ui::launchers::{self, Launchable};
use synshell_ui::ui::{boxed, meter, mi, rx};
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
        crate::resources::set_visible(ctx.cfg().mobile.resources_page && p == 0 && home_shown);
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
        let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), out.clone())));
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
    let (resources, icons, wrap) = (cfg.mobile.resources_page, cfg.wallpaper.desktop_icons, cfg.workspaces.wrap);
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
            let p: Box<dyn Widget> = if i == 0 && resources {
                Box::new(crate::resources::page(ctx))
            } else if icons {
                Box::new(apps_page_view(ctx))
            } else {
                Box::new(DecoratedBox::new())
            };
            pages = pages.child(p);
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

// ─── Сводка ──────────────────────────────────────────────────────────────────

fn summary_page(ctx: ShellCtx) -> impl Widget {
    let clock = rx(move || {
        let now = ctx.now.get();
        Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(synshell_ui::clock::format(now, "%H:%M")).class("home-clock"))
                .child(Text::new(synshell_ui::clock::format(now, "%A, %d %B")).class("home-date")),
        )
    });
    let stats = rx(move || {
        let cpu = ctx.cpu.get();
        let mem = ctx.memory.get();
        let bat = ctx.battery.get();
        let mut col = Column::new().gap(14.0);
        col = col.child(stat_row(mi::CPU, "Процессор", cpu.round() as u32));
        col = col.child(stat_row(mi::MEMORY, "Память", mem.round() as u32));
        if let Some(b) = bat {
            let label = if b.charging { "Батарея · заряжается" } else { "Батарея" };
            col = col.child(stat_row("\u{E1A4}", label, b.percent));
        }
        Box::new(boxed("home-card", col))
    });
    let clock = Row::new().main_axis_alignment(MainAxisAlignment::Center).child(clock);
    ScrollView::new()
        .vertical()
        .child(Column::new().gap(28.0).child(clock).child(stats).class("home-page home-summary"))
}

fn stat_row(glyph: &str, label: &str, percent: u32) -> impl Widget {
    Column::new()
        .gap(6.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(synshell_ui::ui::icon(glyph).class("home-stat-icon"))
                .child(Text::new(label.to_string()).class("home-stat-label grow"))
                .child(Text::new(format!("{percent}%")).class("home-stat-value")),
        )
        .child(meter(percent))
}

// ─── Приложения ──────────────────────────────────────────────────────────────

fn apps_page_view(ctx: ShellCtx) -> impl Widget {
    let cfg = ctx.cfg();
    let entries: Vec<xdg::DesktopEntry> = if cfg.mobile.home_apps.is_empty() {
        // Программы Linux; приложения Android — в меню, своим разделом (закреплённые вручную — остаются)
        let mut v: Vec<_> = xdg::apps().iter().filter(|a| !a.no_display && a.android.is_none()).cloned().collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    } else {
        cfg.mobile.home_apps.iter().filter_map(|id| xdg::app_by_id(id)).collect()
    };
    let cols = cfg.mobile.home_columns.clamp(2, 8) as usize;
    let mut grid = Grid::new(cols).gap(4.0);
    for e in &entries {
        grid = grid.child(app_tile(ctx, e));
    }
    ScrollView::new().vertical().child(Column::new().child(grid).class("home-page home-apps"))
}

fn app_tile(ctx: ShellCtx, e: &xdg::DesktopEntry) -> impl Widget {
    let l = Launchable::from_entry(e);
    let key = l.key();
    let launch = l.clone();
    let id = e.id.clone();
    let icon = launchers::icon_widget(&l.icon, &None, "home-app-icon", 56.0);
    let tile = rx(move || {
        let launching = launchers::is_launching(&ctx, &key);
        let class = if launching { "home-app home-app-launching" } else { "home-app" };
        Box::new(DecoratedBox::new().class(class))
    });
    GestureDetector::new()
        .on_click(move || launchers::launch(ShellCtx::get(), &launch))
        .on_long_press(move |_| {
            let ctx = ShellCtx::get();
            synshell_ui::edit::open_at_press(&ctx, synshell_ui::ctx::PopupKind::HomeAppMenu(id.clone()));
        })
        .child(
            Stack::new().child(tile).child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon)
                    .child(Text::new(e.name.clone()).max_lines(2).class("home-app-name"))
                    .class("home-app-body"),
            ),
        )
}
