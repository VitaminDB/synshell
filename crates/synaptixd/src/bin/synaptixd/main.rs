mod activity;
mod config;
mod defaults;
mod library;
mod lora;
mod devices;
mod federation;
mod flow;
mod gen;
mod hangup;
mod stages;
mod manager;
mod registry;
mod runners;
mod train;
mod voices;

use std::ffi::CStr;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use synaptixd::wire::{self, Mapped};
use synaptixd::{CtcReply, Envelope, Error, ErrorCode, FdRef, Hello, ModelKind, Params, Reply, Request, SettingsChange};

use manager::Manager;

fn is_admin(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return false;
        }
        let name = (*pw).pw_name;
        let base = (*pw).pw_gid;
        let grp = libc::getgrnam(c"wheel".as_ptr());
        if grp.is_null() {
            return false;
        }
        let wheel = (*grp).gr_gid;
        if base == wheel {
            return true;
        }
        let mut groups = vec![0 as libc::gid_t; 256];
        let mut n = groups.len() as libc::c_int;
        if libc::getgrouplist(name, base, groups.as_mut_ptr(), &mut n) < 0 {
            return false;
        }
        let _ = CStr::from_ptr(name);
        groups[..n as usize].contains(&wheel)
    }
}

fn data<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn pcm_from(fds: &mut [Option<OwnedFd>], r: FdRef, samples: usize) -> Result<Mapped, Error> {
    let fd = fds
        .get_mut(r.fd as usize)
        .and_then(Option::take)
        .ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла с аудио"))?;
    Mapped::map(fd, samples * 4).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
}

fn denied() -> Error {
    Error::new(ErrorCode::Denied, "менять параметры нейросетей могут root и группа wheel")
}

struct Conn {
    mgr: Arc<Manager>,
    sock: OwnedFd,
    admin: bool,
    opened: Vec<String>,
    current: u64,
    /// Номер соединения и программа на том конце.
    conn: u64,
    app: synaptixd::AppInfo,
    /// Соединение пришло с другого устройства (мост synlink): только свои
    /// модели, без пересылки дальше и без смены настроек.
    remote: bool,
    /// Мосты к службам устройств (федерация), по id synlink.
    bridges: std::collections::HashMap<String, federation::Bridge>,
}

/// Операция запроса, если это работа с моделью (для учёта работ).
fn work_op(req: &Request) -> Option<&'static str> {
    Some(match req {
        Request::Load { .. } => "load",
        Request::Ctc { .. } => "ctc",
        Request::MfaAlign { .. } => "align",
        Request::Image { .. } => "image",
        Request::Video { .. } => "video",
        Request::Music { .. } => "music",
        Request::Embed { .. } => "embed",
        Request::Transcribe { .. } => "transcribe",
        Request::Speak { .. } => "speak",
        Request::Generate { .. } => "generate",
        Request::Rerank { .. } => "rerank",
        Request::Flow { .. } => "flow",
        Request::Pack { .. } => "pack",
        _ => return None,
    })
}

/// Приоритет запроса в очереди к устройству: короткие ответы живому
/// человеку (ASR, TTS, выравнивание, эмбеддинги) — вперёд, генерация —
/// обычный, `params.priority` (`interactive` | `normal` | `background`)
/// меняет его у Generate и Speak.
fn priority(req: &Request) -> devices::Prio {
    use devices::Prio;
    let asked = |p: &Params| p.text("priority").and_then(Prio::parse);
    match req {
        Request::Transcribe { .. } | Request::Ctc { .. } | Request::MfaAlign { .. } | Request::Embed { .. } | Request::Rerank { .. } | Request::Tokenize { .. } => Prio::Interactive,
        Request::Speak { params, .. } => asked(params).unwrap_or(Prio::Interactive),
        Request::Generate { params, .. } => asked(params).unwrap_or(Prio::Normal),
        _ => Prio::Normal,
    }
}

/// Имя операции запроса для журнала загрузок.
fn req_name(req: &Request) -> &'static str {
    work_op(req).unwrap_or(match req {
        Request::Open { .. } => "open",
        Request::LlmInfo { .. } => "llm_info",
        Request::Tokenize { .. } => "tokenize",
        _ => "запрос",
    })
}

