//! Подключение флешек, телефонов и камер (`synshell_common::drives`):
//! уведомление с кнопками «Открыть» (смонтировать и открыть в проводнике) и
//! «Смонтировать»; после монтирования — «Открыть» и «Извлечь». Вынули
//! устройство — его уведомление убирается. О том, что было подключено до
//! запуска оболочки, не сообщаем.

use std::collections::HashSet;
use std::path::PathBuf;

use synshell_common::drives::{self, Device, Kind};

use crate::ctx::ShellCtx;
use crate::notifications;
use syngui::t;

pub fn start(_ctx: ShellCtx) {
    std::thread::Builder::new()
        .name("shell-devices".into())
        .spawn(|| {
            let known: HashSet<String> = removable().iter().map(|d| d.key().to_string()).collect();
            let known = std::sync::Mutex::new(known);
            drives::watch(move || {
                let now = removable();
                let mut known = known.lock().unwrap_or_else(|e| e.into_inner());
                let added: Vec<Device> = now.iter().filter(|d| !known.contains(d.key())).cloned().collect();
                let keys: HashSet<String> = now.iter().map(|d| d.key().to_string()).collect();
                let removed: Vec<String> = known.difference(&keys).cloned().collect();
                *known = keys;
                if added.is_empty() && removed.is_empty() {
                    return;
                }
                syngui::async_runtime::run_on_main_thread(move || {
                    let ctx = ShellCtx::get();
                    for k in removed {
                        notifications::close_local(ctx, &note_key(&k));
                    }
                    for d in added {
                        log::info!("устройства: подключено «{}» ({})", d.title(), d.key());
                        connected(ctx, d);
                    }
                });
            });
        })
        .ok();
}

fn removable() -> Vec<Device> {
    drives::devices().into_iter().filter(Device::removable).collect()
}

fn note_key(device_key: &str) -> String {
    format!("device:{device_key}")
}

fn icon(d: &Device) -> &'static str {
    match d.kind() {
        Kind::Phone => "phone",
        Kind::Camera => "camera-photo",
        Kind::Usb | Kind::Disk => "drive-removable-media-usb",
    }
}

/// «Подключён телефон».
fn connected_text(d: &Device) -> String {
    match d.kind() {
        Kind::Phone => t!("Подключён телефон"),
        Kind::Camera => t!("Подключена камера"),
        Kind::Usb | Kind::Disk => t!("Подключён съёмный диск"),
    }
}

/// Уведомление о подключении; уже смонтированное (кем-то другим) —
/// сразу с «Извлечь».
fn connected(ctx: ShellCtx, d: Device) {
    let summary = format!("{} «{}»", connected_text(&d), d.title());
    let size = match &d {
        Device::Disk(v) if v.size > 0 => format!(" · {}", drives::format_size(v.size)),
        _ => String::new(),
    };
    if let Some(p) = d.mount_point().map(PathBuf::from) {
        mounted(ctx, d, p, &summary);
        return;
    }
    let body = t!("Открыть в проводнике или только смонтировать{size}", size = size);
    let key = note_key(d.key());
    notifications::local_actions(ctx, &key, &summary, &body, icon(&d), &[("default", &t!("Открыть")), ("open", &t!("Открыть")), ("mount", &t!("Смонтировать"))], move |action| {
        let d = d.clone();
        match action {
            "mount" => mount(d, false),
            _ => mount(d, true),
        }
    });
}

/// Смонтированное: «Открыть» и «Извлечь».
fn mounted(ctx: ShellCtx, d: Device, path: PathBuf, summary: &str) {
    let body = match &d {
        Device::Disk(_) => t!("Смонтировано в {path}", path = path.display()),
        Device::Gadget(_) => t!("Файлы доступны в проводнике").to_string(),
    };
    let key = note_key(d.key());
    notifications::local_actions(ctx, &key, summary, &body, icon(&d), &[("default", &t!("Открыть")), ("open", &t!("Открыть")), ("eject", &t!("Извлечь"))], move |action| match action {
        "eject" => eject(d.clone()),
        _ => crate::launchers::open_path(&path),
    });
}

/// Смонтировать в фоне (polkit может спросить пароль, телефон — разрешение
/// на доступ к файлам) и открыть в проводнике или показать, что смонтировано.
fn mount(d: Device, open: bool) {
    // Телефон по MTP открывается несколько секунд — пусть будет видно, что идёт работа.
    let ctx = ShellCtx::get();
    notifications::local_busy(ctx, &note_key(d.key()), &t!("Подключение к «{title}»…", title = d.title()), "", icon(&d));
    std::thread::spawn(move || {
        let r = d.mount();
        syngui::async_runtime::run_on_main_thread(move || {
            let ctx = ShellCtx::get();
            match r {
                Ok(path) if open => {
                    notifications::close_local(ctx, &note_key(d.key()));
                    crate::launchers::open_path(&path);
                }
                Ok(path) => {
                    let summary = t!("«{title}» смонтировано", title = d.title());
                    mounted(ctx, d, path, &summary);
                }
                Err(e) => {
                    let key = note_key(d.key());
                    let summary = t!("Не удалось открыть «{title}»", title = d.title());
                    notifications::local_actions(ctx, &key, &summary, &e, icon(&d), &[("retry", &t!("Повторить"))], move |a| {
                        if a == "retry" {
                            mount(d.clone(), open);
                        }
                    });
                }
            }
        });
    });
}

fn eject(d: Device) {
    std::thread::spawn(move || {
        let r = d.eject();
        syngui::async_runtime::run_on_main_thread(move || {
            let ctx = ShellCtx::get();
            match r {
                Ok(()) => notifications::local_actions(ctx, &note_key(d.key()), &t!("«{title}» можно отключать", title = d.title()), "", icon(&d), &[], |_| {}),
                Err(e) => notifications::local(ctx, &t!("Не удалось извлечь «{title}»", title = d.title()), &e, None),
            }
        });
    });
}
