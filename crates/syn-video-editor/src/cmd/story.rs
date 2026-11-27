//! Фильм сцена за сценой: план съёмки сцены по готовой озвучке и порядок.
//!
//! `scene.plan` — после озвучки сцены: длина сцены = большее из «нужно
//! действию» (сценарий) и «нужно речи» (реплики + паузы); сцена длиннее
//! предела ролика H3 делится на ролики только в тишине между репликами;
//! для каждого ролика — дорожки говорящих по его окну (губы), для всей
//! сцены — общая дорожка речи (на таймлайн целиком, без стыков). План —
//! в документ «раскадровка».
//!
//! Порядок — в коде: озвучка, план и съёмка сцены N+1 отказывают, пока сцена
//! N не снята (если пользователь не просил «все сцены разом» — `story.mode`).
//! Состояние — `<папка проекта>/story.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{arg, req, Args, Command as Cmd, Out, Ty};

pub static COMMANDS: &[&Cmd] = &[&PLAN, &MODE, &STATUS];

/// Долгая (ffmpeg по дорожкам) — без блокировки документа.
pub const HEAVY: &[&str] = &["scene.plan"];

/// Предел длины одного ролика MiniMax-H3, с (решение пользователя: 18 с при любом разрешении).
pub const SHOT_LIMIT: f64 = 18.0;
const LEAD: f64 = 0.4;
const GAP: f64 = 0.5;
const TAIL: f64 = 0.6;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Story {
    /// Все сцены разом (только по прямой просьбе пользователя).
    #[serde(default)]
    pub all_at_once: bool,
    /// Снятые сцены.
    #[serde(default)]
    pub done: BTreeSet<u32>,
    /// Планы сцен: число роликов.
    #[serde(default)]
    pub planned: BTreeMap<u32, usize>,
    /// Снято роликов по сценам.
    #[serde(default)]
    pub shot: BTreeMap<u32, usize>,
}

fn file(project: &Path) -> PathBuf {
    crate::store::media_dir(project).join("story.json")
}

pub fn load(project: Option<&Path>) -> Story {
    project.and_then(|p| std::fs::read_to_string(file(p)).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(project: &Path, s: &Story) {
    let f = file(project);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, serde_json::to_string_pretty(s).unwrap_or_default());
}

/// Номер сцены из «3», «Сцена 3», «Сцена 3 из 8: …».
pub fn scene_number(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("Сцена").or_else(|| t.strip_prefix("сцена")).unwrap_or(t).trim_start();
    let d: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// Можно ли работать со сценой `n`: снята каждая сцена до неё (или режим «все разом»).
pub fn gate(project: Option<&Path>, n: u32, what: &str) -> Result<(), String> {
    let Some(p) = project else { return Ok(()) };
    let s = load(Some(p));
    if s.all_at_once || n <= 1 {
        return Ok(());
    }
    if let Some(m) = (1..n).find(|m| !s.done.contains(m)) {
        return Err(format!(
            "{what} сцены {n} — рано: фильм делается сцена за сценой, сначала доведи и сними сцену {m} (её реплики → scene.plan → ref-scenes). Все сцены разом — только если пользователь прямо попросил (story.mode all_at_once)."
        ));
    }
    Ok(())
}

/// Сняты ещё `k` роликов сцены `n`; сцена снята, когда сняты все ролики её
/// плана (без плана — с первого успешного ref-scenes).
pub fn add_shots(project: Option<&Path>, n: u32, k: usize) {
    let Some(p) = project else { return };
    let mut s = load(Some(p));
    let got = s.shot.entry(n).or_insert(0);
    *got += k;
    if *got >= s.planned.get(&n).copied().unwrap_or(1) {
        s.done.insert(n);
    }
    save(p, &s);
}

/// Реплика сцены в плане (или её кусок в ролике).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub speaker: String,
    pub file: PathBuf,
    /// Начало звучащей части в файле (пустота в начале записи отрезана).
    pub offset: f64,
    /// Длина звучащей части.
    pub seconds: f64,
    /// Паузы внутри реплики (от её начала): где можно аккуратно разрезать.
    pub pauses: Vec<(f64, f64)>,
    /// Начало в сцене (у куска в ролике — от начала ролика).
    pub at: f64,
}

/// Ролик плана: окно сцены и куски реплик в нём.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub start: f64,
    pub seconds: f64,
    pub lines: Vec<Line>,
}

