//! Нейрофильтры synaptix-vfx как ноды flow (`vfx_upscale`, `vfx_interpolate`):
//! ffmpeg разворачивает ролик в кадры RGBA, synaptix-vfx увеличивает их или
//! дорисовывает промежуточные (RIFE), ffmpeg собирает MP4 со звуком исходника.
//! Модели — бандлы `.syn` в папках `vfx` библиотек службы ([`crate::vfx_models`]).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use synaptix_flow as sf;
use sf::run::Cx;
use sf::Val;

type R = Result<(), String>;

/// Сведения о ролике: ширина, высота, частота, число кадров (оценка), есть ли звук.
fn probe(path: &str) -> Result<(u32, u32, f64, usize, bool), String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate,nb_frames:format=duration", "-of", "json", path])
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe: {e}"))?;
    let s = v["streams"].get(0).ok_or_else(|| format!("{path}: нет видеопотока"))?;
    let (w, h) = (s["width"].as_u64().unwrap_or(0) as u32, s["height"].as_u64().unwrap_or(0) as u32);
    let fps = s["r_frame_rate"].as_str().and_then(|r| {
        let (a, b) = r.split_once('/')?;
        let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
        (b > 0.0).then(|| a / b)
    });
    let fps = fps.filter(|f| *f > 0.0 && *f < 1000.0).unwrap_or(30.0);
    let dur: f64 = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let n = s["nb_frames"].as_str().and_then(|n| n.parse().ok()).unwrap_or((dur * fps).round() as usize);
    let audio = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0", path])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if w == 0 || h == 0 {
        return Err(format!("{path}: размер кадра неизвестен"));
    }
    Ok((w, h, fps, n, audio))
}

fn is_image(path: &str) -> bool {
    let e = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff")
}

/// Кадры исходника потоком RGBA.
struct Decoder {
    child: std::process::Child,
    out: std::process::ChildStdout,
    frame: usize,
}

impl Decoder {
    fn open(path: &str, w: u32, h: u32) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args(["-v", "error", "-i", path, "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let out = child.stdout.take().ok_or("ffmpeg: нет вывода")?;
        Ok(Self { child, out, frame: w as usize * h as usize * 4 })
    }
    fn next(&mut self) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; self.frame];
        self.out.read_exact(&mut buf).ok().map(|_| buf)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Сборка MP4: кадры RGBA из stdin + звук исходника (`atempo` — при замедлении).
struct Encoder {
    child: std::process::Child,
}

impl Encoder {
    fn open(out: &Path, w: u32, h: u32, fps: f64, src: &str, audio: bool, tempo: f64, scale_to: Option<(u32, u32)>) -> Result<Self, String> {
        let mut a: Vec<String> = ["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"].iter().map(|s| s.to_string()).collect();
        a.extend([format!("{w}x{h}"), "-r".into(), format!("{fps}"), "-i".into(), "-".into()]);
        if audio {
            a.extend(["-i".into(), src.to_string(), "-map".into(), "0:v".into(), "-map".into(), "1:a:0".into()]);
            if (tempo - 1.0).abs() > 1e-3 {
                // atempo — 0.5…2 за звено
                let mut t = tempo;
                let mut chain = Vec::new();
                while t < 0.5 {
                    chain.push("atempo=0.5".to_string());
                    t /= 0.5;
                }
                chain.push(format!("atempo={t:.5}"));
                a.extend(["-filter:a".into(), chain.join(",")]);
            }
            a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), "-shortest".into()]);
        }
        if let Some((tw, th)) = scale_to {
            a.extend(["-vf".into(), format!("scale={tw}:{th}:flags=lanczos")]);
        }
        a.extend(["-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
        a.push(out.display().to_string());
        let child = Command::new("ffmpeg").args(&a).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        Ok(Self { child })
    }
    fn push(&mut self, rgba: &[u8]) -> R {
        self.child.stdin.as_mut().ok_or("ffmpeg: нет ввода")?.write_all(rgba).map_err(|e| format!("ffmpeg: {e}"))
    }
    fn finish(mut self) -> R {
        drop(self.child.stdin.take());
        let out = self.child.wait_with_output().map_err(|e| format!("ffmpeg: {e}"))?;
        if !out.status.success() {
            return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ошибка")));
        }
        Ok(())
    }
}

