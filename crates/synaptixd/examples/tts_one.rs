//! Одна фраза персонажем: `tts_one <модель> <speaker> <текст> <out.wav>`.
use synaptixd::{Client, Params};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut c = Client::connect().unwrap();
    let s = c.speak_with(&a[1], &a[3], &Params::new().with("language", "en").with("speaker", a[2].as_str())).unwrap();
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