/// Раскладка сцены: реплики по порядку с паузами, длина — большее из
/// действия и речи, деление на ролики ≤ `limit` только в тишине — между
/// репликами или в паузе внутри длинной фразы (запись режется, а не
/// переозвучивается). Без единой паузы в пределе — разрез на границе предела.
pub fn layout(mut lines: Vec<Line>, action: f64, limit: f64) -> Result<(f64, Vec<Line>, Vec<Shot>), String> {
    let mut t = LEAD;
    for (i, l) in lines.iter_mut().enumerate() {
        if i > 0 {
            t += GAP;
        }
        l.at = t;
        t += l.seconds;
    }
    let speech = if lines.is_empty() { 0.0 } else { t + TAIL };
    let total = speech.max(action).max(2.0);
    // тишина: до первой реплики, между репликами, паузы внутри реплик, после последней
    let mut free: Vec<(f64, f64)> = Vec::new();
    let mut prev = 0.0;
    for l in &lines {
        free.push((prev, l.at));
        for &(a, b) in &l.pauses {
            free.push((l.at + a, l.at + b));
        }
        prev = l.at + l.seconds;
    }
    free.push((prev, total));
    let mut cuts = Vec::new();
    let mut start = 0.0;
    while total - start > limit + 1e-6 {
        let left = total - start;
        let k = (left / limit).ceil();
        let target = start + left / k;
        let mut best: Option<f64> = None;
        for &(a, b) in &free {
            let (lo, hi) = (a.max(start + 1.0), b.min(start + limit));
            if lo > hi {
                continue;
            }
            // точка паузы, ближайшая к цели (ролики примерно равной длины)
            let p = target.clamp(lo, hi);
            if best.is_none_or(|q| (p - target).abs() < (q - target).abs()) {
                best = Some(p);
            }
        }
        let cut = best.unwrap_or(start + limit);
        cuts.push(cut);
        start = cut;
    }
    let mut bounds = vec![0.0];
    bounds.extend(cuts);
    bounds.push(total);
    let out: Vec<Shot> = bounds
        .windows(2)
        .map(|w| {
            let (s, e) = (w[0], w[1]);
            let pieces = lines
                .iter()
                .filter_map(|l| {
                    let (a, b) = (l.at.max(s), (l.at + l.seconds).min(e));
                    (b - a > 0.05).then(|| Line { offset: l.offset + (a - l.at), seconds: b - a, at: a - s, pauses: Vec::new(), ..l.clone() })
                })
                .collect();
            Shot { start: s, seconds: e - s, lines: pieces }
        })
        .collect();
    Ok((total, lines, out))
}

