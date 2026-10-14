//! Стадии моделей графа: модель → условие → латент → картинка/ролик.
//! Модель — конфиги и путь (веса грузит каждая стадия на время своей
//! работы); условие и латенты — тензоры в памяти службы, живут в
//! дескрипторах ядра и в кэше стадий. Перед стадией с весами видеокарта
//! освобождается от других моделей, после — пулы возвращаются драйверу.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use synaptix_flow::{self as sf, Cx, Handle, Val};
use synaptix_gen::h3_stages as h3;
use synaptix_gen::ltx_stages as ltx;
use synaptix_gen::image_stages as img;
use synaptixd::{DeviceKind, ModelKind};

use crate::flow::{mgr, with_progress};
use crate::{devices, manager};

type R = Result<(), String>;

pub fn install() {
    sf::ops::provide("image_model", image_model);
    sf::ops::provide("image_encode", image_encode);
    sf::ops::provide("image_refs", image_refs);
    sf::ops::provide("image_latent", image_latent);
    sf::ops::provide("image_to_latent", image_to_latent);
    sf::ops::provide("image_sample", image_sample);
    sf::ops::provide("image_decode", image_decode);
    sf::ops::provide("h3_model", h3_model);
    sf::ops::provide("h3_latent", h3_latent);
    sf::ops::provide("h3_encode", h3_encode);
    sf::ops::provide("h3_sample", h3_sample);
    sf::ops::provide("h3_decode_video", h3_decode_video);
    sf::ops::provide("h3_decode_audio", h3_decode_audio);
    for (k, f) in [
        ("ltx_model", ltx_model as fn(&mut Cx) -> R),
        ("ltx_encode", ltx_encode),
        ("ltx_nag", ltx_nag),
        ("ltx_image", ltx_image),
        ("ltx_stage1", ltx_stage1),
        ("ltx_upscale", ltx_upscale),
        ("ltx_stage2", ltx_stage2),
        ("ltx_retake", ltx_retake),
        ("ltx_ic_lora", ltx_ic_lora),
        ("ltx_lipdub", ltx_lipdub),
        ("ltx_a2v", ltx_a2v),
        ("ltx_decode_video", ltx_decode_video),
        ("ltx_decode_audio", ltx_decode_audio),
    ] {
        sf::ops::provide(k, f);
    }
}

/// Стадия на устройстве: другие модели уступают ему место, стадии
/// идут по одной на устройство (как запросы службы), пулы — драйверу.
fn on_device<T>(id: &str, dev: DeviceKind, why: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if dev == DeviceKind::Cuda {
        mgr().clear_device(id, dev, why);
    }
    let r = {
        let _l = devices::lock(dev);
        f()
    };
    manager::release_vram(Some(dev));
    r
}

fn short(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if t.len() < s.len() {
        format!("{t}…")
    } else {
        t
    }
}

/// Сид поля; 0 — случайный (тогда результат не кэшируется).
fn seed(cx: &mut Cx) -> u64 {
    match cx.pos("seed") {
        Some(s) => s as u64,
        None => {
            cx.uncached();
            crate::gen::random_seed()
        }
    }
}

fn quant_field(cx: &Cx, from_settings: Option<String>) -> Option<String> {
    cx.str("quant").or(from_settings)
}

// ── Картинки ──────────────────────────────────────────────────────────────

struct ImageModel {
    id: String,
    dev: DeviceKind,
    m: img::ImageModel,
}

fn image_model(cx: &mut Cx) -> R {
    let m = mgr();
    let id = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Image).map_err(|e| e.message)?;
    let (path, dev, q) = m.run(&id, |l| l.image_source()).map_err(|e| e.message)?;
    let quant = quant_field(cx, q);
    let model = img::open(&path, devices::device(dev), quant.as_deref())?.with_resolution(cx.str("resolution").and_then(|r| r.parse().ok()).unwrap_or(0));
    let label = format!("{} ({id}{})", model.name(), quant.map(|q| format!(", {q}")).unwrap_or_default());
    cx.label = label.clone();
    cx.out("model", Val::Handle(Handle::new(label, ImageModel { id, dev, m: model })));
    Ok(())
}

