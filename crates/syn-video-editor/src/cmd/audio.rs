//! Звук и речь: расшифровка (служба synaptixd, слова с таймкодами), поиск
//! тишины (ffmpeg silencedetect), вырезание тишины и кусков по времени
//! источника — основа монтажа «говорящей головы», подкастов, нарезки по словам.

use std::process::Command;

use serde_json::{json, Value};

use super::{arg, req, Args, Command as Cmd, Out, Res, Ty};
use crate::doc::{tc, Id, Project, TranscriptWord};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&TRANSCRIBE, &TRANSCRIPT, &SILENCES, &CUT_SILENCE, &CUT_RANGES];

/// Звук файла: моно f32, 16 кГц.
fn pcm16k(path: &str) -> Result<Vec<f32>, String> {
    let out = Command::new("ffmpeg").args(["-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"]).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

static TRANSCRIBE: Cmd = Cmd {
    id: "media.transcribe",
    category: "Медиа",
    title: "Расшифровать речь",
    shortcut: "",
    keywords: "transcribe расшифровка речь текст субтитры whisper распознать",
    help: "Распознать речь файла (служба нейросетей): текст и слова с таймкодами сохраняются в медиатеке — по ним можно резать (clip.cut_ranges), делать субтитры, искать фразы. Долго для длинных файлов.",
    args: &[req("id", Ty::Id, "медиа (m…)"), arg("language", Ty::Str, "язык (ru, en…); пусто — определить")],
    danger: false,
    run: transcribe,
};

fn transcribe(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let m = ed.project.media(id).ok_or_else(|| format!("нет медиа m{id}"))?.clone();
    if m.kind == crate::doc::MediaKind::Image || (m.kind == crate::doc::MediaKind::Video && !m.has_audio) {
        return Err(format!("m{id}: в файле нет звука"));
    }
    let lang = a.str("language")?;
    syn_assist::tools::progress(0, 0, "звук");
    let pcm = pcm16k(&m.path.to_string_lossy())?;
    syn_assist::tools::progress(0, 0, "распознавание");
    let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let t = c.transcribe_words("", &pcm, 16000, lang.as_deref(), false).map_err(|e| format!("распознавание: {e}"))?;
    let words: Vec<TranscriptWord> = t.words.iter().map(|w| TranscriptWord { text: w.text.trim().to_string(), start: w.start as f64, end: w.end as f64 }).filter(|w| !w.text.is_empty()).collect();
    let n = words.len();
    ed.begin("Расшифровка");
    let mm = ed.project.media.iter_mut().find(|x| x.id == id).unwrap();
    mm.transcript = Some(crate::doc::Transcript { text: t.text.trim().to_string(), language: t.language.clone(), words });
    let preview: String = t.text.trim().chars().take(400).collect();
    Ok(Out::with(format!("m{id}: {n} слов{}. {preview}{}", t.language.map(|l| format!(", язык {l}")).unwrap_or_default(), if t.text.chars().count() > 400 { "…" } else { "" }), json!({"words": n})))
}

static TRANSCRIPT: Cmd = Cmd {
    id: "media.transcript",
    category: "Медиа",
    title: "Текст расшифровки",
    shortcut: "",
    keywords: "transcript текст слова таймкоды",
    help: "Расшифровка файла по фразам с таймкодами источника (после media.transcribe); words: true — каждое слово с таймкодом.",
    args: &[req("id", Ty::Id, "медиа (m…)"), arg("words", Ty::Bool, "по словам")],
    danger: false,
    run: transcript,
};

fn transcript(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let m = ed.project.media(id).ok_or_else(|| format!("нет медиа m{id}"))?;
    let t = m.transcript.as_ref().ok_or_else(|| format!("m{id}: расшифровки нет — сначала media.transcribe"))?;
    let mut lines = Vec::new();
    if a.bool("words")?.unwrap_or(false) {
        for w in &t.words {
            lines.push(format!("{:.2}–{:.2} {}", w.start, w.end, w.text));
        }
    } else {
        // фразы: разрыв по паузе > 0.6 с или по концу предложения
        let mut cur: Vec<&TranscriptWord> = Vec::new();
        for w in &t.words {
            if let Some(last) = cur.last() {
                if w.start - last.end > 0.6 || last.text.ends_with(['.', '!', '?']) {
                    lines.push(format!("{}–{} {}", tc(cur[0].start), tc(last.end), cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")));
                    cur.clear();
                }
            }
            cur.push(w);
        }
        if let (Some(f), Some(l)) = (cur.first(), cur.last()) {
            lines.push(format!("{}–{} {}", tc(f.start), tc(l.end), cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")));
        }
    }
    Ok(Out::text(if lines.is_empty() { t.text.clone() } else { lines.join("\n") }))
}

/// Тишина в файле: (начало, конец) времени источника.
pub fn silences(path: &str, threshold_db: f64, min: f64) -> Result<Vec<(f64, f64)>, String> {
    let out = Command::new("ffmpeg").args(["-hide_banner", "-nostats", "-i", path, "-vn", "-af", &format!("silencedetect=noise={threshold_db}dB:d={min}"), "-f", "null", "-"]).output().map_err(|e| format!("ffmpeg: {e}"))?;
    let log = String::from_utf8_lossy(&out.stderr);
    let mut v = Vec::new();
    let mut start: Option<f64> = None;
    for l in log.lines() {
        if let Some(p) = l.find("silence_start: ") {
            start = l[p + 15..].split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(p) = l.find("silence_end: ") {
            let end: Option<f64> = l[p + 13..].split_whitespace().next().and_then(|s| s.parse().ok());
            if let (Some(s), Some(e)) = (start.take(), end) {
                v.push((s.max(0.0), e));
            }
        }
    }
    if let Some(s) = start {
        v.push((s, f64::MAX));
    }
    Ok(v)
}

static SILENCES: Cmd = Cmd {
    id: "media.silences",
    category: "Медиа",
    title: "Найти тишину",
    shortcut: "",
    keywords: "silence тишина паузы",
    help: "Паузы в звуке файла (время источника): тише threshold_db (по умолчанию −35 дБ) дольше min секунд (0.6).",
    args: &[req("id", Ty::Id, "медиа (m…)"), arg("threshold_db", Ty::Num, "порог, дБ"), arg("min", Ty::Num, "минимальная длина паузы, с")],
    danger: false,
    run: silences_cmd,
};

fn silences_cmd(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let m = ed.project.media(id).ok_or_else(|| format!("нет медиа m{id}"))?;
    let v = silences(&m.path.to_string_lossy(), a.num("threshold_db")?.unwrap_or(-35.0), a.num("min")?.unwrap_or(0.6))?;
    let total: f64 = v.iter().map(|(s, e)| e.min(m.duration) - s).sum();
    let lines: Vec<String> = v.iter().map(|(s, e)| format!("{}–{}", tc(*s), tc(e.min(m.duration)))).collect();
    Ok(Out::with(format!("m{id}: пауз {}, всего {:.1} с\n{}", v.len(), total, lines.join("\n")), json!(v)))
}

/// Вырезать из клипа куски времени источника `cuts` (отсортированы), сдвинуть
/// остальное влево (ripple на дорожке клипа). Возвращает id частей и сколько вырезано, с.
pub fn cut_source(p: &mut Project, id: Id, cuts: &[(f64, f64)]) -> Result<(Vec<Id>, f64), String> {
    let (ti, ci) = p.find_clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let c = p.tracks[ti].clips[ci].clone();
    let src0 = c.offset;
    let src1 = c.offset + c.duration * c.speed;
    // куски, которые остаются: [src0, src1] минус cuts
    let mut keep = Vec::new();
    let mut pos = src0;
    for &(s, e) in cuts {
        let (s, e) = (s.max(src0), e.min(src1));
        if e <= s {
            continue;
        }
        if s > pos {
            keep.push((pos, s));
        }
        pos = pos.max(e);
    }
    if pos < src1 {
        keep.push((pos, src1));
    }
    let fps = p.settings.fps.max(1.0);
    keep.retain(|(s, e)| (e - s) / c.speed >= 1.0 / fps);
    if keep.is_empty() {
        return Err(format!("c{id}: после вырезания ничего не остаётся"));
    }
    let old_len = c.duration;
    let mut parts = Vec::new();
    let mut at = c.start;
    let mut first = true;
    let mut new_clips = Vec::new();
    for (s, e) in keep {
        let mut n = c.clone();
        if !first {
            n.id = p.alloc();
            for ef in &mut n.effects {
                ef.id = p.alloc();
            }
        }
        first = false;
        n.offset = s;
        n.duration = p.snap((e - s) / c.speed);
        n.start = p.snap(at);
        at = n.start + n.duration;
        parts.push(n.id);
        new_clips.push(n);
    }
    let removed = old_len - (at - c.start);
    let t = &mut p.tracks[ti];
    t.clips.remove(ci);
    for o in &mut t.clips {
        if o.start >= c.end() - 1e-6 {
            o.start = (o.start - removed).max(0.0);
        }
    }
    t.clips.extend(new_clips);
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok((parts, removed))
}

static CUT_SILENCE: Cmd = Cmd {
    id: "clip.cut_silence",
    category: "Таймлайн",
    title: "Вырезать тишину",
    shortcut: "",
    keywords: "silence cut тишина паузы вырезать jump cut",
    help: "Вырезать паузы из клипа со звуком (порог threshold_db −35 дБ, пауза дольше min 0.6 с, оставить pad 0.12 с у краёв речи); следующие клипы дорожки сдвигаются. Клип распадается на куски речи.",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("threshold_db", Ty::Num, "порог, дБ"), arg("min", Ty::Num, "минимальная пауза, с"), arg("pad", Ty::Num, "запас у речи, с")],
    danger: false,
    run: cut_silence,
};

fn cut_silence(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?.clone();
    let m = c.media().and_then(|mid| ed.project.media(mid)).ok_or_else(|| format!("c{id}: не медиа-клип"))?.clone();
    let pad = a.num("pad")?.unwrap_or(0.12);
    let sil = silences(&m.path.to_string_lossy(), a.num("threshold_db")?.unwrap_or(-35.0), a.num("min")?.unwrap_or(0.6))?;
    let cuts: Vec<(f64, f64)> = sil.into_iter().map(|(s, e)| (s + pad, e.min(m.duration + 1.0) - pad)).filter(|(s, e)| e > s).collect();
    if cuts.is_empty() {
        return Ok(Out::text(format!("c{id}: пауз не найдено")));
    }
    ed.begin("Вырезать тишину");
    let (parts, removed) = cut_source(&mut ed.project, id, &cuts)?;
    Ok(Out::with(format!("c{id}: вырезано {:.1} с тишины, кусков речи: {} ({})", removed, parts.len(), parts.iter().map(|p| format!("c{p}")).collect::<Vec<_>>().join(", ")), json!({"parts": parts, "removed": removed})))
}

static CUT_RANGES: Cmd = Cmd {
    id: "clip.cut_ranges",
    category: "Таймлайн",
    title: "Вырезать куски",
    shortcut: "",
    keywords: "cut ranges вырезать слова фразы оговорки",
    help: "Вырезать из клипа куски ВРЕМЕНИ ИСТОЧНИКА (как в media.transcript): ranges — [[начало, конец], …] в секундах. Для удаления слов, оговорок, повторов по расшифровке. Следующие клипы дорожки сдвигаются.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("ranges", Ty::Array, "[[начало, конец], …], с источника")],
    danger: false,
    run: cut_ranges,
};

fn cut_ranges(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let raw = a.raw("ranges").cloned().unwrap_or(Value::Null);
    let raw = match raw {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| format!("ranges: {e}"))?,
        v => v,
    };
    let mut cuts: Vec<(f64, f64)> = raw
        .as_array()
        .ok_or("ranges: список [[начало, конец], …]")?
        .iter()
        .map(|r| {
            let r = r.as_array().filter(|r| r.len() == 2).ok_or("ranges: каждый кусок — [начало, конец]")?;
            let s = crate::doc::parse_time(&r[0]).ok_or("ranges: время")?;
            let e = crate::doc::parse_time(&r[1]).ok_or("ranges: время")?;
            Ok::<_, &str>((s.min(e), s.max(e)))
        })
        .collect::<Result<_, _>>()?;
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    ed.project.find_clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    ed.begin("Вырезать куски");
    let (parts, removed) = cut_source(&mut ed.project, id, &cuts)?;
    Ok(Out::with(format!("c{id}: вырезано {:.2} с, кусков: {}", removed, parts.len()), json!({"parts": parts})))
}
