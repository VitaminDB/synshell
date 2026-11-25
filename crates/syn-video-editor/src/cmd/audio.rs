//! Звук и речь: расшифровка (служба synaptixd, слова с таймкодами), поиск
//! тишины (ffmpeg silencedetect), вырезание тишины и кусков по времени
//! источника — основа монтажа «говорящей головы», подкастов, нарезки по словам.

use std::process::Command;

use serde_json::{json, Value};

use super::{arg, req, Args, Command as Cmd, Out, Res, Ty};
use crate::doc::{tc, Id, Project, TranscriptWord};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&TRANSCRIBE, &TRANSCRIPT, &SILENCES, &CUT_SILENCE, &CUT_RANGES, &SUBTITLES];

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
    if matches!(m.kind, crate::doc::MediaKind::Image | crate::doc::MediaKind::Lottie) || (m.kind == crate::doc::MediaKind::Video && !m.has_audio) {
        return Err(format!("m{id}: в файле нет звука"));
    }
    let lang = a.str("language")?;
    let t = transcribe_file(&m.path, lang.as_deref())?;
    let n = t.words.len();
    let (text, language) = (t.text.clone(), t.language.clone());
    ed.begin("Расшифровка");
    let mm = ed.project.media.iter_mut().find(|x| x.id == id).unwrap();
    mm.transcript = Some(t);
    let preview: String = text.chars().take(400).collect();
    Ok(Out::with(format!("m{id}: {n} слов{}. {preview}{}", language.map(|l| format!(", язык {l}")).unwrap_or_default(), if text.chars().count() > 400 { "…" } else { "" }), json!({"words": n})))
}

/// Распознать речь файла службой нейросетей — без доступа к редактору
/// (можно в своём потоке, окно не ждёт).
pub fn transcribe_file(path: &std::path::Path, lang: Option<&str>) -> Result<crate::doc::Transcript, String> {
    syn_assist::tools::progress(0, 0, "звук");
    let pcm = pcm16k(&path.to_string_lossy())?;
    syn_assist::tools::progress(0, 0, "распознавание");
    let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let t = c.transcribe_words("", &pcm, 16000, lang, false).map_err(|e| format!("распознавание: {e}"))?;
    let words: Vec<TranscriptWord> = t.words.iter().map(|w| TranscriptWord { text: w.text.trim().to_string(), start: w.start as f64, end: w.end as f64 }).filter(|w| !w.text.is_empty()).collect();
    Ok(crate::doc::Transcript { text: t.text.trim().to_string(), language: t.language.clone(), words })
}