/// Модель: (путь, вид) по подсказке.
fn find_model(pred: impl Fn(&str, &str) -> bool, prefer: &str) -> Option<std::path::PathBuf> {
    let mut v: Vec<(String, std::path::PathBuf, String)> = crate::vfx_models::bundles().into_iter().filter(|(id, _, arch)| pred(id, arch)).collect();
    v.sort_by_key(|(id, _, _)| if !prefer.is_empty() && id.contains(prefer) { 0 } else { 1 });
    v.into_iter().next().map(|(_, p, _)| p)
}

pub fn upscale(cx: &mut Cx) -> R {
    let src = cx.need_file("video")?;
    let want: u32 = cx.num("scale").map(|v| v as u32).filter(|v| matches!(v, 2 | 4)).unwrap_or(2);
    let hint = cx.str("model").unwrap_or_default().to_ascii_lowercase();
    // покадровые модели (nanovsr — видео с памятью кадров, здесь не берётся)
    let path = find_model(|id, arch| arch != "rife" && arch != "nanovsr" && (hint.is_empty() || id.to_ascii_lowercase().contains(&hint) || arch.contains(&hint)), &format!("x{want}"))
        .ok_or_else(|| format!("моделей увеличения{} нет в папке vfx библиотеки нейросетей", if hint.is_empty() { String::new() } else { format!(" «{hint}»") }))?;
    let mut up = synaptix_vfx::open_upscaler(&path, 0).map_err(|e| format!("нейроувеличение: {e}"))?;
    let k = up.scale();
    let mut run = |rgba: &[u8], w: u32, h: u32| -> Result<(Vec<u8>, u32, u32), String> {
        let mut out = None;
        up.push(rgba, w, h, &mut |f| out = Some((f.rgba.to_vec(), f.width, f.height))).map_err(|e| format!("нейроувеличение: {e}"))?;
        out.ok_or_else(|| "увеличение не отдало кадр".to_string())
    };
    if is_image(&src) {
        let (w, h, _, _, _) = probe(&src)?;
        let mut dec = Decoder::open(&src, w, h)?;
        let fr = dec.next().ok_or_else(|| format!("{src}: не удалось прочитать картинку"))?;
        let (px, ow, oh) = run(&fr, w, h)?;
        let path = cx.out_path("png");
        let mut a: Vec<String> = ["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"].iter().map(|s| s.to_string()).collect();
        a.extend([format!("{ow}x{oh}"), "-i".into(), "-".into()]);
        if k != want {
            a.extend(["-vf".into(), format!("scale={}:{}:flags=lanczos", w * want, h * want)]);
        }
        a.extend(["-frames:v".into(), "1".into(), path.display().to_string()]);
        let mut ch = Command::new("ffmpeg").args(&a).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        ch.stdin.take().ok_or("ffmpeg: нет ввода")?.write_all(&px).map_err(|e| format!("ffmpeg: {e}"))?;
        let o = ch.wait_with_output().map_err(|e| format!("ffmpeg: {e}"))?;
        if !o.status.success() {
            return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("ошибка")));
        }
        cx.label = format!("×{want}: {w}×{h} → {}×{}", w * want, h * want);
        cx.out("video", Val::File(path));
        return Ok(());
    }
    let (w, h, fps, n, audio) = probe(&src)?;
    let path = cx.out_path("mp4");
    let target = ((w * want) & !1, (h * want) & !1);
    let mut dec = Decoder::open(&src, w, h)?;
    let mut enc: Option<Encoder> = None;
    let mut i = 0usize;
    while let Some(fr) = dec.next() {
        cx.cancel.check()?;
        let (px, ow, oh) = run(&fr, w, h)?;
        if enc.is_none() {
            let scale_to = ((ow, oh) != target).then_some(target);
            enc = Some(Encoder::open(&path, ow, oh, fps, &src, audio, 1.0, scale_to)?);
        }
        enc.as_mut().unwrap().push(&px)?;
        i += 1;
        cx.progress(i, n.max(i));
    }
    enc.ok_or("в ролике нет кадров")?.finish()?;
    cx.label = format!("×{want}: {}×{} → {}×{}, кадров {i}", w, h, target.0, target.1);
    cx.out("video", Val::File(path));
    Ok(())
}

