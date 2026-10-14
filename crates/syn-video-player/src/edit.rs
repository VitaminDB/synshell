//! Монтаж: ролик (или несколько) — последовательность кусков исходных файлов. Ножницы режут кусок
//! в точке воспроизведения, куски убираются и возвращаются, переставляются, в конец добавляются
//! другие ролики. Сохранение — `ffmpeg`:
//! - «Точно» — перекодирование (H.264 + AAC, MP4): резка по кадру, склейка любых роликов (размер
//!   и частота кадров — по первому куску, остальные вписываются с полями);
//! - «Быстро» — без перекодирования (concat-демуксер с `inpoint`/`outpoint`): мгновенно, но куски
//!   начинаются с ближайшего ключевого кадра; только если у всех кусков одинаковые кодеки и размер.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use syngui::t;

/// Короче не режем: кусок меньше этого — промах ножниц.
pub const MIN_PIECE: f64 = 0.2;

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    /// Убранный кусок остаётся на шкале (серым), в результат не идёт.
    pub keep: bool,
}

impl Piece {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }

    fn contains(&self, path: &Path, t: f64) -> bool {
        self.path == path && t >= self.start && t < self.end
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Edit {
    pub pieces: Vec<Piece>,
    /// Выбранный кусок: подсказка, какой из кусков считать текущим, если один и тот же
    /// ролик добавлен дважды; иначе текущий — тот, где точка воспроизведения.
    pub sel: usize,
    undo: Vec<(Vec<Piece>, usize)>,
    /// Глубина отмены на момент сохранения: правок с тех пор нет — выход не спрашивает.
    saved: usize,
}

impl Edit {
    pub fn new(path: PathBuf, duration: f64) -> Self {
        Self { pieces: vec![Piece { path, start: 0.0, end: duration, keep: true }], sel: 0, undo: Vec::new(), saved: 0 }
    }

    fn save_undo(&mut self) {
        self.undo.push((self.pieces.clone(), self.sel));
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Есть несохранённые правки.
    pub fn modified(&self) -> bool {
        self.undo.len() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.undo.len();
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some((p, s)) => {
                self.pieces = p;
                self.sel = s;
                true
            }
            None => false,
        }
    }

    /// Кусок, в котором точка `t` ролика `path`: выбранный, если он подходит, иначе первый.
    pub fn locate(&self, path: &Path, t: f64) -> Option<usize> {
        if self.pieces.get(self.sel).is_some_and(|p| p.contains(path, t)) {
            return Some(self.sel);
        }
        self.pieces.iter().position(|p| p.contains(path, t))
    }

    /// Разрезать кусок в точке `t`; выбранным становится правая половина.
    pub fn split(&mut self, path: &Path, t: f64) -> bool {
        let Some(i) = self.locate(path, t) else { return false };
        let p = &self.pieces[i];
        if t - p.start < MIN_PIECE || p.end - t < MIN_PIECE {
            return false;
        }
        self.save_undo();
        let mut right = self.pieces[i].clone();
        right.start = t;
        self.pieces[i].end = t;
        self.pieces.insert(i + 1, right);
        self.sel = i + 1;
        true
    }

    /// Убрать кусок из результата или вернуть.
    pub fn toggle(&mut self, i: usize) {
        if i < self.pieces.len() {
            self.save_undo();
            self.pieces[i].keep = !self.pieces[i].keep;
            self.sel = i;
        }
    }

    /// Сдвинуть кусок на место соседа (`delta` −1 — влево, +1 — вправо).
    pub fn shift(&mut self, i: usize, delta: isize) -> bool {
        let j = i as isize + delta;
        if i >= self.pieces.len() || j < 0 || j as usize >= self.pieces.len() {
            return false;
        }
        self.save_undo();
        self.pieces.swap(i, j as usize);
        self.sel = j as usize;
        true
    }

    /// Добавить ролик целиком в конец.
    pub fn append(&mut self, path: PathBuf, duration: f64) {
        self.save_undo();
        self.pieces.push(Piece { path, start: 0.0, end: duration, keep: true });
        self.sel = self.pieces.len() - 1;
    }

    pub fn kept(&self) -> Vec<Piece> {
        self.pieces.iter().filter(|p| p.keep).cloned().collect()
    }

    /// Длительность результата.
    pub fn total(&self) -> f64 {
        self.pieces.iter().filter(|p| p.keep).map(Piece::len).sum()
    }

    /// Следующий оставленный кусок после `i`.
    pub fn next_kept(&self, i: usize) -> Option<usize> {
        (i + 1..self.pieces.len()).find(|&j| self.pieces[j].keep)
    }

    /// Кусок `j` продолжает `i` в том же файле без разрыва — при просмотре переход не нужен.
    pub fn continues(&self, i: usize, j: usize) -> bool {
        match (self.pieces.get(i), self.pieces.get(j)) {
            (Some(a), Some(b)) => a.path == b.path && (b.start - a.end).abs() < 1e-3,
            _ => false,
        }
    }
}

// ─── сведения о файле ───────────────────────────────────────────────────────

/// Что нужно о ролике для склейки (`ffprobe`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub duration: f64,
    pub vcodec: String,
    /// Размер на экране — с учётом поворота (видео с телефона).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub acodec: Option<String>,
}

