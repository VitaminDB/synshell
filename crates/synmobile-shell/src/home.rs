//! Домашний экран телефона — рабочий стол: одна непрозрачная поверхность
//! под окнами (слой Bottom, на CPU-композиторе без полноэкранного
//! смешивания), в ней обои и страницы, листаемые пальцем:
//!
//! - «Сводка» (`[mobile] resources_page`): часы, процессор, память,
//!   батарея — экран ресурсов развернётся на следующем этапе;
//! - «Приложения»: сетка значков (`[mobile] home_apps` или все).
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
    static BUILT: Cell<Option<(u64, usize)>> = const { Cell::new(None) };
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

/// Номер страницы приложений (после сводки, если она включена).
fn apps_page(ctx: &ShellCtx) -> usize {
    usize::from(ctx.cfg().mobile.resources_page)
}

/// Показать страницу приложений (жест «домой»).
pub fn show_apps() {
    let ctx = ShellCtx::get();
    page_signal().set(apps_page(&ctx));
}

pub fn install(ctx: ShellCtx) {
    let page = page_signal();
    page.set(apps_page(&ctx));
    // Ресурсы снимаются, только пока их страница на экране: домашний экран
    // не закрыт окном (страница 0 режима страниц или окон не видно).
    create_effect(move || {
        let p = page.get();
        let wins = ctx.windows.get();
        let mobile = ctx.mobile.get();
        let home_shown = match &mobile {
            Some(m) if m.mode == synshell_common::action::MobileMode::Pages => m.page.is_none(),
            _ => !wins.iter().any(|w| !w.minimized),
        };
        crate::resources::set_visible(ctx.cfg().mobile.resources_page && p == 0 && home_shown);
    });
    create_effect(move || {
        let outputs = syngui_layer::outputs().get();
        let generation = ctx.generation.get();
        // Пересобрать при смене конфига (страницы, приложения) и выводов.
        let key = (generation, outputs.len());
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
    let cfg = ctx.cfg();
    let mut pages = Carousel::new().page_signal(page).show_arrows(false).show_indicators(true);
    if cfg.mobile.resources_page {
        pages = pages.child(crate::resources::page(ctx));
    }
    let pages = pages.child(apps_page_view(ctx)).class("home-pages");
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
        .child(synshell_ui::manager::wallpaper_view(output, slide))
        .child(DecoratedBox::new().class("home-scrim"))
        .child(gestures)
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
        let mut v: Vec<_> = xdg::apps().iter().filter(|a| !a.no_display).cloned().collect();
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
