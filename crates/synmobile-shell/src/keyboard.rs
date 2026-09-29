//! Экранная клавиатура — отдельный демон `synkeyboard` (крейт synkeyboard).
//! Оболочка запускает его рядом с собой и перезапускает при падении;
//! показывается сама по text-input окна или командой `shell keyboard`.

use std::process::{Command, Stdio};
use std::time::Duration;

pub fn start() {
    std::thread::Builder::new()
        .name("synkeyboard".into())
        .spawn(|| loop {
            match Command::new("synkeyboard").stdin(Stdio::null()).spawn() {
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