fn image_encode(cx: &mut Cx) -> R {
    let m = cx.handle::<ImageModel>("model")?;
    let prompt = cx.need("prompt")?;
    let negative = cx.str("negative");
    let images: Vec<PathBuf> = cx.files("images").into_iter().map(PathBuf::from).collect();
    // Без картинок на входе энкодер Qwen видит картинки референсов.
    let refs = if images.is_empty() { cx.opt_handle::<img::Refs>("refs")? } else { None };
    let seq = cx.pos("seq_len").map(|v| v as usize);
    let cond = with_progress(cx, |_, _| {
        on_device(&m.id, m.dev, "кодирование описания", || match &refs {
            Some(r) => m.m.encode_pics(&prompt, negative.as_deref(), r.pics.clone(), seq),
            None => m.m.encode(&prompt, negative.as_deref(), &images, seq),
        })
    })?;
    let label = format!("условие «{}»", short(&prompt, 40));
    cx.label = label.clone();
    cx.out("cond", Val::Handle(Handle::new(label, cond)));
    Ok(())
}

fn image_refs(cx: &mut Cx) -> R {
    let m = cx.handle::<ImageModel>("model")?;
    let images: Vec<PathBuf> = cx.files("images").into_iter().map(PathBuf::from).collect();
    let before = cx.opt_handle::<img::Refs>("refs")?;
    let own = if images.is_empty() {
        None
    } else {
        Some(with_progress(cx, |_, _| on_device(&m.id, m.dev, "кодирование референсов", || m.m.references(&images)))?)
    };
    let refs = match (before, own) {
        (Some(b), Some(o)) => img::Refs::join(&[&b, &o])?,
        (None, Some(o)) => o,
        (Some(b), None) => img::Refs::join(&[&b])?,
        (None, None) => return Err("нет картинок-референсов".into()),
    };
    let label = format!("референсов {}", refs.sizes.len());
    cx.label = label.clone();
    cx.out("refs", Val::Handle(Handle::new(label, refs)));
    Ok(())
}

fn image_latent(cx: &mut Cx) -> R {
    let w = cx.pos("width").unwrap_or(0.0) as usize;
    let h = cx.pos("height").unwrap_or(0.0) as usize;
    let lat = img::Latent::Size(w, h);
    let label = format!("латент {}", lat.label());
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, lat)));
    Ok(())
}

fn image_to_latent(cx: &mut Cx) -> R {
    let m = cx.handle::<ImageModel>("model")?;
    let image = PathBuf::from(cx.need_file("image")?);
    let size = cx.opt_handle::<img::Latent>("size")?.map(|l| l.size()).unwrap_or((0, 0));
    let crop = cx.str("fit").as_deref() != Some("stretch");
    let lat = with_progress(cx, |_, _| on_device(&m.id, m.dev, "кодирование картинки", || m.m.image_latent(&image, size, crop)))?;
    let label = format!("латент {}", lat.label());
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, lat)));
    Ok(())
}

fn image_sample(cx: &mut Cx) -> R {
    let m = cx.handle::<ImageModel>("model")?;
    let cond = cx.handle::<img::Cond>("cond")?;
    let latent = cx.opt_handle::<img::Latent>("latent")?;
    let refs = cx.opt_handle::<img::Refs>("refs")?;
    let opts = img::SampleOpts {
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: seed(cx),
        denoise: cx.pos("denoise").map(|v| v as f32),
    };
    let lat = with_progress(cx, |progress, stop| {
        on_device(&m.id, m.dev, "сэмплер картинки", || {
            m.m.sample(&cond, latent.as_deref(), refs.as_deref(), &opts, &mut |i, n| {
                progress(i, n);
                !stop.load(Ordering::SeqCst)
            })
        })
    })
    .map_err(|e| if cx.cancel.is_cancelled() { "прогон остановлен".into() } else { e })?;
    let label = format!("{}, сид {}", lat.label(), opts.seed);
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, lat)));
    Ok(())
}

fn image_decode(cx: &mut Cx) -> R {
    let m = cx.handle::<ImageModel>("model")?;
    let lat = cx.handle::<img::Latent>("latent")?;
    let image = with_progress(cx, |_, _| on_device(&m.id, m.dev, "декодер картинки", || m.m.decode(&lat)))?;
    let path = cx.out_path("png");
    synaptix_gen::image::save_png(&image, &path)?;
    let (w, h) = lat.size();
    cx.label = format!("картинка {w}×{h}");
    cx.out("image", Val::File(path));
    Ok(())
}

