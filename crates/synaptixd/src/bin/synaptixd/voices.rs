//! Голоса-персонажи для движков без голоса по описанию (VibeVoice) и
//! сценарии с несколькими голосами («Speaker N: …» + `speakers`).
//!
//! VibeVoice умеет только клонировать голос с образца: персонаж без образца
//! звучал случайным тембром (мужчина и женщина — одной высоты). Поэтому
//! голос персонажа — опорный клип, один раз озвученный OmniVoice по описанию
//! персонажа (пол, возраст) и закреплённым зерном; клип лежит в
//! `<каталог состояния>/voices/<персонаж>-<язык>.wav`.

use std::path::{Path, PathBuf};

use synaptixd::{Error, ErrorCode, ModelKind, Params};

use crate::manager::Manager;

/// Модель, которая озвучивает опорные клипы (голос по описанию).
const CLIP_MODEL: &str = "omnivoice";

/// Реплики сценария: строки «Speaker N: текст» → (N, текст). Строка без
/// метки продолжает прошлую реплику (до первой метки — Speaker 1).
pub fn replicas(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match label(line) {
            Some((n, t)) => out.push((n, t.to_string())),
            None => match out.last_mut() {
                Some(last) => {
                    last.1.push(' ');
                    last.1.push_str(line);
                }
                None => out.push((1, line.to_string())),
            },
        }
    }
    out.retain(|r| !r.1.trim().is_empty());
    out
}

/// «Speaker 2: текст» → (2, «текст»).
fn label(line: &str) -> Option<(usize, &str)> {
    let head = line.get(..7)?;
    if !head.eq_ignore_ascii_case("speaker") {
        return None;
    }
    let rest = line[7..].trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let n: usize = rest[..digits].parse().ok()?;
    let t = rest[digits..].trim_start().strip_prefix(':')?;
    Some((n, t.trim()))
}

/// Список персонажей: `a,b` или JSON-массив `["a","b"]`.
pub fn parse_speakers(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(s) {
            return v.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        }
    }
    s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// Сценарий для одного прохода VibeVoice: номера говорящих — подряд с 1 в
/// порядке номеров; возвращает и исходные номера (по ним — голоса).
pub fn renumber(reps: &[(usize, String)]) -> (String, Vec<usize>) {
    let mut used: Vec<usize> = reps.iter().map(|r| r.0).collect();
    used.sort_unstable();
    used.dedup();
    let script = reps
        .iter()
        .map(|(n, t)| format!("Speaker {}: {t}", used.iter().position(|u| u == n).unwrap_or(0) + 1))
        .collect::<Vec<_>>()
        .join("\n");
    (script, used)
}

/// Опорный клип персонажа: из кэша на диске, иначе озвучить моделью
/// [`CLIP_MODEL`] (описание персонажа и его зерно) и сохранить.
pub fn persona_clip(mgr: &std::sync::Arc<Manager>, speaker: &str, language: Option<&str>) -> Result<PathBuf, Error> {
    let lang = language.map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).unwrap_or_else(|| "en".into());
    let dir = mgr.cfg.state.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/var/lib/synaptix")).join("voices");
    let path = dir.join(format!("{speaker}-{lang}-v2.wav"));
    if path.exists() {
        return Ok(path);
    }
    let failed = |e: String| Error::new(ErrorCode::Failed, format!("голос «{speaker}»: {e}"));
    let model = mgr.resolve_id(CLIP_MODEL, ModelKind::Tts).map_err(|e| failed(format!("нужна модель {CLIP_MODEL} для опорного клипа: {}", e.message)))?;
    let text = clip_text(&lang);
    let (lo, hi) = pitch_range(speaker);
    let base = synaptix::facade::tts::speaker(speaker).map(|s| s.2).unwrap_or(1);
    // Описание голоса OmniVoice выдерживает не на каждом зерне: из
    // нескольких берётся первый клип с высотой голоса в пределах персонажа,
    // иначе — ближайший к середине.
    let mut best: Option<(f32, Vec<f32>, u32)> = None;
    for k in 0..8u64 {
        let p = Params::new().with("language", lang.as_str()).with("speaker", speaker).with("seed", (base + k * 101) as i64);
        let (pcm, sr, _) = mgr.run(&model, |m| m.speak(&text, &p, true))?;
        let f0 = median_f0(&pcm, sr);
        tracing::info!("голос «{speaker}»: зерно {} — F0 {f0:.0} Гц (нужно {lo:.0}–{hi:.0})", base + k * 101);
        let miss = if f0 < lo { lo - f0 } else if f0 > hi { f0 - hi } else { 0.0 };
        if best.as_ref().is_none_or(|b| miss < b.0) {
            best = Some((miss, pcm, sr));
        }
        if miss == 0.0 {
            break;
        }
    }
    let (_, pcm, sr) = best.expect("хотя бы одна попытка");
    std::fs::create_dir_all(&dir).map_err(|e| failed(format!("{}: {e}", dir.display())))?;
    let tmp = path.with_extension("wav.tmp");
    write_wav(&tmp, &pcm, sr).map_err(|e| failed(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &path).map_err(|e| failed(format!("{}: {e}", path.display())))?;
    tracing::info!("голос «{speaker}» ({lang}): опорный клип {:.1} с → {}", pcm.len() as f32 / sr as f32, path.display());
    Ok(path)
}

