//! Передатчик: посылки уходят платформенной программе `syn-ir raw ЧАСТОТА -` (synmobile, hw/ir) — она
//! знает устройство и частоту отсчётов из `/etc/syn-ir.conf`. Посылки идут по очереди из одного потока;
//! пока передатчик занят, новые сверх пары в очереди отбрасываются (удержание кнопки не копит хвост).
//! `SYN_REMOTE_IR=команда` — другая программа с тем же интерфейсом (проверка без телефона: `cat`).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Mutex, OnceLock};

use syngui::t;

use crate::ir::Frame;

type Hook = Box<dyn Fn(Result<(), String>) + Send + Sync>;

static QUEUE: OnceLock<SyncSender<Frame>> = OnceLock::new();
static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

/// Программа передачи, если есть.
pub fn program() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SYN_REMOTE_IR") {
        return Some(PathBuf::from(p));
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push("/usr/local/bin".into());
    dirs.into_iter().map(|d| d.join("syn-ir")).find(|p| p.is_file())
}

/// Что сообщать о каждой передаче (ошибки — для всплывающего сообщения).
pub fn on_result(f: impl Fn(Result<(), String>) + Send + Sync + 'static) {
    *HOOK.lock().unwrap() = Some(Box::new(f));
}

/// Поставить посылку в очередь. `false` — передатчика нет или очередь полна.
pub fn send(frame: Frame) -> bool {
    let q = QUEUE.get_or_init(|| {
        let (tx, rx) = sync_channel::<Frame>(2);
        std::thread::Builder::new()
            .name("ir-tx".into())
            .spawn(move || {
                for f in rx {
                    let r = transmit(&f);
                    if let Err(e) = &r {
                        tracing::warn!("ИК: {e}");
                    }
                    if let Some(h) = HOOK.lock().unwrap().as_ref() {
                        h(r);
                    }
                }
            })
            .expect("поток ИК");
        tx
    });
    match q.try_send(frame) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// Одна передача, синхронно.
pub fn transmit(f: &Frame) -> Result<(), String> {
    let Some(prog) = program() else {
        tracing::debug!("ИК (нет передатчика): {} Гц, {} отрезков", f.freq, f.timings.len());
        return Err(t!("ИК-передатчик не найден"));
    };
    let body: String = f.timings.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" ");
    let mut child = Command::new(&prog)
        .args(["raw", &f.freq.to_string(), "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", prog.display()))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(body.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { out.status.to_string() } else { err })
    }
}