impl Conn {
    fn send(&self, reply: &Reply, fds: &[OwnedFd]) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(reply).unwrap_or_default();
        let borrowed: Vec<_> = fds.iter().map(|f| f.as_fd()).collect();
        wire::send(self.sock.as_fd(), &bytes, &borrowed)
    }

    fn dispatch(&mut self, req: Request, fds: Vec<OwnedFd>) -> Result<(Value, Vec<OwnedFd>), Error> {
        devices::set_priority(priority(&req));
        let mgr = self.mgr.clone();
        let mut fds: Vec<Option<OwnedFd>> = fds.into_iter().map(Some).collect();
        let none = Vec::new;
        Ok(match req {
            Request::Hello { client } => {
                tracing::debug!("клиент {client}");
                if client.starts_with(synaptixd::bridge::REMOTE_PREFIX) {
                    self.remote = true;
                    self.admin = false;
                }
                if self.remote && !client.starts_with(synaptixd::bridge::REMOTE_PREFIX) {
                    // Программа на том устройстве назвалась — метку моста не теряем.
                    self.app.label = format!("{} {client}", self.app.label.split(' ').next().unwrap_or_default());
                } else {
                    self.app.label = client;
                }
                activity::relabel(self.conn, &self.app);
                (data(Hello { version: synaptixd::VERSION, devices: devices::list() }), none())
            }
            Request::List => (data(self.with_remote(mgr.list())), none()),
            Request::Status => (data(mgr.status()), none()),
            Request::Rescan => {
                mgr.rescan();
                (data(self.with_remote(mgr.list())), none())
            }
            Request::Open { model } => {
                let model = mgr.resolve_any(&model)?;
                let info = mgr.open(&model)?;
                self.opened.push(model);
                activity::opened(self.conn, &self.opened);
                (data(info), none())
            }
            Request::Close { model } => {
                let model = mgr.resolve_any(&model)?;
                if let Some(i) = self.opened.iter().position(|m| *m == model) {
                    self.opened.swap_remove(i);
                    mgr.close(&model);
                    activity::opened(self.conn, &self.opened);
                }
                (Value::Null, none())
            }
            Request::Load { model } => {
                let model = mgr.resolve_any(&model)?;
                drop(mgr.acquire(&model)?);
                (data(mgr.entry(&model)?), none())
            }
            Request::Unload { model } => {
                if !self.admin {
                    return Err(denied());
                }
                mgr.unload(&model);
                (data(mgr.entry(&model)?), none())
            }
            Request::Set { model, device, pinned, idle_unload_min, kv_keep_min, preload, quant, params, engine, loras } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set(&model, SettingsChange { device, pinned, idle_unload_min, kv_keep_min, preload, quant, params, engine, loras })?), none())
            }
            Request::Defaults => {
                let mut list = mgr.defaults();
                if let (false, Some(f)) = (self.remote, federation::get()) {
                    f.defaults(&mut list);
                }
                (data(list), none())
            }
            Request::SetDefault { kind, model } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set_default(kind, model)?), none())
            }
            Request::Resolve { kind } => (data(mgr.entry(&mgr.resolve(kind)?)?), none()),
            Request::Libraries => (data(mgr.libraries()), none()),
            Request::AddLibrary { path, name } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.add_library(&path, name)?), none())
            }
            Request::RemoveLibrary { path } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.remove_library(&path)?), none())
            }
            Request::SetDefaultLibrary { path } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set_default_library(&path)?), none())
            }
            Request::SetVramYield { programs } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set_vram_yield(programs)), none())
            }
            Request::DeleteModel { model } => {
                if !self.admin {
                    return Err(denied());
                }
                mgr.delete_model(&model)?;
                (Value::Null, none())
            }
            Request::Ctc { model, pcm, samples, sample_rate, hidden } => {
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let r = mgr.run(&model, |m| m.ctc(pcm.f32s(), sample_rate, hidden))?;
                let mut out = vec![wire::memfd_f32("logprobs", &r.logprobs)?];
                let hidden = match &r.hidden {
                    Some(h) => {
                        out.push(wire::memfd_f32("hidden", h)?);
                        Some(FdRef { fd: 1 })
                    }
                    None => None,
                };
                let reply = CtcReply {
                    frames: r.frames,
                    vocab: r.vocab,
                    frame_sec: r.frame_sec,
                    logprobs: FdRef { fd: 0 },
                    hidden,
                    hidden_dim: r.hidden_dim,
                };
                (data(reply), out)
            }
            Request::MfaAlign { model, pcm, samples, words, options } => {
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let al = mgr.run(&model, |m| m.mfa_align(pcm.f32s(), &words, options.as_ref()))?;
                (data(al), none())
            }
            Request::Image { model, prompt, negative, width, height, steps, guidance, seed, images, init, strength } => {
                let p = synaptixd::ImageParams { negative, width, height, steps, guidance, seed, images, init, strength };
                let sock = self.sock.as_fd();
                let id = self.current;
                // Клиент ушёл (отправка не прошла) — отмена генерации.
                let live = activity::current();
                let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                if let Some(l) = &live {
                    l.cancel_flag(stop.clone());
                }
                let mut send = |step: usize, total: usize| -> bool {
                    if let Some(l) = &live {
                        l.progress(step, total);
                    }
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_ok() && !stop.load(std::sync::atomic::Ordering::Relaxed)
                };
                let r = gen::image(&mgr, &model, prompt, p, &mut send)?;
                let fd = wire::memfd("image", &r.png)?;
                let reply = synaptixd::ImageReply { width: r.width, height: r.height, seed: r.seed, label: r.label, png: FdRef { fd: 0 }, bytes: r.png.len() };
                (data(reply), vec![fd])
            }
            Request::Video { model, prompt, negative, width, height, seconds, steps, guidance, seed, first_frame, last_frame, refs, audio, source_video, retake, extend, control_video, control, two_stage, strength } => {
                let p = synaptixd::VideoParams { negative, width, height, seconds, steps, guidance, seed, first_frame, last_frame, refs, audio, source_video, retake, extend, control_video, control, two_stage, strength };
                let raw = self.sock.as_raw_fd();
                let id = self.current;
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let live = activity::current();
                if let Some(l) = &live {
                    l.cancel_flag(cancel.clone());
                }
                let send = |step: usize, total: usize| {
                    if let Some(l) = &live {
                        l.progress(step, total);
                    }
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    let sock = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                };
                let r = gen::video(&mgr, &model, prompt, p, &cancel, &send)?;
                let fd = wire::memfd("video", &r.mp4)?;
                let o = r.out;
                let reply = synaptixd::VideoReply { width: o.width, height: o.height, frames: o.frames, fps: o.fps, seed: r.seed, label: o.label, mp4: FdRef { fd: 0 }, bytes: r.mp4.len() };
                (data(reply), vec![fd])
            }
            Request::Music { model, prompt, lyrics, seconds, seed, steps } => {
                let sock = self.sock.as_fd();
                let id = self.current;
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let c2 = cancel.clone();
                let live = activity::current();
                if let Some(l) = &live {
                    l.cancel_flag(cancel.clone());
                }
                let mut send = |step: usize, total: usize| {
                    if let Some(l) = &live {
                        l.progress(step, total);
                    }
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        c2.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                };
                let r = gen::music(&mgr, &model, prompt, lyrics, seconds, seed, steps, cancel.clone(), &mut send)?;
                let fd = wire::memfd("music", &r.wav)?;
                let o = r.out;
                let seconds = o.samples.len() as f32 / (o.sample_rate.max(1) as f32 * o.channels.max(1) as f32);
                let reply = synaptixd::MusicReply { seconds, sample_rate: o.sample_rate, channels: o.channels, seed: r.seed, label: o.label, wav: FdRef { fd: 0 }, bytes: r.wav.len() };
                (data(reply), vec![fd])
            }
            Request::Embed { model, texts } => {
                let model = mgr.resolve_id(&model, ModelKind::Embed)?;
                let e = mgr.run(&model, |m| m.embed(&texts))?;
                (data(e), none())
            }
            Request::Transcribe { model, pcm, samples, sample_rate, language, words, verbatim, prompt } => {
                let model = mgr.resolve_id(&model, ModelKind::Asr)?;
                let mut call = Params::new();
                if let Some(v) = language {
                    call.set("language", Some(v.into()));
                }
                let language = mgr.params(&model, &call).text("language").map(String::from);
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let words = words.then_some((verbatim, prompt));
                let t = mgr.run(&model, |m| m.transcribe(pcm.f32s(), sample_rate, language.as_deref(), words.clone()))?;
                (data(t), none())
            }
            Request::Speak { model, text, language, instruct, speed, steps, params, fresh } => {
                let model = mgr.resolve_id(&model, ModelKind::Tts)?;
                let mut call = params;
                if let Some(v) = language {
                    call.set("language", Some(v.into()));
                }
                if let Some(v) = instruct {
                    call.set("voice", Some(v.into()));
                }
                if let Some(v) = speed {
                    call.set("speed", Some(v.into()));
                }
                if let Some(v) = steps {
                    call.set("steps", Some((v as i64).into()));
                }
                // Параметры запроса, а не настройки модели: у движков без
                // своей скорости она — растяжением времени после синтеза.
                let request_only: Vec<(&str, synaptixd::ParamValue)> =
                    ["speaker", "speakers", "stream", "speed", "verify"].into_iter().filter_map(|k| call.get(k).cloned().map(|v| (k, v))).collect();
                let mut params = mgr.params(&model, &call);
                for (k, v) in request_only {
                    params.set(k, Some(v));
                }
                // Персонаж главнее голоса из настроек модели (описание, зерно,
                // образец); заданное в самом запросе — главнее персонажа.
                if call.get("speaker").is_some() {
                    for k in ["voice", "seed", "ref_audio", "ref_text"] {
                        if call.get(k).is_none() {
                            params.set(k, None);
                        }
                    }
                }
                let stream = params.bool("stream").unwrap_or(false);
                let arch = mgr.entry(&model).map(|e| e.arch.to_ascii_lowercase()).unwrap_or_default();
                // Готовые голоса (Leokani, Supertonic): описание голоса («male»,
                // «female, american accent») схема-список отбросила бы — переводим
                // его в готовый голос до неё.
                if let Some(v) = call.text("voice").and_then(|d| voices::preset_for_description(&arch, d)) {
                    params.set("voice", Some(v.into()));
                }
                let vibe = arch.contains("vibevoice");
                // VoxCPM по описанию плохо держит пол и возраст (девочка и
                // пожилой мужчина — одной высоты): у него голос персонажа тоже
                // с опорного клипа, описание — манера поверх тембра.
                // Audio8 голоса по описанию не знает: персонаж — клон с опорного
                // клипа на языке текста (клону нужен и текст клипа).
                let audio8 = arch == "audio8" || arch == "arktts";
                let clone_voice = vibe || audio8 || (arch.starts_with("voxcpm") && std::env::var_os("SYNAPTIXD_VOX_CLIP").is_some());
                let language = params.text("language").map(str::to_string);
                let clip_language = if audio8 { language.clone().or_else(|| Some(runners::text_language(&text).to_string())) } else { language.clone() };
                // Многоголосие: «Speaker N: …» + speakers — N-й персонаж списка.
                let cast = params.text("speakers").map(voices::parse_speakers).unwrap_or_default();
                for sp in &cast {
                    if synaptix::facade::tts::speaker(sp).is_none() {
                        return Err(Error::new(ErrorCode::Failed, format!("неизвестный голос «{sp}»; есть: {}", synaptixd::SPEAKERS.join(", "))));
                    }
                }
                if cast.len() > 4 {
                    return Err(Error::new(ErrorCode::Failed, "speakers: не больше 4 голосов"));
                }
                params.set("speakers", None);
                // Голос персонажа у VibeVoice и VoxCPM — клон с опорного клипа
                // (пол и возраст задаёт клип, озвученный OmniVoice по описанию).
                let own_ref = call.get("ref_audio").is_some();
                let voice_for = |p: &mut Params, speaker: Option<&str>| -> Result<(), Error> {
                    if let Some(sp) = speaker {
                        p.set("speaker", Some(sp.into()));
                    }
                    if clone_voice && !own_ref {
                        if let Some(sp) = p.text("speaker").map(str::to_string) {
                            let clip = voices::persona_clip(&mgr, &sp, clip_language.as_deref())?;
                            p.set("ref_audio", Some(clip.to_string_lossy().into_owned().into()));
                            if audio8 {
                                let lang = clip_language.as_deref().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).unwrap_or_else(|| "en".into());
                                p.set("ref_text", Some(voices::clip_text(&lang).into()));
                            }
                        }
                    }
                    Ok(())
                };
                // Части озвучки: (текст, параметры, номер реплики). Поток — по
                // предложениям; сценарий с голосами — по репликам (кроме
                // VibeVoice без потока: он читает весь сценарий за один проход).
                let mut parts: Vec<(String, Params, usize)> = Vec::new();
                if !cast.is_empty() && vibe && !stream {
                    let reps = voices::replicas(&text);
                    let (script, used) = voices::renumber(&reps);
                    let mut p = params.clone();
                    p.set("speaker", None);
                    if call.get("seed").is_none() {
                        // Зерно первого персонажа — повторяемый результат.
                        let seed = synaptix::facade::tts::speaker(&cast[0]).map(|s| s.2 as i64).unwrap_or(1);
                        p.set("seed", Some(seed.into()));
                    }
                    if !own_ref {
                        let mut clips = Vec::new();
                        for n in &used {
                            let sp = cast.get(n.saturating_sub(1)).or(cast.last()).cloned().unwrap_or_default();
                            clips.push(voices::persona_clip(&mgr, &sp, language.as_deref())?.to_string_lossy().into_owned());
                        }
                        p.set("ref_audio", Some(clips.join("\n").into()));
                    }
                    parts.push((script, p, 0));
                } else if !cast.is_empty() {
                    for (i, (n, line)) in voices::replicas(&text).into_iter().enumerate() {
                        let mut p = params.clone();
                        voice_for(&mut p, cast.get(n.saturating_sub(1)).or(cast.last()).map(String::as_str))?;
                        let pieces = if stream { synaptix::facade::tts::speech_chunks(&line, 60) } else { vec![line] };
                        parts.extend(pieces.into_iter().map(|t| (t, p.clone(), i)));
                    }
                } else {
                    let mut p = params.clone();
                    voice_for(&mut p, None)?;
                    let pieces = if stream { synaptix::facade::tts::speech_chunks(&text, 60) } else { vec![text.clone()] };
                    parts.extend(pieces.into_iter().map(|t| (t, p.clone(), 0)));
                }
                // Каждая часть в потоке — событием `audio` с PCM в fd, как
                // только готова; между предложениями 120 мс, между репликами 350.
                let (sock, id) = (self.sock.as_fd(), self.current);
                let mut wav: Vec<f32> = Vec::new();
                let (mut sr, mut seed) = (24000u32, None);
                let mut last_rep = None;
                // Проверка распознаванием: синтез иногда бормочет нормальной
                // длины (по длительности не поймать) — кусок, где whisper не
                // узнал заметную часть слов (en, ru — больше 25 %, иначе 50 %), — заново другим зерном (до
                // двух раз), берётся лучший. verify=false — без проверки.
                let verify = params.bool("verify").unwrap_or(true);
                // Порог: где whisper точен (en, ru) — строже; казахский он сам
                // распознаёт с WER ~26 %.
                let max_err = if matches!(language.as_deref(), Some("en" | "ru")) { 0.25 } else { 0.5 };
                params.set("verify", None);
                for (index, (chunk, p, rep)) in parts.iter().enumerate() {
                    let base_seed = p.i64("seed").filter(|s| *s >= 0).or_else(|| p.text("speaker").and_then(synaptix::facade::tts::speaker).map(|s| s.2 as i64)).unwrap_or(1);
                    // Высота голоса персонажа: OmniVoice и VoxCPM на отдельных
                    // фразах уходят от неё (мужской голос — к 230 Гц).
                    // У VibeVoice высоту задаёт клип-образец — повтор её не меняет.
                    // У Supertonic голос персонажа — готовый стиль: высота задана им,
                    // повтор с другим зерном её не меняет.
                    let pitch = p.text("speaker").filter(|_| !vibe && arch != "supertonic" && arch != "leokani").map(voices::pitch_range).filter(|r| r.1 < 1000.0);
                    let mut best: Option<(f32, Vec<f32>, u32, Option<u64>)> = None;
                    for attempt in 0..3i64 {
                        let mut p = p.clone();
                        p.set("verify", None);
                        if attempt > 0 {
                            p.set("retry", Some(attempt.into()));
                        }
                        let (pcm, rate, sd) = mgr.run(&model, |m| m.speak(chunk, &p, fresh))?;
                        let err = if verify { voices::speech_error(&mgr, &pcm, rate, language.as_deref(), chunk) } else { None };
                        let mut e = err.unwrap_or(0.0);
                        if let (true, Some((lo, hi))) = (verify, pitch) {
                            // С запасом: живая интонация гуляет.
                            let f0 = voices::median_f0(&pcm, rate);
                            if f0 > 0.0 && (f0 < lo * 0.8 || f0 > hi * 1.25) {
                                tracing::warn!("speak: высота голоса {f0:.0} Гц вне {lo:.0}–{hi:.0} («{}»), попытка {}/3", p.text("speaker").unwrap_or(""), attempt + 1);
                                e = e.max(max_err) + 0.01;
                            }
                        }
                        if e > max_err {
                            let head: String = chunk.chars().take(60).collect();
                            tracing::warn!("speak: проверка — оценка брака {:.0}% («{head}», язык {}), попытка {}/3", e * 100.0, language.as_deref().unwrap_or("—"), attempt + 1);
                        }
                        // Дважды подряд не узнано ничего — скорее язык не тот
                        // (модель его не знает), третий раз не поможет.
                        if attempt > 0 && e >= 0.95 && best.as_ref().is_some_and(|b| b.0 >= 0.95) {
                            break;
                        }
                        if best.as_ref().is_none_or(|b| e < b.0) {
                            best = Some((e, pcm, rate, sd));
                        }
                        if e <= max_err {
                            break;
                        }
                    }
                    let (_, pcm, rate, sd) = best.expect("хотя бы одна попытка");
                    (sr, seed) = (rate, sd);
                    if stream {
                        let fd = wire::memfd_f32("speech-chunk", &pcm)?;
                        let ev = synaptixd::Event::Audio { index, text: chunk.clone(), samples: pcm.len(), sample_rate: rate };
                        let r = Reply { id, event: Some(ev), ..Default::default() };
                        if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[fd.as_fd()]).is_err() {
                            return Err(Error::new(ErrorCode::Failed, "клиент отключился"));
                        }
                    }
                    if !wav.is_empty() {
                        let ms = if last_rep == Some(*rep) { 120 } else { 350 };
                        wav.extend(std::iter::repeat_n(0.0, rate as usize * ms / 1000));
                    }
                    last_rep = Some(*rep);
                    wav.extend_from_slice(&pcm);
                }
                let fd = wire::memfd_f32("speech", &wav)?;
                (data(synaptixd::SpeechReply { samples: wav.len(), sample_rate: sr, audio: FdRef { fd: 0 }, seed }), vec![fd])
            }
            Request::Generate { model, messages, max_tokens, temperature, params, tools, loras } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let want = loras.unwrap_or_else(|| mgr.model_loras(&model));
                let loras = lora::resolve(&mgr.library_paths(), &want).map_err(|e| Error::new(ErrorCode::Failed, e))?;
                let mut call = params;
                if let Some(v) = max_tokens {
                    call.set("max_tokens", Some((v as i64).into()));
                }
                if let Some(v) = temperature {
                    call.set("temperature", Some(v.into()));
                }
                // Параметры запроса, а не настройки модели: схема их не знает.
                let request_only: Vec<(&str, synaptixd::ParamValue)> =
                    ["session", "stop", "max_image_tokens", "json_schema", "priority"].into_iter().filter_map(|k| call.get(k).cloned().map(|v| (k, v))).collect();
                let mut params = mgr.params(&model, &call);
                for (k, v) in request_only {
                    params.set(k, Some(v));
                }
                let mut messages = messages;
                if let Some(user) = mgr.user_system_prompt(&model) {
                    if let Some(sys) = messages.iter_mut().find(|m| m.role == "system") {
                        if !sys.content.contains(&user) {
                            sys.content.push_str("\n\n# Указания пользователя\nРоль, характер и манеру ответов задаёт пользователь — следуй им всегда, не отказываясь от инструментов и правил выше:\n");
                            sys.content.push_str(&user);
                        }
                    }
                }
                let sock = self.sock.as_fd();
                let id = self.current;
                // Отмена: клиент закрыл сокет (кнопка «Стоп») — ход обрывается и
                // на префилле, и между токенами.
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let _watch = {
                    let c = cancel.clone();
                    hangup::watch(sock.as_raw_fd(), move || c.store(true, std::sync::atomic::Ordering::SeqCst))
                };
                let live = activity::current();
                if let Some(l) = &live {
                    l.cancel_flag(cancel.clone());
                }
                let send = |ev: synaptixd::Event| {
                    if let (Some(l), synaptixd::Event::Token { .. } | synaptixd::Event::Thinking { .. } | synaptixd::Event::ToolText { .. }) = (&live, &ev) {
                        l.token();
                    }
                    let r = Reply { id, event: Some(ev), ..Default::default() };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                };
                let g = mgr.run(&model, |m| {
                    m.apply_loras(&loras)?;
                    m.generate(&messages, &params, &tools, &send, &cancel)
                })?;
                if let Some(l) = &live {
                    l.generated(&g);
                }
                (data(g), none())
            }
            Request::Rerank { model, pairs } => {
                let model = mgr.resolve_id(&model, ModelKind::Rerank)?;
                let r = mgr.run(&model, |m| m.rerank(&pairs))?;
                (data(r), none())
            }
            Request::Tokenize { model, texts } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let counts = mgr.run(&model, |m| m.tokenize(&texts))?;
                (data(synaptixd::TokenCounts { counts }), none())
            }
            Request::LlmInfo { model } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let info = mgr.llm_info(&model)?;
                (data(info), none())
            }
            Request::Flow { flow, inputs, files } => flow::serve(&mgr, self.sock.as_fd(), self.current, flow, inputs, files, fds.into_iter().flatten().collect())?,
            Request::Pack { plan, out, options } => {
                let plan: synaptix_bundle::PackPlan = serde_json::from_value(plan).map_err(|e| Error::new(ErrorCode::Protocol, format!("план упаковки: {e}")))?;
                let opts: synaptix::facade::pack::PackOptions = if options.is_null() { Default::default() } else { serde_json::from_value(options).map_err(|e| Error::new(ErrorCode::Protocol, format!("параметры упаковки: {e}")))? };
                let peer = wire::peer(self.sock.as_fd()).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
                let raw = self.sock.as_raw_fd();
                let id = self.current;
                let progress: synaptix_bundle::ProgressCallback = Arc::new(move |ev| {
                    let r = Reply { id, event: Some(synaptixd::Event::Pack { progress: data(ev) }), ..Default::default() };
                    let sock = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) };
                    let _ = wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]);
                });
                // Квант считается на карте: она занята упаковкой, модели ей уступают.
                let quant = !opts.quant.is_empty();
                let _dl = quant.then(|| {
                    mgr.make_vram_room("pack", synaptixd::DeviceKind::Cuda, 4 << 30);
                    devices::lock(synaptixd::DeviceKind::Cuda)
                });
                let out_path = std::path::PathBuf::from(&out);
                if let Some(l) = activity::current() {
                    l.model(&out_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    l.running();
                }
                let r = as_peer(peer.uid, peer.gid, || synaptix::facade::pack::pack(plan, &out_path, &opts, synaptix_core::device::Device::Cuda(0), Some(progress)));
                manager::release_vram(Some(synaptixd::DeviceKind::Cuda));
                r.map_err(|e| Error::new(ErrorCode::Failed, e))?;
                (Value::Null, none())
            }
            Request::Metrics { history } => (data(mgr.metrics(history)), none()),
            Request::Cancel { activity } => {
                let uid = wire::peer(self.sock.as_fd()).map(|p| p.uid).unwrap_or(u32::MAX);
                activity::cancel(activity, uid, self.admin).map_err(|e| Error::new(ErrorCode::Denied, e))?;
                (Value::Null, none())
            }
            Request::Loras => {
                let libs = mgr.library_paths();
                let mut list = lora::scan_loras(&libs);
                // Модель, на которой учился адаптер, — по файлу базы.
                for l in &mut list {
                    if l.base.is_none() && !l.base_path.is_empty() {
                        l.base = mgr.id_of_file(std::path::Path::new(&l.base_path));
                    }
                }
                (data(list), none())
            }
            Request::DeleteLora { name } => {
                let e = lora::find_lora(&mgr.library_paths(), &name).ok_or_else(|| Error::new(ErrorCode::Failed, format!("нет адаптера «{name}»")))?;
                let peer = wire::peer(self.sock.as_fd()).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
                as_peer(peer.uid, peer.gid, || std::fs::remove_dir_all(&e.path)).map_err(|x| Error::new(ErrorCode::Denied, format!("{}: {x}", e.path)))?;
                mgr.notify(synaptixd::Event::LorasChanged);
                (Value::Null, none())
            }
            Request::Datasets => (data(lora::scan_datasets(&mgr.library_paths())), none()),
            Request::DatasetCheck { model, path, options } => {
                let peer = wire::peer(self.sock.as_fd()).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
                (data(train::check_dataset(&mgr, &model, &path, &options, peer.uid, peer.gid)?), none())
            }
            Request::TrainStart { model, dataset, name, options } => {
                let peer = wire::peer(self.sock.as_fd()).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
                (data(train::start(&model, &dataset, &name, options, peer.uid, peer.gid, self.app.clone())?), none())
            }
            Request::TrainJobs => (data(train::jobs()), none()),
            Request::TrainStop { job } => {
                let uid = wire::peer(self.sock.as_fd()).map(|p| p.uid).unwrap_or(u32::MAX);
                train::stop(job, uid, self.admin)?;
                (Value::Null, none())
            }
            Request::TrainForget { job } => {
                let uid = wire::peer(self.sock.as_fd()).map(|p| p.uid).unwrap_or(u32::MAX);
                train::forget(job, uid, self.admin)?;
                (Value::Null, none())
            }
            Request::Federation { prefer } => {
                let Some(fed) = federation::get() else {
                    return Ok((data(synaptixd::FederationInfo::default()), none()));
                };
                if let Some(p) = prefer {
                    if !self.admin {
                        return Err(denied());
                    }
                    fed.set_prefer(&p)?;
                }
                (data(fed.info()), none())
            }
            Request::Subscribe => unreachable!(),
        })
    }

    /// Свой список плюс модели спаренных устройств (не для моста).
    fn with_remote(&self, mut list: Vec<synaptixd::ModelEntry>) -> Vec<synaptixd::ModelEntry> {
        if !self.remote {
            if let Some(f) = federation::get() {
                list.extend(f.entries());
            }
        }
        list
    }

    /// Запрос с другого устройства: входы-файлы из кадра — во временные
    /// файлы; что удалённо нельзя — отказ.
    fn remote_request(&self, bytes: &[u8], fds: &[OwnedFd], made: &mut Vec<std::path::PathBuf>) -> Result<Envelope, Reply> {
        let fail = |id: u64, code: ErrorCode, e: String| Reply { id, ok: Some(false), error: Some(e), code: Some(code), ..Default::default() };
        let mut v: Value = serde_json::from_slice(bytes).map_err(|e| fail(0, ErrorCode::Protocol, e.to_string()))?;
        let id = v["id"].as_u64().unwrap_or(0);
        let dir = self.mgr.cfg.socket.parent().map(|d| d.join("blobs")).unwrap_or_else(std::env::temp_dir);
        *made = federation::materialize(&mut v, fds, &dir).map_err(|e| fail(id, ErrorCode::Protocol, format!("входной файл: {e}")))?;
        let env: Envelope = serde_json::from_value(v).map_err(|e| fail(id, ErrorCode::Protocol, e.to_string()))?;
        if matches!(
            env.req,
            Request::Pack { .. }
                | Request::TrainStart { .. }
                | Request::TrainStop { .. }
                | Request::TrainForget { .. }
                | Request::DeleteLora { .. }
                | Request::DatasetCheck { .. }
                | Request::Cancel { .. }
        ) {
            return Err(fail(id, ErrorCode::Denied, "с другого устройства это нельзя".into()));
        }
        Ok(env)
    }

    fn subscribe(&self, id: u64) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.mgr.subscribe(tx);
        let ok = Reply { id, ok: Some(true), ..Default::default() };
        if self.send(&ok, &[]).is_err() {
            return;
        }
        for ev in rx {
            let r = Reply { id, event: Some(ev), ..Default::default() };
            if self.send(&r, &[]).is_err() {
                return;
            }
        }
    }

    fn serve(mut self) {
        loop {
            let (bytes, fds) = match wire::recv(self.sock.as_fd()) {
                Ok(x) => x,
                Err(_) => break,
            };
            let mut blob_files = Vec::new();
            let parsed = if self.remote {
                self.remote_request(&bytes, &fds, &mut blob_files)
            } else {
                serde_json::from_slice::<Envelope>(&bytes)
                    .map_err(|e| Reply { id: 0, ok: Some(false), error: Some(e.to_string()), code: Some(ErrorCode::Protocol), ..Default::default() })
            };
            // Временные входы моста живут до конца запроса.
            struct Cleanup(Vec<std::path::PathBuf>);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    for p in &self.0 {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
            let _cleanup = Cleanup(blob_files);
            let env = match parsed {
                Ok(e) => e,
                Err(r) => {
                    if self.send(&r, &[]).is_err() {
                        break;
                    }
                    continue;
                }
            };
            // Федерация: модель на спаренном устройстве — переслать туда.
            if !self.remote && !matches!(env.req, Request::Subscribe) {
                if let Some(fed) = federation::get() {
                    match fed.route(&env.req) {
                        federation::Route::Local => {}
                        federation::Route::Fail(e) => {
                            let r = Reply { id: env.id, ok: Some(false), error: Some(e.message), code: Some(e.code), ..Default::default() };
                            if self.send(&r, &[]).is_err() {
                                break;
                            }
                            continue;
                        }
                        federation::Route::Remote(to) => {
                            let cred = wire::peer(self.sock.as_fd()).ok();
                            activity::set_requester(&self.app, req_name(&env.req));
                            let res = fed.forward(&mut self.bridges, self.sock.as_fd(), cred, env.id, &env.req, &fds, &to);
                            match res {
                                federation::Forwarded::Done => continue,
                                federation::Forwarded::Gone => break,
                                federation::Forwarded::Link(why) => {
                                    tracing::info!("федерация: {why}");
                                    if to.explicit {
                                        let r = Reply { id: env.id, ok: Some(false), error: Some(why), code: Some(ErrorCode::Unavailable), ..Default::default() };
                                        if self.send(&r, &[]).is_err() {
                                            break;
                                        }
                                        continue;
                                    }
                                    // Резерв — своя модель.
                                }
                            }
                        }
                    }
                }
            }
            if matches!(env.req, Request::Subscribe) {
                self.subscribe(env.id);
                break;
            }
            let id = env.id;
            self.current = id;
            activity::set_requester(&self.app, req_name(&env.req));
            let work = work_op(&env.req).map(|op| activity::begin(&self.app, op));
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.dispatch(env.req, fds)))
                .unwrap_or_else(|_| Err(Error::new(ErrorCode::Failed, "сбой модели")));
            if let (Some(w), Err(e)) = (&work, &res) {
                w.fail(&e.message);
            }
            drop(work);
            let sent = match res {
                Ok((data, out)) => self.send(&Reply { id, ok: Some(true), data, ..Default::default() }, &out),
                Err(e) => self.send(
                    &Reply { id, ok: Some(false), error: Some(e.message), code: Some(e.code), ..Default::default() },
                    &[],
                ),
            };
            if sent.is_err() {
                break;
            }
        }
        for id in std::mem::take(&mut self.opened) {
            self.mgr.close(&id);
        }
        activity::disconnect(self.conn);
    }
}