pub fn probe(path: &Path) -> Result<Probe, String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let streams = j["streams"].as_array().cloned().unwrap_or_default();
    let v = streams.iter().find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let Some(v) = v else { return Err(t!("в файле нет видео")) };
    let a = streams.iter().find(|s| s["codec_type"] == "audio");
    let num = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64());
    let duration = num(&j["format"]["duration"]).or_else(|| num(&v["duration"])).unwrap_or(0.0);
    let (mut w, mut h) = (v["width"].as_u64().unwrap_or(0) as u32, v["height"].as_u64().unwrap_or(0) as u32);
    let rotation = v["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| num(&v["tags"]["rotate"]))
        .unwrap_or(0.0);
    if (rotation.abs() as i64) % 180 == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    let fps = v["avg_frame_rate"]
        .as_str()
        .and_then(parse_rate)
        .or_else(|| v["r_frame_rate"].as_str().and_then(parse_rate))
        .filter(|f| *f > 1.0 && *f < 241.0)
        .unwrap_or(30.0);
    Ok(Probe {
        duration,
        vcodec: v["codec_name"].as_str().unwrap_or_default().to_string(),
        width: w,
        height: h,
        fps,
        acodec: a.and_then(|a| a["codec_name"].as_str()).map(str::to_string),
    })
}

/// «30000/1001» → 29.97.
fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

/// Можно склеить без перекодирования: одинаковые кодеки, размер и наличие звука.
pub fn copy_compatible(probes: &[Probe]) -> bool {
    probes.windows(2).all(|w| w[0].vcodec == w[1].vcodec && (w[0].width, w[0].height) == (w[1].width, w[1].height) && w[0].acodec == w[1].acodec)
}

// ─── сохранение ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Перекодирование: точно по кадру.
    Precise,
    /// Без перекодирования: по ключевым кадрам.
    Fast,
}

/// Куда сохранить: рядом с первым куском, «<имя> — монтаж.<ext>», без перезаписи.
pub fn output_path(first: &Path, mode: Mode) -> PathBuf {
    let dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    let ext = match mode {
        Mode::Precise => "mp4".to_string(),
        Mode::Fast => first.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "mp4".into()),
    };
    let base = t!("{name} — монтаж", name = stem);
    let mut out = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    out
}

