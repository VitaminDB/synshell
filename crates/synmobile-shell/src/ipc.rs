//! События композитора → состояние оболочки: появилось окно — домашний экран
//! прячется, закрылось последнее — возвращается; `shell home` из привязок и
//! `synwm msg action "shell home"` переключает домашний экран.

use std::time::Duration;
use synshell_common::ipc::{Client, Event, WindowInfo};

use crate::Ctx;

pub fn start(ctx: Ctx) {
    std::thread::Builder::new()
        .name("mobile-ipc".into())
        .spawn(move || loop {
            match Client::connect().and_then(|c| c.event_stream()) {
                Ok(stream) => {
                    log::info!("IPC: подключено к композитору");
                    let mut windows: Vec<WindowInfo> = Vec::new();
                    for ev in stream {
                        match ev {
                            Ok(ev) => handle(ctx, &mut windows, ev),
                            Err(e) => {
                                log::warn!("IPC: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => log::debug!("IPC: нет композитора ({e})"),
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("поток IPC");
}

fn set_home(ctx: Ctx, visible: bool) {
    syngui::async_runtime::run_on_main_thread(move || {
        if ctx.home_visible.get_untracked() != visible {
            ctx.home_visible.set(visible);
        }
    });
}

fn handle(ctx: Ctx, windows: &mut Vec<WindowInfo>, ev: Event) {
    match ev {
        Event::Snapshot { windows: list, .. } => {
            *windows = list;
            if !windows.is_empty() {
                set_home(ctx, false);
            }
        }
        Event::WindowChanged { window } => {
            let new = !windows.iter().any(|w| w.id == window.id);
            match windows.iter_mut().find(|w| w.id == window.id) {
                Some(w) => *w = window,
                None => windows.push(window),
            }
            if new {
                set_home(ctx, false);
            }
        }
        Event::WindowClosed { id } => {
            windows.retain(|w| w.id != id);
            if windows.is_empty() {
                set_home(ctx, true);
            }
        }
        Event::ShellCommand { command } => match command.trim() {
            "home" => syngui::async_runtime::run_on_main_thread(move || ctx.home_visible.set(!ctx.home_visible.get_untracked())),
            "apps" => set_home(ctx, true),
            other => log::debug!("IPC: команда оболочки не поддерживается: {other}"),
        },
        Event::Exiting => syngui::async_runtime::run_on_main_thread(syngui_layer::quit),
        _ => {}
    }
}
