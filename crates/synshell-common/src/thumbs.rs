//! Миниатюры по спецификации freedesktop.org (Thumbnail Managing Standard).
//!
//! Кэш общий с другими программами: `~/.cache/thumbnails/{normal,large,
//! x-large}/<md5(uri)>.png` с тегами `Thumb::URI` и `Thumb::MTime`; неудачи —
//! в `fail/synfiles/`. Генерация — в пуле фоновых потоков, последние
//! запрошенные (видимые сейчас) — первыми; готовность сообщается колбэком.

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::xdg;

/// Размер кэша: 128, 256 или 512.
pub fn bucket(px: u32) -> (u32, &'static str) {
    if px <= 128 {
        (128, "normal")
    } else if px <= 256 {
        (256, "large")
    } else {
        (512, "x-large")
    }
}

fn cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("thumbnails")
}

fn hash(uri: &str) -> String {
    format!("{:x}", md5::compute(uri.as_bytes()))
}

/// Можно ли вообще сделать миниатюру.
pub fn supported(mime: &str) -> bool {
    (mime.starts_with("image/") && mime != "image/svg+xml" && mime != "image/x-xcf" && mime != "image/vnd.adobe.photoshop")
        || (mime.starts_with("video/") && video_tool().is_some())
}

#[derive(Clone, Copy)]
enum VideoTool {
    FfmpegThumbnailer,
    Ffmpeg,
}

fn video_tool() -> Option<VideoTool> {
    static T: OnceLock<Option<VideoTool>> = OnceLock::new();
    *T.get_or_init(|| {
        if xdg::which("ffmpegthumbnailer") {
            Some(VideoTool::FfmpegThumbnailer)
        } else if xdg::which("ffmpeg") {
            Some(VideoTool::Ffmpeg)
        } else {
            None
        }
    })
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    size: u32,
    mtime: i64,
}

#[derive(Default)]
struct Pool {
    queue: VecDeque<(Key, String)>,
    queued: HashSet<Key>,
    ready: HashMap<Key, Option<PathBuf>>,
    workers: usize,
}

struct Shared {
    pool: Mutex<Pool>,
    cv: Condvar,
    notify: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    max_bytes: Mutex<u64>,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| Shared {
        pool: Mutex::new(Pool::default()),
        cv: Condvar::new(),
        notify: Mutex::new(None),
        max_bytes: Mutex::new(64 << 20),
    })
}

/// Колбэк «появились новые миниатюры» (из рабочих потоков).
pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    *shared().notify.lock().unwrap() = Some(Arc::new(f));
}

pub fn set_max_mb(mb: u64) {
    *shared().max_bytes.lock().unwrap() = mb << 20;
}

/// Готовая миниатюра или `None` (тогда она поставлена в очередь).
pub fn get(path: &Path, mime: &str, mtime: i64, size_px: u32) -> Option<PathBuf> {
    if !supported(mime) {
        return None;
    }
    let (size, _) = bucket(size_px);
    let key = Key { path: path.to_path_buf(), size, mtime };
    let s = shared();
    let mut pool = s.pool.lock().unwrap();
    if let Some(r) = pool.ready.get(&key) {
        return r.clone();
    }
    if pool.queued.insert(key.clone()) {
        pool.queue.push_back((key, mime.to_string()));
        let want = std::thread::available_parallelism().map(|n| n.get().clamp(2, 4)).unwrap_or(2);
        if pool.workers < want {
            pool.workers += 1;
            std::thread::Builder::new().name("thumbs".into()).spawn(worker).ok();
        }
        s.cv.notify_one();
    } else if let Some(pos) = pool.queue.iter().position(|(k, _)| *k == key) {
        // Снова запросили — значит, видно сейчас: в конец (берутся с конца).
        let item = pool.queue.remove(pos).unwrap();
        pool.queue.push_back(item);
    }
    None
}

/// Сколько миниатюр ещё в работе.
pub fn pending() -> usize {
    shared().pool.lock().unwrap().queued.len()
}

/// Забыть очередь (ушли из папки) — не делать ненужное.
pub fn cancel_pending() {
    let mut pool = shared().pool.lock().unwrap();
    let dropped: Vec<Key> = pool.queue.drain(..).map(|(k, _)| k).collect();
    for k in dropped {
        pool.queued.remove(&k);
    }
}

fn worker() {
    let s = shared();
    loop {
        let (key, mime) = {
            let mut pool = s.pool.lock().unwrap();
            loop {
                if let Some(item) = pool.queue.pop_back() {
                    break item;
                }
                let (g, timeout) = s.cv.wait_timeout(pool, std::time::Duration::from_secs(30)).unwrap();
                pool = g;
                if timeout.timed_out() && pool.queue.is_empty() {
                    pool.workers -= 1;
                    return;
                }
            }
        };
        let result = produce(&key, &mime);
        {
            let mut pool = s.pool.lock().unwrap();
            pool.queued.remove(&key);
            pool.ready.insert(key, result);
        }
        let f = s.notify.lock().unwrap().clone();
        if let Some(f) = f {
            f();
        }
    }
}

