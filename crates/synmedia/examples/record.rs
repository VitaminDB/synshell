//! Проверка записи: `cargo run -p synmedia --example record -- out.mp4 [секунды] [voice.wav]` —
//! фронтальная камера, микрофон и (если задан) WAV как голос собеседника со 2-й секунды.

use std::time::{Duration, Instant};

fn read_wav(p: &str) -> (Vec<f32>, u32) {
    let d = std::fs::read(p).expect("wav");
    let rate = u32::from_le_bytes(d[24..28].try_into().unwrap());
    let ch = u16::from_le_bytes(d[22..24].try_into().unwrap()) as usize;
    let bits = u16::from_le_bytes(d[34..36].try_into().unwrap());
    let i = d.windows(4).position(|w| w == b"data").expect("data") + 8;
    let body = &d[i..];
    let s: Vec<f32> = match bits {
        16 => body.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
        _ => body.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
    };
    (s.chunks(ch).map(|c| c[0]).collect(), rate)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "/tmp/synmedia-test.mp4".into());
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let cams = synmedia::cameras();
    for c in &cams {
        println!("камера: {} (фронтальная: {})", c.name(), c.front());
    }
    let cam = synmedia::front_camera().ok_or_else(|| anyhow::anyhow!("камер нет"))?;
    let (w, h) = cam.video_size(16, 9, 1280).unwrap_or((1280, 720));
    let frames = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let f2 = frames.clone();
    let eng = synmedia::Engine::new(move |n| match n {
        synmedia::Note::Frame => {
            f2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        synmedia::Note::Started(w, h) => println!("поток {w}×{h}"),
        synmedia::Note::Status(s) if !s.is_empty() => println!("состояние: {s}"),
        _ => {}
    });
    eng.set_rotation(cam.sensor_rotation(), cam.front());
    eng.open(synmedia::OpenSpec::video(cam, w, h), Default::default());
    // превью «берётся» интерфейсом — здесь сразу
    let t = Instant::now();
    while eng.capture().is_none() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    eng.start_recording(synmedia::Settings::new(&out))?;
    println!("кодер: {}", eng.with_recorder(|r| r.encoder().to_string()).unwrap_or_default());
    let voice = eng.track(0.8);
    let t = Instant::now();
    let mut said = false;
    while t.elapsed() < Duration::from_secs(secs) {
        eng.frame_taken();
        std::thread::sleep(Duration::from_millis(30));
        if !said && t.elapsed() > Duration::from_secs(2) {
            said = true;
            if let (Some(v), Some(p)) = (&voice, args.get(3)) {
                let (pcm, rate) = read_wav(p);
                let at = v.play(&pcm, rate);
                println!("голос: {:.2} с звука с {:.2} с записи", pcm.len() as f32 / rate as f32, at.as_secs_f32());
            }
        }
    }
    println!("позиция записи: {:.2} с, кадров превью {}", eng.record_position().unwrap_or_default().as_secs_f32(), frames.load(std::sync::atomic::Ordering::Relaxed));
    let p = eng.stop_recording()?;
    eng.close();
    println!("готово: {}", p.display());
    Ok(())
}
