use std::collections::HashMap;

use synsystem::usb::{self, UsbDev};
use syngui::t;

use crate::ctx::ShellCtx;
use crate::notifications;

pub fn start(_ctx: ShellCtx) {
    std::thread::Builder::new()
        .name("shell-usb".into())
        .spawn(|| {
            let sys = synsystem::Sys::host();
            let mut known: HashMap<String, UsbDev> = usb::devices(&sys).into_iter().map(|d| (d.key(), d)).collect();
            let r = usb::watch(move || {
                let now: HashMap<String, UsbDev> = usb::devices(&sys).into_iter().map(|d| (d.key(), d)).collect();
                let added: Vec<UsbDev> = now.iter().filter(|(k, _)| !known.contains_key(*k)).map(|(_, d)| d.clone()).collect();
                let removed: Vec<UsbDev> = known.iter().filter(|(k, _)| !now.contains_key(*k)).map(|(_, d)| d.clone()).collect();
                known = now;
                if added.is_empty() && removed.is_empty() {
                    return;
                }
                syngui::async_runtime::run_on_main_thread(move || {
                    let ctx = ShellCtx::get();
                    for d in &removed {
                        log::info!("usb: отключено {} ({})", d.lsusb_line(), d.sysname);
                    }
                    for d in &added {
                        log::info!("usb: подключено {} ({})", d.lsusb_line(), d.sysname);
                    }
                    if !added.is_empty() {
                        crate::sounds::play(crate::sounds::Ev::DeviceAdded);
                    } else if !removed.is_empty() {
                        crate::sounds::play(crate::sounds::Ev::DeviceRemoved);
                    }
                    if !ctx.cfg().notifications.usb_devices {
                        return;
                    }
                    for d in removed {
                        notify(ctx, &d, false);
                    }
                    for d in added {
                        notify(ctx, &d, true);
                    }
                });
            });
            if let Err(e) = r {
                log::warn!("usb: нет uevent ({e})");
            }
        })
        .ok();
}

fn notify(ctx: ShellCtx, d: &UsbDev, connected: bool) {
    let summary = match (connected, d.name()) {
        (true, Some(n)) => t!("Подключено: {name}", name = n),
        (false, Some(n)) => t!("Отключено: {name}", name = n),
        (true, None) => t!("Подключено USB-устройство"),
        (false, None) => t!("Отключено USB-устройство"),
    };
    let body = t!("ID {id} · шина {bus} · устройство {dev}", id = d.id(), bus = format!("{:03}", d.busnum), dev = format!("{:03}", d.devnum));
    notifications::local_actions(ctx, &format!("usb:{}", d.sysname), &summary, &body, "drive-removable-media-usb", &[("default", &t!("Параметры"))], |_| {
        crate::actions::spawn("synsettings usb");
    });
}
