//! Запуск Android из оболочки: открыли приложение Android или «Весь Android», а подсистема Android ещё не
//! запущена — карточка OSD с вращающимся кольцом «Запускается подсистема Android», пока она грузится
//! (контейнер, образы, загрузка Android — до полуминуты и больше).
//!
//! Состояние — у демона syndroidd (unix-сокет, JSON `{"op":"status"}` → `{"state": …}`), опрос в фоне.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;

use crate::ctx::ShellCtx;

const SOCKET: &str = "/run/syndroid/syndroidd.sock";
/// Дольше — что-то не так (журнал — в окне «Управление Android»), карточку убрать.
const LIMIT: Duration = Duration::from_secs(150);
/// Android так и не начал запускаться (команда не дошла до демона) — не ждать.
const STOPPED_LIMIT: Duration = Duration::from_secs(10);

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Команда запускает Android (ярлыки Android: `syndroid app launch …`, `syndroid show …`).
pub fn is_android_command(cmd: &str) -> bool {
    let mut w = cmd.split_whitespace();
    w.next().is_some_and(|b| b.rsplit('/').next() == Some("syndroid")) && matches!(w.next(), Some("app" | "show" | "start"))
}

/// Состояние Android: `stopped`, `starting`, `running`, `frozen`, `stopping`; `None` — демона нет.
fn state() -> Option<String> {
    let mut s = UnixStream::connect(SOCKET).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(b"{\"op\":\"status\"}\n").ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    let v: serde_json::Value = serde_json::from_str(&line).ok()?;
    v.get("state")?.as_str().map(str::to_string)
}

fn label(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    if s < 3 {
        "Запускается подсистема Android…".into()
    } else {
        format!("Запускается подсистема Android · {s} с")
    }
}

/// Следить за запуском (один наблюдатель на оболочку; из главного потока).
pub fn watch() {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("shell-android-boot".into())
        .spawn(|| {
            let start = Instant::now();
            let mut shown = false;
            loop {
                let elapsed = start.elapsed();
                let busy = match state().as_deref() {
                    Some("starting") => true,
                    Some("stopped") => elapsed < STOPPED_LIMIT,
                    // Запущен, заморожен (разморозка мгновенная), останавливается или демона нет
                    _ => false,
                };
                if !busy || elapsed > LIMIT {
                    break;
                }
                let text = label(elapsed);
                run_on_main_thread(move || crate::osd::busy(ShellCtx::get(), crate::ui::mi::ANDROID, text));
                shown = true;
                std::thread::sleep(Duration::from_millis(800));
            }
            if shown {
                run_on_main_thread(|| crate::osd::done(ShellCtx::get()));
            }
            WATCHING.store(false, Ordering::SeqCst);
        })
        .ok();
}