/// Аргументы `ffmpeg` для «Точно»: каждый кусок — свой вход с точной перемоткой (`-ss`/`-t` перед
/// `-i`), кадр приводится к размеру и частоте первого куска, звук — 48 кГц стерео (тишина, если у
/// куска звука нет, а у других есть).
fn precise_args(pieces: &[Piece], probes: &[Probe], out: &Path) -> Vec<String> {
    let first = &probes[0];
    // libx264 с yuv420p хочет чётные стороны
    let (w, h) = (first.width.max(2) & !1, first.height.max(2) & !1);
    let fps = first.fps;
    let audio = probes.iter().any(|p| p.acodec.is_some());
    let mut a: Vec<String> = Vec::new();
    for p in pieces {
        a.extend(["-ss".into(), format!("{:.3}", p.start), "-t".into(), format!("{:.3}", p.len()), "-i".into()]);
        a.push(p.path.to_string_lossy().into_owned());
    }
    let mut graph = String::new();
    let mut cat = String::new();
    for (k, (p, pr)) in pieces.iter().zip(probes).enumerate() {
        graph.push_str(&format!(
            "[{k}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps:.3},format=yuv420p[v{k}];"
        ));
        cat.push_str(&format!("[v{k}]"));
        if audio {
            if pr.acodec.is_some() {
                graph.push_str(&format!("[{k}:a:0]aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS[a{k}];"));
            } else {
                graph.push_str(&format!("anullsrc=r=48000:cl=stereo,atrim=duration={:.3}[a{k}];", p.len()));
            }
            cat.push_str(&format!("[a{k}]"));
        }
    }
    let n = pieces.len();
    graph.push_str(&format!("{cat}concat=n={n}:v=1:a={}[v]{}", audio as u8, if audio { "[a]" } else { "" }));
    a.extend(["-filter_complex".into(), graph, "-map".into(), "[v]".into()]);
    if audio {
        a.extend(["-map".into(), "[a]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }
    a.extend(
        ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-movflags", "+faststart"].map(String::from),
    );
    a.push(out.to_string_lossy().into_owned());
    a
}

/// Список concat-демуксера для «Быстро».
fn concat_list(pieces: &[Piece], durations: &[f64]) -> String {
    let mut s = String::from("ffconcat version 1.0\n");
    for (p, d) in pieces.iter().zip(durations) {
        let quoted = p.path.to_string_lossy().replace('\'', "'\\''");
        s.push_str(&format!("file '{quoted}'\n"));
        if p.start > 0.0 {
            s.push_str(&format!("inpoint {:.3}\n", p.start));
        }
        // до конца файла outpoint не нужен — иначе можно потерять хвост звука
        if p.end < d - 0.05 {
            s.push_str(&format!("outpoint {:.3}\n", p.end));
        }
    }
    s
}

fn fast_args(list: &Path, out: &Path, mp4: bool) -> Vec<String> {
    let mut a: Vec<String> =
        ["-f", "concat", "-safe", "0", "-i"].map(String::from).into_iter().chain([list.to_string_lossy().into_owned()]).collect();
    a.extend(["-map", "0:v:0", "-map", "0:a:0?", "-c", "copy", "-avoid_negative_ts", "make_zero"].map(String::from));
    if mp4 {
        a.extend(["-movflags", "+faststart"].map(String::from));
    }
    a.push(out.to_string_lossy().into_owned());
    a
}

/// Сохранить результат; `progress` — доля 0…1, `cancel` — прервать (файл удаляется).
pub fn export(
    pieces: &[Piece],
    mode: Mode,
    cancel: &Arc<AtomicBool>,
    progress: impl Fn(f32),
) -> Result<PathBuf, String> {
    if pieces.is_empty() {
        return Err(t!("Все куски убраны — сохранять нечего"));
    }
    let probes = pieces.iter().map(|p| probe(&p.path)).collect::<Result<Vec<_>, _>>()?;
    if mode == Mode::Fast && !copy_compatible(&probes) {
        return Err(t!("Ролики разные (кодек, размер или звук) — без перекодирования не склеить, выберите «Точно»"));
    }
    let out = output_path(&pieces[0].path, mode);
    let mut list_file = None;
    let args = match mode {
        Mode::Precise => precise_args(pieces, &probes, &out),
        Mode::Fast => {
            let durations: Vec<f64> = probes.iter().map(|p| p.duration).collect();
            let list = std::env::temp_dir().join(format!("syn-video-player-{}.ffconcat", std::process::id()));
            std::fs::write(&list, concat_list(pieces, &durations)).map_err(|e| e.to_string())?;
            let mp4 = matches!(out.extension().and_then(|e| e.to_str()), Some("mp4" | "m4v" | "mov"));
            let a = fast_args(&list, &out, mp4);
            list_file = Some(list);
            a
        }
    };
    let total: f64 = pieces.iter().map(Piece::len).sum();
    tracing::info!("монтаж: ffmpeg {}", args.join(" "));
    let r = run_ffmpeg(&args, total, cancel, progress);
    if let Some(l) = list_file {
        let _ = std::fs::remove_file(l);
    }
    match r {
        Ok(()) => Ok(out),
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            Err(e)
        }
    }
}

fn run_ffmpeg(args: &[String], total: f64, cancel: &Arc<AtomicBool>, progress: impl Fn(f32)) -> Result<(), String> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-progress", "pipe:1", "-nostats"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    // stderr читается отдельно: иначе при длинной ругани ffmpeg встанет на полном канале
    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
        }
        s
    });
    let stdout = child.stdout.take();
    let pid = child.id();
    let watcher = {
        let cancel = cancel.clone();
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        std::thread::spawn(move || {
            while !d.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        done
    };
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                if total > 0.0 {
                    progress((us / 1e6 / total).clamp(0.0, 1.0) as f32);
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    watcher.store(true, Ordering::Relaxed);
    let err = err_thread.join().unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err(t!("Отменено"));
    }
    if !status.success() {
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        return Err(t!("ffmpeg завершился с ошибкой: {e}", e = last));
    }
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed() -> Edit {
        Edit::new("/v/a.mp4".into(), 10.0)
    }

    #[test]
    fn split_toggle_undo() {
        let mut e = ed();
        let a = Path::new("/v/a.mp4");
        assert!(e.split(a, 3.0));
        assert!(e.split(a, 6.0));
        assert_eq!(e.pieces.len(), 3);
        assert_eq!((e.pieces[1].start, e.pieces[1].end), (3.0, 6.0));
        assert_eq!(e.sel, 2);
        e.toggle(1);
        assert!((e.total() - 7.0).abs() < 1e-9);
        assert_eq!(e.next_kept(0), Some(2));
        assert!(e.continues(0, 1) && !e.continues(0, 2));
        // промах у края куска
        assert!(!e.split(a, 6.1));
        assert!(e.undo() && e.undo());
        assert_eq!(e.pieces.len(), 2);
        assert!(e.undo() && !e.undo());
        assert_eq!(e, ed());
    }

    #[test]
    fn locate_prefers_selected() {
        let mut e = ed();
        e.append("/v/a.mp4".into(), 10.0);
        let a = Path::new("/v/a.mp4");
        assert_eq!(e.locate(a, 2.0), Some(1));
        e.sel = 0;
        assert_eq!(e.locate(a, 2.0), Some(0));
        assert_eq!(e.locate(Path::new("/v/b.mp4"), 2.0), None);
    }

    #[test]
    fn shift_moves_selection() {
        let mut e = ed();
        e.append("/v/b.mp4".into(), 5.0);
        assert!(e.shift(1, -1));
        assert_eq!(e.pieces[0].path, Path::new("/v/b.mp4"));
        assert_eq!(e.sel, 0);
        assert!(!e.shift(0, -1));
    }

    #[test]
    fn concat_list_quotes_and_trims() {
        let p = vec![
            Piece { path: "/v/it's.mp4".into(), start: 0.0, end: 4.0, keep: true },
            Piece { path: "/v/b.mp4".into(), start: 2.5, end: 9.99, keep: true },
        ];
        let s = concat_list(&p, &[10.0, 10.0]);
        assert!(s.contains("file '/v/it'\\''s.mp4'\noutpoint 4.000\n"));
        assert!(s.contains("file '/v/b.mp4'\ninpoint 2.500\n"));
        assert!(!s.contains("outpoint 9.990"));
    }

    #[test]
    fn precise_graph_fills_missing_audio() {
        let p = vec![
            Piece { path: "/v/a.mp4".into(), start: 1.0, end: 3.0, keep: true },
            Piece { path: "/v/b.mp4".into(), start: 0.0, end: 2.0, keep: true },
        ];
        let pr = vec![
            Probe { duration: 5.0, vcodec: "h264".into(), width: 1921, height: 1080, fps: 30.0, acodec: Some("aac".into()) },
            Probe { duration: 2.0, vcodec: "hevc".into(), width: 1280, height: 720, fps: 25.0, acodec: None },
        ];
        let a = precise_args(&p, &pr, Path::new("/v/out.mp4")).join(" ");
        assert!(a.starts_with("-ss 1.000 -t 2.000 -i /v/a.mp4 -ss 0.000 -t 2.000 -i /v/b.mp4"));
        assert!(a.contains("scale=1920:1080"));
        assert!(a.contains("anullsrc=r=48000:cl=stereo,atrim=duration=2.000[a1]"));
        assert!(a.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[v][a]"));
        assert!(!copy_compatible(&pr));
    }

    #[test]
    fn rate() {
        assert!((parse_rate("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), None);
    }
}

#[cfg(test)]
mod real {
    use super::*;

    /// Настоящий ffmpeg: `SVP_EDIT_DIR=… cargo test -- --ignored real`.
    #[test]
    #[ignore]
    fn export_real() {
        let dir = PathBuf::from(std::env::var("SVP_EDIT_DIR").unwrap());
        let (a, b) = (dir.join("a.mp4"), dir.join("b.mp4"));
        let c = Arc::new(AtomicBool::new(false));
        let same = vec![
            Piece { path: a.clone(), start: 0.0, end: 3.0, keep: true },
            Piece { path: a.clone(), start: 6.0, end: 10.0, keep: true },
        ];
        let out = export(&same, Mode::Fast, &c, |_| {}).unwrap();
        eprintln!("fast {} {:?}", out.display(), probe(&out));
        let out = export(&same, Mode::Precise, &c, |f| eprint!("{f:.2} ")).unwrap();
        let p = probe(&out).unwrap();
        eprintln!("precise {} {:?}", out.display(), p);
        assert!((p.duration - 7.0).abs() < 0.15);
        let mixed = vec![same[0].clone(), Piece { path: b.clone(), start: 1.0, end: 4.0, keep: true }];
        assert!(export(&mixed, Mode::Fast, &c, |_| {}).is_err());
        let out = export(&mixed, Mode::Precise, &c, |_| {}).unwrap();
        let p = probe(&out).unwrap();
        eprintln!("mixed {} {:?}", out.display(), p);
        assert!((p.duration - 6.0).abs() < 0.15);
    }
}
