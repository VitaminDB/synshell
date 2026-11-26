//! Генерация картинок, видео и музыки — общая для запросов `image`,
//! `video`, `music` и нод графа (`op flow`): модель и умолчания из
//! Параметров, явные значения запроса сильнее; перед генерацией видеокарта
//! освобождается от других моделей, после — пулы возвращаются драйверу.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::{Error, ImageParams, ModelKind, Params, VideoParams};

use crate::server::manager::{self, Manager};

pub fn random_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) % 1_000_000_007
}

/// Сид: запрос, иначе Параметры модели (≥ 0), иначе случайный.
fn seed_of(explicit: Option<u64>, mp: &Params) -> u64 {
    explicit.or_else(|| mp.i64("seed").filter(|s| *s >= 0).map(|s| s as u64)).unwrap_or_else(random_seed)
}

fn negative_of(explicit: Option<String>, mp: &Params) -> Option<String> {
    explicit.or_else(|| mp.text("negative_prompt").filter(|t| !t.trim().is_empty()).map(String::from))
}

pub struct ImageResult {
    pub png: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub seed: u64,
    pub label: String,
}

/// Картинка; `progress(шаг, всего) -> продолжать`.
pub fn image(mgr: &Arc<Manager>, model: &str, prompt: String, p: ImageParams, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<ImageResult, Error> {
    let model = mgr.resolve_id(model, ModelKind::Image)?;
    let mp = mgr.params(&model, &Params::new());
    let seed = seed_of(p.seed, &mp);
    let req = synaptix_gen::image::ImageRequest {
        prompt,
        negative: negative_of(p.negative, &mp),
        width: p.width.or(mp.u32("width").map(|v| v as usize)),
        height: p.height.or(mp.u32("height").map(|v| v as usize)),
        steps: p.steps.or(mp.u32("steps").map(|v| v as usize)),
        guidance: p.guidance.or(mp.f32("guidance")),
        seed,
        images: p.images.into_iter().map(PathBuf::from).collect(),
        init: p.init.map(PathBuf::from),
        strength: p.strength,
        quant: None,
    };
    mgr.clear_device(&model, crate::DeviceKind::Cuda, "генерация картинки");
    // `run` может повторить вызов — ход через RefCell.
    let progress = std::cell::RefCell::new(progress);
    let r = mgr.run(&model, |m| m.image(&req, &mut |i, n| (progress.borrow_mut())(i, n)));
    // Этапы пайплайна оставляют гигабайты в CUDA-пулах — вернуть драйверу.
    manager::release_vram(Some(crate::DeviceKind::Cuda));
    let (png, out) = r?;
    Ok(ImageResult { png, width: out.width, height: out.height, seed, label: out.label })
}

pub struct VideoResult {
    pub mp4: Vec<u8>,
    pub out: synaptix_gen::video::VideoOutput,
    pub seed: u64,
}

/// LoRA для видео: у MiniMax-H3 по умолчанию Turbo на 8 шагов из библиотеки
/// (`lora/h3/*turbo_8step*.safetensors`, свежая версия): шаги и CFG 1 берёт
/// synaptix-gen по имени файла. «off»/«none»/пусто — без LoRA; иначе путь
/// или имя файла в `lora/h3`.
pub fn video_lora(mgr: &Arc<Manager>, model: &str, want: Option<&str>) -> Result<Option<PathBuf>, Error> {
    let h3 = model.contains("minimax-h3") || model.contains("h3-");
    let dirs: Vec<PathBuf> = mgr.library_paths().into_iter().map(|l| l.join(super::lora::LORA_DIR).join("h3")).collect();
    match want.map(str::trim) {
        Some("off" | "none" | "no" | "нет" | "") => Ok(None),
        Some(w) => {
            let p = PathBuf::from(w);
            if p.is_file() {
                return Ok(Some(p));
            }
            dirs.iter().map(|d| d.join(w)).find(|p| p.is_file()).map(Some).ok_or_else(|| Error::new(crate::ErrorCode::Failed, format!("LoRA «{w}» не найдена (путь или файл в lora/h3)")))
        }
        None if h3 => Ok(default_turbo(&dirs)),
        None => Ok(None),
    }
}

/// Turbo 8 шагов: последняя версия по имени файла.
fn default_turbo(dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.ends_with(".safetensors") && n.contains("turbo") && n.contains("8step")
        })
        .collect();
    found.sort();
    found.pop()
}

