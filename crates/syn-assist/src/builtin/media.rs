use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};
use synaptixd::{ModelKind, Params};

use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct MediaTool;

pub fn media_dir() -> PathBuf {
    crate::chat::data_dir().join("media")
}

fn stamp() -> String {
    format!("{:x}", crate::chat::now())
}

pub fn write_wav(path: &Path, pcm: &[f32], rate: u32) -> std::io::Result<()> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for &x in pcm {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, w)
}

/// `model` из аргументов, если это установленная модель нужного вида: модель
/// нередко выдумывает id или берёт модель другого вида (видео для музыки) —
/// тогда `None`, и решают настройки.
fn model_arg(c: &mut synaptixd::Client, args: &Value, kind: ModelKind) -> Option<String> {
    let id = str_arg(args, "model")?.trim();
    if id.is_empty() {
        return None;
    }
    let models: Vec<_> = c.list().ok()?.into_iter().filter(|m| m.kind == kind).collect();
    if let Some(m) = models.iter().find(|m| m.id == id) {
        return Some(m.id.clone());
    }
    // «yue2», «YuE 2» — без регистра и знаков, по началу id или названия; только однозначно.
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let want = norm(id);
    let mut hits = models.iter().filter(|m| !want.is_empty() && (norm(&m.id).starts_with(&want) || norm(&m.title).starts_with(&want)));
    match (hits.next(), hits.next()) {
        (Some(m), None) => Some(m.id.clone()),
        _ => None,
    }
}

fn speak(args: &Value, cancel: &AtomicBool) -> ToolResult {
    let text = need_str(args, "prompt")?;
    let mut params = Params::new();
    if let Some(v) = str_arg(args, "voice") {
        params = params.with("voice", v);
    }
    if let Some(l) = str_arg(args, "language") {
        params = params.with("language", l);
    }
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let stop = c.canceller();
    let sp = crate::tools::watch_cancel(cancel, || stop.map(|s| s.cancel()).unwrap_or(()), || c.speak_with("", text, &params)).map_err(|e| e.to_string())?;
    let path = media_dir().join(format!("speech-{}.wav", stamp()));
    write_wav(&path, sp.audio.f32s(), sp.sample_rate).map_err(|e| e.to_string())?;
    let secs = sp.audio.f32s().len() as f32 / sp.sample_rate.max(1) as f32;
    Ok(ToolOutput { text: format!("Озвучено: {:.1} с, файл {}", secs, path.display()), files: vec![path] })
}

