//! Оборудование: процессор и кластеры, память, аккумулятор (или напряжение
//! из АЦП PMIC, если драйвера батареи нет), графика, дисплей и подсветка,
//! датчики IIO с живыми значениями, температуры, хранилище, сеть, USB,
//! устройства ввода, звук, камеры, светодиоды. Данные —
//! `synsystem::hwinfo` (sysfs/procfs, без udev).
//!
//! Страница — меню разделов (сводка, неисправность — красным); раздел
//! открывается своей подстраницей (`Ctx::sub`, «назад» — обратно в меню).

use syngui::prelude::*;

use crate::state;
use crate::ui::*;

use std::sync::atomic::{AtomicBool, Ordering};

use synsystem::hwinfo::Section;

/// Живые данные страницы: сигнал на каждый раздел (пересобирается только
/// изменившийся — датчики, температуры, питание) и порядок разделов.
#[derive(Clone, Copy)]
struct Live {
    ids: RwSignal<Vec<&'static str>>,
}

thread_local! {
    static LIVE: std::cell::Cell<Option<Live>> = const { std::cell::Cell::new(None) };
    static SECTIONS: std::cell::RefCell<Vec<(&'static str, RwSignal<Option<Section>>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Страница открыта — фоновый опрос идёт (раз в 2 с).
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn section_signal(id: &'static str) -> RwSignal<Option<Section>> {
    SECTIONS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, s)) = m.iter().find(|(k, _)| *k == id) {
            return *s;
        }
        let s = use_signal(None);
        m.push((id, s));
        s
    })
}

/// Новые данные: сигналы только изменившихся разделов.
fn apply(live: Live, sections: Vec<Section>) {
    let ids: Vec<&'static str> = sections.iter().map(|s| s.id).collect();
    for s in sections {
        let sig = section_signal(s.id);
        if sig.get_untracked().as_ref() != Some(&s) {
            sig.set(Some(s));
        }
    }
    if live.ids.get_untracked() != ids {
        live.ids.set(ids);
    }
}

/// Опрос sysfs (IIO-АЦП и термозоны на телефоне читаются медленно) — в
/// фоновом потоке, пока страница открыта; в основном потоке — только
/// применение изменений.
fn live() -> Live {
    LIVE.with(|l| match l.get() {
        Some(v) => v,
        None => {
            let v = Live { ids: use_signal(Vec::new()) };
            l.set(Some(v));
            apply(v, synsystem::hwinfo::collect(&synsystem::Sys::host()));
            std::thread::Builder::new()
                .name("hwinfo".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    syngui::async_runtime::run_on_main_thread(|| {
                        ACTIVE.store(state::ctx().page.get_untracked() == "hardware", Ordering::Relaxed);
                    });
                    if ACTIVE.load(Ordering::Relaxed) {
                        let sections = synsystem::hwinfo::collect(&synsystem::Sys::host());
                        syngui::async_runtime::run_on_main_thread(move || apply(v, sections));
                    }
                })
                .ok();
            v
        }
    })
}

/// Карточка устройства; неисправность — первой строкой с предупреждением.
fn device_card(d: &synsystem::hwinfo::Device) -> W {
    let mut rows: Vec<W> = Vec::new();
    if let Some(f) = &d.fault {
        rows.push(boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new("\u{E002}").class("hw-fault-icon"))
                .child(Text::new(f.clone()).max_lines(3).class("hw-fault-text"))
                .class("hw-fault"),
        ));
    }
    // Длинное значение (производитель USB, модель панели) не помещается рядом
    // с подписью и наезжало на неё — тогда под подписью с переносом.
    let long = if narrow() { 22 } else { 48 };
    rows.extend(d.props.iter().map(|(k, v)| {
        let t = Text::new(v.clone()).selectable(true);
        if v.chars().count() > long {
            row_wide(k, "", t.max_lines(4).class("row-value"))
        } else {
            row_inline(k, "", t.class("row-value"))
        }
    }));
    group(&d.name, rows)
}

/// Ключ подстраницы раздела.
const SUB: &str = "hardware/";

/// Сводка раздела для меню: неисправность или первое устройство.
fn summary(s: &Section) -> (String, bool) {
    if let Some(f) = s.devices.iter().find_map(|d| d.fault.as_ref().map(|f| (d, f))) {
        return (format!("{}: {}", f.0.name, f.1), true);
    }
    let first = s.devices.first().map(|d| d.name.clone()).unwrap_or_default();
    let n = s.devices.len();
    (if n > 1 { format!("{first} и ещё {}", n - 1) } else { first }, false)
}

