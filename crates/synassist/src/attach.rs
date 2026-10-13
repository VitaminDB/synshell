//! Вложения для модели: картинки и видео — медиа сообщения (модель со зрением
//! их видит, см. `synaptixd::Media`), аудио и звук видео — расшифровкой
//! (распознавание речи службы, кэш на диске), PDF — текстом, текстовые файлы —
//! содержимым. У каждого вложения — путь на диске, чтобы инструменты могли
//! его открыть.

use std::path::{Path, PathBuf};

use synaptixd::{Media, MediaKind};

const TEXT_LIMIT: usize = 16_000;
/// Сколько звука расшифровывать (секунд) — длинные записи обрезаются.
const AUDIO_LIMIT_SEC: u32 = 900;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Other,
}

pub fn kind(p: &Path) -> Kind {
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "heic" | "heif" | "ico" => Kind::Image,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpg" | "mpeg" | "wmv" | "flv" | "3gp" | "ogv" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wma" | "amr" => Kind::Audio,
        "pdf" => Kind::Pdf,
        _ if crate::agent::is_text_file(p) => Kind::Text,
        _ => Kind::Other,
    }
}

/// Текст и медиа вложений сообщения (текст — после реплики, медиа служба
/// ставит перед ней).
pub fn describe(files: &[PathBuf]) -> (String, Vec<Media>) {
    let mut s = String::new();
    let mut media = Vec::new();
    for f in files {
        let shown = f.display();
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        match kind(f) {
            Kind::Image => {
                media.push(Media { kind: MediaKind::Image, path: f.to_string_lossy().into_owned() });
                s.push_str(&format!("\n\nИзображение {shown} — показано выше (ты его видишь; путь — для инструментов)."));
            }
            Kind::Video => {
                media.push(Media { kind: MediaKind::Video, path: f.to_string_lossy().into_owned() });
                s.push_str(&format!("\n\nВидео {shown} — показано выше."));
                match transcript(f) {
                    Ok(t) if !t.trim().is_empty() => s.push_str(&format!(" Речь в нём:\n```\n{}\n```", crate::tools::truncate(&t, TEXT_LIMIT))),
                    Ok(_) => s.push_str(" Речи в звуке нет."),
                    Err(_) => {}
                }
            }
            Kind::Audio => match transcript(f) {
                Ok(t) if !t.trim().is_empty() => s.push_str(&format!("\n\nАудио {shown}, расшифровка:\n```\n{}\n```", crate::tools::truncate(&t, TEXT_LIMIT))),
                Ok(_) => s.push_str(&format!("\n\nАудио {shown} — речи не распознано.")),
                Err(e) => s.push_str(&format!("\n\nАудио {shown} ({size} байт) — расшифровать не удалось: {e}.")),
            },
            Kind::Pdf => match pdf_text(f) {
                Ok(t) => s.push_str(&format!("\n\nДокумент {shown}:\n```\n{}\n```", crate::tools::truncate(&t, TEXT_LIMIT))),
                Err(e) => s.push_str(&format!("\n\nДокумент {shown} ({size} байт) — текст не извлечён: {e}.")),
            },
            Kind::Text => match std::fs::read_to_string(f) {
                Ok(c) => s.push_str(&format!("\n\nФайл {shown}:\n```\n{}\n```", crate::tools::truncate(&c, TEXT_LIMIT))),
                Err(e) => s.push_str(&format!("\n\nФайл {shown}: не прочитан ({e})")),
            },
            Kind::Other => s.push_str(&format!("\n\nФайл {shown} ({size} байт) — содержимое не текстовое; при необходимости посмотри его инструментом files или bash.")),
        }
    }
    (s, media)
}

/// Только медиа (результаты инструментов: снимки экрана, сгенерированные картинки).
pub fn media_of(files: &[PathBuf]) -> Vec<Media> {
    files
        .iter()
        .filter_map(|f| match kind(f) {
            Kind::Image => Some(Media { kind: MediaKind::Image, path: f.to_string_lossy().into_owned() }),
            Kind::Video => Some(Media { kind: MediaKind::Video, path: f.to_string_lossy().into_owned() }),
            _ => None,
        })
        .collect()
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("synassist/transcripts")
}

/// Ключ кэша: путь, размер и время изменения.
fn cache_key(f: &Path) -> String {
    let meta = std::fs::metadata(f).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let len = meta.map(|m| m.len()).unwrap_or(0);
    let s = format!("{}:{len}:{mtime}", f.display());
    format!("{:016x}", s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)))
}

/// Расшифровка речи файла (звук видео или аудио) — службой, с кэшем.
pub fn transcript(f: &Path) -> Result<String, String> {
    let cached = cache_dir().join(format!("{}.txt", cache_key(f)));
    if let Ok(t) = std::fs::read_to_string(&cached) {
        return Ok(t);
    }
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(f)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-t", &AUDIO_LIMIT_SEC.to_string(), "-f", "f32le", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        // Нет звуковой дорожки — не ошибка.
        if err.contains("does not contain any stream") || err.contains("Output file is empty") || err.contains("matches no streams") {
            return Ok(String::new());
        }
        return Err(format!("ffmpeg: {}", err.lines().last().unwrap_or("ошибка")));
    }
    let pcm: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    if pcm.len() < 1600 {
        return Ok(String::new());
    }
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let text = c.transcribe("", &pcm, 16000, None).map_err(|e| e.to_string())?.text;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cached, &text);
    Ok(text)
}

fn pdf_text(f: &Path) -> Result<String, String> {
    let out = std::process::Command::new("pdftotext").args(["-layout", "-enc", "UTF-8"]).arg(f).arg("-").output().map_err(|e| format!("pdftotext: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_media() {
        assert_eq!(kind(Path::new("/a/b.JPG")), Kind::Image);
        assert_eq!(kind(Path::new("/a/b.mkv")), Kind::Video);
        assert_eq!(kind(Path::new("/a/b.opus")), Kind::Audio);
        assert_eq!(kind(Path::new("/a/b.pdf")), Kind::Pdf);
        assert_eq!(kind(Path::new("/a/b.rs")), Kind::Text);
        let (text, media) = describe(&[PathBuf::from("/nope/x.png")]);
        assert_eq!(media.len(), 1);
        assert!(text.contains("Изображение /nope/x.png"));
    }
}