// ── MiniMax-H3 ────────────────────────────────────────────────────────────

struct H3Model {
    id: String,
    dev: DeviceKind,
    m: h3::H3Model,
}

fn h3_model(cx: &mut Cx) -> R {
    let m = mgr();
    let want = cx.str("model").unwrap_or_else(|| "minimax-h3-fl2va".into());
    let id = m.resolve_id(&want, ModelKind::Video).map_err(|e| e.message)?;
    let (path, encoder, dev, q) = m.run(&id, |l| l.h3_source()).map_err(|e| e.message)?;
    let quant = quant_field(cx, q);
    let mut model = h3::open(&path, encoder.as_deref(), devices::device(dev), quant.as_deref())?;
    let lora = cx.file("lora").map(PathBuf::from);
    if let Some(p) = &lora {
        model = model.with_lora(p.clone(), cx.num("lora_strength").unwrap_or(1.0) as f32);
    }
    let lora_name = lora.as_ref().and_then(|p| p.file_name()).map(|n| format!(", LoRA {}", n.to_string_lossy())).unwrap_or_default();
    let label = format!("MiniMax-H3 ({id}{}{lora_name})", quant.map(|q| format!(", {q}")).unwrap_or_default());
    cx.label = label.clone();
    cx.out("model", Val::Handle(Handle::new(label, H3Model { id, dev, m: model })));
    Ok(())
}

fn h3_latent(cx: &mut Cx) -> R {
    let size = h3::Size { width: cx.pos("width").unwrap_or(0.0) as usize, height: cx.pos("height").unwrap_or(0.0) as usize, seconds: cx.pos("seconds").unwrap_or(0.0) };
    let label = format!("ролик {}×{}, {} с", size.width, size.height, size.seconds);
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, size)));
    Ok(())
}

fn h3_encode(cx: &mut Cx) -> R {
    let m = cx.handle::<H3Model>("model")?;
    let size = cx.opt_handle::<h3::Size>("latent")?.map(|s| *s).unwrap_or_default();
    let prompt = cx.need("prompt")?;
    let negative = cx.str("negative");
    let first = cx.file("first_frame").map(PathBuf::from);
    let last = cx.file("last_frame").map(PathBuf::from);
    let refs: Vec<PathBuf> = cx.files("refs").into_iter().map(PathBuf::from).collect();
    let ref_size = if cx.str("ref_size").as_deref() == Some("max") { h3::RefSize::Max } else { h3::RefSize::Match };
    let cond = with_progress(cx, |_, _| on_device(&m.id, m.dev, "кодировщик H3", || m.m.encode(size, &prompt, negative.as_deref(), first.as_deref(), last.as_deref(), &refs, ref_size)))?;
    let label = format!("условие H3 {}", cond.label());
    cx.label = label.clone();
    cx.out("cond", Val::Handle(Handle::new(label, cond)));
    Ok(())
}

fn h3_sample(cx: &mut Cx) -> R {
    let m = cx.handle::<H3Model>("model")?;
    let cond = cx.handle::<h3::H3Cond>("cond")?;
    let source = cx.file("source_video").map(PathBuf::from);
    let steps = cx.pos("steps").unwrap_or(20.0) as usize;
    let cfg = cx.num("guidance").filter(|v| *v > 0.0).unwrap_or(5.0) as f32;
    let strength = cx.pos("strength").map(|v| v as f32);
    let seed = seed(cx);
    let lat = with_progress(cx, |progress, stop| on_device(&m.id, m.dev, "сэмплер H3", || m.m.sample(&cond, steps, cfg, seed, source.as_deref(), strength, stop, progress)))
        .map_err(|e| if cx.cancel.is_cancelled() { "прогон остановлен".into() } else { e })?;
    let label = format!("{}, сид {seed}", lat.label);
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, lat)));
    Ok(())
}

fn h3_decode_video(cx: &mut Cx) -> R {
    let m = cx.handle::<H3Model>("model")?;
    let lat = cx.handle::<h3::H3Latent>("latent")?;
    let frames = with_progress(cx, |_, _| on_device(&m.id, m.dev, "декодер видео H3", || m.m.decode_video(&lat)))?;
    let path = cx.out_path("mp4");
    synaptix_gen::video::write_mp4(&frames, None, h3::FPS, &path)?;
    cx.label = format!("ролик {}×{}, {:.1} с", lat.width, lat.height, lat.frames as f64 / h3::FPS);
    cx.out("video", Val::File(path));
    Ok(())
}

