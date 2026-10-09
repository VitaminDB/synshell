//! Разговор у уха: пока идёт звонок через трубку, датчик приближения (iio-sensor-proxy, `ClaimProximity`,
//! `ProximityNear`) гасит экран — щека не нажимает кнопки. Гасит композитор действием `proximity-blank`: без
//! блокировки и без сна системы, касания отбрасываются. Громкая связь или гарнитура (голос разговора —
//! palaudiod, `/run/palaudio/control`) — экран не гасится.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synmodem::api::CallState;
use synshell_common::action::Action;

use crate::ctx::ShellCtx;

/// Идёт звонок, при котором телефон держат у уха (набор, гудки, разговор, удержание).
static IN_CALL: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);

pub fn start(ctx: ShellCtx) {
    create_effect(move || {
        let in_call = ctx.modem.get().is_some_and(|m| {
            m.calls.iter().any(|c| matches!(c.state, CallState::Dialing | CallState::Alerting | CallState::Active | CallState::Held))
        });
        IN_CALL.store(in_call, Ordering::SeqCst);
        if in_call && !WATCHING.swap(true, Ordering::SeqCst) {
            std::thread::Builder::new()
                .name("shell-incall".into())
                .spawn(|| {
                    if let Err(e) = watch() {
                        log::warn!("датчик приближения в разговоре: {e}");
                    }
                    blank(false);
                    WATCHING.store(false, Ordering::SeqCst);
                })
                .ok();
        }
    });
}

fn blank(on: bool) {
    run_on_main_thread(move || crate::actions::run(Action::ProximityBlank(on)));
}

/// Голос разговора идёт в трубку (не громкая связь, не гарнитура).
fn earpiece() -> bool {
    let r = (|| -> std::io::Result<String> {
        let mut s = UnixStream::connect("/run/palaudio/control")?;
        s.set_read_timeout(Some(Duration::from_secs(2)))?;
        s.write_all(b"status\n")?;
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line)?;
        Ok(line)
    })();
    // palaudiod недоступен — считаем трубкой (так безопаснее для щеки)
    let Ok(line) = r else { return true };
    let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
    v.pointer("/voice/output").and_then(|o| o.as_str()).is_none_or(|o| o == "handset")
}

fn watch() -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(&conn, "net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")?;
    if !proxy.get_property::<bool>("HasProximity").unwrap_or(false) {
        return Err(zbus::Error::Failure(t!("нет датчика приближения").into()));
    }
    proxy.call_method("ClaimProximity", &())?;
    let mut ear = true;
    let mut route_at = Instant::now() - Duration::from_secs(10);
    let mut blanked = false;
    while IN_CALL.load(Ordering::SeqCst) {
        if route_at.elapsed() >= Duration::from_secs(1) {
            ear = earpiece();
            route_at = Instant::now();
        }
        let near = proxy.get_property::<bool>("ProximityNear").unwrap_or(false);
        let want = near && ear;
        if want != blanked {
            blanked = want;
            blank(want);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = proxy.call_method("ReleaseProximity", &());
    Ok(())
}
