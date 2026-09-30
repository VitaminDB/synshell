//! Геометрия обоев: какую область картинки показать на экране.
//!
//! Кадр задаётся так же, как в редакторе обоев Android и iOS: масштаб
//! относительно «заполнить» (`zoom`, 1 — картинка ровно закрывает экран) и
//! точка картинки в центре кадра (`center`, доли). Области возвращаются в
//! долях картинки `[x, y, w, h]` — от разрешения декодированной картинки
//! они не зависят.
//!
//! Панорама: картинка кадрируется под «холст» шириной
//! `1 + (столов − 1) × сдвиг` экранов, а на столе `pos` видно окно шириной в
//! экран, сдвинутое на `pos × сдвиг` экрана. `pos` дробный — фон едет вслед
//! за пальцем, листающим столы.

/// Область картинки размером `img` (px), которую надо показать в кадре с
/// пропорциями `view`, чтобы кадр был закрыт целиком.
pub fn crop_uv(img: (f32, f32), view: (f32, f32), zoom: f32, center: [f32; 2]) -> [f32; 4] {
    let ((iw, ih), (vw, vh)) = (img, view);
    if iw <= 0.0 || ih <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let zoom = if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
    let s = (vw / iw).max(vh / ih) * zoom;
    let w = (vw / s / iw).min(1.0);
    let h = (vh / s / ih).min(1.0);
    let cx = if center[0].is_finite() { center[0] } else { 0.5 };
    let cy = if center[1].is_finite() { center[1] } else { 0.5 };
    [(cx - w * 0.5).clamp(0.0, 1.0 - w), (cy - h * 0.5).clamp(0.0, 1.0 - h), w, h]
}

/// Ширина холста панорамы в экранах.
pub fn panorama_span(workspaces: u32, shift: f32) -> f32 {
    1.0 + workspaces.max(1).saturating_sub(1) as f32 * clamp_shift(shift)
}

pub fn clamp_shift(shift: f32) -> f32 {
    if shift.is_finite() {
        shift.clamp(0.05, 1.0)
    } else {
        0.5
    }
}

/// Пропорции холста панорамы для экрана `view`.
pub fn panorama_canvas(view: (f32, f32), workspaces: u32, shift: f32) -> (f32, f32) {
    (view.0 * panorama_span(workspaces, shift), view.1)
}

/// Окно экрана на холсте `canvas` (область картинки под весь холст) на
/// столе `pos` (с 0, дробный во время листания).
pub fn panorama_window(canvas: [f32; 4], workspaces: u32, shift: f32, pos: f32) -> [f32; 4] {
    let n = workspaces.max(1);
    let span = panorama_span(n, shift);
    let w = canvas[2] / span;
    let pos = if pos.is_finite() { pos.clamp(0.0, (n - 1) as f32) } else { 0.0 };
    let x = canvas[0] + canvas[2] * (pos * clamp_shift(shift) / span);
    [x, canvas[1], w, canvas[3]]
}

/// Область картинки для экрана `view` на столе `pos`: обычный кадр или
/// окно панорамы.
pub fn screen_uv(img: (f32, f32), view: (f32, f32), zoom: f32, center: [f32; 2], panorama: Option<(u32, f32, f32)>) -> [f32; 4] {
    match panorama {
        Some((n, shift, pos)) => {
            let canvas = crop_uv(img, panorama_canvas(view, n, shift), zoom, center);
            panorama_window(canvas, n, shift, pos)
        }
        None => crop_uv(img, view, zoom, center),
    }
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif") || is_heif_ext(ext)
}

/// HEIC/HEIF/AVIF (фото с телефона): крейт `image` их не читает — для показа
/// они преобразуются в JPEG ([`displayable`], [`prepare`]).
fn is_heif_ext(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "heic" | "heif" | "hif" | "avif")
}

fn needs_convert(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(is_heif_ext)
}

