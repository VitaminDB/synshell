use std::collections::VecDeque;
use std::result::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

pub use syngui::audio::VisHandle;
use syngui::audio::{AudioPlayer, AudioRecorder};
use synshell_common::config::Assistant;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    #[default]
    Off,
    Starting,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceConfig {
    pub language: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f32>,
    pub microphone: Option<String>,
    /// Уровень фонового шума 0–100 (см. `Assistant::noise_level`); 0 — авто.
    pub noise_level: f32,
}

impl VoiceConfig {
    pub fn from_assistant(a: &Assistant) -> Self {
        let opt = |s: &str| {
            let s = s.trim();
            if s.is_empty() || s.eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(s.to_string())
            }
        };
        Self {
            language: opt(&a.voice_language),
            voice: opt(&a.voice),
            speed: if (a.voice_speed - 1.0).abs() > 0.01 && a.voice_speed > 0.2 { Some(a.voice_speed) } else { None },
            microphone: opt(&a.microphone),
            noise_level: a.noise_level,
        }
    }

    pub fn load() -> Self {
        Self::from_assistant(&synshell_common::Config::load().0.assistant)
    }
}

#[derive(Clone)]
pub enum VoiceEvent {
    Phase(Phase),
    Vis(Option<VisHandle>),
    Heard(String),
    Speaking(bool),
    Error(String),
    Stopped,
}

pub type Listener = Arc<dyn Fn(VoiceEvent) + Send + Sync>;
pub type Turn = Arc<dyn Fn(String) -> Result<String, String> + Send + Sync>;
pub type Done<T> = Arc<dyn Fn(Result<T, String>) + Send + Sync>;

struct Flags {
    stop: AtomicBool,
    running: AtomicBool,
    dict_stop: AtomicBool,
    dictating: AtomicBool,
    muted: AtomicBool,
    speech_gen: AtomicU64,
}

fn flags() -> &'static Flags {
    static F: std::sync::OnceLock<Flags> = std::sync::OnceLock::new();
    F.get_or_init(|| Flags {
        stop: AtomicBool::new(false),
        running: AtomicBool::new(false),
        dict_stop: AtomicBool::new(false),
        dictating: AtomicBool::new(false),
        muted: AtomicBool::new(false),
        speech_gen: AtomicU64::new(0),
    })
}

const FRAME_MS: usize = 20;
const PRE_ROLL_MS: usize = 400;
const START_FRAMES: usize = 3;
const MIN_SPEECH_FRAMES: usize = 12;
const END_SILENCE_FRAMES: usize = 45;
const MAX_UTTERANCE_SEC: usize = 45;

pub struct Vad {
    frame: usize,
    buf: Vec<f32>,
    pre: VecDeque<f32>,
    pre_cap: usize,
    floor: f32,
    /// Нижняя граница шумового пола, заданная пользователем (0 — нет).
    floor_min: f32,
    ticks: usize,
    in_speech: bool,
    above: usize,
    speech_frames: usize,
    silence_frames: usize,
    utter: Vec<f32>,
    max_len: usize,
}