fn produce(key: &Key, mime: &str) -> Option<PathBuf> {
    let uri = xdg::file_uri(&key.path);
    let h = hash(&uri);
    let (size, dir) = bucket(key.size);
    let out = cache_root().join(dir).join(format!("{h}.png"));
    if valid(&out, key.mtime) {
        return Some(out);
    }
    let fail = cache_root().join("fail/synfiles").join(format!("{h}.png"));
    if valid(&fail, key.mtime) {
        return None;
    }
    let meta = std::fs::metadata(&key.path).ok()?;
    let img = if mime.starts_with("video/") {
        video_frame(&key.path, size)
    } else if meta.len() > *shared().max_bytes.lock().unwrap() {
        return None;
    } else {
        decode(&key.path).or_else(|| heif_frame(&key.path, mime))
    };
    let Some(img) = img else {
        let _ = write_png(&fail, &image::DynamicImage::new_rgba8(1, 1), &uri, key.mtime, meta.len());
        return None;
    };
    let thumb = if img.width() > size || img.height() > size { img.thumbnail(size, size) } else { img };
    write_png(&out, &thumb, &uri, key.mtime, meta.size()).ok()?;
    Some(out)
}

fn decode(path: &Path) -> Option<image::DynamicImage> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Some(img)
}

/// HEIC/HEIF/AVIF: крейт `image` их не читает — через `heif-convert`.
fn heif_frame(path: &Path, mime: &str) -> Option<image::DynamicImage> {
    if !matches!(mime, "image/heif" | "image/heic" | "image/avif" | "image/heif-sequence" | "image/heic-sequence") || !xdg::which("heif-convert") {
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("synshell-thumb-{}-{}.jpg", std::process::id(), hash(&path.to_string_lossy())));
    let ok = std::process::Command::new("heif-convert")
        .args(["-q", "85"])
        .arg(path)
        .arg(&tmp)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let img = if ok { image::open(&tmp).ok() } else { None };
    let _ = std::fs::remove_file(&tmp);
    img
}

fn video_frame(path: &Path, size: u32) -> Option<image::DynamicImage> {
    let tmp = std::env::temp_dir().join(format!("synshell-thumb-{}-{}.png", std::process::id(), hash(&path.to_string_lossy())));
    let ok = match video_tool()? {
        VideoTool::FfmpegThumbnailer => std::process::Command::new("ffmpegthumbnailer")
            .arg("-i")
            .arg(path)
            .arg("-o")
            .arg(&tmp)
            .args(["-s", &size.to_string(), "-c", "png", "-t", "10%"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        VideoTool::Ffmpeg => std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-ss", "3", "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf", &format!("scale={size}:{size}:force_original_aspect_ratio=decrease")])
            .arg(&tmp)
            .stdin(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
    };
    let img = if ok { image::open(&tmp).ok() } else { None };
    let _ = std::fs::remove_file(&tmp);
    img
}

/// Миниатюра существует и снята с той же версии файла.
fn valid(thumb: &Path, mtime: i64) -> bool {
    let Ok(f) = std::fs::File::open(thumb) else { return false };
    let dec = png::Decoder::new(std::io::BufReader::new(f));
    let Ok(reader) = dec.read_info() else { return false };
    let info = reader.info();
    info.uncompressed_latin1_text.iter().any(|t| t.keyword == "Thumb::MTime" && t.text.trim() == mtime.to_string())
}

fn write_png(out: &Path, img: &image::DynamicImage, uri: &str, mtime: i64, size: u64) -> std::io::Result<()> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
        // Спецификация: каталоги кэша — только владельцу.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let rgba = img.to_rgba8();
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), rgba.width(), rgba.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let _ = enc.add_text_chunk("Thumb::URI".into(), uri.into());
        let _ = enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string());
        let _ = enc.add_text_chunk("Thumb::Size".into(), size.to_string());
        let _ = enc.add_text_chunk("Software".into(), "synshell".into());
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&rgba).map_err(std::io::Error::other)?;
    }
    std::fs::rename(&tmp, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_validates() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CACHE_HOME", d.path().join("cache"));
        let src = d.path().join("pic.png");
        image::DynamicImage::new_rgb8(600, 300).save(&src).unwrap();
        let m = std::fs::metadata(&src).unwrap();
        let key = Key { path: src.clone(), size: 256, mtime: m.mtime() };
        let t = produce(&key, "image/png").expect("миниатюра");
        assert!(t.starts_with(d.path().join("cache/thumbnails/large")));
        let img = image::open(&t).unwrap();
        assert_eq!((img.width(), img.height()), (256, 128));
        assert!(valid(&t, m.mtime()));
        assert!(!valid(&t, m.mtime() + 1));
    }
}