/// JPEG-копия картинки в кэше: `~/.cache/synshell/wallpapers/<хэш пути,
/// размера и времени изменения>.jpg` — новая после правки файла.
fn converted_path(p: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(p).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    m.len().hash(&mut h);
    m.mtime().hash(&mut h);
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".cache"));
    Some(cache.join("synshell/wallpapers").join(format!("{:016x}.jpg", h.finish())))
}

/// Файл, который можно показать сразу: сама картинка или её готовая
/// JPEG-копия (HEIC/AVIF). `None` — копии ещё нет: [`prepare`].
pub fn displayable(p: &std::path::Path) -> Option<std::path::PathBuf> {
    if !needs_convert(p) {
        return Some(p.to_path_buf());
    }
    converted_path(p).filter(|c| c.is_file())
}

/// Преобразовать HEIC/AVIF в JPEG-копию в фоне (`heif-convert`, иначе
/// `magick`); `done` — по окончании (и при неудаче), из фонового потока.
/// Повторный вызов для того же файла, пока идёт преобразование, — no-op.
pub fn prepare(p: &std::path::Path, done: impl FnOnce() + Send + 'static) {
    static BUSY: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
    let Some(out) = converted_path(p) else { return };
    {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains(&out) {
            return;
        }
        busy.push(out.clone());
    }
    let src = p.to_path_buf();
    std::thread::spawn(move || {
        if let Err(e) = convert(&src, &out) {
            tracing::warn!("обои: не преобразовать {}: {e}", src.display());
        }
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).retain(|b| b != &out);
        done();
    });
}

fn convert(src: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let dir = out.parent().ok_or("нет каталога кэша")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Во временный файл и переименованием — показ не увидит недописанный.
    let stem = format!(".{}-{:?}.tmp", std::process::id(), std::thread::current().id()).replace(['(', ')'], "");
    let tmp = dir.join(format!("{stem}.jpg"));
    // heif-convert пишет рядом вспомогательные картинки (глубина, альфа):
    // `<имя>-urn:….jpg` — убрать.
    let cleanup = || {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(&stem) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    };
    let tools: [(&str, Vec<std::ffi::OsString>); 2] = [
        ("heif-convert", vec!["-q".into(), "92".into(), src.into(), tmp.clone().into()]),
        ("magick", vec![src.into(), "-auto-orient".into(), "-quality".into(), "92".into(), tmp.clone().into()]),
    ];
    for (tool, args) in tools {
        if !crate::xdg::which(tool) {
            continue;
        }
        let ok = Command::new(tool).args(&args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if ok && tmp.is_file() {
            let r = std::fs::rename(&tmp, out).map_err(|e| e.to_string());
            cleanup();
            return r;
        }
        cleanup();
    }
    Err("нужен пакет libheif (heif-convert) или imagemagick".into())
}

/// Картинка ли это (по расширению) — годится ли в обои.
pub fn is_image(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(is_image_ext)
}

/// Картинки каталога по имени (без скрытых).
pub fn list_images(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| is_image(p) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

/// Картинка для показа: файл, либо очередная (`slot`) из каталога.
pub fn pick(path: &str, slot: u64) -> Option<std::path::PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    let p = crate::paths::expand_tilde(path.trim());
    if p.is_dir() {
        let files = list_images(&p);
        if files.is_empty() {
            return None;
        }
        return files.get((slot as usize) % files.len()).cloned();
    }
    p.exists().then_some(p)
}

/// Каталоги с обоями, которые стоит предложить: системные и свои, только
/// существующие.
pub fn wallpaper_dirs() -> Vec<std::path::PathBuf> {
    let home = crate::paths::expand_tilde("~");
    let mut out = Vec::new();
    let mut push = |p: std::path::PathBuf| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    push(xdg_pictures_dir(&home));
    push(home.join("Pictures/Wallpapers"));
    push(home.join(".local/share/wallpapers"));
    push(home.join(".local/share/backgrounds"));
    push("/usr/share/backgrounds".into());
    push("/usr/share/wallpapers".into());
    out
}

/// `XDG_PICTURES_DIR` из `user-dirs.dirs`, иначе `~/Pictures`.
pub fn xdg_pictures_dir(home: &std::path::Path) -> std::path::PathBuf {
    if let Ok(s) = std::fs::read_to_string(home.join(".config/user-dirs.dirs")) {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("XDG_PICTURES_DIR=") {
                return v.trim_matches('"').replace("$HOME", &home.to_string_lossy()).into();
            }
        }
    }
    home.join("Pictures")
}