impl Vad {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(8000) as usize;
        let frame = sr * FRAME_MS / 1000;
        Self {
            frame,
            buf: Vec::with_capacity(frame * 2),
            pre: VecDeque::with_capacity(sr * PRE_ROLL_MS / 1000),
            pre_cap: sr * PRE_ROLL_MS / 1000,
            floor: 0.01,
            floor_min: 0.0,
            ticks: 0,
            in_speech: false,
            above: 0,
            speech_frames: 0,
            silence_frames: 0,
            utter: Vec::new(),
            max_len: sr * MAX_UTTERANCE_SEC,
        }
    }

    /// Уровень фонового шума 0–100 → нижняя граница шумового пола (RMS по
    /// логарифмической шкале: 50 ≈ 0,01, 100 ≈ 0,1). 0 — только автоподбор.
    pub fn with_noise_level(mut self, level: f32) -> Self {
        let level = level.clamp(0.0, 100.0);
        self.floor_min = if level <= 0.0 { 0.0 } else { 0.001 * 10f32.powf(level / 50.0) };
        self
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.pre.clear();
        self.in_speech = false;
        self.above = 0;
        self.speech_frames = 0;
        self.silence_frames = 0;
        self.utter.clear();
    }

    pub fn thresholds(&self) -> (f32, f32) {
        let floor = self.floor.max(self.floor_min);
        let silence = (floor * 1.8 + 0.004).max(0.006);
        let speech = (floor * 3.0 + 0.01).max(silence * 1.3);
        (speech, silence)
    }

    pub fn push(&mut self, chunk: &[f32]) -> Option<Vec<f32>> {
        self.buf.extend_from_slice(chunk);
        let mut out = None;
        while self.buf.len() >= self.frame {
            let frame: Vec<f32> = self.buf.drain(..self.frame).collect();
            if let Some(u) = self.frame_in(&frame) {
                out = Some(u);
            }
        }
        out
    }

    fn frame_in(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt();
        self.ticks += 1;
        if self.ticks <= 3 {
            self.floor = rms.max(0.001);
            return None;
        }
        if rms < self.floor {
            self.floor = rms.max(0.001);
        } else if !self.in_speech {
            self.floor += (rms - self.floor) * 0.03;
        }
        let (speech, silence) = self.thresholds();
        if !self.in_speech {
            for &s in frame {
                if self.pre.len() >= self.pre_cap {
                    self.pre.pop_front();
                }
                self.pre.push_back(s);
            }
            if rms > speech {
                self.above += 1;
                if self.above >= START_FRAMES {
                    self.in_speech = true;
                    self.speech_frames = self.above;
                    self.silence_frames = 0;
                    self.utter = self.pre.iter().copied().collect();
                    self.pre.clear();
                }
            } else {
                self.above = 0;
            }
            return None;
        }
        self.utter.extend_from_slice(frame);
        if rms > silence {
            self.speech_frames += 1;
            self.silence_frames = 0;
        } else {
            self.silence_frames += 1;
        }
        let ended = self.silence_frames >= END_SILENCE_FRAMES || self.utter.len() >= self.max_len;
        if !ended {
            return None;
        }
        self.in_speech = false;
        self.above = 0;
        let enough = self.speech_frames >= MIN_SPEECH_FRAMES;
        let mut u = std::mem::take(&mut self.utter);
        self.speech_frames = 0;
        self.silence_frames = 0;
        if !enough {
            return None;
        }
        let keep_tail = self.frame * 15;
        let cut = self.frame * END_SILENCE_FRAMES;
        if u.len() > cut + keep_tail {
            u.truncate(u.len() - cut + keep_tail);
        }
        Some(u)
    }

    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.in_speech && self.speech_frames >= MIN_SPEECH_FRAMES / 2 {
            let u = std::mem::take(&mut self.utter);
            self.reset();
            return Some(u);
        }
        self.reset();
        None
    }
}

pub fn list_microphones() -> Vec<String> {
    syngui::audio::list_input_devices()
}

pub fn transcribe(pcm: &[f32], sample_rate: u32, cfg: &VoiceConfig) -> Result<String, String> {
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let t = c.transcribe("", pcm, sample_rate, cfg.language.as_deref()).map_err(|e| e.to_string())?;
    Ok(t.text.trim().to_string())
}

pub fn strip_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            if !in_code {
                out.push_str("(код). ");
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.starts_with('|') && t.ends_with('|') {
            let cells: Vec<&str> = t.trim_matches('|').split('|').map(|c| c.trim()).filter(|c| !c.is_empty() && !c.chars().all(|ch| ch == '-' || ch == ':')).collect();
            if cells.is_empty() {
                continue;
            }
            out.push_str(&cells.join(", "));
            out.push_str(". ");
            continue;
        }
        let mut s = t.trim_start_matches('#').trim_start_matches('>').trim().to_string();
        for marker in ["- ", "* ", "+ "] {
            if let Some(rest) = s.strip_prefix(marker) {
                s = rest.to_string();
            }
        }
        if let Some(pos) = s.find(". ") {
            if pos > 0 && pos <= 3 && s[..pos].chars().all(|c| c.is_ascii_digit()) {
                s = s[pos + 2..].to_string();
            }
        }
        let mut cleaned = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '~' => {}
                '[' => {
                    let mut label = String::new();
                    let mut closed = false;
                    for ch in chars.by_ref() {
                        if ch == ']' {
                            closed = true;
                            break;
                        }
                        label.push(ch);
                    }
                    cleaned.push_str(&label);
                    if closed && chars.peek() == Some(&'(') {
                        for ch in chars.by_ref() {
                            if ch == ')' {
                                break;
                            }
                        }
                    }
                }
                _ => cleaned.push(c),
            }
        }
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        out.push_str(cleaned);
        if !cleaned.ends_with(['.', '!', '?', ':', ';', ',']) {
            out.push('.');
        }
        out.push(' ');
    }
    out.trim().to_string()
}

