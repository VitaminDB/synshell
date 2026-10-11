//! Страницы: каркас (заголовок, главный график, прокрутка) и обзор.

mod hw;
mod procs;
mod system;

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::parts::{self, boxed, Line, W};
use crate::sampler::Snap;
use crate::state::{Page, St};

/// Что рисовать на странице: ширина и окно графиков.
#[derive(Clone, Copy)]
pub struct Cx {
    pub st: St,
    pub w: f32,
    pub span: usize,
    pub narrow: bool,
}

/// Страница целиком: панель (у процессов — поиск и сортировка) и прокручиваемое содержимое,
/// которое перестраивается с каждым снимком.
pub fn page(st: St, page: Page) -> Column {
    let content = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.snap.get();
        let w = super::content_width();
        let cx = Cx { st, w, span: st.span.get(), narrow: viewport_size().get().width < super::NARROW_BELOW };
        let items = match page {
            Page::Overview => overview(&cx, &s),
            Page::Cpu => hw::cpu(&cx, &s),
            Page::Gpu => hw::gpu(&cx, &s),
            Page::Vram => hw::vram(&cx, &s),
            Page::Npu => hw::npu(&cx, &s),
            Page::Memory => system::memory(&cx, &s),
            Page::Network => system::network(&cx, &s),
            Page::Disk => system::disk(&cx, &s),
            Page::Sensors => system::sensors(&cx, &s),
            Page::Battery => system::battery(&cx, &s),
            Page::Processes => procs::list(&cx, &s),
        };
        vec![boxed(Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(items).class(if cx.narrow { "page narrow" } else { "page" }))]
    });
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if page == Page::Processes {
        col = col.child(procs::toolbar(st, viewport_size().get_untracked().width < super::NARROW_BELOW));
    }
    col.child(ScrollView::new().vertical().child(content).class("grow"))
}

/// Заголовок страницы: значок, название, подпись; справа — выбор окна графиков.
pub fn header(cx: &Cx, page: Page, subtitle: Option<String>, span: bool) -> W {
    let mut text = Column::new().gap(1.0).child(Text::new(syngui::i18n::t(page.title())).max_lines(1).class("h1"));
    if let Some(s) = subtitle.filter(|s| !s.is_empty()) {
        text = text.child(Text::new(s).max_lines(2).selectable(true).class("muted"));
    }
    let mut row = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge_icon(page.icon(), page.color(), 46.0)).child(text.class("grow"));
    if span {
        row = row.child(span_switch(cx.st));
    }
    boxed(row.class("page-head"))
}

/// «1 мин | 5 мин».
fn span_switch(st: St) -> W {
    let cur = st.span.get_untracked();
    let mut row = Row::new().gap(2.0).class("seg");
    for (secs, label) in [(60usize, t!("1 мин")), (300, t!("5 мин"))] {
        row = row.child(
            GestureDetector::new()
                .on_click(move || st.span.set(secs))
                .child(DecoratedBox::new().child(Text::new(label).class("seg-label")).class(if cur == secs { "seg-item active" } else { "seg-item" })),
        );
    }
    boxed(row)
}

/// Главная карточка: крупное значение, подпись, график истории и легенда.
pub fn hero(cx: &Cx, color: Color, big: String, sub: String, lines: Vec<Line>, max: f32, legend: Vec<W>) -> W {
    let h = if cx.narrow { 130.0 } else { 180.0 };
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(big).max_lines(1).class("hero-value").style("color", color))
                .child(Text::new(sub).max_lines(2).class("hero-sub grow")),
        )
        .child(parts::area_chart(lines, cx.span, max, h))
        .child(
            Row::new()
                .child(Text::new(if cx.span >= 300 { t!("5 минут назад") } else { t!("минуту назад") }).class("axis grow"))
                .child(Text::new(t!("сейчас")).class("axis")),
        );
    if !legend.is_empty() {
        col = col.child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(legend));
    }
    boxed(parts::card(col))
}

// ─── Обзор ──────────────────────────────────────────────────────────────────

