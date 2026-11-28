//! Озвучка героев (служба synaptixd): кастинг голоса героя (Drama 3 по
//! описанию в [скобках]), реплики его голосом (клон по пробе), дорожка сцены
//! (реплики на своих местах, вокруг тишина — для синхронизации губ H3 через
//! `<Audio N>`). Seed-VC (переозвучка записи) пока не в работе — решение
//! пользователя 27.11.2026.
//! Голоса проекта — `<проект>.media/voices/voices.json`, файлы речи —
//! `<проект>.media/speech/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{arg, req, Args, Command as Cmd, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&CAST, &VOICES, &SAY, &TRACK, &MUSIC];

/// Долгие команды (синтез минутами) — агент зовёт их без блокировки документа.
pub const HEAVY: &[&str] = &["voice.cast", "voice.say", "voice.track", "music.make"];

pub const TTS_MODEL: &str = "fish-drama-3";

/// Голос героя: проба и её текст (для клона), зерно.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Voice {
    pub audio: PathBuf,
    pub text: String,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub model: String,
    /// Как задан голос (описание из кастинга).
    #[serde(default)]
    pub description: String,
}

fn base_dir(project: Option<&Path>) -> PathBuf {
    match project {
        Some(p) => crate::store::media_dir(p),
        None => std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/voice"),
    }
}

fn bank_file(project: Option<&Path>) -> PathBuf {
    base_dir(project).join("voices/voices.json")
}