pub fn chunks(text: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let end = matches!(c, '.' | '!' | '?' | '…') && chars.get(i + 1).map_or(true, |n| n.is_whitespace());
        if end || c == '\n' {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                sentences.push(s);
            }
            cur.clear();
        }
    }
    let s = cur.trim().to_string();
    if !s.is_empty() {
        sentences.push(s);
    }
    let mut out: Vec<String> = Vec::new();
    for s in sentences {
        let n = out.len();
        match out.last_mut() {
            Some(last) if n > 2 && last.chars().count() + s.chars().count() < 240 => {
                last.push(' ');
                last.push_str(&s);
            }
            _ => out.push(s),
        }
    }
    out
}

fn speak_blocking(text: &str, gen: u64, cfg: &VoiceConfig) -> Result<(), String> {
    let parts = chunks(text);
    if parts.is_empty() {
        return Ok(());
    }
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Vec<f32>>();
    let mut rx: Option<Receiver<Vec<f32>>> = Some(rx);
    let mut player: Option<AudioPlayer> = None;
    let current = || flags().speech_gen.load(Ordering::Acquire) == gen;
    for part in parts {
        if !current() {
            break;
        }
        let sp = c.speak("", &part, cfg.language.as_deref(), cfg.voice.as_deref(), cfg.speed).map_err(|e| e.to_string())?;
        if !current() {
            break;
        }
        if player.is_none() {
            let r = rx.take().ok_or_else(|| "поток вывода уже занят".to_string())?;
            player = Some(AudioPlayer::start_streaming(r, sp.sample_rate).map_err(|e| e.to_string())?);
        }
        let _ = tx.send(sp.audio.f32s().to_vec());
    }
    drop(tx);
    if let Some(p) = player {
        while !p.is_done() && current() {
            std::thread::sleep(Duration::from_millis(30));
        }
        p.stop();
    }
    Ok(())
}

