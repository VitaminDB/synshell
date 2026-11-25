//! Индексация медиатеки и поиск по смыслу (E1).

use std::collections::HashMap;

use serde_json::{json, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, MediaKind};
use crate::editor::Editor;
use crate::index::{self, MediaIndex, Vectors};

pub static COMMANDS: &[&Command] = &[&INDEX, &SEARCH];

static INDEX: Command = Command {
    id: "media.index",
    category: "Медиа",
    title: "Проиндексировать медиатеку",
    shortcut: "",
    keywords: "индекс индексация сцены склейки дикторы кто говорит бит темп музыка описание кадров поиск по смыслу",
    help: "Индекс файлов медиатеки для поиска и монтажа: расшифровка речи (если её нет), дикторы (кто когда говорит), сцены по склейкам с описанием кадра моделью со зрением, бит и темп музыки, векторы для поиска по смыслу (media.search). ids — медиа (по умолчанию все); captions false — без описаний сцен (быстрее); speakers false — без дикторов. Долго на длинных файлах: минуты.",
    args: &[arg("ids", Ty::Ids, "медиа (m…), по умолчанию все"), arg("captions", Ty::Bool, "описывать сцены (true)"), arg("speakers", Ty::Bool, "дикторы (true)"), arg("threshold", Ty::Num, "чувствительность склеек 0.1–0.6 (0.2)"), arg("model", Ty::Str, "модель со зрением для описаний")],
    danger: false,
    run: index_media,
};

