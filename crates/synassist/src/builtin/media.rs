use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use synaptixd::{ModelKind, Params};

use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct MediaTool;

pub fn media_dir() -> PathBuf {
    crate::chat::data_dir().join("media")
}

fn stamp() -> String {
    format!("{:x}", crate::chat::now())
}

pub fn write_wav(path: &Path, pcm: &[f32], rate: u32) -> std::io::Result<()> {
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, w)
}

fn speak(args: &Value) -> ToolResult {
    let text = need_str(args, "prompt")?;
    let mut params = Params::new();
    if let Some(v) = str_arg(args, "voice") {
        params = params.with("voice", v);
    }
    if let Some(l) = str_arg(args, "language") {
        params = params.with("language", l);
    }
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let sp = c.speak_with("", text, &params).map_err(|e| e.to_string())?;
    let path = media_dir().join(format!("speech-{}.wav", stamp()));
    write_wav(&path, sp.audio.f32s(), sp.sample_rate).map_err(|e| e.to_string())?;
    let secs = sp.audio.f32s().len() as f32 / sp.sample_rate.max(1) as f32;
    Ok(ToolOutput { text: format!("Озвучено: {:.1} с, файл {}", secs, path.display()), files: vec![path] })
}

fn generate(kind: ModelKind, what: &str) -> ToolResult {
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    match c.resolve(kind) {
        Ok(m) if !m.runnable => Err(format!(
            "модель для «{what}» установлена ({}), но служба нейросетей пока не умеет её исполнять — эта возможность появится позже. Скажи пользователю об этом.",
            m.title
        )),
        Ok(m) => Err(format!("генерация «{what}» через службу ещё не подключена (модель {})", m.title)),
        Err(_) => Err(format!("не установлена модель для «{what}» — её можно добавить в «Параметры → Нейросети → Хранилище моделей»")),
    }
}

impl Tool for MediaTool {
    fn name(&self) -> &'static str {
        "media"
    }
    fn group(&self) -> &'static str {
        "медиа"
    }
    fn description(&self) -> String {
        "Создание медиа системными нейросетями. action: speak — озвучить текст (prompt — текст, voice — описание голоса, language — код языка), image — картинка по описанию (prompt), video — видео (prompt), music — музыка или песня (prompt). Результат — файл, он покажется пользователю в чате.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["speak", "image", "video", "music"]},
                "prompt": {"type": "string"},
                "voice": {"type": "string"},
                "language": {"type": "string"}
            },
            "required": ["action", "prompt"]
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        let p: String = str_arg(args, "prompt").unwrap_or("").chars().take(60).collect();
        match str_arg(args, "action").unwrap_or("?") {
            "speak" => format!("Озвучить: {p}"),
            "image" => format!("Картинка: {p}"),
            "video" => format!("Видео: {p}"),
            "music" => format!("Музыка: {p}"),
            other => other.to_string(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        match need_str(args, "action")? {
            "speak" => speak(args),
            "image" => generate(ModelKind::Image, "картинки"),
            "video" => generate(ModelKind::Video, "видео"),
            "music" => generate(ModelKind::Music, "музыки"),
            other => Err(format!("неизвестное действие {other}")),
        }
    }
}