fn h3_decode_audio(cx: &mut Cx) -> R {
    let m = cx.handle::<H3Model>("model")?;
    let lat = cx.handle::<h3::H3Latent>("latent")?;
    let (pcm, rate, ch) = with_progress(cx, |_, _| on_device(&m.id, m.dev, "декодер звука H3", || m.m.decode_audio(&lat)))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, synaptix_gen::music::wav_bytes(&pcm, rate, ch)).map_err(|e| e.to_string())?;
    cx.label = format!("звук {:.1} с", pcm.len() as f32 / (rate.max(1) as f32 * ch.max(1) as f32));
    cx.out("audio", Val::File(path));
    Ok(())
}

// ── LTX-2.3 ───────────────────────────────────────────────────────────────

struct LtxModel {
    id: String,
    dev: DeviceKind,
    m: ltx::LtxModel,
}

/// Живые модели LTX: держат DiT и Gemma между стадиями.
fn ltx_live() -> &'static std::sync::Mutex<Vec<std::sync::Weak<LtxModel>>> {
    static L: std::sync::OnceLock<std::sync::Mutex<Vec<std::sync::Weak<LtxModel>>>> = std::sync::OnceLock::new();
    L.get_or_init(Default::default)
}

/// Отпустить DiT/Gemma моделей LTX (кроме `except` — той, что сейчас работает).
pub fn release_ltx(except: Option<&str>) {
    let models: Vec<Arc<LtxModel>> = ltx_live().lock().map(|mut l| {
        l.retain(|w| w.strong_count() > 0);
        l.iter().filter_map(|w| w.upgrade()).collect()
    }).unwrap_or_default();
    for m in models.iter().filter(|m| Some(m.id.as_str()) != except) {
        if m.m.holds() {
            tracing::info!("LTX {}: DiT/Gemma отпущены — карта нужна другой модели", m.id);
            m.m.release();
        }
    }
}

/// Отпустить DiT/Gemma, простаивающие дольше двух минут.
pub fn release_idle_ltx() {
    let models: Vec<Arc<LtxModel>> = ltx_live().lock().map(|l| l.iter().filter_map(|w| w.upgrade()).collect()).unwrap_or_default();
    for m in models {
        if m.m.release_idle(std::time::Duration::from_secs(120)) {
            tracing::info!("LTX {}: DiT/Gemma отпущены после простоя", m.id);
        }
    }
}

fn ltx_model(cx: &mut Cx) -> R {
    let m = mgr();
    let want = cx.str("model").unwrap_or_else(|| "ltx-2-3-22b-distilled-1-1".into());
    let id = m.resolve_id(&want, ModelKind::Video).map_err(|e| e.message)?;
    let (paths, dev, q) = m.run(&id, |l| l.ltx_source()).map_err(|e| e.message)?;
    let quant = quant_field(cx, q);
    let strength = cx.num("lora_strength").unwrap_or(1.0) as f32;
    let distilled = paths.model.file_name().map(|n| n.to_string_lossy().to_lowercase().contains("distilled")).unwrap_or(false);
    // LoRA: имя компонента модели или файл; dev-чекпойнту — distilled-LoRA (сигмы дистиллированные).
    let lora = match cx.str("lora").as_deref() {
        Some("union") => Some(paths.ic_union.clone().ok_or("у модели нет IC-LoRA union-control")?),
        Some("motion") => Some(paths.ic_motion.clone().ok_or("у модели нет IC-LoRA motion-track")?),
        Some("lipdub") => Some(paths.ic_lipdub.clone().ok_or("у модели нет IC-LoRA LipDub")?),
        Some("distilled") => Some(paths.distilled_lora.clone().ok_or("у модели нет distilled-LoRA")?),
        Some(_) => Some(PathBuf::from(cx.need_file("lora")?)),
        None if !distilled => paths.distilled_lora.clone(),
        None => None,
    };
    let depth = cx.file("depth").map(PathBuf::from);
    let lora_name = lora.as_ref().and_then(|p| p.file_name()).map(|n| format!(", LoRA {}", n.to_string_lossy())).unwrap_or_default();
    let model = ltx::open(paths, devices::device(dev), quant.as_deref(), lora.map(|p| (p, strength)), depth)?;
    let label = format!("LTX-2.3 ({id}{}{lora_name})", quant.map(|q| format!(", {q}")).unwrap_or_default());
    let h = Arc::new(LtxModel { id, dev, m: model });
    if let Ok(mut l) = ltx_live().lock() {
        l.push(Arc::downgrade(&h));
    }
    cx.label = label.clone();
    cx.out("model", Val::Handle(Handle::from_arc(label, h)));
    Ok(())
}

