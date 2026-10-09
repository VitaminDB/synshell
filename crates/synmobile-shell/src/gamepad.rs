//! Экранный контроллер — отдельный демон `syngamepad` (крейт syngamepad). Оболочка запускает его
//! рядом с собой и перезапускает при падении; показывает по режиму ввода приложения в фокусе
//! (`synshell_ui::input_mode`).

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn start() {
    std::thread::Builder::new()
        .name("syngamepad".into())
        .spawn(|| loop {
            let mut cmd = Command::new("syngamepad");
            cmd.stdin(Stdio::null());
            // Оболочка ушла — и демон с ней (иначе копии копились бы при перезапусках).
            // SAFETY: в pre_exec — только async-signal-safe prctl.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    log::info!("syngamepad запущен (pid {})", child.id());
                    // Новый демон ничего не показывает — режим применить заново.
                    synshell_ui::input_mode::daemon_restarted();
                    match child.wait() {
                        Ok(st) if st.success() => break,
                        Ok(st) => log::warn!("syngamepad завершился: {st}"),
                        Err(e) => log::warn!("syngamepad: {e}"),
                    }
                }
                Err(e) => {
                    log::warn!("syngamepad не запускается ({e})");
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("поток syngamepad");
}
