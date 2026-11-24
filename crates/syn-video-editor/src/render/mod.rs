//! Движок кадра: чтение кадров, LUT, сборка слоёв, экспорт. Тяжёлая работа
//! (экспорт, кадр в файл) — отдельным процессом этой же программы
//! ([`run_child`]): у композитора свой GPU и поток, окно не замирает.

pub mod compose;
pub mod export;
pub mod lut;

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::doc::Project;

/// Сохранить проект во временный файл для дочернего процесса.
fn temp_project(p: &Project) -> Result<PathBuf, String> {
    let dir = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("render-{}-{}.syn", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    crate::store::save(p, &path)?;
    Ok(path)
}

/// Запустить `syn-video-editor <args…>` над копией проекта; строки `progress N M`
/// дочернего процесса — в `progress`.
pub fn run_child(p: &Project, args: &[String], mut progress: impl FnMut(usize, usize)) -> Result<String, String> {
    let tmp = temp_project(p)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe).arg("--render-child").arg(&tmp).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let mut last = String::new();
    for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("progress ") {
            let mut it = rest.split_whitespace().filter_map(|v| v.parse::<usize>().ok());
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                progress(a, b);
            }
        } else {
            last = line;
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    if out.status.success() {
        Ok(last)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().filter(|l| !l.contains(" INFO ") && !l.contains(" WARN ")).collect::<Vec<_>>().join("\n");
        Err(if err.trim().is_empty() { format!("код выхода {:?}", out.status.code()) } else { err.trim().to_string() })
    }
}

/// Дочерний процесс: `--render-child ПРОЕКТ export ВЫХОД [crf] [t0 t1]` или `frame t ВЫХОД.png [ширина]`.
pub fn child_main(args: &[String]) -> Result<String, String> {
    let p = crate::store::load(Path::new(args.first().ok_or("нет проекта")?))?;
    match args.get(1).map(String::as_str) {
        Some("export") => {
            let out = PathBuf::from(args.get(2).ok_or("нет пути")?);
            let mut opts = export::Opts::default();
            if let Some(c) = args.get(3).and_then(|v| v.parse().ok()) {
                opts.crf = c;
            }
            if let (Some(a), Some(b)) = (args.get(4).and_then(|v| v.parse().ok()), args.get(5).and_then(|v| v.parse().ok())) {
                opts.range = Some((a, b));
            }
            let started = std::time::Instant::now();
            export::export(&p, &out, &opts, |i, n| {
                if i == n || i % 5 == 0 {
                    println!("progress {i} {n}");
                }
            })?;
            Ok(format!("{} ({:.1} с)", out.display(), started.elapsed().as_secs_f64()))
        }
        Some("frame") => {
            let t: f64 = args.get(2).and_then(|v| v.parse().ok()).ok_or("нет времени")?;
            let out = args.get(3).ok_or("нет пути")?;
            let width: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(p.settings.width);
            let k = width as f32 / p.settings.width as f32;
            let (w, h) = (((p.settings.width as f32 * k) as u32) & !1, ((p.settings.height as f32 * k) as u32) & !1);
            let mut src = compose::FrameSource::new(w, h);
            let frames = src.frames(&p, t);
            let mut off = syngui::offscreen::Offscreen::new((w, h), "")?;
            let pc = p.clone();
            let rgba = off.render(move || compose::frame_widget(&pc, t, &frames, k))?;
            image::save_buffer(out, &rgba, w, h, image::ExtendedColorType::Rgba8).map_err(|e| format!("{out}: {e}"))?;
            Ok(out.clone())
        }
        _ => Err("режим: export или frame".into()),
    }
}