/// Текст опорного клипа: повествовательный, без приветствия — иначе
/// VibeVoice на похожей фразе («Hello! …») продолжал читать текст клипа.
pub fn clip_text(lang: &str) -> String {
    match lang {
        "en" => "The morning was quiet, so we walked along the river for a long time and talked about our plans for the summer.",
        "kk" => "Таң тыныш болды, сондықтан біз өзен жағасымен ұзақ жүріп, жазғы жоспарларымызды әңгімелестік.",
        _ => "Утро было тихим, и мы долго шли вдоль реки, обсуждая планы на лето.",
    }
    .to_string()
}

/// Готовый голос движка по словесному описанию (пол, возраст); уже готовое
/// имя голоса и описание без пола — `None` (решает схема/умолчание).
pub fn preset_for_description(arch: &str, desc: &str) -> Option<&'static str> {
    let words: Vec<String> = desc.to_lowercase().split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(String::from).collect();
    let has = |ws: &[&str]| words.iter().any(|w| ws.contains(&w.as_str()));
    let female = has(&["female", "woman", "women", "girl", "lady", "feminine", "женский", "женщина", "девочка"]);
    let male = !female && has(&["male", "man", "men", "boy", "guy", "masculine", "мужской", "мужчина", "мальчик"]);
    let child = has(&["girl", "boy", "child", "kid", "девочка", "мальчик", "ребёнок"]);
    let old = has(&["elderly", "old", "senior", "пожилой", "пожилая", "старый"]);
    let young = has(&["young", "молодой", "молодая"]);
    match arch {
        "leokani" if female => Some("dala"),
        "leokani" if male => Some("pono"),
        "supertonic" if female => Some(if child { "F2" } else if old { "F3" } else if young { "F4" } else { "F1" }),
        "supertonic" if male => Some(if child { "M4" } else if old { "M3" } else if young { "M1" } else { "M2" }),
        _ => None,
    }
}

/// Ожидаемая высота голоса персонажа (медиана F0, Гц).
pub fn pitch_range(speaker: &str) -> (f32, f32) {
    match speaker {
        "female_young" => (185.0, 260.0),
        "female_adult" => (160.0, 215.0),
        "female_elderly" => (150.0, 225.0),
        "male_young" => (100.0, 155.0),
        "male_adult" => (80.0, 135.0),
        "male_elderly" => (75.0, 140.0),
        "girl" | "boy" => (240.0, 400.0),
        _ => (0.0, 1000.0),
    }
}