/// Фразы расшифровки: разрыв по паузе > 0.6 с, концу предложения, запятой у
/// уже длинной фразы (≥ 55 % `max_chars`) или длине `max_chars` — тогда по
/// последней запятой внутри, чтобы не рвать мысль посередине.
pub fn phrases(words: &[TranscriptWord], max_chars: usize) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    let mut cur: Vec<&TranscriptWord> = Vec::new();
    let emit = |part: &[&TranscriptWord], out: &mut Vec<(f64, f64, String)>| {
        if let (Some(f), Some(l)) = (part.first(), part.last()) {
            out.push((f.start, l.end, part.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")));
        }
    };
    let chars = |ws: &[&TranscriptWord]| ws.iter().map(|w| w.text.chars().count() + 1).sum::<usize>().saturating_sub(1);
    let soft = |w: &TranscriptWord| w.text.ends_with([',', ';', ':', '—']);
    for w in words {
        if let Some(last) = cur.last() {
            let len = chars(&cur) + 1 + w.text.chars().count();
            if w.start - last.end > 0.6 || last.text.ends_with(['.', '!', '?', '…']) || (soft(last) && chars(&cur) * 100 >= max_chars * 55) {
                emit(&cur, &mut out);
                cur.clear();
            } else if len > max_chars {
                // переполнение: разорвать по последней запятой внутри, если она не в самом начале
                match cur.iter().rposition(|x| soft(x)).filter(|&j| (j + 1) * 3 >= cur.len()) {
                    Some(j) if j + 1 < cur.len() => {
                        emit(&cur[..=j], &mut out);
                        cur.drain(..=j);
                    }
                    _ => {
                        emit(&cur, &mut out);
                        cur.clear();
                    }
                }
            }
        }
        cur.push(w);
    }
    emit(&cur, &mut out);
    out
}

static SUBTITLES: Cmd = Cmd {
    id: "subtitles.add",
    category: "Звук",
    title: "Субтитры из речи",
    shortcut: "",
    keywords: "subtitles субтитры титры речь расшифровка captions",
    help: "Субтитры по расшифровке (media.transcribe): для клипов ids (по умолчанию все клипы с расшифровкой) фразы встают титрами на дорожку «Субтитры» внизу кадра, по времени речи с учётом сдвига и скорости клипа. max_chars — длина строки (42), size — размер шрифта (52). Прежние субтитры на этом отрезке заменяются.",
    args: &[arg("ids", Ty::Ids, "клипы (c…)"), arg("max_chars", Ty::Num, "символов в фразе (42)"), arg("size", Ty::Num, "размер шрифта (52)")],
    danger: false,
    run: subtitles,
};

fn subtitles(ed: &mut Editor, a: &Args) -> Res {
    let max_chars = a.num("max_chars")?.unwrap_or(42.0).clamp(8.0, 200.0) as usize;
    let size = a.num("size")?.unwrap_or(52.0).clamp(10.0, 300.0);
    let p = &ed.project;
    let with_t = |c: &crate::doc::Clip| c.media().and_then(|m| p.media(m)).is_some_and(|m| m.transcript.is_some());
    // видео со связанным звуком — один раз (по звуковому клипу, если он есть)
    let mut clips: Vec<crate::doc::Clip> = match a.ids("ids")? {
        Some(ids) => ids.iter().filter_map(|i| p.clip(*i).cloned()).collect(),
        None => p.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| with_t(c)).cloned().collect(),
    };
    clips.retain(|c| with_t(c));
    let mut seen = std::collections::HashSet::new();
    clips.retain(|c| seen.insert(p.linked_peer(c.id).map_or(c.id, |q| q.min(c.id))));
    if clips.is_empty() {
        return Err("нет клипов с расшифровкой — сначала media.transcribe".into());
    }
    let mut subs: Vec<(f64, f64, String)> = Vec::new();
    for c in &clips {
        let m = p.media(c.media().unwrap()).unwrap();
        for (s, e, text) in phrases(&m.transcript.as_ref().unwrap().words, max_chars) {
            // время источника → таймлайн; фраза за краями клипа — обрезается или пропускается
            let ts = c.start + (s - c.offset) / c.speed;
            let te = (c.start + (e - c.offset) / c.speed).min(c.end());
            let ts = ts.max(c.start);
            if te - ts > 0.02 {
                subs.push((ts, te, text));
            }
        }
    }
    if subs.is_empty() {
        return Err("в границах клипов нет речи".into());
    }
    subs.sort_by(|a, b| a.0.total_cmp(&b.0));
    ed.begin("Субтитры");
    let p = &mut ed.project;
    let ti = match p.tracks.iter().position(|t| t.kind == crate::doc::TrackKind::Video && t.name == "Субтитры") {
        Some(i) => i,
        None => {
            let id = p.alloc();
            p.tracks.push(crate::doc::Track { id, kind: crate::doc::TrackKind::Video, name: "Субтитры".into(), muted: false, hidden: false, clips: Vec::new() });
            p.tracks.len() - 1
        }
    };
    let fps = p.settings.fps.max(1.0);
    let (lo, hi) = (subs[0].0, subs.last().unwrap().1);
    p.tracks[ti].clips.retain(|c| c.end() <= lo + 1e-6 || c.start >= hi - 1e-6);
    let n = subs.len();
    let mut prev_end = 0.0f64;
    let starts: Vec<f64> = subs.iter().map(|x| x.0).collect();
    for (i, (s, e, text)) in subs.into_iter().enumerate() {
        let s = ((s * fps).round() / fps).max(prev_end);
        // короткую фразу показать хотя бы 0.8 с, не налезая на следующую
        let next = starts.get(i + 1).copied().unwrap_or(f64::MAX);
        // не дальше начала следующей и конца речи (последняя — не удлиняет проект)
        let cap = next.min(hi);
        let (s, e) = if e < s + 0.8 && s + 0.8 > cap { (s.min((cap - 0.8).max(prev_end)), cap.max(e)) } else { (s, e.max((s + 0.8).min(cap))) };
        let s = ((s * fps).round() / fps).max(prev_end);
        let e = ((e * fps).round() / fps).max(s + 1.0 / fps);
        prev_end = e;
        let id = p.alloc();
        let mut style = serde_json::Map::new();
        style.insert("position".into(), json!("bottom"));
        style.insert("size".into(), json!(size));
        style.insert("color".into(), json!("#ffffff"));
        style.insert("in".into(), json!("none"));
        style.insert("out".into(), json!("none"));
        p.tracks[ti].clips.push(crate::doc::Clip { id, source: crate::doc::Source::Title { text, style }, start: s, duration: e - s, offset: 0.0, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None });
    }
    p.tracks[ti].clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(Out::with(format!("Субтитров: {n} ({}–{})", tc(lo), tc(hi)), json!({"count": n})))
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
/// остальное влево (ripple на дорожке клипа). Если у клипа есть связанный звук
/// (или видео), режет и сдвигает оба синхронно, сохраняя связи частей.
pub fn cut_source(p: &mut Project, id: Id, cuts: &[(f64, f64)]) -> Result<(Vec<Id>, f64), String> {
    let peer = p.linked_peer(id);
    let (parts, removed) = cut_source_one(p, id, cuts)?;
    if let Some(pid) = peer {
        if let Ok((p_parts, _)) = cut_source_one(p, pid, cuts) {
            if p_parts.len() == parts.len() {
                for (&a, &b) in parts.iter().zip(p_parts.iter()) {
                    if let Some(c) = p.clip_mut(a) {
                        c.link = Some(b);
                    }
                    if let Some(c) = p.clip_mut(b) {
                        c.link = Some(a);
                    }
                }
            } else {
                for &cid in parts.iter().chain(p_parts.iter()) {
                    if let Some(c) = p.clip_mut(cid) {
                        c.link = None;
                    }
                }
            }
        } else {
            for &cid in &parts {
                if let Some(c) = p.clip_mut(cid) {
                    c.link = None;
                }
            }
        }
    }
    Ok((parts, removed))
}

fn cut_source_one(p: &mut Project, id: Id, cuts: &[(f64, f64)]) -> Result<(Vec<Id>, f64), String> {
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
            n.transition = None;
            n.link = None;
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
