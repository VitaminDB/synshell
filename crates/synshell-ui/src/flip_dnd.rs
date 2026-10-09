//! Экраном вниз — «Не беспокоить» (`[mobile] flip_to_dnd`): алгоритм SLPI `screen_down`
//! (synmobile docs/15) через `/usr/lib/syn-sensors/ssc-events` — событие `screen_down 2 2`
//! (лежит экраном вниз, положение устоялось) включает «Не беспокоить», поднятие (первое
//! значение 1) возвращает как было. Включённое руками раньше — не трогается.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use syngui::prelude::*;

use crate::ctx::ShellCtx;

const PROGRAM: &str = "/usr/lib/syn-sensors/ssc-events";

static RUNNING: Mutex<Option<Child>> = Mutex::new(None);

thread_local! {
    /// «Не беспокоить» включили мы (переворотом) — нам и выключать.
    static OURS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn start(ctx: ShellCtx) {
    if !std::path::Path::new(PROGRAM).exists() {
        return;
    }
    create_effect(move || {
        let on = ctx.config.get().mobile.flip_to_dnd;
        set(ctx, on);
    });
}

/// Положение: (экраном вниз, устоялось).
fn parse(line: &str) -> Option<(bool, bool)> {
    let mut it = line.split_whitespace();
    if it.next()? != "screen_down" {
        return None;
    }
    let pos: f32 = it.next()?.parse().ok()?;
    let stable: f32 = it.next()?.parse().ok()?;
    Some((pos.round() as i32 == 2, stable.round() as i32 == 2))
}

fn on_change(ctx: ShellCtx, down: bool, stable: bool) {
    if down && stable {
        if !ctx.dnd.get_untracked() {
            log::info!("экраном вниз — «Не беспокоить»");
            OURS.with(|o| o.set(true));
            ctx.dnd.set(true);
        }
    } else if !down && OURS.with(|o| o.replace(false)) {
        log::info!("телефон подняли — «Не беспокоить» выключено");
        ctx.dnd.set(false);
    }
}

fn set(ctx: ShellCtx, on: bool) {
    let mut running = RUNNING.lock().unwrap();
    if running.is_some() == on {
        return;
    }
    if let Some(mut child) = running.take() {
        // закрытый stdin — программа закрывает датчик в SSC и выходит
        drop(child.stdin.take());
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        if OURS.with(|o| o.replace(false)) {
            ctx.dnd.set(false);
        }
        return;
    }
    let child = Command::new(PROGRAM).arg("screen_down").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log::warn!("экраном вниз: {PROGRAM}: {e}");
            return;
        }
    };
    if let Some(out) = child.stdout.take() {
        std::thread::Builder::new()
            .name("flip-dnd".into())
            .spawn(move || {
                for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
                    if let Some((down, stable)) = parse(&line) {
                        syngui::async_runtime::run_on_main_thread(move || on_change(ShellCtx::get(), down, stable));
                    }
                }
            })
            .ok();
    }
    log::info!("экраном вниз — «Не беспокоить»: включено");
    *running = Some(child);
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn screen_down_values() {
        assert_eq!(parse("screen_down 2.000000 2.000000 0.000000"), Some((true, true)));
        assert_eq!(parse("screen_down 2 1 0"), Some((true, false)));
        assert_eq!(parse("screen_down 1 1 0"), Some((false, false)));
        assert_eq!(parse("pickup 1"), None);
    }
}