fn ltx_shape(cx: &Cx) -> ltx::Shape {
    ltx::Shape {
        width: cx.pos("width").unwrap_or(1024.0) as usize,
        height: cx.pos("height").unwrap_or(576.0) as usize,
        seconds: cx.pos("seconds").unwrap_or(10.0),
        fps: cx.str("fps").and_then(|f| f.parse().ok()).unwrap_or(24.0),
    }
}

/// Стадия LTX на видеокарте с ходом и отменой.
fn ltx_run<T: Send>(cx: &mut Cx, m: &Arc<LtxModel>, why: &str, f: impl FnOnce(&ltx::Hooks) -> Result<T, String> + Send) -> Result<T, String> {
    with_progress(cx, |progress, stop| on_device(&m.id, m.dev, why, || f(&ltx::Hooks { cancel: stop, progress })))
        .map_err(|e| if cx.cancel.is_cancelled() { "прогон остановлен".into() } else { e })
}

fn ltx_latent(cx: &mut Cx, lat: ltx::LtxLatent, seed: Option<u64>) {
    let label = format!("{}, {}×{}, {:.1} с{}", lat.label, lat.width(), lat.height(), lat.seconds(), seed.map(|s| format!(", сид {s}")).unwrap_or_default());
    cx.label = label.clone();
    cx.out("latent", Val::Handle(Handle::new(label, lat)));
}

fn ltx_encode(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let prompt = cx.need("prompt")?;
    let cond = ltx_run(cx, &m, "кодировщик LTX", |_| m.m.encode(&prompt))?;
    let label = format!("условие «{}»", short(&prompt, 40));
    cx.label = label.clone();
    cx.out("cond", Val::Handle(Handle::new(label, cond)));
    Ok(())
}

fn ltx_nag(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let prompt = cx.str("prompt").unwrap_or_default();
    let (scale, alpha, tau) = (cx.num("scale").unwrap_or(11.0) as f32, cx.num("alpha").unwrap_or(0.25) as f32, cx.num("tau").unwrap_or(2.5) as f32);
    let nag = ltx_run(cx, &m, "NAG LTX", |_| m.m.nag(&prompt, scale, alpha, tau))?;
    let label = format!("NAG ×{scale}");
    cx.label = label.clone();
    cx.out("nag", Val::Handle(Handle::new(label, nag)));
    Ok(())
}

fn ltx_image(cx: &mut Cx) -> R {
    let path = PathBuf::from(cx.need_file("image")?);
    let strength = cx.num("strength").unwrap_or(1.0) as f32;
    let frame = cx.num("frame").unwrap_or(0.0).max(0.0) as usize;
    let image = ltx::LtxImage::load(&path, strength, frame)?;
    let label = if frame == 0 { format!("первый кадр, сила {strength}") } else { format!("ключевой кадр {frame}, сила {strength}") };
    cx.label = label.clone();
    cx.out("image", Val::Handle(Handle::new(label, image)));
    Ok(())
}

fn ltx_stage1(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let nag = cx.opt_handle::<ltx::LtxNag>("nag")?;
    let image = cx.opt_handle::<ltx::LtxImage>("image")?;
    let shape = ltx_shape(cx);
    let seed = seed(cx);
    let lat = ltx_run(cx, &m, "Stage1 LTX", |h| m.m.stage1(&cond, nag.as_deref(), image.as_deref(), shape, seed, h))?;
    ltx_latent(cx, lat, Some(seed));
    Ok(())
}

fn ltx_upscale(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let lat = cx.handle::<ltx::LtxLatent>("latent")?;
    let up = ltx_run(cx, &m, "апскейл LTX", |_| m.m.upscale(&lat))?;
    ltx_latent(cx, up, None);
    Ok(())
}