pub fn load_voices(project: Option<&Path>) -> BTreeMap<String, Voice> {
    std::fs::read_to_string(bank_file(project)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_voices(project: Option<&Path>, v: &BTreeMap<String, Voice>) -> Result<(), String> {
    let f = bank_file(project);
    std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&f, serde_json::to_string_pretty(v).unwrap_or_default()).map_err(|e| format!("{}: {e}", f.display()))
}

/// Текст пробы без указаний: `[…]`, `<…>`, `<|speaker:N|>` — для `ref_text`.
pub fn plain_text(s: &str) -> String {
    let mut out = String::new();
    let mut depth_sq = 0;
    let mut in_angle = false;
    for c in s.chars() {
        match c {
            '[' => depth_sq += 1,
            ']' if depth_sq > 0 => depth_sq -= 1,
            '<' => in_angle = true,
            '>' if in_angle => in_angle = false,
            _ if depth_sq == 0 && !in_angle => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").replace(" ,", ",").replace(" .", ".").replace(" !", "!").replace(" ?", "?")
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    t.trim_matches('_').chars().take(40).collect()
}

fn write_wav(path: &Path, pcm: &[f32], rate: u32) -> Result<(), String> {
    let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
    let data_len = (pcm.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &x in pcm {
        wav.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, wav).map_err(|e| format!("{}: {e}", path.display()))
}

/// Доля слов реплики, услышанных распознаванием (без регистра, ё = е, без знаков).
pub fn word_match(expected: &str, heard: &str) -> f64 {
    let norm = |s: &str| -> Vec<String> {
        s.to_lowercase().replace('ё', "е").split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
    };
    let exp = norm(expected);
    if exp.is_empty() {
        return 1.0;
    }
    let mut pool = norm(heard);
    let mut hit = 0;
    for w in &exp {
        if let Some(i) = pool.iter().position(|h| h == w) {
            pool.remove(i);
            hit += 1;
        }
    }
    hit as f64 / exp.len() as f64
}

/// Слоги реплики (гласные русского и латиницы) — мера её естественной длины.
pub fn syllables(text: &str) -> usize {
    text.to_lowercase().chars().filter(|c| "аеёиоуыэюяaeiouy".contains(*c)).count()
}

/// Сколько секунд реплика может звучать при живой игре: ~3 слога/с плюс
/// запас на вступление, плюс по 1,5 с на каждое указание в [скобках]
/// (вздох, пауза, смех — часть игры).
pub fn max_speech_seconds(text_with_tags: &str) -> f64 {
    let tags = text_with_tags.matches('[').count() as f64;
    syllables(&plain_text(text_with_tags)) as f64 / 3.0 + 2.5 + tags * 1.5
}

/// Проверка дубля Drama 3 сверх совпадения слов: аномальная длина (модель
/// тянет, повторяет, мычит) и лишние слова в распознанном. `None` — годен.
pub fn take_problem(text_with_tags: &str, heard: &str, speech: f64) -> Option<String> {
    let max = max_speech_seconds(text_with_tags);
    if speech > max {
        return Some(format!("дубль аномально длинный: звучит {speech:.1} с на {} слогов (живая игра — до {max:.1} с) — переозвучь другим seed", syllables(&plain_text(text_with_tags))));
    }
    let words = |s: &str| s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).count();
    let (exp, got) = (words(&plain_text(text_with_tags)), words(heard));
    if exp > 0 && got as f64 > exp as f64 * 1.3 + 1.0 {
        return Some(format!("в дубле лишние слова или повторы (распознано {got} слов вместо {exp}) — переозвучь другим seed"));
    }
    None
}

/// Синтез службой: (pcm, частота, зерно).
fn synth(model: &str, text: &str, params: &synaptixd::Params) -> Result<(Vec<f32>, u32, Option<u64>), String> {
    let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let sp = c.speak_fresh(model, text, params).map_err(|e| format!("{model}: {e}"))?;
    Ok((sp.audio.f32s().to_vec(), sp.sample_rate, sp.seed))
}

fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let mut k = 1;
    loop {
        let p = dir.join(format!("{stem}-{k:02}.wav"));
        if !p.exists() {
            return p;
        }
        k += 1;
    }
}

// ── команды ────────────────────────────────────────────────────────────

static CAST: Cmd = Cmd {
    id: "voice.cast",
    category: "Озвучка",
    title: "Голос героя (кастинг)",
    shortcut: "",
    keywords: "voice cast голос героя кастинг озвучка drama персонаж",
    help: "Подобрать голос героя: Drama 3 произносит пробу (8–15 с характерной речи героя) с описанием голоса в [скобках] после первой фразы, например «<первая фраза> [<описание голоса: тембр, высота, темп, характер>] <ещё фразы>». Проба становится голосом героя `name`: все его реплики (voice.say) — клоном с неё. Повтор с другим seed — другой голос. Навык drama3.",
    args: &[
        req("name", Ty::Str, "имя героя"),
        req("text", Ty::Str, "проба: реплики героя по-русски с [описанием голоса] и звуками"),
        arg("seed", Ty::Num, "зерно (повторяемость; другое — другой голос)"),
        arg("model", Ty::Str, "модель (по умолчанию fish-drama-3)"),
    ],
    danger: false,
    run: |ed, a| heavy(ed, "voice.cast", a),
};

static VOICES: Cmd = Cmd {
    id: "voice.list",
    category: "Озвучка",
    title: "Голоса героев",
    shortcut: "",
    keywords: "voices голоса героев список",
    help: "Голоса героев проекта (после voice.cast): имя, файл пробы, её текст, зерно.",
    args: &[],
    danger: false,
    run: list,
};

static SAY: Cmd = Cmd {
    id: "voice.say",
    category: "Озвучка",
    title: "Реплика голосом героя",
    shortcut: "",
    keywords: "say tts реплика озвучить голос drama диалог",
    help: "Озвучить реплику (Drama 3): `voice` — имя героя (клон его пробы из voice.cast) или пусто; эмоции и звуки — в [скобках] по-английски, парные <whisper>…</whisper>; текст с буквой ё. Файл — в медиатеку. Диалог одной записью: `<|speaker:0|> … <|speaker:1|> …` и `voices` — имена героев по порядку. Проверь media_transcribe.",
    args: &[
        req("text", Ty::Str, "реплика"),
        arg("voice", Ty::Str, "имя героя (голос из voice.cast)"),
        arg("voices", Ty::Array, "диалог: имена героев для <|speaker:0..|> по порядку"),
        arg("seed", Ty::Num, "зерно"),
        arg("scene", Ty::Str, "сцена («3» или «Сцена 3: <название>») — реплика попадёт в документ «диалоги» проекта"),
        arg("name", Ty::Str, "имя файла"),
        arg("temperature", Ty::Num, "температура (1.0)"),
    ],
    danger: false,
    run: |ed, a| heavy(ed, "voice.say", a),
};

static MUSIC: Cmd = Cmd {
    id: "music.make",
    category: "Озвучка",
    title: "Музыка к фильму",
    shortcut: "",
    keywords: "music музыка тема саундтрек score minimax песня",
    help: "Сочинить музыку службой (модель музыки из настроек, по умолчанию MiniMax Music 3): `prompt` по-английски — жанр, настроение, инструменты, темп; `seconds` — длина (обычно длина фильма или его части); `lyrics` — слова песни (пусто — без слов, для фона под речь). Файл — в папку проекта music/ и в медиатеку; на таймлайн — clip_add на свою звуковую дорожку, под речью громкость 0.2–0.3, с нарастанием и затуханием.",
    args: &[
        req("prompt", Ty::Str, "описание музыки по-английски: жанр, настроение, инструменты, темп"),
        arg("seconds", Ty::Num, "длина, с"),
        arg("lyrics", Ty::Str, "слова песни (пусто — инструментал)"),
        arg("name", Ty::Str, "имя файла (тема, финал…)"),
        arg("model", Ty::Str, "модель музыки (пусто — из настроек)"),
        arg("seed", Ty::Num, "зерно"),
    ],
    danger: false,
    run: |ed, a| heavy(ed, "music.make", a),
};

static TRACK: Cmd = Cmd {
    id: "voice.track",
    category: "Озвучка",
    title: "Дорожка сцены",
    shortcut: "",
    keywords: "track дорожка сцены реплики тишина lipsync audio",
    help: "Дорожка сцены для синхронизации губ (H3 Ref2VA, <Audio N> fully_copy): реплики на своих местах (`lines`: [{file или media, at — секунда}]), вокруг тишина, длина `seconds`. Делай по дорожке на каждого говорящего, одинаковой длины (длина сцены = длина звука, до 15 с).",
    args: &[
        req("seconds", Ty::Num, "длина дорожки, с (до 15)"),
        req("lines", Ty::Array, "реплики: [{\"file\": путь или \"media\": m…, \"at\": секунда}]"),
        arg("name", Ty::Str, "имя файла (сцена-3-<герой>)"),
    ],
    danger: false,
    run: |ed, a| heavy(ed, "voice.track", a),
};

fn list(ed: &mut Editor, _a: &Args) -> Res {
    let v = load_voices(ed.path.as_deref());
    if v.is_empty() {
        return Ok(Out::text("Голосов героев нет — voice.cast".to_string()));
    }
    let lines: Vec<String> = v.iter().map(|(n, x)| format!("{n}: {} — «{}»{}", x.audio.display(), x.text, x.seed.map(|s| format!(", зерно {s}")).unwrap_or_default())).collect();
    Ok(Out::with(lines.join("\n"), json!(v)))
}

/// Из окна/CLI: долгая часть под уже взятой блокировкой, затем импорт.
fn heavy(ed: &mut Editor, id: &str, a: &Args) -> Res {
    let project = ed.path.clone();
    let (text, files, data) = run_unlocked(id, a, project.as_deref(), Some(ed))?;
    import(ed, &files);
    Ok(Out::with(text, data))
}

/// Файлы — в медиатеку (если их там нет).
pub fn import(ed: &mut Editor, files: &[PathBuf]) {
    // заменённые дубли (файла больше нет) — из медиатеки убрать
    let gone: Vec<u64> = ed.project.media.iter().filter(|m| m.kind == crate::doc::MediaKind::Audio && !m.path.exists() && m.path.parent().is_some_and(|d| d.ends_with("speech"))).map(|m| m.id).collect();
    for id in gone {
        let _ = super::run(ed, "media.remove", &json!({"id": id}));
    }
    let have: std::collections::HashSet<PathBuf> = ed.project.media.iter().map(|m| m.path.clone()).collect();
    let fresh: Vec<String> = files.iter().filter(|f| f.exists() && !have.contains(*f)).map(|f| f.display().to_string()).collect();
    if !fresh.is_empty() {
        let _ = super::run(ed, "media.import", &json!({"paths": fresh}));
    }
}

/// Долгая часть команды без документа: (ответ, файлы для медиатеки, данные).
/// `ed` — только чтобы найти файлы медиатеки по m… (из агента — None, пути).
pub fn run_unlocked(id: &str, a: &Args, project: Option<&Path>, ed: Option<&Editor>) -> Result<(String, Vec<PathBuf>, Value), String> {
    let dir = base_dir(project);
    match id {
        "music.make" => {
            let prompt = a.need_str("prompt")?;
            let lyrics = a.str("lyrics")?.unwrap_or_default();
            let model = a.str("model")?.filter(|m| !m.trim().is_empty()).or_else(|| crate::settings::load().model_opt("music")).unwrap_or_else(|| "minimax-music3".into());
            let seconds = a.num("seconds")?.map(|s| s.clamp(5.0, 600.0).round() as u32);
            let seed = a.num("seed")?.map(|s| s as u64);
            syn_assist::tools::progress(0, 0, &format!("музыка ({model})"));
            let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
            let (reply, wav) = c
                .music(&model, &prompt, &lyrics, seconds, seed, |s, t| syn_assist::tools::progress(s, t, "музыка"))
                .map_err(|e| format!("{model}: {e}"))?;
            let stem = a.str("name")?.filter(|n| !n.trim().is_empty()).map(|n| slug(&n)).unwrap_or_else(|| "музыка".into());
            let out = free_path(&dir.join("music"), &stem);
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(&out, &wav).map_err(|e| format!("{}: {e}", out.display()))?;
            let secs = super::story::media_seconds(&out).unwrap_or(0.0);
            Ok((
                format!("Музыка: {} ({secs:.1} с, {model}). На таймлайн — clip_add на отдельную звуковую дорожку; под речью громкость 0.2–0.3, нарастание и затухание (effect_add audio_fade_in/audio_fade_out). Или передай путь в cinematic_montage параметром music.", out.display()),
                vec![out.clone()],
                json!({"path": out, "seconds": secs, "model": model, "seed": reply.seed}),
            ))
        }
        "voice.cast" => {
            let name = a.need_str("name")?;
            let text = crate::yo::yofy(&a.need_str("text")?);
            let model = a.str("model")?.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| TTS_MODEL.into());
            let mut p = synaptixd::Params::default();
            p.set("seed", a.num("seed")?.map(|s| (s as i64).into()));
            syn_assist::tools::progress(0, 0, &format!("голос «{name}» (speak)"));
            let (pcm, rate, seed) = synth(&model, &text, &p)?;
            let out = free_path(&dir.join("voices"), &slug(&name));
            write_wav(&out, &pcm, rate)?;
            let secs = pcm.len() as f64 / rate.max(1) as f64;
            let mut bank = load_voices(project);
            let desc: Vec<String> = text.split('[').skip(1).filter_map(|x| x.split(']').next()).map(str::to_string).collect();
            bank.insert(name.clone(), Voice { audio: out.clone(), text: plain_text(&text), seed, model, description: desc.join("; ") });
            save_voices(project, &bank)?;
            Ok((format!("Голос «{name}»: {} ({secs:.1} с{}) — теперь реплики героя этим голосом (voice.say voice=\"{name}\"). Проверь на слух/media_transcribe; не тот — voice.cast с другим seed или описанием.", out.display(), seed.map(|s| format!(", зерно {s}")).unwrap_or_default()), vec![out.clone()], json!({"path": out, "seconds": secs, "seed": seed})))
        }
        "voice.say" => {
            let text = crate::yo::yofy(&a.need_str("text")?);
            // без слов (только «…» и указания в скобках) Drama 3 выдаёт мусор
            if !plain_text(&text).chars().any(char::is_alphanumeric) {
                return Err("в реплике нет слов для озвучки (только знаки или указания в скобках) — молчание делай паузой в сцене, а не репликой".into());
            }
            if let Some(n) = a.str("scene")?.and_then(|s| super::story::scene_number(&s)) {
                super::story::gate(project, n, "Озвучка")?;
            }
            let bank = load_voices(project);
            let mut names: Vec<String> = a.strs("voices")?.unwrap_or_default();
            if let Some(v) = a.str("voice")?.filter(|v| !v.trim().is_empty()) {
                names.insert(0, v);
            }
            names.dedup();
            let mut p = synaptixd::Params::default();
            if !names.is_empty() {
                let mut aud = Vec::new();
                let mut txt = Vec::new();
                for n in &names {
                    let v = bank.get(n).ok_or_else(|| format!("нет голоса «{n}» — сначала voice.cast (есть: {})", bank.keys().cloned().collect::<Vec<_>>().join(", ")))?;
                    aud.push(v.audio.display().to_string());
                    txt.push(v.text.clone());
                }
                p.set("ref_audio", Some(aud.join("\n").into()));
                p.set("ref_text", Some(txt.join("\n").into()));
            }
            p.set("seed", a.num("seed")?.map(|s| (s as i64).into()));
            p.set("temperature", a.num("temperature")?.map(Into::into));
            let who = names.first().cloned().unwrap_or_else(|| "речь".into());
            syn_assist::tools::progress(0, 0, &format!("реплика: {who} (speak)"));
            let (pcm, rate, seed) = synth(TTS_MODEL, &text, &p)?;
            let stem = a.str("name")?.filter(|n| !n.trim().is_empty()).map(|n| slug(&n)).unwrap_or_else(|| slug(&who));
            // новый дубль той же реплики (то же имя) заменяет прежний — дубли
            // не копятся в медиатеке и в «диалогах»
            let mut replaced: Vec<String> = Vec::new();
            if a.str("name")?.is_some_and(|n| !n.trim().is_empty()) {
                if let Ok(rd) = std::fs::read_dir(dir.join("speech")) {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().into_owned();
                        let base = n.trim_end_matches(".wav");
                        if base.rsplit_once('-').is_some_and(|(st, k)| st == stem && k.chars().all(|c| c.is_ascii_digit())) {
                            let _ = std::fs::remove_file(e.path());
                            replaced.push(n);
                        }
                    }
                }
            }
            let out = free_path(&dir.join("speech"), &stem);
            write_wav(&out, &pcm, rate)?;
            let secs = pcm.len() as f64 / rate.max(1) as f64;
            // речь без тишины по краям (её обрежет scene.plan) и проверка
            // распознаванием: совпадают ли слова — единственная причина переозвучки
            let speech = super::story::speech_span(&out).map(|(a0, b0, _)| b0 - a0).unwrap_or(secs);
            let heard = super::audio::transcribe_file(&out, Some("ru")).map(|t| t.text).unwrap_or_default();
            let score = word_match(&plain_text(&text), &heard);
            let problem = if heard.is_empty() { None } else { take_problem(&text, &heard, speech) };
            let verdict = if heard.is_empty() {
                "распознать не удалось — проверь на слух".to_string()
            } else if let (Some(p), true) = (&problem, score >= 0.8) {
                format!("распознано «{heard}» — слова совпадают, но {p}")
            } else if score >= 0.8 {
                format!("распознано «{heard}» — совпадает ({:.0}%), дубль годен, переозвучивать не нужно", score * 100.0)
            } else {
                format!("распознано «{heard}» — совпадает только {:.0}%: переозвучь (другой seed)", score * 100.0)
            };
            // каждая озвученная реплика — строкой в «диалоги.md» проекта:
            // сцена, кто, что, файл, длина (пользователь читает и правит)
            if let Some(proj) = project {
                let scene = a.str("scene")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into());
                let doc = crate::store::media_dir(proj).join("диалоги.md");
                let mut body = std::fs::read_to_string(&doc).unwrap_or_default();
                if !body.contains("| Сцена | Кто |") {
                    if !body.is_empty() && !body.ends_with('\n') {
                        body.push('\n');
                    }
                    body.push_str("\n## Озвучка\n\n| Сцена | Кто | Реплика | Файл | Длина |\n|---|---|---|---|---|\n");
                }
                let file = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                // строки прежних дублей этой реплики — убрать
                if !replaced.is_empty() || body.contains(&format!("| {file} |")) {
                    body = body.lines().filter(|l| !replaced.iter().any(|r| l.contains(&format!("| {r} |"))) && !l.contains(&format!("| {file} |"))).collect::<Vec<_>>().join("\n") + "\n";
                }
                body.push_str(&format!("| {scene} | {who} | {} | {file} | {secs:.1} с |\n", text.replace('|', "/").replace('\n', " ")));
                let _ = std::fs::write(&doc, body);
            }
            Ok((
                format!(
                    "Реплика «{}» ({who}): {} — {secs:.1} с (звучит {speech:.1} с; вздохи, смех и паузы Drama 3 в пределах живой игры сохраняются; пустую тишину по краям обрежет scene.plan){}{}. {verdict}",
                    plain_text(&text),
                    out.display(),
                    seed.map(|s| format!(", зерно {s}")).unwrap_or_default(),
                    if replaced.is_empty() { String::new() } else { format!(", заменён прежний дубль {}", replaced.join(", ")) }
                ),
                vec![out.clone()],
                json!({"path": out, "seconds": secs, "speech_seconds": speech, "seed": seed, "heard": heard, "match": score, "replaced": replaced}),
            ))
        }
        "voice.track" => {
            if let Some(n) = a.str("name")?.and_then(|s| s.to_lowercase().strip_prefix("сцена-").and_then(|r| r.split('-').next().and_then(|d| d.parse::<u32>().ok()))) {
                super::story::gate(project, n, "Дорожка")?;
            }
            let secs = a.num("seconds")?.ok_or("нужен seconds")?.clamp(0.5, 30.0);
            let lines = a.value("lines").and_then(|v| v.as_array().cloned()).ok_or("нужен lines: [{file|media, at}]")?;
            let stem = a.str("name")?.filter(|n| !n.trim().is_empty()).map(|n| slug(&n)).unwrap_or_else(|| "дорожка".into());
            let out = free_path(&dir.join("speech"), &stem);
            // ffmpeg: каждая реплика — задержка на `at`, смесь, тишина до длины
            let mut cmd = std::process::Command::new("ffmpeg");
            cmd.args(["-v", "error", "-y", "-f", "lavfi", "-t", &format!("{secs:.3}"), "-i", "anullsrc=r=44100:cl=mono"]);
            let mut filt = String::new();
            let mut n = 0;
            for l in &lines {
                let file = match (l.get("file").and_then(Value::as_str), l.get("media")) {
                    (Some(f), _) => crate::doc::expand(f),
                    (None, Some(m)) => {
                        let id = m.as_str().map(|s| s.trim_start_matches('m').to_string()).or_else(|| m.as_u64().map(|x| x.to_string())).unwrap_or_default();
                        let id: u64 = id.parse().map_err(|_| "media: m…".to_string())?;
                        ed.and_then(|e| e.project.media(id)).map(|m| m.path.clone()).ok_or_else(|| format!("нет медиа m{id}"))?
                    }
                    _ => return Err("у реплики нужен file или media".into()),
                };
                let at = l.get("at").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
                cmd.arg("-i").arg(&file);
                n += 1;
                filt.push_str(&format!("[{n}:a]aresample=44100,aformat=channel_layouts=mono,adelay={}:all=1[a{n}];", (at * 1000.0).round() as u64));
            }
            if n == 0 {
                return Err("lines пуст".into());
            }
            let ins: String = (0..=n).map(|k| if k == 0 { "[0:a]".to_string() } else { format!("[a{k}]") }).collect();
            filt.push_str(&format!("{ins}amix=inputs={}:normalize=0:duration=first[o]", n + 1));
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let r = cmd.args(["-filter_complex", &filt, "-map", "[o]", "-ar", "44100", "-ac", "1"]).arg(&out).output().map_err(|e| format!("ffmpeg: {e}"))?;
            if !r.status.success() {
                return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&r.stderr).trim()));
            }
            Ok((format!("Дорожка {} — {secs:.1} с, реплик {n}", out.display()), vec![out.clone()], json!({"path": out, "seconds": secs})))
        }
        other => Err(format!("{other}: не команда озвучки")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn plain() {
        assert_eq!(super::plain_text("Ну что за шум? [deep, raspy voice] Опять <whisper>лягушки</whisper>! [sighs] Ладно."), "Ну что за шум? Опять лягушки! Ладно.");
        assert_eq!(super::plain_text("<|speaker:0|> Привет"), "Привет");
        assert!(super::word_match("Суп? У меня и без того полный набор…", "Суп у меня и без того полный набор") > 0.99);
        assert!(super::word_match("Ещё немного!", "еще немного") > 0.99);
        assert!(super::word_match("Король объявил конкурс", "Король объявил") < 0.8);
    }
}

#[cfg(test)]
mod take_tests {
    use super::*;

    #[test]
    fn long_take_and_repeats_are_flagged() {
        // две слова растянуты на 10,7 с — переозвучить
        assert!(take_problem("Суп побеждает!", "суп побеждает суп побеждает", 10.7).is_some());
        // нормальная длина и те же слова — годен
        assert!(take_problem("Суп побеждает!", "суп побеждает", 1.6).is_none());
        // вздох в скобках даёт запас
        assert!(take_problem("Ну ладно. [sighs] Пошли.", "ну ладно пошли", 4.5).is_none());
        // повторы слов при нормальной длине
        assert!(take_problem("Вот оно!", "вот оно вот оно вот оно", 2.0).is_some());
        assert_eq!(syllables("Самый вкусный"), 4);
    }
}