/// Видео; `cancel` — прервать денойз.
pub fn video(mgr: &Arc<Manager>, model: &str, prompt: String, p: VideoParams, cancel: &AtomicBool, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<VideoResult, Error> {
    let model = mgr.resolve_id(model, ModelKind::Video)?;
    let mp = mgr.params(&model, &Params::new());
    let seed = seed_of(p.seed, &mp);
    let lora = video_lora(mgr, &model, p.lora.as_deref())?;
    // дистиллированная LoRA (Turbo): шаги по ней, CFG 1 и без негатива —
    // шаги/CFG/негатив из настроек модели (под 20 шагов) не подставляются
    let turbo = lora.as_deref().and_then(synaptix_gen::video::lora_steps).is_some();
    let req = synaptix_gen::video::VideoRequest {
        prompt,
        negative: if turbo { p.negative } else { negative_of(p.negative, &mp) },
        width: p.width.or(mp.u32("width").map(|v| v as usize)),
        height: p.height.or(mp.u32("height").map(|v| v as usize)),
        seconds: p.seconds.or(mp.f32("duration").map(|v| v as f64)),
        steps: if turbo { p.steps } else { p.steps.or(mp.u32("steps").map(|v| v as usize)) },
        guidance: if turbo { p.guidance } else { p.guidance.or(mp.f32("guidance")) },
        seed,
        first_frame: p.first_frame.map(PathBuf::from),
        last_frame: p.last_frame.map(PathBuf::from),
        refs: p.refs.into_iter().map(PathBuf::from).collect(),
        quant: None,
        audio: p.audio.map(PathBuf::from),
        source_video: p.source_video.map(PathBuf::from),
        retake: p.retake,
        extend: p.extend,
        control_video: p.control_video.map(PathBuf::from),
        control: p.control,
        two_stage: p.two_stage,
        strength: p.strength,
        context: p.context.map(PathBuf::from),
        context_frames: p.context_frames,
        tail_out: p.tail_out.map(PathBuf::from),
        lora,
        lora_strength: p.lora_strength,
        model_id: Some(model.clone()),
    };
    mgr.clear_device(&model, crate::DeviceKind::Cuda, "генерация видео");
    let r = mgr.run(&model, |m| m.video(&req, cancel, progress));
    manager::release_vram(Some(crate::DeviceKind::Cuda));
    let (mp4, out) = r?;
    Ok(VideoResult { mp4, out, seed })
}

pub struct MusicResult {
    pub wav: Vec<u8>,
    pub out: synaptix_gen::music::MusicOutput,
    pub seed: u64,
}

pub fn music(
    mgr: &Arc<Manager>,
    model: &str,
    prompt: String,
    lyrics: String,
    seconds: Option<u32>,
    seed: Option<u64>,
    steps: Option<usize>,
    cancel: Arc<AtomicBool>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<MusicResult, Error> {
    let model = mgr.resolve_id(model, ModelKind::Music)?;
    let mp = mgr.params(&model, &Params::new());
    let seed = seed_of(seed, &mp);
    // Длительность не задана: песня с текстом — как обычная песня, 3 минуты
    // (модель может закончить раньше); инструментал — по умолчанию движка.
    let song = if lyrics.trim().is_empty() { 0 } else { 180 };
    let req = synaptix_gen::music::MusicRequest {
        prompt,
        lyrics,
        seconds: seconds.filter(|&s| s > 0).or(mp.f32("duration").map(|v| v.round() as u32)).unwrap_or(song),
        seed,
        steps: steps.or(mp.u32("steps").map(|v| v as usize)),
        guidance: mp.f32("guidance"),
        quant: None,
    };
    mgr.clear_device(&model, crate::DeviceKind::Cuda, "генерация музыки");
    let progress = std::cell::RefCell::new(progress);
    let r = mgr.run(&model, |m| m.music(&req, cancel.clone(), &mut |i, n| (progress.borrow_mut())(i, n)));
    manager::release_vram(Some(crate::DeviceKind::Cuda));
    let (wav, out) = r?;
    Ok(MusicResult { wav, out, seed })
}