/// Работа с файлами от имени программы-клиента: служба — root, а читать и
/// писать она должна только то, что доступно пользователю. fsuid и fsgid
/// у Linux свои у каждого потока, поэтому другие соединения не затронуты.
fn as_peer<T>(uid: u32, gid: u32, f: impl FnOnce() -> T) -> T {
    let (old_gid, old_uid) = unsafe { (libc::setfsgid(gid), libc::setfsuid(uid)) };
    let r = f();
    unsafe {
        libc::setfsuid(old_uid as u32);
        libc::setfsgid(old_gid as u32);
    }
    r
}

static NEXT_CONN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn raise_nofile() {
    unsafe {
        let mut r: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 && r.rlim_cur < r.rlim_max {
            r.rlim_cur = r.rlim_max;
            libc::setrlimit(libc::RLIMIT_NOFILE, &r);
        }
    }
}

fn main() -> anyhow::Result<()> {
    raise_nofile();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synaptixd=info".into()))
        .init();
    devices::init();
    let cfg = config::Config::load();
    if let Some(dir) = cfg.socket.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = wire::listen(&cfg.socket)?;
    std::fs::set_permissions(&cfg.socket, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!("слушаю {}", cfg.socket.display());
    let fed_cfg = cfg.federation.clone();
    let state_dir = cfg.state.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| "/var/lib/synaptix".into());
    let mgr = Manager::new(cfg);
    // Нехватка видеопамяти (загрузка или работа модели) сперва выгружает
    // простаивающие модели — раньше, чем загрузчик уведёт слои на хост или
    // закреплённая модель ужмёт свой кэш: служба в реестре первая.
    let vram_reclaimer: Arc<dyn synaptix_core::memory::reclaim::Reclaimable> = Arc::new(manager::IdleEvictor(Arc::downgrade(&mgr)));
    synaptix_core::memory::reclaim::register(&vram_reclaimer);
    federation::init(mgr.clone(), fed_cfg, state_dir);
    train::init(mgr.clone());
    {
        let m = mgr.clone();
        std::thread::Builder::new().name("monitor".into()).spawn(move || m.monitor())?;
    }
    {
        let m = mgr.clone();
        std::thread::Builder::new().name("vram-yield".into()).spawn(move || m.watch_vram_yield())?;
        let m = mgr.clone();
        std::thread::Builder::new().name("preload".into()).spawn(move || m.preload())?;
        let m = mgr.clone();
        activity::sampler(move || m.models_mem());
    }
    loop {
        let sock = match wire::accept(listener.as_fd()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept: {e}");
                continue;
            }
        };
        let peer = wire::peer(sock.as_fd()).ok();
        let admin = peer.as_ref().map(|p| is_admin(p.uid)).unwrap_or(false);
        let app = peer.map(|p| activity::app_of(p.pid, p.uid)).unwrap_or_default();
        let id = NEXT_CONN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        activity::connect(id, &app);
        let conn = Conn { mgr: mgr.clone(), sock, admin, opened: Vec::new(), current: 0, conn: id, app, remote: false, bridges: Default::default() };
        let _ = std::thread::Builder::new().name("conn".into()).spawn(move || conn.serve());
    }
}
