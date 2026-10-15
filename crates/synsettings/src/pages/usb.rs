use std::cell::Cell;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synsystem::usb::UsbDev;

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

type Sig = RwSignal<Vec<UsbDev>>;

thread_local! {
    static SIG: Cell<Option<Sig>> = const { Cell::new(None) };
}

fn load(sig: Sig) {
    std::thread::spawn(move || {
        let v = synsystem::usb::devices(&synsystem::Sys::host());
        run_on_main_thread(move || {
            if sig.get_untracked() != v {
                sig.set(v);
            }
        });
    });
}

fn sig() -> Sig {
    SIG.with(|w| match w.get() {
        Some(s) => s,
        None => {
            let s = use_signal(Vec::new());
            w.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == "usb" {
                        load(s);
                    }
                });
            });
            s
        }
    })
}

fn device_row(d: &UsbDev) -> W {
    let mut hint = t!("шина {bus} · устройство {dev}", bus = format!("{:03}", d.busnum), dev = format!("{:03}", d.devnum)).to_string();
    if let Some(s) = d.speed_mbps {
        hint = format!("{hint} · {}", t!("{s} Мбит/с", s = s));
    }
    if let Some(ma) = d.max_ma {
        hint = format!("{hint} · {}", t!("до {ma} мА", ma = ma));
    }
    let name = d.name().unwrap_or_else(|| t!("Неизвестное устройство").into());
    row_inline(name, hint, Text::new(d.id()).selectable(true).class("row-value"))
}

pub fn usb() -> W {
    let c = store::config();
    let sig = sig();
    load(sig);
    let list = Reactive::new(move || -> Vec<W> {
        let v = sig.get();
        let rows: Vec<W> = if v.is_empty() { vec![note(&t!("USB-устройств не найдено"))] } else { v.iter().map(device_row).collect() };
        vec![group(&t!("Подключённые устройства"), rows)]
    });
    page(
        t!("USB-устройства"),
        t!("Подключённые устройства USB и уведомления об их подключении."),
        vec![
            group(
                "",
                vec![switch_row(
                    t!("Уведомлять о подключении"),
                    t!("Уведомление при подключении и отключении USB-устройства: VID:PID и название"),
                    op!["notifications", "usb_devices"],
                    c.notifications.usb_devices,
                )],
            ),
            boxed(list),
        ],
    )
}