fn caption(c: &mut synaptixd::Client, model: &str, path: &std::path::Path, at: f64) -> Result<String, String> {
    let png = std::env::temp_dir().join(format!("syn-video-editor-idx-{}-{}.png", std::process::id(), (at * 1000.0) as i64));
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-ss", &format!("{at:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale=640:-2"])
        .arg(&png)
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() || !png.exists() {
        return Err("кадр не извлёкся".into());
    }
    let mut m = synaptxd_msg(&png);
    m.content = format!("{}\nОпиши кадр одной короткой фразой по-русски (до 20 слов): кто или что в кадре, где, что происходит, крупность плана. Если это снимок экрана, интерфейс программы, текст на экране, титр или графика — начни с этого (например: «Интерфейс видеоредактора, в окне просмотра…»). Без вступлений.", synaptixd::MEDIA_MARK);
    let r = c.generate(model, vec![m], 120, Some(0.2), |_| {}).map_err(|e| format!("модель зрения: {e}"));
    let _ = std::fs::remove_file(&png);
    Ok(r?.text.trim().trim_matches('"').lines().next().unwrap_or("").trim().to_string())
}

fn synaptxd_msg(png: &std::path::Path) -> synaptixd::Message {
    let mut m = synaptixd::Message::user(String::new());
    m.media = vec![synaptixd::Media { kind: synaptixd::MediaKind::Image, path: png.display().to_string() }];
    m
}

fn index_media(ed: &mut Editor, a: &Args) -> Res {
    let want: Option<Vec<u64>> = a.ids("ids")?;
    let captions = a.bool("captions")?.unwrap_or(true);
    let speakers = a.bool("speakers")?.unwrap_or(true);
    let threshold = a.num("threshold")?.unwrap_or(0.2).clamp(0.05, 0.9);
    let media: Vec<crate::doc::Media> = ed.project.media.iter().filter(|m| want.as_ref().is_none_or(|w| w.contains(&m.id))).filter(|m| matches!(m.kind, MediaKind::Video | MediaKind::Audio)).cloned().collect();
    if media.is_empty() {
        return Err("нет видео или звука для индекса".into());
    }
    let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let vision = if captions { super::project::vision_model(&mut c, a.str("model")?).ok() } else { None };
    let mut lines = Vec::new();
    let mut done: Vec<(u64, MediaIndex, Option<crate::doc::Transcript>)> = Vec::new();
    for (n, m) in media.iter().enumerate() {
        syn_assist::tools::progress(n, media.len(), &format!("индекс {}", m.name()));
        let mut ix = MediaIndex::default();
        let mut notes = Vec::new();
        // речь
        let mut transcript = None;
        if m.has_audio || m.kind == MediaKind::Audio {
            if m.transcript.is_none() {
                match super::audio::transcribe_file(&m.path, None) {
                    Ok(t) => transcript = Some(t),
                    Err(e) => notes.push(format!("речь: {e}")),
                }
            }
            if speakers {
                let node = synaptix_flow::FlowNode::new(1, "diarize");
                let dir = std::env::temp_dir().join(format!("syn-video-editor-diar-{}", std::process::id()));
                let inputs = HashMap::from([("audio".to_string(), synaptix_flow::Val::File(m.path.clone()))]);
                match synaptixd::flow::run_node(&node, inputs, &dir, &synaptix_flow::Cancel::new(), &mut |_| {}) {
                    Ok((out, _)) => ix.speakers = index::parse_speakers(&out.get("json").map(|v| v.text()).unwrap_or_default()),
                    Err(e) => notes.push(format!("дикторы: {e}")),
                }
                let _ = std::fs::remove_dir_all(&dir);
            }
            match super::audio::pcm16k(&m.path.to_string_lossy()) {
                Ok(pcm) => {
                    let (b, bpm) = index::beats(&pcm, 16000.0);
                    ix.bpm = bpm;
                    if bpm.is_some() {
                        ix.beats = b.into_iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
                    }
                }
                Err(e) => notes.push(format!("звук: {e}")),
            }
        }
        // сцены
        if m.kind == MediaKind::Video {
            match index::scene_cuts(&m.path, threshold) {
                Ok(cuts) => ix.scenes = index::scenes_from_cuts(&cuts, m.duration),
                Err(e) => notes.push(format!("склейки: {e}")),
            }
            if let Some(model) = &vision {
                let total = ix.scenes.len().min(60);
                for (k, sc) in ix.scenes.iter_mut().take(60).enumerate() {
                    syn_assist::tools::progress(k, total, &format!("сцены {}", m.name()));
                    match caption(&mut c, model, &m.path, (sc.start + sc.end) / 2.0) {
                        Ok(t) => sc.caption = t,
                        Err(e) => {
                            notes.push(format!("описание сцен: {e}"));
                            break;
                        }
                    }
                }
            } else if captions {
                notes.push("описания сцен: нет модели со зрением".into());
            }
        }
        let who: std::collections::BTreeSet<&str> = ix.speakers.iter().map(|s| s.speaker.as_str()).collect();
        lines.push(format!(
            "m{} {}: сцен {}, дикторов {}, {}{}{}",
            m.id,
            m.name(),
            ix.scenes.len(),
            who.len(),
            match ix.bpm {
                Some(b) => format!("темп {b} уд/мин, ударов {}", ix.beats.len()),
                None => "бита нет".into(),
            },
            transcript.as_ref().map(|t| format!(", расшифровано слов {}", t.words.len())).unwrap_or_default(),
            if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) }
        ));
        for sc in ix.scenes.iter().take(12).filter(|s| !s.caption.is_empty()) {
            lines.push(format!("  {}–{} {}", tc(sc.start), tc(sc.end), sc.caption));
        }
        done.push((m.id, ix, transcript));
    }
    ed.begin("Индекс медиатеки");
    for (id, ix, tr) in done {
        if let Some(m) = ed.project.media.iter_mut().find(|m| m.id == id) {
            if tr.is_some() {
                m.transcript = tr;
            }
            m.index = Some(ix);
        }
    }
    // векторы для поиска — сразу (кэш у файла)
    let ids: Vec<u64> = media.iter().map(|m| m.id).collect();
    match ensure_vectors(ed, &mut c, &ids) {
        Ok(n) => lines.push(format!("Для поиска: кусков {n} (media.search)")),
        Err(e) => lines.push(format!("Поиск по смыслу недоступен: {e} — media.search найдёт по словам")),
    }
    Ok(Out::text(lines.join("\n")))
}

/// Векторы кусков медиа (из кэша или заново). Возвращает число кусков.
fn ensure_vectors(ed: &mut Editor, c: &mut synaptixd::Client, ids: &[u64]) -> Result<usize, String> {
    let mut total = 0;
    for id in ids {
        let Some(m) = ed.project.media(*id).cloned() else { continue };
        let chunks = index::chunks(&m);
        if chunks.is_empty() {
            continue;
        }
        let model = m.index.as_ref().map(|i| i.embed_model.clone()).filter(|s| !s.is_empty()).unwrap_or_default();
        let file = index::vectors_file(&m.path, &model);
        let cached: Option<Vectors> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok());
        if cached.as_ref().is_some_and(|v| v.chunks == chunks) {
            total += chunks.len();
            continue;
        }
        let texts: Vec<String> = chunks.iter().map(|ch| ch.text.clone()).collect();
        let e = c.embed(&model, &texts).map_err(|e| format!("эмбеддинги: {e}"))?;
        let v = Vectors { chunks: chunks.clone(), vectors: e.vectors };
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&file, serde_json::to_string(&v).unwrap_or_default()).map_err(|e| format!("{}: {e}", file.display()))?;
        total += chunks.len();
    }
    Ok(total)
}

