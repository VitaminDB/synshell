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
    let model = img::open(&path, devices::device(dev), quant.as_deref())?;
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
    let model = h3::open(&path, encoder.as_deref(), devices::device(dev), quant.as_deref())?;
    let label = format!("MiniMax-H3 ({id}{})", quant.map(|q| format!(", {q}")).unwrap_or_default());
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
    let cond = with_progress(cx, |_, _| on_device(&m.id, m.dev, "кодировщик H3", || m.m.encode(size, &prompt, negative.as_deref(), first.as_deref(), last.as_deref(), &refs)))?;
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
}