/// Медиана основного тона (YIN по окнам 40 мс с голосом), Гц.
pub fn median_f0(pcm: &[f32], sr: u32) -> f32 {
    // Без прореживания: оно (без фильтра) давало ошибки на октаву вниз.
    let x = pcm;
    let sr = sr as usize;
    let win = sr * 40 / 1000;
    let (lo, hi) = (sr / 450, sr / 60);
    let mut f0s = Vec::new();
    let mut i = 0;
    while i + win + hi < x.len() {
        let w = &x[i..i + win + hi];
        i += win / 2;
        if w[..win].iter().map(|v| v * v).sum::<f32>() / (win as f32) < 1e-4 {
            continue;
        }
        let d: Vec<f32> = (0..=hi).map(|tau| (0..win).map(|j| (w[j] - w[j + tau]).powi(2)).sum()).collect();
        let mut run = 0f32;
        let cm: Vec<f32> = d.iter().enumerate().map(|(tau, &v)| if tau == 0 { 1.0 } else { run += v; v * tau as f32 / run.max(1e-9) }).collect();
        let mut tau = lo;
        while tau < hi {
            if cm[tau] < 0.15 {
                while tau + 1 < hi && cm[tau + 1] < cm[tau] {
                    tau += 1;
                }
                f0s.push(sr as f32 / tau as f32);
                break;
            }
            tau += 1;
        }
    }
    f0s.sort_by(f32::total_cmp);
    f0s.get(f0s.len() / 2).copied().unwrap_or(0.0)
}

/// Модель проверки синтеза распознаванием.
const CHECK_ASR: &str = "whisper-large-v3-turbo";

/// Доля слов текста, не узнанных в синтезированной речи (WER по словам без
/// пунктуации); `None` — проверить нечем.
pub fn speech_error(mgr: &std::sync::Arc<Manager>, pcm: &[f32], sr: u32, language: Option<&str>, text: &str) -> Option<f32> {
    let model = mgr.resolve_id(CHECK_ASR, ModelKind::Asr).ok()?;
    let t = mgr.run(&model, |m| m.transcribe(pcm, sr, language, None)).ok()?;
    // Метки «Speaker N:» сценария не произносятся.
    let spoken: Vec<String> = replicas(text).into_iter().map(|r| r.1).collect();
    let r = words(&spoken.join(" "));
    if r.is_empty() {
        return None;
    }
    Some(wer(&r, &words(&t.text)))
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect()
}

fn wer(r: &[String], h: &[String]) -> f32 {
    let mut d: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = d[0];
        d[0] = i;
        for j in 1..=h.len() {
            let t = d[j];
            d[j] = (d[j] + 1).min(d[j - 1] + 1).min(prev + (r[i - 1] != h[j - 1]) as usize);
            prev = t;
        }
    }
    d[h.len()] as f32 / r.len().max(1) as f32
}

fn write_wav(path: &Path, pcm: &[f32], sr: u32) -> std::io::Result<()> {
    let data = pcm.len() as u32 * 2;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sr.to_le_bytes());
    w.extend_from_slice(&(sr * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for x in pcm {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_from_description() {
        assert_eq!(preset_for_description("leokani", "male"), Some("pono"));
        assert_eq!(preset_for_description("leokani", "female, american accent"), Some("dala"));
        assert_eq!(preset_for_description("supertonic", "elderly male"), Some("M3"));
        assert_eq!(preset_for_description("leokani", "dala"), None);
        assert_eq!(preset_for_description("omnivoice", "male"), None);
    }

    #[test]
    fn script() {
        let r = replicas("Speaker 1: Hello there.\nSpeaker 3: Hi!\nhow are you?\nspeaker 1 : Fine.");
        assert_eq!(r, vec![(1, "Hello there.".into()), (3, "Hi! how are you?".into()), (1, "Fine.".into())]);
        let (s, used) = renumber(&r);
        assert_eq!(used, vec![1, 3]);
        assert_eq!(s, "Speaker 1: Hello there.\nSpeaker 2: Hi! how are you?\nSpeaker 1: Fine.");
        assert_eq!(replicas("Just text."), vec![(1, "Just text.".into())]);
        assert_eq!(parse_speakers(r#"["a", "b"]"#), vec!["a", "b"]);
        assert_eq!(parse_speakers("a, b"), vec!["a", "b"]);
    }
}
