//! Окно: на широком экране — боковая панель ресурсов с живыми мини-графиками и страница
//! справа; на узком (телефон) — шапка, лента чипов со значениями и страница под ней.

mod pages;
mod parts;

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;

use crate::sampler::{self, Snap};
use crate::state::{icons, Page, St};
use parts::{boxed, W};

/// Уже этой ширины — телефонная раскладка.
const NARROW_BELOW: f32 = 760.0;
/// Ширина боковой панели.
const SIDEBAR_W: f32 = 280.0;

/// Открыть страницу (чип, пункт панели, повторный запуск с аргументом).
pub fn open(st: St, p: Page) {
    if st.page.get_untracked() != p {
        st.selected.set(None);
        st.page.set(p);
    }
    sampler::set_page(p);
}

pub fn root(st: St) -> impl Widget {
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        vec![if narrow.get() { phone(st, page) } else { desktop(st, page) }]
    });
    DecoratedBox::new().child(body).class("root")
}

/// Ширина области страницы.
pub fn content_width() -> f32 {
    let vp = viewport_size().get();
    if vp.width < NARROW_BELOW {
        (vp.width - 32.0).max(260.0)
    } else {
        (vp.width - SIDEBAR_W - 56.0).clamp(300.0, 1100.0)
    }
}

/// Коротко о ресурсе: значение, история для мини-графика и её шкала.
fn summary(p: Page, s: &Snap) -> (String, Option<(Vec<f32>, f32)>) {
    let n = 60;
    match p {
        Page::Overview => (t!("{n} процессов", n = s.procs.len()), None),
        Page::Cpu => (parts::pct(s.cpu.usage), Some((s.hist.cpu.tail(n), 100.0))),
        Page::Gpu => match s.gpus.first() {
            Some(g) if g.asleep => (t!("спит"), None),
            Some(g) => (g.busy_percent.map(parts::pct).or(g.cur_mhz.map(parts::mhz)).unwrap_or_else(|| "—".into()), s.hist.gpu.first().map(|h| (h.tail(n), 100.0))),
            None => (t!("нет"), None),
        },
        Page::Vram => match s.gpus.iter().position(|g| g.mem_used.is_some()) {
            Some(i) => {
                let g = &s.gpus[i];
                let max = g.mem_total.unwrap_or(s.mem.total * 1024).max(1) as f32;
                (parts::bytes(g.mem_used.unwrap_or(0)), s.hist.vram.get(i).map(|h| (h.tail(n), max)))
            }
            None => (if s.gpus.iter().any(|g| g.asleep) { t!("спит") } else { "—".into() }, None),
        },
        Page::Npu => match &s.npu {
            Some(npu) => match npu.busy_percent {
                Some(b) => (parts::pct(b), Some((s.hist.npu.tail(n), 100.0))),
                None => (if npu.online { t!("включён") } else { t!("выключен") }, None),
            },
            None => (t!("нет"), None),
        },
        Page::Memory => (parts::pct(s.hist.mem.last()), Some((s.hist.mem.tail(n), 100.0))),
        Page::Network => {
            let max = parts::nice_max(s.hist.rx.max(n).max(s.hist.tx.max(n)), 1024.0 * 64.0);
            (format!("↓ {}", parts::rate(s.rx())), Some((s.hist.rx.tail(n), max)))
        }
        Page::Disk => {
            let max = parts::nice_max(s.hist.disk_read.max(n).max(s.hist.disk_write.max(n)), 1024.0 * 1024.0);
            (parts::rate(s.disk_read() + s.disk_write()), Some((s.hist.disk_read.tail(n), max)))
        }
        Page::Sensors => match s.cpu_temp.or(s.temps.first().map(|t| t.celsius)) {
            Some(t) => (parts::celsius(t), Some((s.hist.cpu_temp.tail(n), 100.0))),
            None => ("—".into(), None),
        },
        Page::Battery => match &s.battery {
            Some(b) => (format!("{}%{}", b.percent, if b.charging { " ⚡" } else { "" }), Some((s.hist.battery.tail(n), 100.0))),
            None => (s.adc_volt.map(|v| format!("{v:.2} В")).unwrap_or_else(|| t!("нет")), None),
        },
        Page::Processes => (s.procs.iter().filter(|p| !p.is_kernel()).count().to_string(), None),
    }
}

