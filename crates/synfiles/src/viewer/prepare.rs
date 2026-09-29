//! Подготовка картинки к показу — в фоне, результат в кэше процесса.
//!
//! - Что syngui декодирует сам (PNG, JPEG, GIF, BMP, ICO, WebP, SVG), идёт
//!   как есть; поворот из EXIF отдаётся виду четвертями и отражением, без
//!   перекодирования.
//! - Остальное (HEIC/HEIF/AVIF, TIFF, JPEG XL, RAW…) преобразуется в PNG в
//!   `~/.cache/synshell/viewer/`: TIFF — крейтом `image`, прочее —
//!   `heif-convert`, `magick` или `ffmpeg`, что найдётся.
//! - Фон сцены — копия картинки 96 px, размытая (как в synthos: один раз на
//!   процессоре, а не фильтром GPU на каждый кадр).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use synshell_common::xdg;

use super::Item;

/// Как показывать картинку.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    /// Файл, который декодирует syngui; `None` — не получилось.
    pub display: Option<PathBuf>,
    /// Размер `display` (до поворота); нули — неизвестен.
    pub natural: (u32, u32),
    /// Поворот из EXIF: четверти по часовой и отражение по горизонтали.
    pub exif_turns: i32,
    pub exif_flip: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Cache {
    ready: HashMap<(PathBuf, i64), Prepared>,
    blur: HashMap<(PathBuf, i64), Option<PathBuf>>,
    busy: HashSet<(PathBuf, i64, bool)>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(Default::default)
}

type Notify = Arc<dyn Fn() + Send + Sync>;

fn notifier() -> &'static Mutex<Option<Notify>> {
    static N: OnceLock<Mutex<Option<Notify>>> = OnceLock::new();
    N.get_or_init(Default::default)
}

pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    *notifier().lock().unwrap() = Some(Arc::new(f));
}

fn notify() {
    let f = notifier().lock().unwrap().clone();
    if let Some(f) = f {
        f();
    }
}

/// Идёт ли подготовка (снимок без окна ждёт её).
pub fn busy() -> bool {
    !cache().lock().unwrap().busy.is_empty() || crate::thumbs::pending() > 0
}

/// Готово ли; `None` — поставлено в работу, придёт уведомление.
pub fn get(it: &Item) -> Option<Prepared> {
    let key = (it.path.clone(), it.mtime);
    let mut c = cache().lock().unwrap();
    if let Some(p) = c.ready.get(&key) {
        return Some(p.clone());
    }
    if c.busy.insert((key.0.clone(), key.1, false)) {
        let it = it.clone();
        std::thread::Builder::new()
            .name("viewer-prepare".into())
            .spawn(move || {
                let p = prepare(&it);
                let mut c = cache().lock().unwrap();
                c.busy.remove(&(it.path.clone(), it.mtime, false));
                c.ready.insert((it.path.clone(), it.mtime), p);
                drop(c);
                notify();
            })
            .ok();
    }
    None
}

/// Размытый фон: готовый файл, `None` — делается или не выйдет.
pub fn blur(it: &Item, prepared: &Prepared) -> Option<PathBuf> {
    let key = (it.path.clone(), it.mtime);
    let mut c = cache().lock().unwrap();
    if let Some(b) = c.blur.get(&key) {
        return b.clone();
    }
    let src = prepared.display.clone()?;
    if it.mime == "image/svg+xml" {
        return None;
    }
    if c.busy.insert((key.0.clone(), key.1, true)) {
        let it = it.clone();
        let turns = prepared.exif_turns;
        let flip = prepared.exif_flip;
        std::thread::Builder::new()
            .name("viewer-blur".into())
            .spawn(move || {
                let out = make_blur(&it, &src, turns, flip);
                let mut c = cache().lock().unwrap();
                c.busy.remove(&(it.path.clone(), it.mtime, true));
                c.blur.insert((it.path.clone(), it.mtime), out);
                drop(c);
                notify();
            })
            .ok();
    }
    None
}

/// Форматы, которые syngui (крейт `image` с его набором + resvg) читает сам.
fn native(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/x-bmp" | "image/vnd.microsoft.icon" | "image/x-icon" | "image/webp" | "image/svg+xml"
    )
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("synshell/viewer")
}

/// Выбросить из кэша то, что не открывали 30 дней (в фоне, при запуске).
pub fn prune_cache() {
    std::thread::spawn(|| {
        let Ok(rd) = std::fs::read_dir(cache_dir()) else { return };
        let limit = std::time::Duration::from_secs(30 * 24 * 3600);
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            let used = m.accessed().or_else(|_| m.modified()).ok();
            if used.and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    });
}

fn cache_name(it: &Item, suffix: &str) -> PathBuf {
    let h = format!("{:x}", md5::compute(xdg::file_uri(&it.path).as_bytes()));
    cache_dir().join(format!("{h}-{}{suffix}", it.mtime))
}

fn prepare(it: &Item) -> Prepared {
    let mut p = Prepared::default();
    let native = native(&it.mime) || sniff_native(&it.path);
    let display = if native {
        Some(it.path.clone())
    } else {
        match convert(it) {
            Ok(out) => Some(out),
            Err(e) => {
                p.error = Some(e);
                None
            }
        }
    };
    if let Some(d) = &display {
        if it.mime != "image/svg+xml" {
            if let Ok(r) = image::ImageReader::open(d).and_then(|r| r.with_guessed_format()) {
                use image::ImageDecoder;
                if let Ok(mut dec) = r.into_decoder() {
                    p.natural = dec.dimensions();
                    if native {
                        if let Ok(o) = dec.orientation() {
                            (p.exif_turns, p.exif_flip) = orientation(o);
                        }
                    }
                }
            }
        }
    }
    p.display = display;
    p
}