fn ltx_stage2(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let lat = cx.handle::<ltx::LtxLatent>("latent")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let image = cx.opt_handle::<ltx::LtxImage>("image")?;
    let seed = seed(cx);
    let out = ltx_run(cx, &m, "Stage2 LTX", |h| m.m.stage2(&lat, &cond, image.as_deref(), seed, h))?;
    ltx_latent(cx, out, Some(seed));
    Ok(())
}

fn ltx_retake(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let video = PathBuf::from(cx.need_file("video")?);
    let shape = ltx_shape(cx);
    let (start, end) = (cx.num("start").unwrap_or(0.0), cx.num("end").unwrap_or(2.0));
    if end <= start {
        return Err(format!("отрезок retake пуст: {start}–{end} с"));
    }
    let seed = seed(cx);
    let lat = ltx_run(cx, &m, "retake LTX", |h| m.m.retake(&cond, &video, shape, start, end, seed, h))?;
    ltx_latent(cx, lat, Some(seed));
    Ok(())
}

fn ltx_ic_lora(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let video = PathBuf::from(cx.need_file("video")?);
    let shape = ltx_shape(cx);
    let downscale = cx.pos("downscale").unwrap_or(2.0) as usize;
    let strength = cx.num("strength").unwrap_or(1.0) as f32;
    let control = match cx.str("control").as_deref() {
        Some("canny") => ltx::Control::Canny { low: cx.num("canny_low").unwrap_or(0.1) as f32, high: cx.num("canny_high").unwrap_or(0.3) as f32 },
        Some("depth") => ltx::Control::Depth,
        _ => ltx::Control::None,
    };
    let seed = seed(cx);
    let lat = ltx_run(cx, &m, "IC-LoRA LTX", |h| m.m.ic_lora(&cond, &video, shape, downscale, strength, control, seed, h))?;
    ltx_latent(cx, lat, Some(seed));
    Ok(())
}

fn ltx_lipdub(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let video = PathBuf::from(cx.need_file("video")?);
    let audio = PathBuf::from(cx.need_file("audio")?);
    let shape = ltx_shape(cx);
    let lat = ltx_run(cx, &m, "LipDub LTX", |_| m.m.lipdub(&cond, &video, &audio, shape))?;
    ltx_latent(cx, lat, None);
    Ok(())
}

fn ltx_a2v(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let cond = cx.handle::<ltx::LtxCond>("cond")?;
    let audio = PathBuf::from(cx.need_file("audio")?);
    let shape = ltx_shape(cx);
    let seed = seed(cx);
    let lat = ltx_run(cx, &m, "видео по звуку LTX", |h| m.m.audio_to_video(&cond, &audio, shape, seed, h))?;
    ltx_latent(cx, lat, Some(seed));
    Ok(())
}

fn ltx_decode_video(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let lat = cx.handle::<ltx::LtxLatent>("latent")?;
    let frames = ltx_run(cx, &m, "декодер видео LTX", |_| m.m.decode_video(&lat))?;
    let path = cx.out_path("mp4");
    synaptix_gen::video::write_mp4(&frames, None, lat.fps, &path)?;
    cx.label = format!("ролик {}×{}, {:.1} с", lat.width(), lat.height(), frames.len() as f64 / lat.fps);
    cx.out("video", Val::File(path));
    Ok(())
}

fn ltx_decode_audio(cx: &mut Cx) -> R {
    let m = cx.handle::<LtxModel>("model")?;
    let lat = cx.handle::<ltx::LtxLatent>("latent")?;
    let pcm = ltx_run(cx, &m, "декодер звука LTX", |_| m.m.decode_audio(&lat))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, synaptix_gen::music::wav_bytes(&pcm, ltx::AUDIO_RATE, 2)).map_err(|e| e.to_string())?;
    cx.label = format!("звук {:.1} с", pcm.len() as f32 / (2.0 * ltx::AUDIO_RATE as f32));
    cx.out("audio", Val::File(path));
    Ok(())
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn ok<T: Send + Sync>() {}
    ok::<Arc<ImageModel>>();
    ok::<img::Cond>();
    ok::<img::Latent>();
    ok::<img::Refs>();
    ok::<H3Model>();
    ok::<h3::H3Cond>();
    ok::<h3::H3Latent>();
    ok::<LtxModel>();
    ok::<ltx::LtxCond>();
    ok::<ltx::LtxNag>();
    ok::<ltx::LtxImage>();
    ok::<ltx::LtxLatent>();
}
