//! Вложения: приём (фото, видео, документы, аудио — в «Загрузки/Курьер»,
//! голосовые — распознавание речи synaptix) и отправка файлов и голосовых
//! (синтез речи synaptix → Opus).

use std::path::{Path, PathBuf};

/// Папка для принятых файлов: `XDG_DOWNLOAD_DIR/Курьер` (иначе `~/Downloads/Курьер`).
pub fn inbox_dir() -> PathBuf {
    let home = syn_courier::expand_home("~");
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let mut dl = home.join("Downloads");
    if let Ok(t) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for line in t.lines() {
            if let Some(v) = line.strip_prefix("XDG_DOWNLOAD_DIR=") {
                dl = PathBuf::from(v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy()));
            }
        }
    }
    dl.join("Курьер")
}

/// Безопасное имя файла: без каталогов и управляющих символов.
pub fn safe_name(name: &str) -> String {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let s: String = base.chars().map(|c| if c == '/' || c == '\0' || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "файл".into()
    } else {
        s
    }
}

/// Расширение по MIME-типу.
pub fn ext_for(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/aac" => "m4a",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    }
}

/// MIME-тип по расширению (для отправки).
pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("ogg" | "opus" | "oga") => "audio/ogg; codecs=opus",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Картинку отправлять документом: с прозрачностью (фото пережимается в JPEG) или больше 10 МБ.
pub fn needs_document(path: &Path) -> bool {
    let mime = mime_for(path);
    if !mime.starts_with("image/") || mime == "image/gif" {
        return false;
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len > 10 * 1024 * 1024 {
        return true;
    }
    let head = {
        use std::io::Read;
        let mut b = vec![0u8; 64 * 1024];
        let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
        b.truncate(n);
        b
    };
    has_alpha(&head)
}

/// Есть ли прозрачность: PNG (тип цвета 4/6 или блок tRNS), WebP (флаг альфы VP8X или VP8L).
pub fn has_alpha(b: &[u8]) -> bool {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() > 25 {
        return matches!(b[25], 4 | 6) || b.windows(4).any(|w| w == b"tRNS");
    }
    if b.len() > 20 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8X" => b[20] & 0x10 != 0,
            b"VP8L" => true,
            _ => false,
        };
    }
    false
}

/// Сохранить принятые байты: `ГГГГ-ММ-ДД_ЧЧММСС_имя`, без перезаписи.
pub fn save(bytes: &[u8], name: &str) -> std::io::Result<PathBuf> {
    let dir = inbox_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = local_stamp();
    let name = safe_name(name);
    let mut p = dir.join(format!("{stamp}_{name}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stamp}_{n}_{name}"));
        n += 1;
    }
    std::fs::write(&p, bytes)?;
    Ok(p)
}

fn local_stamp() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d_%H%M%S").output().ok();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| crate::hub::now_ms().to_string())
}

/// Принятое вложение: сохранить; голосовое — распознать; Claude — путь и подпись.
pub async fn deliver(hub: &std::sync::Arc<crate::hub::Hub>, channel: &str, bytes: &[u8], name: &str, caption: &str, what: &str, voice: bool) {
    let path = match save(bytes, name) {
        Ok(p) => p,
        Err(e) => {
            hub.say(&format!("⚠️ Не сохранил {what}: {e}")).await;
            return;
        }
    };
    tracing::info!("{channel}: {what} → {}", path.display());
    hub.set_last_channel(channel);
    if voice {
        match transcribe(&path).await {
            // Расшифровку пользователю обратно не шлём: в WhatsApp и в своём Telegram
            // (kz.syn.telegram, локальный Whisper) она и так видна под голосовым.
            Ok(t) if !t.is_empty() => {
                hub.incoming(channel, format!("[голосовое] {t}"));
                return;
            }
            Ok(_) => hub.say("🎤 Речь не распознана — передаю файл Claude.").await,
            Err(e) => hub.say(&format!("🎤 Не распознал ({e:#}) — передаю файл Claude.")).await,
        }
    }
    let mut msg = format!("[Пользователь прислал {what}: {}]", path.display());
    if !caption.trim().is_empty() {
        msg.push('\n');
        msg.push_str(caption.trim());
    }
    hub.incoming(channel, msg);
}

/// Распознать речь (голосовое, Opus) через synaptix: ffmpeg → WAV 16 кГц → `synaptixctl asr`.
pub async fn transcribe(audio: &Path) -> anyhow::Result<String> {
    let wav = audio.with_extension("asr.wav");
    let st = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(audio)
        .args(["-ar", "16000", "-ac", "1"])
        .arg(&wav)
        .status()
        .await?;
    anyhow::ensure!(st.success(), "ffmpeg не перекодировал голосовое");
    let out = tokio::process::Command::new("synaptixctl").args(["asr", "-"]).arg(&wav).arg("ru").output().await;
    let _ = std::fs::remove_file(&wav);
    let out = out?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(asr_text(&String::from_utf8_lossy(&out.stdout)))
}

/// Вывод `synaptixctl asr`: текст, последняя строка — время «[1.2 с]».
pub fn asr_text(out: &str) -> String {
    out.lines().filter(|l| !(l.trim_start().starts_with('[') && l.trim_end().ends_with("с]"))).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Синтез речи в голосовое WhatsApp (Opus в OGG): `synaptixctl say` → ffmpeg.
pub async fn synthesize(text: &str) -> anyhow::Result<(PathBuf, u32)> {
    let dir = syn_courier::data_dir().join("voice");
    std::fs::create_dir_all(&dir)?;
    let stamp = crate::hub::now_ms();
    let wav = dir.join(format!("{stamp}.wav"));
    let ogg = dir.join(format!("{stamp}.ogg"));
    let out = tokio::process::Command::new("synaptixctl").env("LANG_CODE", "ru").args(["say", "-"]).arg(&wav).arg(text).output().await?;
    anyhow::ensure!(out.status.success() && wav.exists(), "синтез речи: {}", String::from_utf8_lossy(&out.stderr).trim());
    let st = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&wav)
        .args(["-c:a", "libopus", "-b:a", "32k", "-ac", "1", "-ar", "48000", "-application", "voip"])
        .arg(&ogg)
        .status()
        .await?;
    let _ = std::fs::remove_file(&wav);
    anyhow::ensure!(st.success(), "ffmpeg не собрал голосовое");
    Ok((ogg.clone(), duration_secs(&ogg).await))
}

/// Длительность медиа в секундах (ffprobe), 0 — неизвестно.
pub async fn duration_secs(p: &Path) -> u32 {
    let out = tokio::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(p).output().await;
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()).map(|d| d.round() as u32).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_detection() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x10\0\0\0\x10\x08".to_vec();
        png.push(6); // RGBA
        png.extend_from_slice(&[0, 0, 0]);
        assert!(has_alpha(&png));
        png[25] = 2; // RGB
        assert!(!has_alpha(&png));
        assert!(!has_alpha(b"\xff\xd8\xff\xe0 jpeg"));
    }

    #[test]
    fn names_and_types() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name(""), "файл");
        assert_eq!(ext_for("audio/ogg; codecs=opus"), "ogg");
        assert_eq!(mime_for(Path::new("/x/Снимок.PNG")), "image/png");
        assert_eq!(asr_text("Привет, как дела\n[0.8 с]\n"), "Привет, как дела");
    }
}