static SEARCH: Command = Command {
    id: "media.search",
    category: "Медиа",
    title: "Найти в медиатеке по смыслу",
    shortcut: "",
    keywords: "найти поиск где момент сцена фраза сказал кто по смыслу медиатека",
    help: "Поиск моментов в медиатеке по смыслу: «где кот включает фонарь», «где говорят про цены» — по фразам расшифровки (с диктором) и описаниям сцен (после media.index или media.transcribe). Ответ — медиа, отрезок источника, что там; дальше clip.add {media, offset, duration} ставит этот кусок на таймлайн. limit — сколько показать (8).",
    args: &[req("query", Ty::Str, "что найти словами"), arg("limit", Ty::Num, "сколько показать (8)"), arg("ids", Ty::Ids, "искать только в этих медиа")],
    danger: false,
    run: search,
};

fn search(ed: &mut Editor, a: &Args) -> Res {
    let q = a.need_str("query")?;
    let limit = a.num("limit")?.unwrap_or(8.0).clamp(1.0, 50.0) as usize;
    let want: Option<Vec<u64>> = a.ids("ids")?;
    let ids: Vec<u64> = ed.project.media.iter().filter(|m| want.as_ref().is_none_or(|w| w.contains(&m.id))).map(|m| m.id).collect();
    let mut all: Vec<(index::Chunk, Option<Vec<f32>>)> = Vec::new();
    let mut c = synaptixd::Client::connect().ok();
    let semantic = c.as_mut().map(|c| ensure_vectors(ed, c, &ids).is_ok()).unwrap_or(false);
    for id in &ids {
        let Some(m) = ed.project.media(*id) else { continue };
        let chunks = index::chunks(m);
        let model = m.index.as_ref().map(|i| i.embed_model.clone()).unwrap_or_default();
        let v: Option<Vectors> = std::fs::read_to_string(index::vectors_file(&m.path, &model)).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(|v: &Vectors| v.chunks == chunks);
        match v {
            Some(v) => all.extend(v.chunks.into_iter().zip(v.vectors.into_iter().map(Some))),
            None => all.extend(chunks.into_iter().map(|ch| (ch, None))),
        }
    }
    if all.is_empty() {
        return Err("в медиатеке нечего искать — сначала media.index (или media.transcribe)".into());
    }
    let qv = if semantic { c.as_mut().and_then(|c| c.embed("", &[q.clone()]).ok()).and_then(|e| e.vectors.into_iter().next()) } else { None };
    let ql = q.to_lowercase();
    let words: Vec<&str> = ql.split_whitespace().filter(|w| w.chars().count() > 2).collect();
    let mut scored: Vec<(f32, &index::Chunk)> = all
        .iter()
        .map(|(ch, v)| {
            let t = ch.text.to_lowercase();
            // по словам: первые 5 букв (грубая основа — «фонарь» найдёт «фонарик»)
            let stem = |w: &str| -> String { w.chars().take(5).collect() };
            let lex = if words.is_empty() { 0.0 } else { words.iter().filter(|w| t.contains(&stem(w))).count() as f32 / words.len() as f32 };
            let sem = match (&qv, v) {
                (Some(q), Some(v)) => index::cosine(q, v),
                _ => 0.0,
            };
            (sem + 0.25 * lex, ch)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut lines = vec![format!("Поиск «{q}» ({}):", if qv.is_some() { "по смыслу" } else { "по словам — служба эмбеддингов недоступна" })];
    let mut data = Vec::new();
    for (s, ch) in scored.iter().take(limit).filter(|(s, _)| *s > 0.0) {
        let name = ed.project.media(ch.media).map(|m| m.name()).unwrap_or_default();
        lines.push(format!("m{} {} {}–{} [{}] {} ({:.2})", ch.media, name, tc(ch.start), tc(ch.end), ch.kind, ch.text, s));
        data.push(json!({"media": ch.media, "start": ch.start, "end": ch.end, "kind": ch.kind, "text": ch.text, "score": s}));
    }
    if data.is_empty() {
        lines.push("ничего похожего".into());
    }
    Ok(Out::with(lines.join("\n"), Value::Array(data)))
}
