//! Камера телефона в оболочке: какая включена (`ctx.camera`) — подписка на
//! службу камеры syncamd ([`synsystem::camera`]); значок «камера
//! используется» в строке состояния. Без syncamd (компьютер) — ничего.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;

use crate::ctx::ShellCtx;

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = ctx;
    std::thread::Builder::new()
        .name("shell-camera".into())
        .spawn(|| loop {
            if !std::path::Path::new(synsystem::camera::SOCKET).exists() {
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
            let r = synsystem::camera::watch(|st| {
                run_on_main_thread(move || ShellCtx::get().camera.set(st.camera));
                true
            });
            if let Err(e) = r {
                log::debug!("syncamd: {e}");
            }
            // syncamd перезапускается — камера выключена
            run_on_main_thread(|| ShellCtx::get().camera.set(None));
            std::thread::sleep(Duration::from_secs(3));
        })
        .ok();
}