fn image(args: &Value, cancel: &AtomicBool) -> ToolResult {
    let prompt = need_str(args, "prompt")?;
    let num = |k: &str| args.get(k).and_then(Value::as_u64);
    let images: Vec<String> = args
        .get("images")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|p| crate::builtin::expand(p).to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let p = synaptixd::ImageParams {
        negative: str_arg(args, "negative").map(String::from),
        width: num("width").map(|v| v as usize),
        height: num("height").map(|v| v as usize),
        steps: num("steps").map(|v| v as usize),
        guidance: args.get("guidance").and_then(Value::as_f64).map(|v| v as f32),
        seed: num("seed"),
        images,
        init: None,
        strength: None,
    };
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let model = model_arg(&mut c, args, ModelKind::Image).unwrap_or_default();
    let stop = c.canceller();
    let (reply, png) = match crate::tools::watch_cancel(cancel, || stop.map(|s| s.cancel()).unwrap_or(()), || c.image(&model, prompt, &p, |i, n| crate::tools::progress(i, n, &synshell_tr::t!("картинка")))) {
        Ok(r) => r,
        Err(e) if e.code == synaptixd::ErrorCode::NoModel => return generate(ModelKind::Image, "картинки"),
        Err(e) => return Err(e.to_string()),
    };
    let path = media_dir().join(format!("image-{}.png", stamp()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &png).map_err(|e| e.to_string())?;
    Ok(ToolOutput {
        text: format!("Картинка готова и уже показана пользователю под ответом ({}, зерно {}; файл {} — для инструментов, пользователю путь не называй).", reply.label, reply.seed, path.display()),
        files: vec![path],
    })
}

fn video(args: &Value, cancel: &AtomicBool) -> ToolResult {
    let prompt = need_str(args, "prompt")?;
    let num = |k: &str| args.get(k).and_then(Value::as_u64);
    let path_arg = |k: &str| str_arg(args, k).map(|p| crate::builtin::expand(p).to_string_lossy().into_owned());
    // Сцены — по ролику на каждую; общая длина — seconds (до 3 минут).
    let scenes: Vec<String> = args.get("scenes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).filter(|s| !s.trim().is_empty()).collect()).unwrap_or_default();
    let total = args.get("seconds").and_then(Value::as_f64).unwrap_or(if scenes.is_empty() { 4.0 } else { 8.0 * scenes.len() as f64 }).clamp(1.0, 180.0);
    let mut p = synaptixd::VideoParams {
        negative: str_arg(args, "negative").map(String::from),
        width: Some(num("width").map(|v| v as usize).unwrap_or(960)),
        height: Some(num("height").map(|v| v as usize).unwrap_or(544)),
        seconds: Some(total.min(CLIP_MAX)),
        steps: num("steps").map(|v| v as usize),
        guidance: None,
        seed: num("seed"),
        first_frame: path_arg("first_frame"),
        last_frame: path_arg("last_frame"),
        refs: args
            .get("refs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|p| crate::builtin::expand(p).to_string_lossy().into_owned()).collect())
            .unwrap_or_default(),
        audio: path_arg("audio"),
        source_video: path_arg("source_video"),
        retake: match (args.get("retake_start").and_then(Value::as_f64), args.get("retake_end").and_then(Value::as_f64)) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        },
        extend: args.get("extend").and_then(Value::as_bool).unwrap_or(false),
        control_video: path_arg("control_video"),
        control: str_arg(args, "control").map(String::from),
        two_stage: args.get("two_stage").and_then(Value::as_bool).unwrap_or(false),
        strength: args.get("strength").and_then(Value::as_f64).map(|v| v as f32),
        ..Default::default()
    };
    // Перерисовка ролика (restyle) — у MiniMax-H3.
    let restyle = args.get("restyle").and_then(Value::as_bool).unwrap_or(false) && p.source_video.is_some();
    // Возможности только LTX — быстрая distilled LTX; референсы — H3 Ref2VA; ключевые кадры без них — H3 FL2VA.
    let ltx_only = !restyle && (p.audio.is_some() || p.source_video.is_some() || p.control_video.is_some() || p.two_stage);
    let keyed = p.first_frame.is_some() || p.last_frame.is_some();
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let model = model_arg(&mut c, args, ModelKind::Video).unwrap_or_else(|| {
        if ltx_only {
            "ltx-2-3-22b-distilled-1-1"
        } else if !p.refs.is_empty() {
            "minimax-h3-ref2va"
        } else if keyed || restyle {
            "minimax-h3-fl2va"
        } else {
            ""
        }
        .to_string()
    });
    // Длиннее одного ролика или по сценам — графом с латентным хвостом (`chain: true`):
    // без перекодирования через PNG и с сохранением якорных `refs` на каждом ролике цепочки.
    let chainable = !ltx_only && !restyle && p.retake.is_none() && !p.extend;
    let n = if !chainable {
        1
    } else if scenes.is_empty() {
        (total / CLIP_MAX).ceil().max(1.0) as usize
    } else {
        scenes.len()
    };
    let per = if n > 1 { (total / n as f64).clamp(1.0, CLIP_MAX) } else { total.min(CLIP_MAX) };
    let dir = media_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = stamp();
    if n > 1 {
        let prompts: Vec<Value> = (0..n)
            .map(|k| {
                let text = match scenes.get(k) {
                    Some(s) => format!("{prompt}\n{s}"),
                    None => prompt.to_string(),
                };
                Value::String(text)
            })
            .collect();
        let mut flow = synaptix_flow::Flow::default();
        let list_id = flow.next_id();
        let mut list_node = synaptix_flow::FlowNode::new(list_id, "list");
        list_node.label = "scenes".into();
        list_node.fields.insert("items".into(), Value::Array(prompts));
        flow.add(list_node);

        let vid_id = flow.next_id();
        let mut vid_node = synaptix_flow::FlowNode::new(vid_id, "video");
        vid_node.label = "shots".into();
        vid_node.fields.insert("seconds".into(), json!(per));
        vid_node.fields.insert("chain".into(), json!(true));
        if let Some(w) = p.width {
            vid_node.fields.insert("width".into(), json!(w));
        }
        if let Some(h) = p.height {
            vid_node.fields.insert("height".into(), json!(h));
        }
        if let Some(steps) = p.steps {
            vid_node.fields.insert("steps".into(), json!(steps));
        }
        if let Some(ref neg) = p.negative {
            vid_node.fields.insert("negative".into(), json!(neg));
        }
        if let Some(seed) = p.seed {
            vid_node.fields.insert("seed".into(), json!(seed));
        }
        if !model.is_empty() {
            vid_node.fields.insert("model".into(), json!(model));
        }
        flow.add(vid_node);

        let cat_id = flow.next_id();
        let mut cat_node = synaptix_flow::FlowNode::new(cat_id, "concat");
        cat_node.label = "film".into();
        flow.add(cat_node);

        flow.link(list_id, "list", vid_id, "prompt");
        flow.link(vid_id, "video", cat_id, "files");
        flow.outputs = vec![cat_id];

        let mut inputs = std::collections::HashMap::new();
        // Якорные refs держат героев и мир на всех клипах цепочки (MiniMax-H3 Ref2VA);
        // если задан только first_frame без явной модели — подаём его и в refs (как якорь),
        // и в first_frame (запасной для FL2VA, если Ref2VA не установлена).
        let mut anchor_refs: Vec<synaptix_flow::Val> = p.refs.iter().map(|r| synaptix_flow::Val::File(PathBuf::from(r))).collect();
        if anchor_refs.is_empty() && model.is_empty() {
            if let Some(ref ff) = p.first_frame {
                anchor_refs.push(synaptix_flow::Val::File(PathBuf::from(ff)));
            }
        }
        if !anchor_refs.is_empty() {
            inputs.insert((vid_id, "refs".into()), synaptix_flow::Val::List(anchor_refs));
        }
        if let Some(ref ff) = p.first_frame {
            inputs.insert((vid_id, "first_frame".into()), synaptix_flow::Val::File(PathBuf::from(ff)));
        }

        let run_dir = dir.join(format!("video-{id}-run"));
        let flow_cancel = synaptix_flow::Cancel::new();
        let fc = flow_cancel.clone();
        let mut current = synshell_tr::t!("видео 1/{n}", n = n);
        let report = crate::tools::watch_cancel(
            cancel,
            move || fc.cancel(),
            || {
                let opts = synaptix_flow::RunOpts { cancel: flow_cancel.clone(), inputs, ..Default::default() };
                synaptixd::flow::run_with(&flow, &run_dir, &opts, &mut |ev| match ev {
                    synaptix_flow::Event::Started { id: nid, item, .. } => {
                        if nid == vid_id {
                            if let Some((k, total_n)) = item {
                                current = synshell_tr::t!("видео {k}/{n}", k = k, n = total_n);
                            }
                            crate::tools::progress(0, 0, &current);
                        } else if nid == cat_id {
                            crate::tools::progress(0, 0, &synshell_tr::t!("склейка роликов"));
                        }
                    }
                    synaptix_flow::Event::Progress { id: nid, step, total: t } if nid == vid_id => {
                        crate::tools::progress(step, t, &current);
                    }
                    _ => {}
                })
            },
        )?;
        let out = dir.join(format!("video-{id}.mp4"));
        let src = report.files.first().ok_or_else(|| "цепочка видео не дала файла".to_string())?;
        if std::fs::rename(src, &out).is_err() {
            std::fs::copy(src, &out).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_dir_all(&run_dir);
        let secs = media_seconds(&out).unwrap_or(0.0);
        let label = report.steps.iter().find(|s| s.starts_with("shots:")).cloned().unwrap_or_else(|| format!("{n} роликов цепочкой"));
        return Ok(ToolOutput {
            text: format!(
                "Видео готово и уже показано пользователю под ответом: длительность {secs:.0} с — называй пользователю только её ({label}; файл {} — для инструментов, пользователю путь не называй).",
                out.display()
            ),
            files: vec![out],
        });
    }

    p.seconds = Some(per);
    let what = synshell_tr::t!("видео");
    let stop = c.canceller();
    let (reply, mp4) = match crate::tools::watch_cancel(cancel, || stop.map(|s| s.cancel()).unwrap_or(()), || c.video(&model, prompt, &p, |i, t| crate::tools::progress(i, t, &what))) {
        Ok(r) => r,
        Err(e) if e.code == synaptixd::ErrorCode::NoModel => return generate(ModelKind::Video, "видео"),
        Err(e) => return Err(e.to_string()),
    };
    let path = dir.join(format!("video-{id}.mp4"));
    std::fs::write(&path, &mp4).map_err(|e| e.to_string())?;
    let secs = media_seconds(&path).unwrap_or(0.0);
    Ok(ToolOutput {
        text: format!(
            "Видео готово и уже показано пользователю под ответом: длительность {secs:.0} с — называй пользователю только её ({}, зерно {}; файл {} — для инструментов, пользователю путь не называй).",
            reply.label,
            reply.seed,
            path.display()
        ),
        files: vec![path],
    })
}

/// Предел одного ролика модели, с; длиннее — цепочкой.
const CLIP_MAX: f64 = 15.0;

/// Длительность ролика или звука, с (ffprobe).
pub fn media_seconds(path: &Path) -> Option<f64> {
    let out = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|s: &f64| *s > 0.0)
}

fn music(args: &Value, cancel: &AtomicBool) -> ToolResult {
    let prompt = need_str(args, "prompt")?;
    let lyrics = str_arg(args, "lyrics").unwrap_or("");
    // Длительность — только если пользователь её назвал: модель ставит 30 с
    // по привычке (повторяет прошлые вызовы), и песня обрывается на куплете.
    let seconds = args.get("seconds").and_then(Value::as_u64).filter(|_| crate::tools::asked_duration()).map(|v| v.clamp(5, 300) as u32);
    let seed = args.get("seed").and_then(Value::as_u64);
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    // Модель из настроек главнее; не выбрана — инструментал (без текста) на
    // ACE-Step (YuE2 — песенная модель), иначе — по умолчанию.
    let chosen = c.defaults().ok().and_then(|d| d.into_iter().find(|d| d.kind == ModelKind::Music)).and_then(|d| d.model);
    let picked = match model_arg(&mut c, args, ModelKind::Music) {
        Some(m) => Some(m),
        None if chosen.is_some() => chosen,
        None if lyrics.trim().is_empty() => c
            .list()
            .ok()
            .and_then(|l| l.into_iter().filter(|m| m.kind == ModelKind::Music && m.id.contains("acestep")).min_by_key(|m| !m.id.contains("turbo")))
            .map(|m| m.id),
        None => None,
    };
    let model = picked.as_deref().unwrap_or("");
    let stop = c.canceller();
    let (reply, wav) = match crate::tools::watch_cancel(cancel, || stop.map(|s| s.cancel()).unwrap_or(()), || c.music(model, prompt, lyrics, seconds, seed, |i, n| crate::tools::progress(i, n, &synshell_tr::t!("музыка")))) {
        Ok(r) => r,
        Err(e) if e.code == synaptixd::ErrorCode::NoModel => return generate(ModelKind::Music, "музыки"),
        Err(e) => return Err(e.to_string()),
    };
    let path = media_dir().join(format!("music-{}.wav", stamp()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &wav).map_err(|e| e.to_string())?;
    Ok(ToolOutput {
        text: format!("Музыка готова и уже показана пользователю под ответом ({}, {:.0} с, зерно {}; файл {} — для инструментов, пользователю путь не называй).", reply.label, reply.seconds, reply.seed, path.display()),
        files: vec![path],
    })
}

fn generate(kind: ModelKind, what: &str) -> ToolResult {
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    match c.resolve(kind) {
        Ok(m) if !m.runnable => Err(format!(
            "модель для «{what}» установлена ({}), но служба нейросетей пока не умеет её исполнять — эта возможность появится позже. Скажи пользователю об этом.",
            m.title
        )),
        Ok(m) => Err(format!("генерация «{what}» через службу ещё не подключена (модель {})", m.title)),
        Err(_) => Err(format!("не установлена модель для «{what}» — её можно добавить в «Параметры → Нейросети → Хранилище моделей»")),
    }
}

impl Tool for MediaTool {
    fn name(&self) -> &'static str {
        "media"
    }
    fn group(&self) -> &'static str {
        "медиа"
    }
    fn description(&self) -> String {
        "Создание медиа системными нейросетями. action: speak — озвучить текст (prompt — текст, voice — описание голоса, language — код языка); image — картинка: prompt — подробное описание по-английски (объекты, стиль, свет, композиция), width/height (по умолчанию ~1024, кратно 16), steps, seed, negative; images — пути картинок-референсов, чтобы изменить или скомбинировать их (prompt — что сделать); video — видео со звуком: prompt — подробная сцена по-английски (персонажи, действие, камера, звук); реплики героев пиши прямо в prompt в кавычках (Tom shouts: \"…\") — модель озвучивает их сама (lyrics для видео нет); seconds — общая длина (по умолчанию 4, до 180): длиннее 15 с ролик собирается цепочкой по латентному хвосту с сохранением якорных refs на каждом клипе; scenes — список описаний сцен по-английски (с репликами) — по ролику на сцену, prompt тогда — общее (персонажи, стиль, место); генерация идёт минуты, first_frame / last_frame — пути картинок, с которых начать или которыми закончить; refs — пути референсов (картинки персонажей и предметов, видео, звук — голос для липсинка или музыка; до 12, не вместе с кадрами), в prompt опиши, что с ними делать; возможности LTX-2.3 (модель выбирается сама): audio — путь звука, видео строится под него (липсинк речи, движение под музыку); source_video + retake_start/retake_end — перегенерировать отрезок ролика по prompt; source_video + extend=true — продолжить ролик; control_video — повторить движение/композицию образца (control: canny — контуры, motion — движение); control_video + audio — LipDub (переозвучить ролик: губы под новый звук); two_stage=true — выше качество и разрешение (дольше); source_video + restyle=true — перерисовать ролик в новом стиле со звуком: в prompt опиши всю сцену целиком в новом стиле (что не названо — может исчезнуть), strength 0..1 — насколько сильно менять (0.4 — стиль поверх, 0.6 по умолчанию, 0.8 — почти заново). model: ltx-2-3-22b-distilled-1-1 — быстрая, minimax-h3-* (по умолчанию) — refs; music — музыка или песня: prompt — жанр, инструменты, настроение по-английски, lyrics — текст песни с метками [verse]/[chorus] (пусто — инструментал; для полной песни пиши 2–3 куплета и припевы), seconds — длительность, только если пользователь сам её назвал (иначе не указывай: песня — около 3 минут). Результат — файл, он покажется пользователю в чате, и ты его увидишь.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["speak", "image", "video", "music"]},
                "prompt": {"type": "string"},
                "voice": {"type": "string"},
                "language": {"type": "string"},
                "negative": {"type": "string"},
                "width": {"type": "integer"},
                "height": {"type": "integer"},
                "steps": {"type": "integer"},
                "seed": {"type": "integer"},
                "guidance": {"type": "number"},
                "images": {"type": "array", "items": {"type": "string"}, "description": "пути картинок-референсов для правки"},
                "lyrics": {"type": "string", "description": "текст песни (music)"},
                "refs": {"type": "array", "items": {"type": "string"}, "description": "пути референсов для видео: картинки, видео, звук"},
                "audio": {"type": "string", "description": "видео: путь звука для липсинка или под музыку"},
                "source_video": {"type": "string", "description": "видео: исходный ролик для retake или продления"},
                "retake_start": {"type": "number"},
                "retake_end": {"type": "number"},
                "extend": {"type": "boolean"},
                "control_video": {"type": "string", "description": "видео: образец движения/композиции (IC-LoRA) или ролик для LipDub"},
                "control": {"type": "string", "enum": ["canny", "motion"]},
                "two_stage": {"type": "boolean"},
                "restyle": {"type": "boolean", "description": "видео: перерисовать source_video по prompt"},
                "strength": {"type": "number", "description": "сила перерисовки 0..1"},
                "scenes": {"type": "array", "items": {"type": "string"}, "description": "видео: сцены по порядку — по ролику на каждую, цепочкой"},
                "first_frame": {"type": "string", "description": "путь картинки — первый кадр видео"},
                "last_frame": {"type": "string", "description": "путь картинки — последний кадр видео"},
                "seconds": {"type": "integer", "description": "длительность, с — только если её назвал пользователь (видео — общая длина)"},
                "model": {"type": "string", "description": "id модели, только если пользователь сам её назвал; обычно не указывай — модель берётся из настроек"}
            },
            "required": ["action", "prompt"]
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        let p: String = str_arg(args, "prompt").unwrap_or("").chars().take(60).collect();
        match str_arg(args, "action").unwrap_or("?") {
            "speak" => synshell_tr::t!("Озвучить: {p}", p = p),
            "image" => synshell_tr::t!("Картинка: {p}", p = p),
            "video" => synshell_tr::t!("Видео: {p}", p = p),
            "music" => synshell_tr::t!("Музыка: {p}", p = p),
            other => other.to_string(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        self.run_ctx(args, &AtomicBool::new(false))
    }
    fn pooled(&self) -> bool {
        true
    }
    fn when(&self) -> String {
        "создать картинку, видео (со звуком и репликами), музыку или песню, озвучить текст".into()
    }
    fn guide(&self) -> String {
        crate::autoguide::models()
    }
    /// «Стоп» обрывает запрос — служба прерывает генерацию.
    fn run_ctx(&self, args: &Value, cancel: &AtomicBool) -> ToolResult {
        let r = match need_str(args, "action")? {
            "speak" => speak(args, cancel),
            "image" => image(args, cancel),
            "video" => video(args, cancel),
            "music" => music(args, cancel),
            other => Err(format!("неизвестное действие {other}")),
        };
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("остановлено пользователем".into());
        }
        r
    }
}
