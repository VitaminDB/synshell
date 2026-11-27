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

pub static COMMANDS: &[&Cmd] = &[&CAST, &VOICES, &SAY, &TRACK];

/// Долгие команды (синтез минутами) — агент зовёт их без блокировки документа.
pub const HEAVY: &[&str] = &["voice.cast", "voice.say", "voice.track"];

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
        arg("name", Ty::Str, "имя файла"),
        arg("temperature", Ty::Num, "температура (1.0)"),
    ],
    danger: false,
    run: |ed, a| heavy(ed, "voice.say", a),
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
            let out = free_path(&dir.join("speech"), &stem);
            write_wav(&out, &pcm, rate)?;
            let secs = pcm.len() as f64 / rate.max(1) as f64;
            Ok((format!("Реплика «{}» ({who}): {} — {secs:.1} с{}", plain_text(&text), out.display(), seed.map(|s| format!(", зерно {s}")).unwrap_or_default()), vec![out.clone()], json!({"path": out, "seconds": secs, "seed": seed})))
        }
        "voice.track" => {
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
    }
}
