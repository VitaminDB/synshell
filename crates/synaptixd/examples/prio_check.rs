//! Приоритеты устройства: во время фоновой длинной генерации по схеме
//! (`priority=background`) озвучка фразы и распознавание со словами должны
//! отвечать за секунды.
//!
//! `cargo run --example prio_check -- <llm> [tts] [asr]`.

use std::time::Instant;
use serde_json::json;
use synaptixd::{Client, Message, Params};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let llm = a.get(1).cloned().unwrap_or_else(|| "qwen3-8-flash-next-uncensored".into());
    let tts = a.get(2).cloned().unwrap_or_else(|| "omnivoice".into());
    let asr = a.get(3).cloned().unwrap_or_else(|| "whisper-large-v3-turbo".into());
    let prio = std::env::var("PRIO").unwrap_or_else(|_| "background".into());
    // Прогрев всех трёх, чтобы мерить ожидание, а не загрузку.
    let mut c = Client::connect().unwrap();
    let s = c.speak_with(&tts, "Hello, how are you today?", &Params::new().with("language", "en").with("speaker", "male_adult")).unwrap();
    let pcm: Vec<f32> = s.audio.f32s().to_vec();
    let sr = s.sample_rate;
    c.transcribe_words(&asr, &pcm, sr, Some("en"), false).unwrap();
    let _ = c.generate_with(&llm, vec![Message::user("Hi")], &Params::new().with("max_tokens", 4i64), |_| {});

    let llm2 = llm.clone();
    let prio2 = prio.clone();
    let bg = std::thread::spawn(move || {
        let mut c = Client::connect().unwrap();
        let schema = json!({"type": "object", "properties": {"texts": {"type": "array", "minItems": 6, "maxItems": 6, "items": {"type": "object", "properties": {"title": {"type": "string"}, "text": {"type": "string"}, "questions": {"type": "array", "items": {"type": "string"}, "minItems": 3, "maxItems": 3}}, "required": ["title", "text", "questions"]}}}, "required": ["texts"]});
        let p = Params::new().with("max_tokens", 2500i64).with("priority", prio2.as_str());
        let t = Instant::now();
        let r = c.generate_json(&llm2, vec![Message::user("Write six B1 reading texts (about 200 words each) on different everyday topics, each with three comprehension questions.")], &schema, &p, |_| {});
        let g = r.map(|(_, g)| g).unwrap();
        println!("фон: {} токенов за {:.1} с ({:.1} ток/с)", g.tokens, t.elapsed().as_secs_f32(), g.tokens as f32 * 1e3 / g.decode_ms.max(1) as f32);
    });
    std::thread::sleep(std::time::Duration::from_secs(15));
    for i in 0..3 {
        let t = Instant::now();
        let s = c.speak_with(&tts, "That's a great answer. Could you give me an example from your last job?", &Params::new().with("language", "en").with("speaker", "male_adult")).unwrap();
        let ts = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let w = c.transcribe_words(&asr, s.audio.f32s(), s.sample_rate, Some("en"), false).unwrap();
        let ta = t.elapsed().as_secs_f32();
        println!("#{i}: speak {:.1} с звука за {ts:.2} с, transcribe_words {} слов за {ta:.2} с", s.audio.f32s().len() as f32 / s.sample_rate as f32, w.words.len());
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    bg.join().unwrap();
}
