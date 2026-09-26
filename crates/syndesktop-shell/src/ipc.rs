//! Поток событий композитора → сигналы оболочки. Пока композитора
//! syndesktop нет (оболочка под другим композитором) — переподключение
//! раз в пару секунд, апплеты столов/задач пустые.

use std::time::Duration;
use syndesktop_common::ipc::{Client, Event};

use crate::ctx::ShellCtx;

pub fn start(ctx: ShellCtx) {
    std::thread::Builder::new()
        .name("shell-ipc".into())
        .spawn(move || loop {
            match Client::connect().and_then(|c| c.event_stream()) {
                Ok(stream) => {
                    log::info!("IPC: подключено к композитору");
                    ctx.connected.set(true);
                    for ev in stream {
                        match ev {
                            Ok(ev) => handle(ctx, ev),
                            Err(e) => {
                                log::warn!("IPC: {e}");
                                break;
                            }
                        }
                    }
                    ctx.connected.set(false);
                    log::info!("IPC: соединение закрыто");
                }
                Err(e) => log::debug!("IPC: нет композитора ({e})"),
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("поток IPC");
}

fn handle(ctx: ShellCtx, ev: Event) {
    match ev {
        Event::Snapshot { windows, workspaces, outputs, keyboard } => {
            let focused = windows.iter().find(|w| w.focused).map(|w| w.id);
            ctx.windows.set(windows);
            ctx.workspaces.set(workspaces);
            ctx.comp_outputs.set(outputs);
            ctx.keyboard.set(keyboard);
            ctx.focused.set(focused);
        }
        Event::WindowChanged { window } => {
            let w = window;
            syngui::async_runtime::run_on_main_thread(move || {
                ctx.windows.update(|list| match list.iter_mut().find(|x| x.id == w.id) {
                    Some(x) => *x = w.clone(),
                    None => list.push(w.clone()),
                });
            });
        }
        Event::WindowClosed { id } => {
            syngui::async_runtime::run_on_main_thread(move || {
                ctx.windows.update(|list| list.retain(|x| x.id != id));
            });
        }
        Event::WindowFocused { id } => {
            syngui::async_runtime::run_on_main_thread(move || {
                ctx.focused.set(id);
                ctx.windows.update(|list| {
                    for w in list.iter_mut() {
                        w.focused = Some(w.id) == id;
                    }
                });
            });
        }
        Event::WorkspacesChanged { workspaces } => ctx.workspaces.set(workspaces),
        Event::OutputsChanged { outputs } => ctx.comp_outputs.set(outputs),
        Event::KeyboardLayoutChanged { keyboard } => ctx.keyboard.set(keyboard),
        Event::ShellCommand { command } => {
            syngui::async_runtime::run_on_main_thread(move || crate::commands::handle(&command));
        }
        Event::ConfigReloaded { error } => {
            if let Some(e) = error {
                log::warn!("композитор: ошибка конфига: {e}");
            }
        }
        Event::Exiting => {
            log::info!("композитор завершает сеанс");
            syngui::async_runtime::run_on_main_thread(syngui_layer::quit);
        }
    }
}
