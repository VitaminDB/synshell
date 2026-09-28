//! Что делается с вырезанной областью: PNG в каталог снимков, буфер
//! обмена (wl-copy), путь в буфер, открыть программой по умолчанию;
//! уведомление — через оболочку.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use syndesktop_common::{ipc, paths, Action, Config};

use crate::sel::Choice;

pub fn run(choice: Choice, w: u32, h: u32, rgba: &[u8], cfg: &Config) -> anyhow::Result<()> {
    let png = encode_png(w, h, rgba)?;
    match choice {
        Choice::Copy => {
            copy("image/png", &png)?;
            notify("screenshot-copied".into());
        }
        Choice::Save => {
            let path = save(&png, cfg)?;
            notify(format!("screenshot-taken {}", path.display()));
        }
        Choice::CopyPath => {
            let path = save(&png, cfg)?;
            copy("text/plain;charset=utf-8", path.to_string_lossy().as_bytes())?;
            notify(format!("screenshot-path {}", path.display()));
        }
        Choice::Open => {
            let path = save(&png, cfg)?;
            open(&path);
        }
    }
    Ok(())
}

pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(out)
}

/// `Снимок_ГГГГ-ММ-ДД_ЧЧ-ММ-СС.png` в `general.screenshot_dir`; занято —
/// с номером.
fn save(png: &[u8], cfg: &Config) -> anyhow::Result<PathBuf> {
    let dir = paths::expand_tilde(&cfg.general.screenshot_dir);
    std::fs::create_dir_all(&dir)?;
    let stem = format!("Снимок_{}", timestamp());
    let mut path = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}.png"));
        n += 1;
    }
    std::fs::write(&path, png)?;
    Ok(path)
}

fn timestamp() -> String {
    let mut t: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::time(&mut t);
        libc::localtime_r(&t, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// В буфер обмена через wl-copy: он остаётся в фоне и отдаёт данные,
/// когда эта программа уже вышла.
fn copy(mime: &str, data: &[u8]) -> anyhow::Result<()> {
    let mut child = Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("не удалось запустить wl-copy (пакет wl-clipboard): {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(data)?;
    }
    child.wait()?;
    Ok(())
}

fn open(path: &Path) {
    let cmd = syndesktop_common::mime::default_app("image/png")
        .and_then(|app| app.commands_for(&[path.to_path_buf()]).into_iter().next())
        .unwrap_or_else(|| format!("syndesktop-files --viewer '{}'", path.to_string_lossy().replace('\'', "'\\''")));
    let _ = Command::new("sh").args(["-c", &cmd]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Уведомление оболочкой (команда `shell …` композитора).
pub fn notify(command: String) {
    if let Err(e) = ipc::send_action(Action::Shell(command)) {
        log::warn!("уведомление не отправлено: {e}");
    }
}
