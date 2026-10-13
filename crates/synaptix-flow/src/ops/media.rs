//! Ноды монтажа — ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::run::Cx;
use crate::value::Val;

type R = Result<(), String>;

fn ffmpeg(args: &[&str]) -> R {
    let out = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ошибка")))
    }
}

pub fn has_audio(p: &Path) -> bool {
    Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

fn video_size(p: &Path) -> Option<(u32, u32)> {
    let out = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"]).arg(p).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().split(',');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Длительность медиафайла, с.
pub fn media_seconds(path: &Path) -> Option<f64> {
    let out = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

pub fn is_video(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"))
}

pub fn concat(cx: &mut Cx) -> R {
    let mut files: Vec<PathBuf> = cx.files("files").into_iter().map(PathBuf::from).collect();
    for k in ["in1", "in2", "in3", "in4"] {
        files.extend(cx.files(k).into_iter().map(PathBuf::from));
    }
    if files.is_empty() {
        return Err("склейка: нет файлов".into());
    }
    if files.iter().all(|f| is_video(f)) {
        // Ролики приводятся к размеру первого, 24 кадра/с, стерео-звук (тишина, где его нет).
        let (w, h) = video_size(&files[0]).unwrap_or((960, 544));
        let tmp = cx.scratch();
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut list = String::new();
        for (i, f) in files.iter().enumerate() {
            cx.cancel.check()?;
            cx.progress(i, files.len());
            let part = tmp.join(format!("{i:03}.mp4"));
            let fs = f.display().to_string();
            let ps = part.display().to_string();
            let vf = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps=24,setsar=1");
            if has_audio(f) {
                ffmpeg(&["-i", &fs, "-vf", &vf, "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000", "-ac", "2", &ps])?;
            } else {
                ffmpeg(&["-i", &fs, "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-shortest", "-vf", &vf, "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", &ps])?;
            }
            list.push_str(&format!("file '{}'\n", ps.replace('\'', "'\\''")));
        }
        let lp = tmp.join("list.txt");
        std::fs::write(&lp, list).map_err(|e| e.to_string())?;
        let path = cx.out_path("mp4");
        ffmpeg(&["-f", "concat", "-safe", "0", "-i", &lp.display().to_string(), "-c", "copy", "-movflags", "+faststart", &path.display().to_string()])?;
        let _ = std::fs::remove_dir_all(&tmp);
        cx.label = format!("склеено {} роликов", files.len());
        cx.out("file", Val::File(path));
    } else {
        let mut args: Vec<String> = Vec::new();
        for f in &files {
            args.push("-i".into());
            args.push(f.display().to_string());
        }
        let inputs: String = (0..files.len()).map(|i| format!("[{i}:a]")).collect();
        args.extend(["-filter_complex".into(), format!("{inputs}concat=n={}:v=0:a=1[a]", files.len()), "-map".into(), "[a]".into()]);
        let path = cx.out_path("wav");
        args.push(path.display().to_string());
        ffmpeg(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        cx.label = format!("склеено {} дорожек", files.len());
        cx.out("file", Val::File(path));
    }
    Ok(())
}

pub fn mux(cx: &mut Cx) -> R {
    let video = cx.need_file("video")?;
    let audio = cx.need_file("audio")?;
    let path = cx.out_path("mp4");
    let ps = path.display().to_string();
    let dur = media_seconds(Path::new(&video)).unwrap_or(10.0).to_string();
    if cx.str("mode").as_deref() == Some("mix") && has_audio(Path::new(&video)) {
        let vol = cx.num("volume").unwrap_or(0.8);
        ffmpeg(&["-i", &video, "-i", &audio, "-filter_complex", &format!("[1:a]volume={vol}[m];[0:a][m]amix=inputs=2:duration=first[a]"), "-map", "0:v", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-t", &dur, &ps])?;
    } else {
        ffmpeg(&["-i", &video, "-i", &audio, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-t", &dur, &ps])?;
    }
    cx.label = "звук наложен".into();
    cx.out("video", Val::File(path));
    Ok(())
}

pub fn frame(cx: &mut Cx) -> R {
    let video = cx.need_file("video")?;
    let path = cx.out_path("png");
    let ps = path.display().to_string();
    match cx.str("at").as_deref() {
        Some("last") | None => ffmpeg(&["-sseof", "-0.2", "-i", &video, "-frames:v", "1", "-update", "1", &ps])?,
        Some("first") => ffmpeg(&["-i", &video, "-frames:v", "1", &ps])?,
        Some(t) => ffmpeg(&["-ss", t, "-i", &video, "-frames:v", "1", &ps])?,
    }
    cx.label = "кадр".into();
    cx.out("image", Val::File(path));
    Ok(())
}

pub fn trim(cx: &mut Cx) -> R {
    let src = cx.need_file("file")?;
    let ext = Path::new(&src).extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
    let path = cx.out_path(&ext);
    let start = cx.num("start").unwrap_or(0.0);
    let mut args = vec!["-ss".to_string(), start.to_string(), "-i".into(), src];
    if let Some(end) = cx.pos("end") {
        args.extend(["-t".into(), (end - start).max(0.1).to_string()]);
    }
    args.push(path.display().to_string());
    ffmpeg(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    cx.label = "обрезано".into();
    cx.out("file", Val::File(path));
    Ok(())
}

pub fn still(cx: &mut Cx) -> R {
    let image = cx.need_file("image")?;
    let path = cx.out_path("mp4");
    let ps = path.display().to_string();
    match cx.file("audio") {
        Some(audio) => ffmpeg(&["-loop", "1", "-i", &image, "-i", &audio, "-c:v", "libx264", "-tune", "stillimage", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:a", "aac", "-shortest", &ps])?,
        None => {
            let secs = cx.pos("seconds").unwrap_or(5.0).to_string();
            ffmpeg(&["-loop", "1", "-i", &image, "-t", &secs, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", &ps])?
        }
    }
    cx.label = "картинка в ролик".into();
    cx.out("video", Val::File(path));
    Ok(())
}