pub fn interpolate(cx: &mut Cx) -> R {
    let src = cx.need_file("video")?;
    let fps_out = cx.num("fps").filter(|f| *f >= 1.0).unwrap_or(60.0).min(240.0);
    let slow = cx.num("slowmo").filter(|s| *s >= 1.0).unwrap_or(1.0).min(16.0);
    let path_m = find_model(|_, arch| arch == "rife", "rife-4-25").ok_or("модели RIFE нет (rife-*.syn в папке vfx библиотеки нейросетей)")?;
    let wts = synaptix_vfx::weights::Weights::open(&path_m).map_err(|e| format!("RIFE: {e}"))?;
    let g = synaptix_vfx::gpu::Gpu::new(0).map_err(|e| format!("RIFE: {e}"))?;
    let mut rife = synaptix_vfx::rife::Rife::load(g, &wts).map_err(|e| format!("RIFE: {e}"))?;
    let (w, h, fps_in, n_in, audio) = probe(&src)?;
    rife.set_flow_scale(if h > 1440 { 0.5 } else { 1.0 });
    // время источника на кадр результата: 1/fps_out, при замедлении — в slow раз меньше
    let step = 1.0 / (fps_out * slow) * fps_in; // в кадрах источника
    let total = ((n_in.saturating_sub(1)) as f64 / step).floor() as usize + 1;
    let path = cx.out_path("mp4");
    let mut dec = Decoder::open(&src, w, h)?;
    let mut enc = Encoder::open(&path, w, h, fps_out, &src, audio, 1.0 / slow, None)?;
    let (mut a, mut b) = (dec.next().ok_or("в ролике нет кадров")?, dec.next());
    let mut ia = 0usize; // номер кадра `a`
    let mut pushed = false; // пара (a, b) уже в RIFE
    let mut cut = false;
    let (mut out_n, mut drawn) = (0usize, 0usize);
    loop {
        cx.cancel.check()?;
        let pos = out_n as f64 * step; // позиция в кадрах источника
        // сдвинуть окно, пока b не правее позиции
        while pos >= (ia + 1) as f64 {
            match dec.next() {
                Some(nb) => {
                    a = b.take().unwrap_or_else(|| a.clone());
                    b = Some(nb);
                    ia += 1;
                    if pushed {
                        // окно RIFE скользит: прошлый второй — новый первый
                        rife.push(b.as_ref().unwrap(), w as usize, h as usize).map_err(|e| format!("RIFE: {e}"))?;
                        cut = rife.scene_diff().map_err(|e| format!("RIFE: {e}"))? > 28.0;
                    }
                }
                None => {
                    if b.is_some() {
                        a = b.take().unwrap();
                        ia += 1;
                    }
                    break;
                }
            }
        }
        let Some(bb) = b.as_ref() else {
            // последний кадр источника
            if pos <= ia as f64 + 1e-6 {
                enc.push(&a)?;
                out_n += 1;
            }
            break;
        };
        let t = (pos - ia as f64) as f32;
        if t <= 0.01 {
            enc.push(&a)?;
        } else if t >= 0.99 {
            enc.push(bb)?;
        } else {
            if !pushed {
                rife.reset();
                rife.push(&a, w as usize, h as usize).map_err(|e| format!("RIFE: {e}"))?;
                rife.push(bb, w as usize, h as usize).map_err(|e| format!("RIFE: {e}"))?;
                cut = rife.scene_diff().map_err(|e| format!("RIFE: {e}"))? > 28.0;
                pushed = true;
            }
            if cut {
                // склейка сцен — без каши из двух планов
                enc.push(if t < 0.5 { &a } else { bb })?;
            } else {
                let px = rife.interpolate(t).map_err(|e| format!("RIFE: {e}"))?;
                enc.push(&px)?;
                drawn += 1;
            }
        }
        out_n += 1;
        cx.progress(out_n, total.max(out_n));
    }
    enc.finish()?;
    cx.label = format!("{fps_in:.2} → {fps_out} к/с{}: кадров {out_n}, дорисовано {drawn}", if slow > 1.0 { format!(", замедление ×{slow}") } else { String::new() });
    cx.out("video", Val::File(path));
    Ok(())
}
