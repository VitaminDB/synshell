//! Встроенный терминал под файлами (F12): открывается в папке
//! активной панели и идёт за ней (`cd`, пока в терминале ничего не
//! запущено); `cd` в самом терминале переводит панель. Выход из оболочки
//! закрывает терминал.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use syngui::prelude::*;
use syngui::widgets::*;

use super::{boxed, W};
use crate::loc::Location;
use crate::state;

thread_local! {
    static SESSION: RefCell<Option<TerminalSession>> = const { RefCell::new(None) };
    /// Папка, в которой терминал сейчас (по нашему `cd` или по его собственному).
    static SYNCED: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn session() -> Option<TerminalSession> {
    SESSION.with(|s| s.borrow().clone())
}

fn active_dir() -> Option<PathBuf> {
    state::pane().loc.get_untracked().dir().map(Path::to_path_buf)
}

/// Строка оболочки: `cd` в папку (с пробелом впереди — не в историю).
pub fn cd_line(dir: &Path) -> String {
    let quoted = dir.to_string_lossy().replace('\'', r"'\''");
    format!("\x15 cd -- '{quoted}'\r")
}

fn start() -> Option<TerminalSession> {
    if let Some(s) = session().filter(|s| s.is_alive()) {
        return Some(s);
    }
    let cwd = active_dir().filter(|d| d.is_dir()).unwrap_or_else(synshell_common::paths::home);
    let config = TerminalConfig {
        cwd: Some(cwd.clone()),
        env: vec![("TERM_PROGRAM".into(), "synfiles".into())],
        ..TerminalConfig::default()
    };
    match TerminalSession::new(config) {
        Ok(s) => {
            SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
            SYNCED.with(|x| *x.borrow_mut() = Some(cwd));
            follow_terminal();
            Some(s)
        }
        Err(e) => {
            state::toast_error(t!("Не удалось запустить терминал: {e}", e = e));
            None
        }
    }
}

/// Панель перешла в другую папку — терминал за ней (если он свободен).
pub fn pane_moved() {
    if !state::ctx().terminal.get_untracked() {
        return;
    }
    let (Some(s), Some(dir)) = (session(), active_dir()) else { return };
    if s.is_busy() || SYNCED.with(|x| x.borrow().as_deref() == Some(dir.as_path())) {
        return;
    }
    // gvfs и прочие пути без доступа — не трогаем оболочку.
    if !dir.is_dir() {
        return;
    }
    s.write(cd_line(&dir).as_bytes());
    SYNCED.with(|x| *x.borrow_mut() = Some(dir));
}

/// Раз в 0,7 с: оболочка вышла — закрыть; сделали `cd` в терминале — перевести панель.
fn follow_terminal() {
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(700));
        let (tx, rx) = std::sync::mpsc::channel();
        syngui::async_runtime::run_on_main_thread(move || {
            let Some(s) = session() else {
                let _ = tx.send(false);
                return;
            };
            if s.has_exited() || !s.is_alive() {
                SESSION.with(|x| *x.borrow_mut() = None);
                if let Some(ctx) = state::try_ctx() {
                    ctx.terminal.set(false);
                }
                let _ = tx.send(false);
                return;
            }
            if !s.is_busy() {
                if let Some(cwd) = s.cwd() {
                    let moved = SYNCED.with(|x| x.borrow().as_deref() != Some(cwd.as_path()));
                    if moved {
                        SYNCED.with(|x| *x.borrow_mut() = Some(cwd.clone()));
                        let p = state::pane();
                        if p.loc.get_untracked().dir() != Some(cwd.as_path()) {
                            state::navigate(p, Location::Dir(cwd), true);
                        }
                    }
                }
            }
            let _ = tx.send(true);
        });
        if !rx.recv().unwrap_or(false) {
            return;
        }
    });
}

pub fn toggle() {
    let ctx = state::ctx();
    let on = !ctx.terminal.get_untracked();
    if on && start().is_none() {
        return;
    }
    ctx.terminal.set(on);
}

pub fn panel() -> W {
    let Some(s) = start() else { return boxed(DecoratedBox::new()) };
    s.reset_autofocus();
    boxed(
        DecoratedBox::new().class("term-panel").child(
            Terminal::new()
                .attach(s)
                .autofocus(true)
                .font_family("monospace")
                .font_size(13.0)
                .on_key(|k, _| {
                    if k == Key::F12 {
                        syngui::async_runtime::run_on_main_thread(toggle);
                        return true;
                    }
                    false
                })
                .class("grow term"),
        ),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn cd_quotes() {
        assert_eq!(super::cd_line(std::path::Path::new("/a/it's dir")), "\x15 cd -- '/a/it'\\''s dir'\r");
    }
}
