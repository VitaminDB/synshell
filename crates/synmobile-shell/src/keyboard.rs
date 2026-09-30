//! Экранная клавиатура — отдельный демон `synkeyboard` (крейт synkeyboard).
//! Оболочка запускает его рядом с собой и перезапускает при падении;
//! показывается сама по text-input окна или командой `shell keyboard`.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn start() {
    std::thread::Builder::new()
        .name("synkeyboard".into())
        .spawn(|| loop {
            let mut cmd = Command::new("synkeyboard");
            cmd.stdin(Stdio::null());
            // Оболочка упала или перезапущена — демон уходит вместе с ней,
            // иначе копии копились и все отвечали на `synkeyboard show`.
            // SAFETY: в pre_exec — только async-signal-safe prctl.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    log::info!("synkeyboard запущен (pid {})", child.id());
                    match child.wait() {
                        Ok(st) if st.success() => break,
                        Ok(st) => log::warn!("synkeyboard завершился: {st}"),
                        Err(e) => log::warn!("synkeyboard: {e}"),
                    }
                }
                Err(e) => {
                    log::warn!("synkeyboard не запускается ({e})");
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("поток synkeyboard");
}

pub fn toggle() {
    synshell_ui::actions::spawn("synkeyboard toggle");
}