fn menu_item(ctx: state::Ctx, s: &Section) -> W {
    let (text, fault) = summary(s);
    let (id, title) = (s.id, s.title);
    boxed(
        syngui::GestureDetector::new()
            .on_click(move || ctx.sub.set(Some((format!("{SUB}{id}"), title.to_string()))))
            .child(
                DecoratedBox::new().class("phone-nav-item").child(
                    Row::new()
                        .gap(14.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            DecoratedBox::new()
                                .class(if fault { "hw-badge hw-badge-fault" } else { "hw-badge" })
                                .child(Icon::new(if fault { "\u{E002}" } else { s.icon }).class(if fault { "hw-icon hw-icon-fault" } else { "hw-icon" })),
                        )
                        .child(
                            Column::new()
                                .gap(2.0)
                                .class("grow")
                                .child(Text::new(title).class("phone-nav-label"))
                                .child(Text::new(text).max_lines(1).class(if fault { "hw-sum hw-sum-fault" } else { "hw-sum" })),
                        )
                        .child(Icon::new(icons::CHEVRON_RIGHT).class("phone-nav-chevron")),
                ),
            ),
    )
}

/// Подстраница раздела `id`: карточки устройств, живое обновление только его.
fn section_page(ctx: state::Ctx, id: &'static str) -> W {
    let sig = section_signal(id);
    let body = Reactive::new(move || -> Vec<W> {
        let Some(s) = sig.get() else { return vec![boxed(Text::new("Нет данных").class("row-hint"))] };
        let mut col = Column::new().gap(10.0);
        for d in &s.devices {
            col = col.child(device_card(d));
        }
        vec![boxed(col)]
    });
    let title = sig.get_untracked().map(|s| s.title).unwrap_or("Оборудование");
    let mut parts: Vec<W> = Vec::new();
    if !narrow() {
        // На телефоне «назад» — в шапке; в окне — кнопка над разделом.
        parts.push(boxed(Row::new().child(button("‹ Оборудование", move || ctx.sub.set(None)))));
    }
    parts.push(boxed(body));
    page(title, "", parts)
}

pub fn hardware() -> W {
    let live = live();
    ACTIVE.store(true, Ordering::Relaxed);
    let ctx = state::ctx();
    let view = Reactive::new(move || -> Vec<W> {
        let sub = ctx.sub.get().and_then(|(k, _)| k.strip_prefix(SUB).map(str::to_string));
        let id = sub.and_then(|k| live.ids.get_untracked().into_iter().find(|i| *i == k));
        let key = match id { Some(i) => 2 + live.ids.get_untracked().iter().position(|x| *x == i).unwrap_or(0) as u64, None => 1 };
        vec![boxed(
            AnimatedSwitcher::new(key, move || match id {
                Some(i) => section_page(ctx, i),
                None => menu(ctx, live),
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(240)
            .exit_duration_ms(160)
            .animate_size(false)
            .class("grow"),
        )]
    });
    boxed(view)
}

fn menu(ctx: state::Ctx, live: Live) -> W {
    // Меню: строки по сигналам разделов — сводка (неисправность) живая.
    let body = Reactive::new(move || -> Vec<W> {
        let mut card = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
        for (n, id) in live.ids.get().into_iter().enumerate() {
            if n > 0 {
                card = card.child(DecoratedBox::new().class("row-sep"));
            }
            let sig = section_signal(id);
            card = card.child(Reactive::new(move || sig.get().as_ref().map(|s| menu_item(ctx, s)).into_iter().collect()));
        }
        vec![boxed(card)]
    });
    let actions = Row::new()
        .gap(8.0)
        .child(button("Скопировать отчёт", || {
            let text = synsystem::hwinfo::report(&synsystem::hwinfo::collect(&synsystem::Sys::host()));
            syngui::clipboard::copy(&text);
            state::toast("Отчёт об оборудовании скопирован");
        }))
        .child(button("Обновить", move || apply(live, synsystem::hwinfo::collect(&synsystem::Sys::host()))));
    page("Оборудование", "Всё, что видит система: процессор, память, питание, датчики, устройства.", vec![boxed(actions), boxed(body)])
}
