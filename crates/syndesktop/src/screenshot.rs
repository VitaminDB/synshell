//! Снимки экрана: вывод под указателем или окно в фокусе → PNG; застывший
//! экран для программы снимков (`syndesktop-screenshot`).

use std::path::{Path, PathBuf};

use syndesktop_common::ipc::{CaptureInfo, CapturedOutput, CapturedWindow};

use crate::state::State;

/// Каталог кадров в `$XDG_RUNTIME_DIR`.
fn capture_dir() -> PathBuf {
    syndesktop_common::paths::runtime_dir().join("syndesktop-capture")
}

impl State {
    /// Снять все выводы разом (без указателя): кадры — сырой RGBA в
    /// `$XDG_RUNTIME_DIR/syndesktop-capture`, плюс окна и указатель.
    /// Кадры прошлого захвата, которые никто не забрал, удаляются.
    pub fn capture_all(&mut self) -> anyhow::Result<CaptureInfo> {
        let dir = capture_dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let mut info = CaptureInfo::default();
        let outputs: Vec<_> = self.core.space.outputs().cloned().collect();
        for output in outputs {
            let g = self.core.space.output_geometry(&output).unwrap_or_default();
            let (w, h, mut data) = self.backend.screenshot(&mut self.core, &output)?;
            for px in data.chunks_exact_mut(4) {
                px[3] = 255;
            }
            let path = dir.join(format!("{}-{stamp}.rgba", output.name()));
            write_private(&path, &data)?;
            info.outputs.push(CapturedOutput {
                name: output.name(),
                geometry: [g.loc.x, g.loc.y, g.size.w, g.size.h],
                scale: output.current_scale().fractional_scale(),
                width: w,
                height: h,
                path: path.to_string_lossy().into_owned(),
            });
        }
        let title_h = self.core.deco_theme.height;
        for id in self.core.wm.visible_ids().into_iter().rev() {
            let Some(m) = self.core.wm.get(id) else { continue };
            let g = m.geometry();
            let top = if m.has_titlebar() { title_h } else { 0 };
            info.windows.push(CapturedWindow {
                title: m.title(),
                app_id: m.app_id(),
                rect: [g.loc.x, g.loc.y - top, g.size.w, g.size.h + top],
            });
        }
        let p = self.core.pointer.current_location();
        info.pointer = [p.x, p.y];
        Ok(info)
    }

    /// Print: экран застывает сразу по нажатию, затем запускается
    /// программа снимков с этим кадром. Без неё — обычный снимок вывода.
    pub fn screenshot_interactive(&mut self) {
        if self.core.is_locked() {
            return;
        }
        if !crate::spawn::which("syndesktop-screenshot") {
            tracing::warn!("нет syndesktop-screenshot — снимок всего вывода");
            self.screenshot(false);
            return;
        }
        let info = match self.capture_all() {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(?e, "захват экрана не удался");
                return;
            }
        };
        let json = capture_dir().join("capture.json");
        if let Err(e) = serde_json::to_vec(&info).map_err(anyhow::Error::from).and_then(|b| Ok(write_private(&json, &b)?)) {
            tracing::warn!(?e, "описание захвата не записано");
            return;
        }
        crate::spawn::spawn_shell(&self.core, &format!("syndesktop-screenshot --capture '{}'", json.display()));
    }

    pub fn screenshot(&mut self, window_only: bool) {
        let Some(output) = self.core.output_under_pointer() else { return };
        let (w, h, mut data) = match self.backend.screenshot(&mut self.core, &output) {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(?e, "снимок экрана не удался");
                return;
            }
        };
        let (mut w, mut h) = (w, h);
        if window_only {
            let title_h = self.core.deco_theme.height;
            let scale = output.current_scale().fractional_scale();
            let og = self.core.space.output_geometry(&output).unwrap_or_default();
            if let Some(m) = self.core.wm.focused.and_then(|id| self.core.wm.get(id)) {
                let g = m.geometry();
                let top = if m.has_titlebar() { title_h } else { 0 };
                let x0 = (((g.loc.x - og.loc.x) as f64) * scale).round().max(0.0) as u32;
                let y0 = (((g.loc.y - top - og.loc.y) as f64) * scale).round().max(0.0) as u32;
                let cw = ((g.size.w as f64 * scale).round() as u32).min(w.saturating_sub(x0));
                let ch = (((g.size.h + top) as f64 * scale).round() as u32).min(h.saturating_sub(y0));
                if cw > 0 && ch > 0 {
                    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
                    for row in y0..y0 + ch {
                        let start = ((row * w + x0) * 4) as usize;
                        out.extend_from_slice(&data[start..start + (cw * 4) as usize]);
                    }
                    data = out;
                    w = cw;
                    h = ch;
                }
            }
        }
        // Непрозрачность: альфа кадра нам не нужна в файле.
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let dir = syndesktop_common::paths::expand_tilde(&self.core.config.general.screenshot_dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(file_name());
        match write_png(&path, w, h, &data) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "снимок экрана");
                self.core.ipc.broadcast(&syndesktop_common::ipc::Event::ShellCommand {
                    command: format!("screenshot-taken {}", path.display()),
                });
                if crate::spawn::which("wl-copy") {
                    crate::spawn::spawn_shell(&self.core, &format!("wl-copy --type image/png < '{}'", path.display()));
                }
            }
            Err(e) => tracing::warn!(?e, "снимок не сохранён"),
        }
    }
}

fn file_name() -> String {
    let mut t: libc::time_t = 0;
    unsafe { libc::time(&mut t) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!(
        "Снимок_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.png",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Файл только для владельца: на кадре может быть что угодно.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)
}

pub fn write_png(path: &PathBuf, w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