/// Звучащая часть записи и паузы внутри (поиск тишины по энергии: ffmpeg
/// silencedetect, −38 дБ, от 0,18 с): (начало, конец, паузы от начала).
pub fn speech_span(p: &Path) -> Result<(f64, f64, Vec<(f64, f64)>), String> {
    let dur = media_seconds(p)?;
    let o = std::process::Command::new("ffmpeg")
        .args(["-v", "info", "-nostats", "-i"])
        .arg(p)
        .args(["-af", "silencedetect=noise=-38dB:d=0.18", "-f", "null", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let log = String::from_utf8_lossy(&o.stderr);
    let mut sil: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for line in log.lines() {
        if let Some(v) = line.split("silence_start: ").nth(1) {
            open = v.trim().parse().ok();
        } else if let Some(v) = line.split("silence_end: ").nth(1) {
            let end: f64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(dur);
            sil.push((open.take().unwrap_or(0.0).max(0.0), end.min(dur)));
        }
    }
    if let Some(s) = open {
        sil.push((s.max(0.0), dur));
    }
    let mut start = 0.0;
    let mut end = dur;
    if let Some(&(a, b)) = sil.first().filter(|x| x.0 <= 0.01) {
        let _ = a;
        start = b;
    }
    if let Some(&(a, b)) = sil.last().filter(|x| x.1 >= dur - 0.01) {
        let _ = b;
        end = a;
    }
    if end - start < 0.15 {
        return Err(format!("{}: в записи нет речи (одна тишина)", p.display()));
    }
    // запас 0,08 с по краям речи, чтобы не срезать начало и хвост слов
    let start = (start - 0.08).max(0.0);
    let end = (end + 0.08).min(dur);
    let pauses = sil.into_iter().filter(|&(a, b)| a > start + 0.05 && b < end - 0.05).map(|(a, b)| (a - start, b - start)).collect();
    Ok((start, end, pauses))
}

static PLAN: Cmd = Cmd {
    id: "scene.plan",
    category: "Озвучка",
    title: "План съёмки сцены",
    shortcut: "",
    keywords: "scene plan сцена план ролики длина дорожки раскадровка",
    help: "После озвучки сцены, до её съёмки: длина сцены = большее из `action_seconds` (сколько нужно действию по сценарию) и речи (реплики + паузы); сцена длиннее ролика H3 делится на ролики только в паузах между репликами (продолжение по латенту в одном ref-scenes с chain). Делает дорожки говорящих для каждого ролика и общую дорожку речи сцены, пишет план в документ «раскадровка». Сцена без речи — `lines` пустой, длина по действию. Ответ — ролики: окно, дорожки (их в refs как <Audio k> fully_copy), реплики.",
    args: &[
        req("scene", Ty::Num, "номер сцены"),
        arg("lines", Ty::Array, "реплики сцены по порядку: [{\"file\": путь из voice.say, \"speaker\": \"<герой>\"}]; пусто — сцена без речи"),
        arg("action_seconds", Ty::Num, "сколько секунд нужно действию по сценарию (сцены без речи — обязательно)"),
        arg("title", Ty::Str, "название сцены"),
    ],
    danger: false,
    run: |ed, a| {
        let project = ed.path.clone();
        let (text, files, data) = run_plan(a, project.as_deref())?;
        super::voice::import(ed, &files);
        Ok(Out::with(text, data))
    },
};

static MODE: Cmd = Cmd {
    id: "story.mode",
    category: "Озвучка",
    title: "Порядок съёмки фильма",
    shortcut: "",
    keywords: "story mode все сцены разом сцена за сценой порядок",
    help: "Порядок работы над фильмом: по умолчанию сцена за сценой (озвучка, план и съёмка сцены N+1 — только после съёмки сцены N). `all_at_once: true` — все сцены разом; ставить ТОЛЬКО если пользователь прямо попросил.",
    args: &[req("all_at_once", Ty::Bool, "все сцены разом (только по просьбе пользователя)")],
    danger: false,
    run: |ed, a| {
        let p = ed.path.clone().ok_or("проект ещё не сохранён")?;
        let mut s = load(Some(&p));
        s.all_at_once = a.bool("all_at_once")?.unwrap_or(false);
        save(&p, &s);
        Ok(Out::text(if s.all_at_once { "Порядок: все сцены разом (по просьбе пользователя)".to_string() } else { "Порядок: сцена за сценой".to_string() }))
    },
};

static STATUS: Cmd = Cmd {
    id: "story.status",
    category: "Озвучка",
    title: "Ход фильма",
    shortcut: "",
    keywords: "story status сцены сняты следующая",
    help: "Какие сцены сняты, какая следующая, порядок работы.",
    args: &[],
    danger: false,
    run: |ed, _a| {
        let s = load(ed.path.as_deref());
        let next = (1..).find(|n| !s.done.contains(n)).unwrap_or(1);
        let done: Vec<String> = s.done.iter().map(|n| n.to_string()).collect();
        Ok(Out::with(format!("Сняты сцены: {}; следующая — {next}; порядок: {}", if done.is_empty() { "нет".into() } else { done.join(", ") }, if s.all_at_once { "все разом" } else { "сцена за сценой" }), json!({"done": s.done, "next": next, "all_at_once": s.all_at_once})))
    },
};

/// Каркас строки Ref2VA для ролика: говорящие — (S1), (S2)… по порядку первой
/// реплики, их дорожки — `<Audio base+1…>` fully_copy, обязательная постановка
/// (камера, место и разворот каждого героя), правило речи. Агент заполняет <…>.
pub fn skeleton(shot: usize, tracks: &[Value], base: usize) -> String {
    let speakers: Vec<String> = tracks.iter().filter_map(|t| t["speaker"].as_str().map(str::to_string)).collect();
    let mut sd = String::from("subject_definitions: <Picture a> is <герой: кто, внешность>. <Picture b> is <второй герой>. <Picture c> is <локация>.");
    let mut ra = String::from("retention_analysis: <Picture a>: fully_preserved - <герой>'s appearance. <Picture b>: fully_preserved - <второй>'s appearance. <Picture c>: fully_preserved - the location; walls, doors, windows and props stay exactly as in the reference.");
    for (j, sp) in speakers.iter().enumerate() {
        let k = base + j + 1;
        sd.push_str(&format!(" <Audio {k}> is the exact dialogue track of {sp}."));
        ra.push_str(&format!(" <Audio {k}>: fully_copy - {sp}'s speech, copied exactly and lip-synced."));
    }
    let who: Vec<String> = speakers.iter().enumerate().map(|(j, sp)| format!("{sp} = (S{})", j + 1)).collect();
    let cont = if shot > 0 { " Continues seamlessly from the previous shot: same positions, lighting and props." } else { "" };
    format!(
        "{sd} summary: [reference generation]{} <суть ролика одной фразой>. {ra} detailed_description: <стиль>.{cont} Camera: <крупность>, <где стоит камера и куда смотрит, например: side view across the room, the hearth in the background>, static, the layout does not change. [Shot 1] Blocking: <Picture a> stands <слева/справа, у чего>, body in three-quarter profile turned toward <Picture b>; <Picture b> stands <слева/справа>, three-quarter profile turned toward <Picture a>; they look at each other and never at the camera. <действие без слов речи>.{} Each character moves the mouth only while their own audio is audible; otherwise the mouth stays closed. All speech comes only from the audio tracks; outside them nobody speaks, hums or makes vocal sounds. overall_soundscape: <шумы места и действий>. non_diegetic_music: <музыка или N/A>.",
        if speakers.is_empty() { "" } else { " + [audio reuse]" },
        if speakers.is_empty() { " No one speaks in this shot.".to_string() } else { format!(" Speakers ({}): each line as (Sk): <d>[Russian] <слова из диалогов></d>, in order of the audio.", who.join(", ")) }
    )
}

fn media_seconds(p: &Path) -> Result<f64, String> {
    let o = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p).output().map_err(|e| format!("ffprobe: {e}"))?;
    String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().map_err(|_| format!("{}: не звук", p.display()))
}

/// Дорожка: куски записей на своих местах, тишина вокруг, длина `secs`.
/// Кусок — (файл, начало в файле, длина, место в дорожке); края куска —
/// мягкие (10 мс), чтобы разрез в паузе не щёлкал.
pub fn make_track(lines: &[(PathBuf, f64, f64, f64)], secs: f64, out: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-t", &format!("{secs:.3}"), "-i", "anullsrc=r=44100:cl=mono"]);
    let mut filt = String::new();
    for (k, (f, off, len, at)) in lines.iter().enumerate() {
        cmd.arg("-i").arg(f);
        let n = k + 1;
        let fade = 0.01f64.min(len / 4.0);
        filt.push_str(&format!(
            "[{n}:a]aresample=44100,aformat=channel_layouts=mono,atrim=start={off:.4}:duration={len:.4},asetpts=PTS-STARTPTS,afade=t=in:d={fade:.3},afade=t=out:st={:.4}:d={fade:.3},adelay={}:all=1[a{n}];",
            (len - fade).max(0.0),
            (at.max(0.0) * 1000.0).round() as u64
        ));
    }
    let n = lines.len();
    let ins: String = (0..=n).map(|k| if k == 0 { "[0:a]".to_string() } else { format!("[a{k}]") }).collect();
    filt.push_str(&format!("{ins}amix=inputs={}:normalize=0:duration=first[o]", n + 1));
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let r = cmd.args(["-filter_complex", &filt, "-map", "[o]", "-ar", "44100", "-ac", "1"]).arg(out).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if !r.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&r.stderr).trim()));
    }
    Ok(())
}

fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>().trim_matches('_').chars().take(40).collect()
}

pub fn run_plan(a: &Args, project: Option<&Path>) -> Result<(String, Vec<PathBuf>, Value), String> {
    let n = a.num("scene")?.ok_or("нужен scene")? as u32;
    gate(project, n, "План")?;
    let action = a.num("action_seconds")?.unwrap_or(0.0).max(0.0);
    let raw = a.value("lines").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut lines = Vec::new();
    for l in &raw {
        let f = l.get("file").and_then(Value::as_str).ok_or("у реплики нужен file (путь из voice.say)")?;
        let file = crate::doc::expand(f);
        let speaker = l.get("speaker").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if speaker.is_empty() {
            return Err(format!("у реплики {} нужен speaker — кто говорит", file.display()));
        }
        let (a0, b0, pauses) = speech_span(&file)?;
        lines.push(Line { offset: a0, seconds: b0 - a0, pauses, speaker, file, at: 0.0 });
    }
    if lines.is_empty() && action <= 0.0 {
        return Err("сцена без речи — укажи action_seconds (сколько нужно действию по сценарию)".into());
    }
    let (total, placed, shots) = layout(lines, action, SHOT_LIMIT)?;
    let base = project.map(crate::store::media_dir).unwrap_or_else(std::env::temp_dir).join("speech");
    let mut files = Vec::new();
    // общая дорожка речи сцены — на таймлайн целиком (без стыков между роликами)
    let mix = if placed.is_empty() {
        None
    } else {
        let p = base.join(format!("сцена-{n}-речь.wav"));
        make_track(&placed.iter().map(|l| (l.file.clone(), l.offset, l.seconds, l.at)).collect::<Vec<_>>(), total, &p)?;
        files.push(p.clone());
        Some(p)
    };
    let mut shots_json = Vec::new();
    let mut md = format!("\n## Сцена {n}{}\n\nДлина {total:.1} с (действие {action:.1} с, речь {:.1} с), роликов {}.\n\n| Ролик | Окно сцены | Длина | Дорожки | Реплики |\n|---|---|---|---|---|\n", a.str("title")?.map(|t| format!(" — {t}")).unwrap_or_default(), placed.last().map(|l| l.at + l.seconds + TAIL).unwrap_or(0.0), shots.len());
    for (i, s) in shots.iter().enumerate() {
        let mut speakers: Vec<String> = s.lines.iter().map(|l| l.speaker.clone()).collect();
        speakers.dedup();
        let mut uniq: Vec<String> = Vec::new();
        for sp in speakers {
            if !uniq.contains(&sp) {
                uniq.push(sp);
            }
        }
        let mut tracks = Vec::new();
        for sp in &uniq {
            let p = base.join(format!("сцена-{n}-ролик-{}-{}.wav", i + 1, slug(sp)));
            make_track(&s.lines.iter().filter(|l| &l.speaker == sp).map(|l| (l.file.clone(), l.offset, l.seconds, l.at)).collect::<Vec<_>>(), s.seconds, &p)?;
            files.push(p.clone());
            tracks.push(json!({"speaker": sp, "file": p}));
        }
        let said: Vec<String> = s.lines.iter().map(|l| format!("{} @{:.1}", l.speaker, l.at)).collect();
        md.push_str(&format!(
            "| {} | {:.1}–{:.1} с | {:.1} с | {} | {} |\n",
            i + 1,
            s.start,
            s.start + s.seconds,
            s.seconds,
            tracks.iter().filter_map(|t| t["file"].as_str()).map(|f| Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>().join(", "),
            if said.is_empty() { "без речи".into() } else { said.join("; ") }
        ));
        shots_json.push(json!({"shot": i + 1, "start": s.start, "seconds": s.seconds, "tracks": tracks, "lines": s.lines.iter().map(|l| json!({"speaker": l.speaker, "at": l.at, "seconds": l.seconds, "file": l.file})).collect::<Vec<_>>()}));
    }
    if let Some(p) = project {
        // раскадровка: раздел сцены заменяется новым планом
        let doc = crate::store::media_dir(p).join("раскадровка.md");
        let old = std::fs::read_to_string(&doc).unwrap_or_else(|_| "# Раскадровка — план съёмки по сценам\n".into());
        let head = format!("\n## Сцена {n}");
        let kept = match old.find(&head) {
            Some(i) => {
                let rest = &old[i + head.len()..];
                let end = rest.find("\n## Сцена ").map(|j| i + head.len() + j).unwrap_or(old.len());
                format!("{}{}", &old[..i], &old[end..])
            }
            None => old,
        };
        let _ = std::fs::write(&doc, format!("{}{md}", kept.trim_end()));
        let mut s = load(Some(p));
        s.planned.insert(n, shots.len());
        s.shot.remove(&n);
        s.done.remove(&n);
        save(p, &s);
    }
    let summary: Vec<String> = shots
        .iter()
        .enumerate()
        .map(|(i, s)| format!("ролик {}: {:.1} с, реплик {}", i + 1, s.seconds, s.lines.len()))
        .collect();
    // готовая нумерация звука: refs = [картинки…, затем дорожки роликов по порядку]
    let mut k = 0;
    let mut numbering = Vec::new();
    let mut skeletons: Vec<String> = Vec::new();
    for (i, sj) in shots_json.iter().enumerate() {
        let tr = sj["tracks"].as_array().cloned().unwrap_or_default();
        let parts: Vec<String> = tr
            .iter()
            .map(|t| {
                k += 1;
                format!("<Audio {k}> — {} ({})", t["speaker"].as_str().unwrap_or(""), Path::new(t["file"].as_str().unwrap_or("")).file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default())
            })
            .collect();
        numbering.push(format!("ролик {} (строка {} storyboard): {}", i + 1, i + 1, if parts.is_empty() { "без речи — ни одного <Audio>".into() } else { parts.join(", ") }));
        skeletons.push(skeleton(i, &tr, k - tr.len()));
    }
    Ok((
        format!(
            "Каркасы строк storyboard по роликам (заполни <…>, остальное не меняй; герои — <Picture k> по порядку картинок в refs):\n{}\n\nНумерация звука, если в refs после картинок идут дорожки роликов по порядку: {}. В строке ролика упоминай ТОЛЬКО его <Audio k> (не больше 3). Сцена {n}: {total:.1} с, роликов {} ({}). Дорожки — в refs как <Audio k> fully_copy, по ролику; ролики сцены — строками одного ref-scenes с chain (без [CUT] между ними); общая речь сцены{} — на таймлайн под ролики целиком. План — в документе «раскадровка».",
            skeletons.iter().enumerate().map(|(i, s)| format!("ролик {}: {s}", i + 1)).collect::<Vec<_>>().join("\n"),
            numbering.join("; "),
            shots.len(),
            summary.join("; "),
            mix.as_ref().map(|m| format!(" ({})", m.display())).unwrap_or_default()
        ),
        files,
        json!({"scene": n, "seconds": total, "shots": shots_json, "speech": mix, "audio_numbering": numbering, "skeletons": skeletons}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(sp: &str, s: f64) -> Line {
        Line { speaker: sp.into(), file: PathBuf::from(format!("/{sp}.wav")), offset: 0.0, seconds: s, pauses: Vec::new(), at: 0.0 }
    }

    #[test]
    fn short_scene_one_shot_action_wins() {
        let (total, placed, shots) = layout(vec![l("A", 2.0), l("B", 2.0)], 9.0, 18.0).unwrap();
        assert_eq!(total, 9.0);
        assert_eq!(shots.len(), 1);
        assert!((placed[1].at - (0.4 + 2.0 + 0.5)).abs() < 1e-9);
    }

    #[test]
    fn long_scene_splits_between_lines() {
        let lines = vec![l("A", 4.0), l("B", 5.0), l("A", 6.0), l("B", 4.0), l("A", 5.0), l("B", 6.0)];
        let (total, placed, shots) = layout(lines, 0.0, 18.0).unwrap();
        assert!(total > 18.0 && shots.len() >= 2, "{total} {}", shots.len());
        for s in &shots {
            assert!(s.seconds <= 18.0 + 1e-6);
        }
        // ни одна реплика не разрезана: сумма кусков = число реплик
        let n: usize = shots.iter().map(|s| s.lines.len()).sum();
        assert_eq!(n, placed.len());
    }

    #[test]
    fn long_line_is_cut_in_its_pause() {
        let mut long = l("A", 26.0);
        long.offset = 0.3;
        long.pauses = vec![(12.0, 12.4)];
        let (_, _, shots) = layout(vec![long], 0.0, 18.0).unwrap();
        assert_eq!(shots.len(), 2);
        let cut = shots[1].start;
        assert!(cut >= 0.4 + 12.0 && cut <= 0.4 + 12.4, "{cut}");
        // второй кусок продолжает запись с того же места
        let p = &shots[1].lines[0];
        assert!((p.offset - (0.3 + cut - 0.4)).abs() < 1e-6);
        assert!((shots[0].lines[0].seconds + p.seconds - 26.0).abs() < 1e-6);
    }

    #[test]
    fn silent_scene() {
        let (total, _, shots) = layout(vec![], 30.0, 18.0).unwrap();
        assert_eq!(total, 30.0);
        assert_eq!(shots.len(), 2);
    }

    #[test]
    fn scene_done_after_all_planned_shots() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("Ф").join("Ф.syn");
        std::fs::create_dir_all(proj.parent().unwrap()).unwrap();
        let mut st = Story::default();
        st.planned.insert(1, 2);
        save(&proj, &st);
        add_shots(Some(&proj), 1, 1);
        assert!(!load(Some(&proj)).done.contains(&1));
        assert!(gate(Some(&proj), 2, "Озвучка").is_err());
        add_shots(Some(&proj), 1, 1);
        assert!(load(Some(&proj)).done.contains(&1));
        assert!(gate(Some(&proj), 2, "Озвучка").is_ok());
        assert!(gate(Some(&proj), 3, "Озвучка").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(scene_number("Сцена 3 из 8: x"), Some(3));
        assert_eq!(scene_number("12"), Some(12));
        assert_eq!(scene_number("сцена-4"), None);
    }
}