fn overview(cx: &Cx, s: &Snap) -> Vec<W> {
    let mut out = vec![header(cx, Page::Overview, s.device_model.clone().or(s.cpu_model.clone()), false)];
    let mut tiles: Vec<W> = Vec::new();
    for p in Page::ALL {
        let show = match p {
            Page::Overview | Page::Processes => false,
            Page::Vram => s.gpus.iter().any(|g| g.mem_used.is_some()),
            Page::Npu => s.npu.is_some(),
            Page::Battery => s.battery.is_some(),
            Page::Gpu => !s.gpus.is_empty(),
            _ => true,
        };
        if show {
            tiles.push(resource_tile(cx, p, s));
        }
    }
    let cols = ((cx.w / 200.0) as usize).clamp(2, 4);
    out.push(boxed(Grid::new(cols).gap(10.0).children(tiles)));

    // Система.
    let mut rows = Vec::new();
    if let Some(m) = &s.device_model {
        rows.push((t!("Устройство"), m.clone()));
    }
    if let Some(m) = &s.cpu_model {
        rows.push((t!("Процессор"), t!("{m}, ядер: {n}", m = m, n = s.cpu.cores.len())));
    }
    rows.push((t!("Память"), parts::kb(s.mem.total)));
    if !s.gpus.is_empty() {
        rows.push((t!("Графика"), s.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ")));
    }
    if let Some(n) = &s.npu {
        rows.push((t!("NPU"), n.name.clone()));
    }
    rows.push((t!("Ядро"), s.kernel.clone()));
    rows.push((t!("Работает"), parts::duration(s.uptime)));
    if let Some((l, tasks)) = s.load {
        rows.push((t!("Средняя нагрузка"), format!("{:.2} · {:.2} · {:.2}", l[0], l[1], l[2])));
        rows.push((t!("Задач"), tasks.to_string()));
    }
    out.push(boxed(parts::card(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(parts::card_head(crate::state::icons::INFO, t!("Система"), None, Page::Overview.color()))
            .child(parts::kv_list(rows)),
    )));

    // Самые активные процессы.
    let mut top: Vec<&synsystem::procs::Process> = s.procs.iter().filter(|p| !p.is_kernel()).collect();
    top.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent).then(b.rss_kb.cmp(&a.rss_kb)));
    let st = cx.st;
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        GestureDetector::new()
            .on_click(move || super::open(st, Page::Processes))
            .child(parts::card_head(Page::Processes.icon(), t!("Активнее всего"), Some(t!("все процессы →")), Page::Processes.color())),
    );
    for p in top.into_iter().take(6) {
        let pid = p.pid;
        let st = cx.st;
        col = col.child(
            GestureDetector::new()
                .on_click(move || {
                    super::open(st, Page::Processes);
                    st.selected.set(Some(pid));
                })
                .child(
                    DecoratedBox::new()
                        .child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Text::new(p.name.clone()).max_lines(1).class("grow"))
                                .child(Text::new(parts::pct(p.cpu_percent)).class("num cpu-num"))
                                .child(Text::new(parts::kb(p.rss_kb)).class("num mem-num")),
                        )
                        .class("row-item"),
                ),
        );
    }
    out.push(boxed(parts::card(col)));
    out
}

/// Плитка ресурса на обзоре: значок, имя, значение, мини-график; тап — его страница.
fn resource_tile(cx: &Cx, p: Page, s: &Snap) -> W {
    let (value, hist) = super::summary(p, s);
    let color = p.color();
    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(parts::badge_icon(p.icon(), color, 28.0))
                .child(Text::new(syngui::i18n::t(p.title())).max_lines(1).class("tile-label grow")),
        )
        .child(Text::new(value).max_lines(1).class("tile-value"));
    col = match hist {
        Some((values, max)) => col.child(parts::area_chart(vec![Line::new(values, color)], 60, max, 40.0)),
        None => col.child(DecoratedBox::new().style("height", StyleValue::px(40.0))),
    };
    let st = cx.st;
    boxed(GestureDetector::new().on_click(move || super::open(st, p)).child(DecoratedBox::new().child(col).class("res-tile")))
}
