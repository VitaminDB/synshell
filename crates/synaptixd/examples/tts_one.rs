//! Одна фраза персонажем: `tts_one <модель> <speaker> <текст> <out.wav>`.
use synaptixd::{Client, Params};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut c = Client::connect().unwrap();
    // EXTRA="guidance=3,position_temperature=0" — добавочные параметры.
    let mut p = Params::new().with("language", "en").with("speaker", a[2].as_str());
    for kv in std::env::var("EXTRA").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap();
        p = match v.parse::<f64>() {
            Ok(x) => p.with(k, x),
            Err(_) => p.with(k, v),
        };
    }
    let s = if std::env::var_os("FRESH").is_some() { c.speak_fresh(&a[1], &a[3], &p) } else { c.speak_with(&a[1], &a[3], &p) }.unwrap();
    let pcm = s.audio.f32s();
    let mut w = Vec::new();
    let n = pcm.len() as u32 * 2;
    for (k, v) in [(b"RIFF", 36 + n)] { w.extend_from_slice(k); w.extend_from_slice(&v.to_le_bytes()); }
    w.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    w.extend_from_slice(&s.sample_rate.to_le_bytes());
    w.extend_from_slice(&(s.sample_rate * 2).to_le_bytes());
    w.extend_from_slice(b"\x02\0\x10\0data");
    w.extend_from_slice(&n.to_le_bytes());
    for x in pcm { w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
    std::fs::write(&a[4], w).unwrap();
}
