//! Звуки камеры (синтез, без файлов): затвор, начало и конец записи, отсчёт таймера. Каждый звук —
//! короткий поток вывода cpal (PipeWire) в фоновом потоке.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy)]
pub enum Sound {
    Shutter,
    RecordStart,
    RecordStop,
    Tick,
}

fn synth(s: Sound, rate: f32) -> Vec<f32> {
    let n = |sec: f32| (sec * rate) as usize;
    let mut seed = 0x1234_5678u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    match s {
        // два щелчка шторки: шум с быстрым затуханием и низкий «удар»
        Sound::Shutter => {
            let mut v = vec![0.0; n(0.16)];
            for (k, start) in [0.0f32, 0.07].iter().enumerate() {
                let s0 = n(*start);
                for i in 0..n(0.06) {
                    let t = i as f32 / rate;
                    let env = (-t * 90.0).exp();
                    let body = (TAU * if k == 0 { 180.0 } else { 140.0 } * t).sin() * (-t * 60.0).exp();
                    if s0 + i < v.len() {
                        v[s0 + i] += 0.45 * env * noise() + 0.35 * body;
                    }
                }
            }
            v
        }
        Sound::RecordStart | Sound::RecordStop => {
            let (f1, f2) = if matches!(s, Sound::RecordStart) { (880.0, 1320.0) } else { (1320.0, 880.0) };
            let mut v = vec![0.0; n(0.26)];
            for (i, x) in v.iter_mut().enumerate() {
                let t = i as f32 / rate;
                let f = if t < 0.12 { f1 } else { f2 };
                let tt = if t < 0.12 { t } else { t - 0.12 };
                let env = (tt * 400.0).min(1.0) * (-tt * 18.0).exp();
                *x = 0.3 * env * (TAU * f * t).sin();
            }
            v
        }
        Sound::Tick => {
            let mut v = vec![0.0; n(0.08)];
            for (i, x) in v.iter_mut().enumerate() {
                let t = i as f32 / rate;
                *x = 0.3 * (-t * 50.0).exp() * (TAU * 1000.0 * t).sin();
            }
            v
        }
    }
}

pub fn play(s: Sound) {
    std::thread::Builder::new()
        .name("camera-sound".into())
        .spawn(move || {
            if let Err(e) = play_blocking(s) {
                tracing::debug!("звук: {e}");
            }
        })
        .ok();
}

fn play_blocking(s: Sound) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or_else(|| anyhow::anyhow!("нет вывода звука"))?;
    let cfg = dev.default_output_config()?;
    let rate = cfg.sample_rate().0 as f32;
    let ch = cfg.channels() as usize;
    let samples = Arc::new(synth(s, rate));
    let len = samples.len();
    let done = Arc::new(AtomicBool::new(false));
    let d2 = done.clone();
    let mut pos = 0usize;
    let sc: cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::I16 => dev.build_output_stream(
            &sc,
            move |out: &mut [i16], _: &_| {
                for fr in out.chunks_mut(ch) {
                    let v = samples.get(pos).copied().unwrap_or(0.0);
                    pos += 1;
                    fr.fill((v.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
                if pos >= len {
                    d2.store(true, Ordering::SeqCst);
                }
            },
            |e| tracing::debug!("звук: {e}"),
            None,
        )?,
        _ => dev.build_output_stream(
            &sc,
            move |out: &mut [f32], _: &_| {
                for fr in out.chunks_mut(ch) {
                    let v = samples.get(pos).copied().unwrap_or(0.0);
                    pos += 1;
                    fr.fill(v);
                }
                if pos >= len {
                    d2.store(true, Ordering::SeqCst);
                }
            },
            |e| tracing::debug!("звук: {e}"),
            None,
        )?,
    };
    stream.play()?;
    for _ in 0..100 {
        if done.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(60));
    Ok(())
}
