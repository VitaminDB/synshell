//! Ноды монтажа — ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::run::Cx;
use crate::value::Val;

type R = Result<(), String>;

pub(crate) fn ffmpeg(args: &[&str]) -> R {
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

/// Моно 16 бит WAV из отсчётов −1..1.
pub fn wav_mono(pcm: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for &x in pcm {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

/// Звук или звуковая дорожка ролика — моно f32 с частотой `rate`.
pub fn decode_pcm(src: &str, rate: u32) -> Result<Vec<f32>, String> {
    let out = Command::new("ffmpeg").args(["-v", "error", "-i", src, "-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"]).output().map_err(|e| format!("ffmpeg: {e}"))?;
    let pcm: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    if pcm.is_empty() {
        return Err(format!("в {src} нет звука"));
    }
    Ok(pcm)
}

/// Панель листа-референса: какую картинку, какой вырез (x, y, w, h в долях
/// исходника) и куда (x, y, w, h в пикселях листа).
#[derive(Debug, Clone, PartialEq)]
pub struct SheetPanel {
    pub src: usize,
    pub crop: Option<(f64, f64, f64, f64)>,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Раскладка листа-референса LTX-2.5 Ingredients (правила Lightricks): чёрный
/// холст `w×h`; сверху герои — у каждого лицо крупно (квадрат из верхней
/// середины портрета в полный рост) и фигура целиком; снизу место (самая
/// крупная панель) и предметы. Ряд, не влезающий по ширине, ужимается целиком.
/// `sizes` — размеры картинок: сначала герои, потом место (если есть), потом предметы.
pub fn sheet_layout(w: u32, h: u32, sizes: &[(u32, u32)], chars: usize, location: bool, face: bool) -> Vec<SheetPanel> {
    let rest = sizes.len() - chars;
    let rows = usize::from(chars > 0) + usize::from(rest > 0);
    if rows == 0 {
        return Vec::new();
    }
    let row_h = h as f64 / rows as f64;
    // (источник, вырез, ширина при высоте ряда)
    let mut top: Vec<(usize, Option<(f64, f64, f64, f64)>, f64)> = Vec::new();
    for (i, &(iw, ih)) in sizes.iter().enumerate().take(chars) {
        let (iw, ih) = (iw.max(1) as f64, ih.max(1) as f64);
        if face && ih > iw {
            // Голова портрета в полный рост — в верхней четверти по центру.
            let side = (ih * 0.23).min(iw);
            top.push((i, Some(((iw - side) / 2.0 / iw, ih * 0.03 / ih, side / iw, side / ih)), row_h));
        }
        top.push((i, None, row_h * iw / ih));
    }
    let bottom: Vec<(usize, Option<(f64, f64, f64, f64)>, f64)> = (chars..sizes.len())
        .map(|i| {
            let (iw, ih) = (sizes[i].0.max(1) as f64, sizes[i].1.max(1) as f64);
            // Место — крупнее предметов: до половины ширины листа.
            let cap = if location && i == chars { w as f64 * 0.5 } else { w as f64 * 0.3 };
            (i, None, (row_h * iw / ih).min(cap))
        })
        .collect();
    let mut out = Vec::new();
    for (r, row) in [top, bottom].into_iter().filter(|r| !r.is_empty()).enumerate() {
        let total: f64 = row.iter().map(|p| p.2).sum();
        let s = (w as f64 / total).min(1.0);
        let ph = row_h * s;
        let mut x = (w as f64 - total * s) / 2.0;
        let y = r as f64 * row_h + (row_h - ph) / 2.0;
        for (src, crop, pw) in row {
            let pw = pw * s;
            let even = |v: f64| ((v / 2.0).floor() * 2.0).max(2.0) as u32;
            out.push(SheetPanel { src, crop, x: x.round() as u32, y: y.round() as u32, w: even(pw), h: even(ph) });
            x += pw;
        }
    }
    out
}

/// Файлы со входа: провод-список или текст (по строке на путь — в путях бывают пробелы).
fn input_files(cx: &Cx, name: &str) -> Vec<String> {
    match cx.str(name) {
        Some(t) if !t.contains('\n') && Path::new(t.trim()).is_file() => vec![t.trim().to_string()],
        Some(t) if t.lines().all(|l| l.trim().is_empty() || Path::new(l.trim()).is_file()) => {
            t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
        }
        _ => cx.files(name),
    }
}

/// Узел «Лист-референс»: картинки героев, места и предметов → один лист на
/// чёрном фоне без текста (для IC-LoRA Ingredients LTX-2.5).
pub fn ref_sheet(cx: &mut Cx) -> R {
    let chars = input_files(cx, "characters");
    let location = cx.file("location");
    let props = input_files(cx, "props");
    let mut files: Vec<String> = chars.clone();
    files.extend(location.iter().cloned());
    files.extend(props.iter().cloned());
    if files.is_empty() {
        return Err("нужны картинки героев, места или предметов".into());
    }
    let w = cx.pos("width").map(|v| v as u32).unwrap_or(960) / 2 * 2;
    let h = cx.pos("height").map(|v| v as u32).unwrap_or(544) / 2 * 2;
    let face = cx.str("face").as_deref() != Some("false");
    let sizes: Vec<(u32, u32)> = files.iter().map(|f| video_size(Path::new(f)).ok_or_else(|| format!("{f}: не картинка"))).collect::<Result<_, _>>()?;
    let panels = sheet_layout(w, h, &sizes, chars.len(), location.is_some(), face);
    let mut args: Vec<String> = Vec::new();
    for f in &files {
        args.extend(["-i".to_string(), f.clone()]);
    }
    let mut graph = format!("color=black:s={w}x{h}:d=1[c0];");
    for (k, p) in panels.iter().enumerate() {
        let crop = match p.crop {
            Some((x, y, cw, ch)) => format!("crop=iw*{cw:.4}:ih*{ch:.4}:iw*{x:.4}:ih*{y:.4},"),
            None => String::new(),
        };
        graph.push_str(&format!("[{}:v]{crop}scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[p{k}];", p.src, p.w, p.h, p.w, p.h));
    }
    for (k, p) in panels.iter().enumerate() {
        let last = if k + 1 == panels.len() { "out".to_string() } else { format!("c{}", k + 1) };
        graph.push_str(&format!("[c{k}][p{k}]overlay={}:{}[{last}];", p.x, p.y));
    }
    graph.pop();
    let path = cx.out_path("png");
    let ps = path.display().to_string();
    let mut all: Vec<&str> = args.iter().map(String::as_str).collect();
    all.extend(["-filter_complex", &graph, "-map", "[out]", "-frames:v", "1", &ps]);
    ffmpeg(&all)?;
    cx.label = format!("лист: героев {}, место {}, предметов {}", chars.len(), if location.is_some() { "есть" } else { "нет" }, props.len());
    cx.out("image", Val::File(path));
    Ok(())
}

#[cfg(test)]
mod sheet_tests {
    use super::sheet_layout;

    #[test]
    fn two_characters_location_and_prop() {
        // Два портрета 768×1152, кухня 1344×768, предмет 1024×1024 на 960×544.
        let p = sheet_layout(960, 544, &[(768, 1152), (768, 1152), (1344, 768), (1024, 1024)], 2, true, true);
        assert_eq!(p.len(), 2 * 2 + 2);
        // Верхний ряд: лицо, фигура, лицо, фигура; всё в пределах листа и в верхней половине.
        assert!(p[..4].iter().all(|q| q.y + q.h <= 272 && q.x + q.w <= 960));
        assert!(p[0].crop.is_some() && p[1].crop.is_none());
        // Нижний ряд: место крупнее предмета, в нижней половине.
        assert!(p[4].w > p[5].w && p[4].y >= 272 && p[4].y + p[4].h <= 544);
        // Без пересечений по горизонтали в ряду.
        for row in [&p[..4], &p[4..]] {
            for w in row.windows(2) {
                assert!(w[0].x + w[0].w <= w[1].x + 1);
            }
        }
    }

    #[test]
    fn only_characters_fill_height_and_shrink_to_width() {
        let p = sheet_layout(960, 544, &[(768, 1152); 4], 4, false, true);
        assert_eq!(p.len(), 8);
        let right = p.iter().map(|q| q.x + q.w).max().unwrap();
        assert!(right <= 960, "ряд ужат по ширине: {right}");
    }
}

/// Живой прогон узла на настоящих картинках:
/// `SHEET_CHARS="a.png\nb.png" SHEET_LOC=k.png SHEET_OUT=каталог cargo test -p synaptix-flow sheet_live -- --ignored`.
#[cfg(test)]
mod sheet_live {
    #[test]
    #[ignore]
    fn sheet_live() {
        use crate::doc::{Flow, FlowNode};
        let (Ok(chars), Ok(out)) = (std::env::var("SHEET_CHARS"), std::env::var("SHEET_OUT")) else { return };
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("characters".to_string(), serde_json::Value::String(chars.replace("\\n", "\n")));
        if let Ok(l) = std::env::var("SHEET_LOC") {
            fields.insert("location".to_string(), serde_json::Value::String(l));
        }
        let flow = Flow { nodes: vec![FlowNode { id: 1, kind: "ref_sheet".into(), fields, enabled: true, label: String::new() }], links: Vec::new(), outputs: vec![1] };
        let r = crate::run::run(&flow, std::path::Path::new(&out), &crate::Cancel::new(), &mut |_| {}).expect("прогон");
        eprintln!("{r:?}");
    }
}
