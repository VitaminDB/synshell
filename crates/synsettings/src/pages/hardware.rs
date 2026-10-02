//! Оборудование: процессор и кластеры, память, аккумулятор (или напряжение
//! из АЦП PMIC, если драйвера батареи нет), графика, дисплей и подсветка,
//! датчики IIO с живыми значениями, температуры, хранилище, сеть, USB,
//! устройства ввода, звук, камеры, светодиоды. Данные —
//! `synsystem::hwinfo` (sysfs/procfs, без udev).

use syngui::prelude::*;

use crate::state;
use crate::ui::*;

thread_local! {
    static AUTO: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
}

/// Счётчик обновления: датчики и температуры перечитываются раз в 2 с,
/// пока страница открыта.
fn ticker() -> RwSignal<u64> {
    AUTO.with(|a| match a.get() {
        Some(s) => s,
        None => {
            let s = use_signal(0u64);
            a.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                syngui::async_runtime::run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == "hardware" {
                        s.set(s.get_untracked() + 1);
                    }
                });
            });
            s
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
    rows.extend(d.props.iter().map(|(k, v)| row_inline(k, "", Text::new(v.clone()).selectable(true).class("row-value"))));
    group(&d.name, rows)
}

pub fn hardware() -> W {
    let tick = ticker();
    let live = Reactive::new(move || -> Vec<W> {
        let _ = tick.get();
        let sections = synsystem::hwinfo::collect(&synsystem::Sys::host());
        let mut col = Column::new().gap(22.0);
        for s in &sections {
            let mut sec = Column::new().gap(10.0).child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("hw-badge").child(Icon::new(s.icon).class("hw-icon")))
                    .child(Text::new(s.title).class("hw-title")),
            );
            for d in &s.devices {
                sec = sec.child(device_card(d));
            }
            col = col.child(sec);
        }
        vec![boxed(col)]
    });
    let actions = Row::new()
        .gap(8.0)
        .child(button("Скопировать отчёт", || {
            let text = synsystem::hwinfo::report(&synsystem::hwinfo::collect(&synsystem::Sys::host()));
            syngui::clipboard::copy(&text);
            state::toast("Отчёт об оборудовании скопирован");
        }))
        .child(button("Обновить", || state::bump()));
    page("Оборудование", "Всё, что видит система: процессор, память, питание, датчики, устройства.", vec![boxed(actions), boxed(live)])
}
