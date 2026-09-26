//! Снимки экрана: вывод под указателем или окно в фокусе → PNG.

use std::path::PathBuf;

use crate::state::State;

impl State {
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

pub fn write_png(path: &PathBuf, w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