// ─── Компьютер ──────────────────────────────────────────────────────────────

fn desktop(st: St, page: Page) -> W {
    let nav = Reactive::new(move || -> Vec<W> {
        let cur = st.page.get();
        let s = st.snap.get();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for p in Page::ALL {
            col = col.child(nav_item(st, p, p == cur, &s));
        }
        vec![boxed(col)]
    });
    let sidebar = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(brand())
        .child(ScrollView::new().vertical().child(nav).class("grow"))
        .class("sidebar")
        .style("width", StyleValue::px(SIDEBAR_W));
    boxed(Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(sidebar).child(pages::page(st, page).class("main grow")))
}

fn brand() -> W {
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(Icon::new(icons::SPEED).class("brand-icon")).class("brand-logo"))
            .child(Column::new().gap(0.0).child(Text::new(t!("Ресурсы")).class("brand-title")).child(Text::new(t!("synshell")).class("muted small")))
            .class("brand"),
    )
}

fn nav_item(st: St, p: Page, active: bool, s: &Snap) -> W {
    let (value, hist) = summary(p, s);
    let color = p.color();
    let mut row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(parts::badge_icon(p.icon(), color, 34.0))
        .child(Column::new().gap(1.0).child(Text::new(syngui::i18n::t(p.title())).max_lines(1).class("nav-title")).child(Text::new(value).max_lines(1).class("nav-value")).class("grow"));
    if let Some((values, max)) = hist {
        row = row.child(parts::spark(values, 60, max, color, 64.0, 28.0));
    }
    let mut item = DecoratedBox::new().child(row).class(if active { "nav-item active" } else { "nav-item" });
    if active {
        item = item.style("border-color", color.with_alpha(0.55));
    }
    boxed(GestureDetector::new().on_click(move || open(st, p)).child(item))
}

// ─── Телефон ────────────────────────────────────────────────────────────────

fn phone(st: St, page: Page) -> W {
    let chips = Reactive::new(move || -> Vec<W> {
        let cur = st.page.get();
        let s = st.snap.get();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for p in Page::ALL {
            row = row.child(chip(st, p, p == cur, &s));
        }
        vec![boxed(row.class("chips-row"))]
    });
    let head = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(icons::SPEED).class("brand-icon")).class("brand-logo"))
        .child(Text::new(t!("Ресурсы")).class("phone-title grow"))
        .child(Reactive::new(move || -> Vec<W> {
            let s = st.snap.get();
            vec![boxed(Text::new(t!("работает {d}", d = parts::duration(s.uptime))).max_lines(1).class("muted small"))]
        }))
        .class("phone-head");
    boxed(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(head)
            .child(ScrollView::new().horizontal().child(chips).class("chips"))
            .child(pages::page(st, page).class("grow")),
    )
}

fn chip(st: St, p: Page, active: bool, s: &Snap) -> W {
    let color = p.color();
    let (value, _) = summary(p, s);
    let mut row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(p.icon()).class("chip-icon").style("icon-color", color))
        .child(Text::new(syngui::i18n::t(p.short())).max_lines(1).class("chip-label"));
    if p != Page::Overview {
        row = row.child(Text::new(value).max_lines(1).class("chip-value"));
    }
    let mut b = DecoratedBox::new().child(row).class(if active { "chip active" } else { "chip" });
    if active {
        b = b.style("background-color", color.with_alpha(0.2)).style("border-color", color.with_alpha(0.7));
    }
    boxed(GestureDetector::new().on_click(move || open(st, p)).child(b))
}