/// Расширение соврало (JPEG под `.png` и т.п.) — судим по сигнатуре.
fn sniff_native(path: &Path) -> bool {
    let Ok(r) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) else { return false };
    matches!(
        r.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::Bmp | image::ImageFormat::Ico | image::ImageFormat::WebP)
    )
}

/// EXIF → (четверти по часовой, отражение по горизонтали после поворота) —
/// в том порядке, в каком их применяет `ImageViewport`.
fn orientation(o: image::metadata::Orientation) -> (i32, bool) {
    use image::metadata::Orientation::*;
    match o {
        NoTransforms => (0, false),
        Rotate90 => (1, false),
        Rotate180 => (2, false),
        Rotate270 => (3, false),
        FlipHorizontal => (0, true),
        FlipVertical => (2, true),
        Rotate90FlipH => (1, true),
        Rotate270FlipH => (3, true),
    }
}

fn convert(it: &Item) -> Result<PathBuf, String> {
    // HEIC/HEIF (фото с телефона) — в JPEG: прозрачности в них нет, а PNG
    // на 30–50 Мп кодируется секунды. Остальное — в PNG, без потерь.
    let photo = matches!(it.mime.as_str(), "image/heif" | "image/heic" | "image/heif-sequence" | "image/heic-sequence");
    let ext = if photo { "jpg" } else { "png" };
    let out = cache_name(it, &format!(".{ext}"));
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(cache_dir()).map_err(|e| e.to_string())?;
    let tmp = out.with_extension(format!("part.{ext}"));
    let _ = std::fs::remove_file(&tmp);
    let mut ok = false;
    // TIFF и то, что `image` вдруг прочтёт, — без внешних программ.
    if let Ok(img) = image::ImageReader::open(&it.path).and_then(|r| r.with_guessed_format()).map_err(|e| e.to_string()).and_then(|r| r.decode().map_err(|e| e.to_string())) {
        ok = if photo { img.to_rgb8().save_with_format(&tmp, image::ImageFormat::Jpeg).is_ok() } else { img.save_with_format(&tmp, image::ImageFormat::Png).is_ok() };
    }
    let heif = photo || it.mime == "image/avif";
    let tools: [(&str, Vec<std::ffi::OsString>); 3] = [
        ("heif-convert", vec!["-q".into(), "92".into(), it.path.clone().into(), tmp.clone().into()]),
        ("magick", vec![format!("{}[0]", it.path.display()).into(), "-quality".into(), "92".into(), format!("{ext}:{}", tmp.display()).into()]),
        ("ffmpeg", vec!["-v".into(), "quiet".into(), "-y".into(), "-i".into(), it.path.clone().into(), "-frames:v".into(), "1".into(), tmp.clone().into()]),
    ];
    for (tool, args) in tools {
        if ok {
            break;
        }
        if tool == "heif-convert" && !heif {
            continue;
        }
        if !xdg::which(tool) {
            continue;
        }
        ok = std::process::Command::new(tool)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && tmp.exists();
    }
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(if heif && !xdg::which("heif-convert") && !xdg::which("magick") {
            "Для HEIC/AVIF нужен пакет libheif или imagemagick".into()
        } else {
            "Формат не поддерживается".into()
        });
    }
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

const BLUR_SIDE: u32 = 96;
const BLUR_SIGMA: f32 = 3.0;

fn make_blur(it: &Item, src: &Path, turns: i32, flip: bool) -> Option<PathBuf> {
    let out = cache_name(it, ".blur.png");
    if out.exists() {
        return Some(out);
    }
    let img = image::ImageReader::open(src).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let mut small = img.thumbnail(BLUR_SIDE, BLUR_SIDE);
    small = match turns.rem_euclid(4) {
        1 => small.rotate90(),
        2 => small.rotate180(),
        3 => small.rotate270(),
        _ => small,
    };
    if flip {
        small = small.fliph();
    }
    let small = small.blur(BLUR_SIGMA).to_rgb8();
    std::fs::create_dir_all(cache_dir()).ok()?;
    // Через временный файл: недописанный PNG показался бы готовым фоном.
    let tmp = out.with_extension("part.png");
    small.save_with_format(&tmp, image::ImageFormat::Png).ok()?;
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_matches_viewport_order() {
        use image::metadata::Orientation::*;
        assert_eq!(orientation(Rotate90), (1, false));
        assert_eq!(orientation(Rotate270FlipH), (3, true));
        assert_eq!(orientation(FlipVertical), (2, true));
    }

    #[test]
    fn tiff_is_converted_and_blurred() {
        let dir = std::env::temp_dir().join(format!("sd-viewer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.tiff");
        image::RgbImage::from_fn(300, 100, |x, _| image::Rgb([(x % 256) as u8, 40, 200])).save(&src).unwrap();
        let it = Item { path: src.clone(), name: "a.tiff".into(), mime: "image/tiff".into(), size: 1, mtime: 7 };
        let p = prepare(&it);
        let d = p.display.clone().expect("преобразовано");
        assert_ne!(d, src);
        assert_eq!(p.natural, (300, 100));
        let b = make_blur(&it, &d, 1, false).unwrap();
        let img = image::open(&b).unwrap();
        assert_eq!((img.width(), img.height()), (32, 96), "повёрнут вместе с картинкой");
        // Кэш общий с настоящим просмотрщиком (переменные окружения в
        // параллельных тестах не подменить) — убираем за собой.
        let _ = std::fs::remove_file(&d);
        let _ = std::fs::remove_file(&b);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