/// Картинки для галереи из каталога: сами картинки и картинки подкаталогов
/// первого уровня (темы обоев KDE и GNOME лежат по папкам:
/// `/usr/share/wallpapers/Next/contents/images/*.png` — там берётся самая
/// большая).
pub fn gallery(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = list_images(dir);
    let mut subdirs: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    subdirs.sort();
    for d in subdirs {
        if d.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        // Пакет обоев KDE: contents/images/<ширина>x<высота>.<ext>.
        let kde = d.join("contents/images");
        if kde.is_dir() {
            let biggest = list_images(&kde).into_iter().max_by_key(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.split_once('x'))
                    .and_then(|(w, h)| Some(w.parse::<u64>().ok()? * h.parse::<u64>().ok()?))
                    .unwrap_or(0)
            });
            out.extend(biggest);
            continue;
        }
        out.extend(list_images(&d).into_iter().take(64));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn cover_crops_sides_of_wide_image() {
        // 2:1 картинка на квадратном экране: видна середина по ширине.
        let uv = crop_uv((200.0, 100.0), (50.0, 50.0), 1.0, [0.5, 0.5]);
        assert!(close(uv, [0.25, 0.0, 0.5, 1.0]), "{uv:?}");
    }

    #[test]
    fn zoom_and_center_clamped_inside() {
        let uv = crop_uv((100.0, 100.0), (10.0, 10.0), 2.0, [0.0, 1.0]);
        assert!(close(uv, [0.0, 0.5, 0.5, 0.5]), "{uv:?}");
        // Меньше «заполнить» не бывает.
        assert!(close(crop_uv((100.0, 100.0), (10.0, 10.0), 0.3, [0.5, 0.5]), [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn panorama_windows_cover_canvas() {
        // 4 стола, сдвиг на пол-экрана: холст 2.5 экрана.
        let canvas = [0.0, 0.0, 1.0, 1.0];
        let first = panorama_window(canvas, 4, 0.5, 0.0);
        let last = panorama_window(canvas, 4, 0.5, 3.0);
        assert!(close(first, [0.0, 0.0, 0.4, 1.0]), "{first:?}");
        assert!(close(last, [0.6, 0.0, 0.4, 1.0]), "{last:?}");
        // Листание за край не уводит окно с холста.
        assert!(close(panorama_window(canvas, 4, 0.5, -0.3), first));
        // Один стол — окно на весь холст.
        assert!(close(panorama_window(canvas, 1, 0.5, 0.0), canvas));
    }

    #[test]
    fn screen_uv_panorama_uses_wide_canvas() {
        // Картинка 3:1, экран 1:1, 3 стола со сдвигом 1: холст 3:1 — вся картинка.
        let uv = screen_uv((300.0, 100.0), (100.0, 100.0), 1.0, [0.5, 0.5], Some((3, 1.0, 1.0)));
        assert!(close(uv, [1.0 / 3.0, 0.0, 1.0 / 3.0, 1.0]), "{uv:?}");
    }

    #[test]
    fn gallery_takes_biggest_kde_image() {
        let d = std::env::temp_dir().join(format!("synshell-wall-{}", std::process::id()));
        let imgs = d.join("Next/contents/images");
        std::fs::create_dir_all(&imgs).unwrap();
        for n in ["1920x1080.png", "3840x2160.png", "800x600.png"] {
            std::fs::write(imgs.join(n), b"").unwrap();
        }
        std::fs::write(d.join("a.jpg"), b"").unwrap();
        std::fs::write(d.join(".hidden.jpg"), b"").unwrap();
        let g = gallery(&d);
        let names: Vec<_> = g.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["a.jpg", "3840x2160.png"]);
        std::fs::remove_dir_all(&d).ok();
    }
}