fn next_gen() -> u64 {
    flags().speech_gen.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn stop_speaking() {
    next_gen();
}

pub fn speak(cfg: VoiceConfig, text: String, done: Done<()>) -> u64 {
    let gen = next_gen();
    if text.trim().is_empty() {
        done(Ok(()));
        return gen;
    }
    std::thread::Builder::new()
        .name("assistant-tts".into())
        .spawn(move || {
            let r = speak_blocking(&text, gen, &cfg);
            if flags().speech_gen.load(Ordering::Acquire) == gen {
                done(r);
            } else {
                done(Ok(()));
            }
        })
        .ok();
    gen
}

pub fn set_muted(muted: bool) {
    flags().muted.store(muted, Ordering::Release);
}

pub fn muted() -> bool {
    flags().muted.load(Ordering::Acquire)
}

pub fn conversation_running() -> bool {
    flags().running.load(Ordering::Acquire)
}

pub fn dictating() -> bool {
    flags().dictating.load(Ordering::Acquire)
}

pub fn start_conversation(cfg: VoiceConfig, turn: Turn, on: Listener) -> bool {
    let f = flags();
    if f.running.swap(true, Ordering::AcqRel) {
        return false;
    }
    f.stop.store(false, Ordering::Release);
    f.dict_stop.store(true, Ordering::Release);
    on(VoiceEvent::Phase(Phase::Starting));
    std::thread::Builder::new().name("assistant-voice".into()).spawn(move || conversation(cfg, turn, on)).ok();
    true
}

pub fn stop_conversation() {
    flags().stop.store(true, Ordering::Release);
    next_gen();
}

pub fn shutdown() {
    flags().stop.store(true, Ordering::Release);
    flags().dict_stop.store(true, Ordering::Release);
    next_gen();
}

fn open_mic(cfg: &VoiceConfig) -> Result<(AudioRecorder, Receiver<Vec<f32>>, u32), String> {
    let rec = AudioRecorder::start_with_device(cfg.microphone.as_deref()).map_err(|e| e.to_string())?;
    let stream = rec.open_stream().ok_or_else(|| "поток микрофона уже занят".to_string())?;
    let rx = stream.take_receiver().ok_or_else(|| "поток микрофона уже занят".to_string())?;
    let mut sr = rec.sample_rate();
    if sr == 0 {
        sr = stream.sample_rate;
    }
    Ok((rec, rx, sr.max(8000)))
}

fn conversation(cfg: VoiceConfig, turn: Turn, on: Listener) {
    let f = flags();
    let (rec, rx, sr) = match open_mic(&cfg) {
        Ok(v) => v,
        Err(e) => {
            f.running.store(false, Ordering::Release);
            on(VoiceEvent::Error(e));
            on(VoiceEvent::Phase(Phase::Off));
            on(VoiceEvent::Stopped);
            return;
        }
    };
    on(VoiceEvent::Vis(Some(rec.vis_handle())));
    let mut vad = Vad::new(sr).with_noise_level(cfg.noise_level);
    on(VoiceEvent::Phase(Phase::Listening));
    loop {
        if f.stop.load(Ordering::Acquire) {
            break;
        }
        let chunk = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if f.muted.load(Ordering::Acquire) {
            vad.reset();
            continue;
        }
        let Some(utt) = vad.push(&chunk) else { continue };
        rec.pause();
        on(VoiceEvent::Phase(Phase::Transcribing));
        let text = match transcribe(&utt, sr, &cfg) {
            Ok(t) => t,
            Err(e) => {
                on(VoiceEvent::Error(e));
                String::new()
            }
        };
        if text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
            vad.reset();
            while rx.try_recv().is_ok() {}
            rec.resume();
            on(VoiceEvent::Phase(Phase::Listening));
            continue;
        }
        on(VoiceEvent::Heard(text.clone()));
        on(VoiceEvent::Phase(Phase::Thinking));
        let answer = turn(text);
        if f.stop.load(Ordering::Acquire) {
            break;
        }
        match answer {
            Ok(ans) => {
                let spoken = strip_markdown(&ans);
                if !spoken.is_empty() {
                    on(VoiceEvent::Phase(Phase::Speaking));
                    on(VoiceEvent::Speaking(true));
                    let gen = next_gen();
                    if let Err(e) = speak_blocking(&spoken, gen, &cfg) {
                        on(VoiceEvent::Error(e));
                    }
                    on(VoiceEvent::Speaking(false));
                }
            }
            Err(e) if !e.is_empty() => on(VoiceEvent::Error(e)),
            Err(_) => {}
        }
        if f.stop.load(Ordering::Acquire) {
            break;
        }
        vad.reset();
        while rx.try_recv().is_ok() {}
        rec.resume();
        on(VoiceEvent::Phase(Phase::Listening));
    }
    drop(rec);
    f.running.store(false, Ordering::Release);
    on(VoiceEvent::Vis(None));
    on(VoiceEvent::Phase(Phase::Off));
    on(VoiceEvent::Stopped);
}

pub fn toggle_dictation(cfg: VoiceConfig, on: Listener, done: Done<String>) -> bool {
    let f = flags();
    if f.dictating.load(Ordering::Acquire) {
        f.dict_stop.store(true, Ordering::Release);
        return false;
    }
    if f.running.load(Ordering::Acquire) {
        return false;
    }
    f.dictating.store(true, Ordering::Release);
    f.dict_stop.store(false, Ordering::Release);
    on(VoiceEvent::Phase(Phase::Starting));
    std::thread::Builder::new().name("assistant-dictation".into()).spawn(move || dictation(cfg, on, done)).ok();
    true
}

