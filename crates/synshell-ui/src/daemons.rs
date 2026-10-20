//! Демоны рядом с оболочкой: экранная клавиатура `synkeyboard` и экранный
//! контроллер `syngamepad`. Оболочка запускает их, перезапускает при падении
//! и уносит с собой при выходе (`PR_SET_PDEATHSIG`) — иначе при перезапусках
//! копии копились бы и все отвечали на команды.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

fn supervise(bin: &'static str, on_start: fn()) {
    std::thread::Builder::new()
        .name(bin.into())
        .spawn(move || loop {
            let mut cmd = Command::new(bin);
            cmd.stdin(Stdio::null());
            // SAFETY: в pre_exec — только async-signal-safe prctl.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    log::info!("{bin} запущен (pid {})", child.id());
                    on_start();
                    match child.wait() {
                        Ok(st) if st.success() => break,
                        Ok(st) => log::warn!("{bin} завершился: {st}"),
                        Err(e) => log::warn!("{bin}: {e}"),
                    }
                }
                Err(e) => {
                    log::warn!("{bin} не запускается ({e})");
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .unwrap_or_else(|e| panic!("поток {bin}: {e}"));
}

/// Экранная клавиатура: показывается сама по text-input окна или командой
/// `shell keyboard`.
pub fn keyboard() {
    supervise("synkeyboard", || {});
}

/// Экранный контроллер: показывается по режиму ввода приложения в фокусе
/// ([`crate::input_mode`]); новый демон ничего не показывает — режим
/// применяется заново.
pub fn gamepad() {
    supervise("syngamepad", crate::input_mode::daemon_restarted);
}
