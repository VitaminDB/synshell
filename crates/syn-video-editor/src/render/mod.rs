//! Движок кадра: чтение кадров, LUT, сборка слоёв, экспорт. Тяжёлая работа
//! (экспорт, кадр в файл) — отдельным процессом этой же программы
//! ([`run_child`]): у композитора свой GPU и поток, окно не замирает.

pub mod compose;
pub mod export;
pub mod grade;
pub mod lut;
pub mod neural;
pub mod preview;

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
    // после обновления пакета работающее окно видит «/usr/bin/… (deleted)» — берём новый файл
    let exe = synshell_common::paths::self_exe("syn-video-editor");
    let mut child = Command::new(exe).arg("--render-child").arg(&tmp).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    // «Стоп» в панели ИИ прерывает и рендер агента (дочерний процесс)
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (pid, done) = (child.id(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                if crate::ui::AI_CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = Command::new("kill").arg(pid.to_string()).status();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
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
    let out = child.wait_with_output().map_err(|e| e.to_string());
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let out = out?;
    let _ = std::fs::remove_file(&tmp);
    if crate::ui::AI_CANCEL.load(std::sync::atomic::Ordering::SeqCst) && !out.status.success() {
        return Err("остановлено пользователем («Стоп»)".into());
    }
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
    // именованные (encoder=…) — отдельно от позиционных
    let encoder = args.iter().find_map(|a| a.strip_prefix("encoder=")).map(str::to_string);
    let loudness = args.iter().find_map(|a| a.strip_prefix("loudness=")).and_then(|v| v.parse::<f64>().ok());
    let args: Vec<String> = args.iter().filter(|a| !a.starts_with("encoder=") && !a.starts_with("loudness=")).cloned().collect();
    let args = &args[..];
    let p = crate::store::load(Path::new(args.first().ok_or("нет проекта")?))?;
    match args.get(1).map(String::as_str) {
        Some("export") => {
            let out = PathBuf::from(args.get(2).ok_or("нет пути")?);
            let mut opts = export::Opts::default();
            opts.encoder = encoder;
            if let Some(l) = loudness {
                opts.loudness = (l < 0.0).then_some(l);
            }
            if let Some(c) = args.get(3).and_then(|v| v.parse().ok()) {
                opts.crf = c;
            }
            if let (Some(a), Some(b)) = (args.get(4).and_then(|v| v.parse().ok()), args.get(5).and_then(|v| v.parse().ok())) {
                opts.range = Some((a, b));
            }
            let started = std::time::Instant::now();
            let notes = export::export(&p, &out, &opts, |i, n| {
                if i == n || i % 5 == 0 {
                    println!("progress {i} {n}");
                }
            })?;
            let mut s = format!("{} (ролик {}, экспорт занял {:.1} с)", out.display(), crate::doc::tc(opts.range.map(|(a, b)| b - a).unwrap_or(p.duration())), started.elapsed().as_secs_f64());
            for n in notes {
                s.push_str(&format!("; {n}"));
            }
            Ok(s)
        }
        Some("frame") => {
            let t: f64 = args.get(2).and_then(|v| v.parse().ok()).ok_or("нет времени")?;
            let out = args.get(3).ok_or("нет пути")?;
            let width: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(p.settings.width);
            let k = width as f32 / p.settings.width as f32;
            let (w, h) = (((p.settings.width as f32 * k) as u32) & !1, ((p.settings.height as f32 * k) as u32) & !1);
            let mut src = compose::FrameSource::new(w, h);
            let frames = src.frames(&p, t);
            let mut off = syngui::offscreen::Offscreen::new((w, h), "").or_else(|_| syngui::offscreen::Offscreen::low_power((w, h), ""))?;
            let pc = p.clone();
            let fr = frames.clone();
            let rgba = match off.render(move || compose::frame_widget(&pc, t, &frames, k)) {
                Ok(v) => v,
                // дискретная видеокарта занята нейросетями — кадр на встроенной
                Err(_) if !off.low_power => {
                    let mut low = syngui::offscreen::Offscreen::low_power((w, h), "")?;
                    let pc = p.clone();
                    low.render(move || compose::frame_widget(&pc, t, &fr, k))?
                }
                Err(e) => return Err(e),
            };
            image::save_buffer(out, &rgba, w, h, image::ExtendedColorType::Rgba8).map_err(|e| format!("{out}: {e}"))?;
            Ok(out.clone())
        }
        _ => Err("режим: export или frame".into()),
    }
}