fn dictation(cfg: VoiceConfig, on: Listener, done: Done<String>) {
    let f = flags();
    let finish = |r: Result<String, String>| {
        f.dictating.store(false, Ordering::Release);
        on(VoiceEvent::Vis(None));
        if !f.running.load(Ordering::Acquire) {
            on(VoiceEvent::Phase(Phase::Off));
        }
        done(r);
    };
    let (rec, rx, sr) = match open_mic(&cfg) {
        Ok(v) => v,
        Err(e) => {
            finish(Err(e));
            return;
        }
    };
    on(VoiceEvent::Vis(Some(rec.vis_handle())));
    on(VoiceEvent::Phase(Phase::Listening));
    let mut vad = Vad::new(sr).with_noise_level(cfg.noise_level);
    let started = std::time::Instant::now();
    let utt = loop {
        if f.dict_stop.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(90) {
            break vad.flush();
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(c) => {
                if let Some(u) = vad.push(&c) {
                    break Some(u);
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break vad.flush(),
        }
    };
    drop(rec);
    let Some(utt) = utt else {
        finish(Ok(String::new()));
        return;
    };
    on(VoiceEvent::Phase(Phase::Transcribing));
    finish(transcribe(&utt, sr, &cfg));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_stripped_for_speech() {
        let s = strip_markdown("## Заголовок\n\n- **жирный** и `код`\n- [ссылка](http://x)\n\n```rust\nlet a = 1;\n```\n| a | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(s, "Заголовок. жирный и код. ссылка. (код). a, b. 1, 2.");
    }

    #[test]
    fn chunks_keep_first_sentence_short() {
        let c = chunks("Привет! Это первый ответ. А это вторая фраза подлиннее, чтобы проверить слияние. И третья.");
        assert_eq!(c[0], "Привет!");
        assert_eq!(c[1], "Это первый ответ.");
        assert_eq!(c[2], "А это вторая фраза подлиннее, чтобы проверить слияние. И третья.");
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn vad_detects_utterance_and_trims_tail() {
        let sr = 16_000u32;
        let mut vad = Vad::new(sr);
        let frame = sr as usize * FRAME_MS / 1000;
        let quiet = vec![0.001f32; frame];
        for _ in 0..30 {
            assert!(vad.push(&quiet).is_none());
        }
        let loud: Vec<f32> = (0..frame).map(|i| (i as f32 * 0.3).sin() * 0.3).collect();
        let mut got = None;
        for _ in 0..30 {
            got = got.or(vad.push(&loud));
        }
        assert!(got.is_none());
        for _ in 0..END_SILENCE_FRAMES + 2 {
            if let Some(u) = vad.push(&quiet) {
                got = Some(u);
                break;
            }
        }
        let u = got.expect("фраза должна завершиться по тишине");
        assert!(u.len() >= 30 * frame);
        assert!(u.len() < (30 + END_SILENCE_FRAMES + 20) * frame + sr as usize * PRE_ROLL_MS / 1000);
    }

    #[test]
    fn short_noise_burst_is_ignored() {
        let sr = 16_000u32;
        let mut vad = Vad::new(sr);
        let frame = sr as usize * FRAME_MS / 1000;
        let quiet = vec![0.001f32; frame];
        let loud = vec![0.3f32; frame];
        for _ in 0..20 {
            vad.push(&quiet);
        }
        for _ in 0..5 {
            vad.push(&loud);
        }
        for _ in 0..END_SILENCE_FRAMES + 2 {
            assert!(vad.push(&quiet).is_none());
        }
    }
}

#[cfg(test)]
mod daemon_tests {
    use super::*;

    #[test]
    #[ignore = "нужна служба synaptixd с моделями TTS и ASR: cargo test -p synassist-voice -- --ignored"]
    fn speech_roundtrip_through_vad() {
        let cfg = VoiceConfig { language: Some("ru".into()), ..Default::default() };
        let mut c = synaptixd::Client::connect().expect("служба");
        let sp = c.speak("", "Привет! Сегодня в Костанае ясно и прохладно, около двенадцати градусов.", Some("ru"), None, None).expect("озвучка");
        let sr = sp.sample_rate;
        let pcm = sp.audio.f32s();
        assert!(pcm.len() > sr as usize / 2, "слишком короткая озвучка");
        let mut vad = Vad::new(sr);
        let frame = sr as usize / 50;
        let quiet = vec![0.0005f32; frame];
        for _ in 0..30 {
            assert!(vad.push(&quiet).is_none());
        }
        let mut got = None;
        for chunk in pcm.chunks(frame * 3) {
            got = got.or(vad.push(chunk));
        }
        if got.is_none() {
            for _ in 0..END_SILENCE_FRAMES + 5 {
                if let Some(u) = vad.push(&quiet) {
                    got = Some(u);
                    break;
                }
            }
        }
        let utt = got.expect("детектор речи не выделил фразу из озвучки");
        assert!(utt.len() >= pcm.len() / 2, "фраза обрезана: {} из {}", utt.len(), pcm.len());
        let text = transcribe(&utt, sr, &cfg).expect("распознавание");
        let low = text.to_lowercase();
        assert!(low.contains("ясно") && low.contains("прохладно"), "распознано: {text:?}");
        let parts = chunks(&strip_markdown("Сегодня **ясно**, +12°. Ветер слабый. Дождя не ожидается, зонт не нужен."));
        assert_eq!(parts.len(), 3, "{parts:?}");
        for p in parts {
            let s = c.speak("", &p, Some("ru"), None, None).expect("озвучка части");
            assert_eq!(s.sample_rate, sr);
        }
    }
}
