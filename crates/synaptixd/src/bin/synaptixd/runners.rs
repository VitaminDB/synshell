use std::path::Path;

use synaptix_asr_wav2vec2::Wav2Vec2Ctc;
use synaptix_core::dtype::DType;
use synaptixd::{Alignment, CtcInfo, DeviceKind, ModelKind, Params, PhoneSpan};

use crate::devices;

const SESSION_STEP: usize = 16384;
/// Сколько токенов эмбеддингов медиа держать в видеопамяти между запросами.
const MEDIA_BUDGET: usize = 24_576;
/// Кадров видео, когда модель видео не принимает (идут картинками).
const VIDEO_FRAMES: usize = 6;

pub struct LlmSlot {
    llm: synaptix::facade::llm::Llm,
    tok: synaptix::facade::llm::LlmTokenizer,
    window: usize,
    session: std::sync::Mutex<Option<synaptix::facade::llm::LlmKvSession>>,
    /// Эмбеддинги картинок и видео истории: ключ — файл, время изменения и
    /// вид; старые вытесняются по бюджету токенов.
    media: std::sync::Mutex<Vec<(String, synaptix::facade::llm::MediaEmbedding)>>,
}

/// Медиа сообщения для промпта: блок токенов (или строка-замена) по порядку.
enum MediaPart {
    Embed(String),
    Text(String),
}

/// Движок синтеза речи — по архитектуре модели.
pub enum TtsEngine {
    Omni(synaptix_tts_omnivoice::OmniVoicePipeline),
    Vibe(synaptix_tts_vibevoice::VibeVoicePipeline),
    Vox(synaptix_tts_voxcpm::VoxCpmPipeline),
}

/// Голос, которым продолжать: у OmniVoice — референс в кодах кодека, у
/// остальных — уже сказанная речь (PCM), подаваемая как образец.
pub enum Voice {
    Omni(synaptix_tts_omnivoice::VoiceClonePrompt),
    Pcm { pcm: Vec<f32>, sr: u32 },
}

/// Синтезатор речи и запомненные голоса: `voices` — голоса разговоров
/// (ключ: язык + описание голоса + закреплённое зерно), чтобы все фразы одной
/// озвучки и все ответы звучали одним голосом; `anchors` — опорные фразы по
/// зерну (см. [`anchor_text`]); `refs` — клоны с образцов пользователя (ключ:
/// путь + текст образца).
pub struct TtsState {
    engine: TtsEngine,
    voices: std::collections::HashMap<String, Voice>,
    anchors: std::collections::HashMap<String, Voice>,
    refs: std::collections::HashMap<String, Voice>,
}

/// Опорные фразы, задающие голос без образца пользователя. Голос модели без
/// образца выбирают зерно и сам текст: одно зерно на разных фразах — разные
/// голоса, и ответ ассистента звучал не тем голосом, что проверка в
/// Параметрах (там текст один и тот же). Поэтому голос ключа задаёт опорная
/// запись — эти две фразы, озвученные зерном (у закреплённого зерна она всегда
/// одна), а любой текст — и проверка, и ответы — продолжает её как образец.
/// Образец ответами не дорастает: иначе голос уплывал бы с каждой фразой.
/// Первая фраза — та же, что в проверке Параметров по умолчанию: закреплённое
/// по ней зерно даёт тот же голос.
fn anchor_text(language: Option<&str>) -> [&'static str; 2] {
    match language.map(|l| l.trim().to_ascii_lowercase()).as_deref() {
        Some("en") => ["Hello! This is how this model sounds with the chosen settings.", "I will read any text in this voice."],
        Some("kk") => ["Сәлем! Бұл модель таңдалған параметрлермен осылай естіледі.", "Кез келген мәтінді осы дауыспен оқимын."],
        _ => ["Привет! Так звучит эта модель с заданными настройками.", "Этим голосом я прочитаю любой текст."],
    }
}

fn anchor_key(key: &str, seed: u64) -> String {
    format!("{key}\x1f{seed}")
}

/// Запомнить опорную фразу зерна: проверка со случайным зерном и его
/// закрепление потом не озвучивают её заново. Старые — по 16 штук — забываются.
fn remember_anchor(anchors: &mut std::collections::HashMap<String, Voice>, key: String, voice: Voice) {
    if anchors.len() >= 16 {
        anchors.clear();
    }
    anchors.insert(key, voice);
}

/// VibeVoice читает сценарий «Speaker N: …»; обычный текст — одним голосом.
fn vibe_script(text: &str) -> String {
    let has_speakers = text.lines().any(|l| l.trim_start().to_ascii_lowercase().starts_with("speaker"));
    if has_speakers {
        text.to_string()
    } else {
        format!("Speaker 0: {}", text.trim().replace('\n', " "))
    }
}

/// Голос PCM для продолжения (VibeVoice, VoxCPM): запомненный по ключу
/// (не при `fresh`), иначе опорная фраза зерна — из кэша или озвученная `synth`.
fn pcm_voice(
    voices: &std::collections::HashMap<String, Voice>,
    anchors: &mut std::collections::HashMap<String, Voice>,
    key: &str,
    akey: String,
    fresh: bool,
    synth: impl FnOnce() -> anyhow::Result<(Vec<f32>, u32)>,
) -> anyhow::Result<(Vec<f32>, u32)> {
    if let Some(Voice::Pcm { pcm, sr }) = voices.get(key).filter(|_| !fresh) {
        return Ok((pcm.clone(), *sr));
    }
    if let Some(Voice::Pcm { pcm, sr }) = anchors.get(&akey) {
        return Ok((pcm.clone(), *sr));
    }
    let (pcm, sr) = synth()?;
    remember_anchor(anchors, akey, Voice::Pcm { pcm: pcm.clone(), sr });
    Ok((pcm, sr))
}

pub enum Loaded {
    Ctc(Box<Wav2Vec2Ctc>),
    Mfa(Box<synaptix_align_mfa::MfaModel>),
    Asr(std::sync::Mutex<synaptix::facade::asr::Transcriber>),
    Llm(Box<LlmSlot>),
    Tts(std::sync::Mutex<TtsState>, DeviceKind),
    Embed(Box<dyn synaptix::facade::embedding::Embedder + Send + Sync>),
}

pub struct CtcResult {
    pub frames: usize,
    pub vocab: usize,
    pub frame_sec: f32,
    pub logprobs: Vec<f32>,
    pub hidden: Option<Vec<f32>>,
    pub hidden_dim: usize,
}

pub fn runnable(kind: ModelKind) -> bool {
    matches!(kind, ModelKind::Ctc | ModelKind::Mfa | ModelKind::Asr | ModelKind::Llm | ModelKind::Tts | ModelKind::Embed)
}

fn llm_policy(path: &Path, dev: DeviceKind, quant: Option<&str>) -> synaptix::facade::llm::QuantPolicy {
    use synaptix::facade::llm::{optimal_profile_for, optimal_profile_for_device, QuantPolicy};
    let mut p = optimal_profile_for_device(path, devices::device(dev)).policy;
    match quant {
        Some("int8") => {
            let q8 = DType::Sq { bits: 8 };
            p.weights_storage = q8;
            p.attn_storage = None;
            p.lm_head_storage = q8;
        }
        Some("nvfp4") if dev == DeviceKind::Cuda => p = optimal_profile_for(path, None).policy,
        Some("mxfp8") if dev == DeviceKind::Cuda => {
            p = optimal_profile_for(path, None).policy;
            let fp8 = |d: DType| if d == DType::NVFP4 { DType::MXFP8 } else { d };
            p.weights_storage = fp8(p.weights_storage);
            p.attn_storage = p.attn_storage.map(fp8);
            p.lm_head_storage = fp8(p.lm_head_storage);
            p.embed_storage = fp8(p.embed_storage);
        }
        Some("bf16") if dev == DeviceKind::Cuda => p = QuantPolicy::quality(),
        _ => {}
    }
    p
}

pub fn dtype_label(d: DType) -> String {
    match d {
        DType::Sq { bits } => format!("sq{bits}"),
        DType::NVFP4 => "nvfp4".into(),
        DType::MXFP8 => "mxfp8".into(),
        DType::BF16 => "bf16".into(),
        DType::F16 => "f16".into(),
        DType::F32 => "f32".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Варианты точности аудио-моделей (ASR, TTS, CTC): квантование линейных
/// слоёв на лету при загрузке (int8, NVFP4, MXFP8) независимо от формата
/// весов, либо плотная точность счёта.
pub const AUDIO_PRECISIONS: [&str; 6] = ["int8", "nvfp4", "mxfp8", "f16", "bf16", "f32"];

/// (квант линейных слоёв, точность счёта) аудио-модели по выбору
/// пользователя; без выбора — плотные веса в `accel` на ускорителе и F32
/// на CPU. Квант-GEMM считает в F16 (на BF16-входе NVFP4 через QuantLinear
/// заметно теряет качество — VibeVoice), на CPU — F32.
fn audio_precision(dev: DeviceKind, quant: Option<&str>, accel: DType) -> (Option<DType>, DType) {
    let cpu = dev == DeviceKind::Cpu;
    match quant {
        // NPU исполняет сжатые веса только как int8 по строке (q8row): SQ-блоки,
        // MXFP8 и NVFP4 — тем же int8.
        Some("int8" | "mxfp8" | "nvfp4") if dev == DeviceKind::Npu => (Some(DType::Q8Row), DType::F16),
        Some("int8") => (Some(DType::Sq { bits: 8 }), if cpu { DType::F32 } else { DType::F16 }),
        Some("nvfp4") => (Some(DType::NVFP4), if cpu { DType::F32 } else { DType::F16 }),
        Some("mxfp8") => (Some(DType::MXFP8), if cpu { DType::F32 } else { DType::F16 }),
        Some("f16") => (None, DType::F16),
        Some("bf16") => (None, DType::BF16),
        Some("f32") => (None, DType::F32),
        _ => (None, if cpu { DType::F32 } else { accel }),
    }
}

/// Подпись точности: квант линейных слоёв, иначе точность счёта.
fn precision_tag(quant: Option<DType>, compute: DType) -> String {
    dtype_label(quant.unwrap_or(compute))
}

pub fn load(kind: ModelKind, arch: &str, path: &Path, dev: DeviceKind, quant: Option<&str>) -> anyhow::Result<(Loaded, Option<String>)> {
    let plain = |l: Loaded| Ok((l, None));
    match kind {
        ModelKind::Ctc => {
            let device = devices::device(dev);
            let (q, dtype) = audio_precision(dev, quant, DType::F16);
            let m = Wav2Vec2Ctc::load_quant(path, &device, dtype, q).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok((Loaded::Ctc(Box::new(m)), Some(precision_tag(q, dtype))))
        }
        ModelKind::Mfa => {
            let m = synaptix_align_mfa::MfaModel::load(path).map_err(|e| anyhow::anyhow!("{e}"))?;
            plain(Loaded::Mfa(Box::new(m)))
        }
        ModelKind::Asr => {
            use synaptix::facade::asr::{AsrConfig, AsrModelKind, ComputeDType, Transcriber};
            let device = devices::device(dev);
            let (q, dtype) = audio_precision(dev, quant, DType::F16);
            let compute = match dtype {
                DType::F32 => ComputeDType::F32,
                DType::BF16 => ComputeDType::BF16,
                _ => ComputeDType::F16,
            };
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            let kind = if arch.to_lowercase().contains("gigaam") || stem.contains("gigaam") { AsrModelKind::GigaAm } else { AsrModelKind::Whisper };
            let t = Transcriber::load(AsrConfig {
                kind,
                model_path: path.to_path_buf(),
                language: None,
                device,
                storage_dtype: q.unwrap_or_else(|| compute.to_dtype()),
                compute_dtype: compute,
            })
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok((Loaded::Asr(std::sync::Mutex::new(t)), Some(precision_tag(q, dtype))))
        }
        ModelKind::Llm => {
            use synaptix::facade::llm::load_llm;
            let device = devices::device(dev);
            let policy = llm_policy(path, dev, quant);
            let label = dtype_label(policy.weights_storage);
            let precision = policy.to_precision().map_err(|e| anyhow::anyhow!("{e}"))?;
            let max_seq = if dev == DeviceKind::Npu { Some(2048) } else { synaptix::facade::arch::config_max_seq(path) };
            let (llm, tok) = load_llm(path, device, precision, max_seq).map_err(|e| anyhow::anyhow!("{e}"))?;
            let window = llm.config().max_seq_len.max(2048);
            Ok((Loaded::Llm(Box::new(LlmSlot { llm, tok, window, session: std::sync::Mutex::new(None), media: std::sync::Mutex::new(Vec::new()) })), Some(label)))
        }
        ModelKind::Tts => {
            let device = devices::device(dev);
            let (q, dtype) = audio_precision(dev, quant, if dev == DeviceKind::Npu { DType::F16 } else { DType::BF16 });
            let a = arch.to_ascii_lowercase();
            let engine = if a.contains("vibevoice") {
                TtsEngine::Vibe(synaptix_tts_vibevoice::VibeVoicePipeline::from_syn_quant(path, device, dtype, q).map_err(|e| anyhow::anyhow!("{e}"))?)
            } else if a.starts_with("voxcpm") {
                TtsEngine::Vox(synaptix_tts_voxcpm::VoxCpmPipeline::from_bundle_quant(path, device, dtype, q).map_err(|e| anyhow::anyhow!("{e}"))?)
            } else {
                TtsEngine::Omni(synaptix_tts_omnivoice::OmniVoicePipeline::from_syn_quant(path, device, dtype, q).map_err(|e| anyhow::anyhow!("{e}"))?)
            };
            let st = TtsState { engine, voices: Default::default(), anchors: Default::default(), refs: Default::default() };
            Ok((Loaded::Tts(std::sync::Mutex::new(st), dev), Some(precision_tag(q, dtype))))
        }
        ModelKind::Embed => {
            use synaptix::facade::embedding::{load_embedder, DType as EDType, EmbedderConfig};
            let dtype = if dev == DeviceKind::Cpu { EDType::F32 } else { EDType::F16 };
            let e = load_embedder(EmbedderConfig::new(path.to_path_buf()).with_device(devices::device(dev)).with_dtype(dtype)).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok((Loaded::Embed(e), Some(dtype_label(match dtype { EDType::F32 => DType::F32, _ => DType::F16 }))))
        }
        other => anyhow::bail!("модели {other:?} ({arch}) служба пока не исполняет"),
    }
}

impl Loaded {
    pub fn ctc_info(&self) -> Option<CtcInfo> {
        match self {
            Loaded::Ctc(m) => Some(CtcInfo {
                vocab: m.vocab().tokens().to_vec(),
                blank_id: m.blank_id(),
                hidden_size: m.config().hidden_size,
                frame_sec: m.config().hop_samples() as f32 / 16000.0,
                sample_rate: 16000,
                length_buckets: if m.length_buckets_active() { Wav2Vec2Ctc::length_buckets().to_vec() } else { Vec::new() },
            }),
            _ => None,
        }
    }

    pub fn ctc(&self, pcm: &[f32], sample_rate: u32, hidden: bool) -> anyhow::Result<CtcResult> {
        let Loaded::Ctc(m) = self else { anyhow::bail!("модель не CTC") };
        if hidden {
            let (lp, h, hd) = m.log_probs_hidden(pcm, sample_rate).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(CtcResult { frames: lp.frames, vocab: lp.vocab, frame_sec: lp.frame_sec, logprobs: lp.data, hidden: Some(h), hidden_dim: hd })
        } else {
            let lp = m.log_probs(pcm, sample_rate).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(CtcResult { frames: lp.frames, vocab: lp.vocab, frame_sec: lp.frame_sec, logprobs: lp.data, hidden: None, hidden_dim: 0 })
        }
    }

    pub fn mfa_align(
        &self,
        pcm16k: &[f32],
        words: &[synaptixd::AlignWordAlts],
        options: Option<&synaptixd::AlignOptions>,
    ) -> anyhow::Result<Alignment> {
        use synaptix_align_mfa as mfa;
        let Loaded::Mfa(m) = self else { anyhow::bail!("модель не MFA") };
        let words: Vec<mfa::AlignWordAlts> = words
            .iter()
            .map(|w| mfa::AlignWordAlts {
                alts: w
                    .alts
                    .iter()
                    .map(|(a, p)| {
                        (
                            mfa::AlignWord {
                                phones: a.phones.clone(),
                                silence_after: a.silence_after,
                                silence_before_correction: a.silence_before_correction,
                                non_silence_before_correction: a.non_silence_before_correction,
                            },
                            *p,
                        )
                    })
                    .collect(),
            })
            .collect();
        let opts = options
            .map(|o| mfa::AlignOptions {
                acoustic_scale: o.acoustic_scale,
                transition_scale: o.transition_scale,
                self_loop_scale: o.self_loop_scale,
                beam: o.beam,
                retry_beam: o.retry_beam,
            })
            .unwrap_or_default();
        let feats = m.features(pcm16k);
        let al = mfa::align_alts(m, &feats, &words, &opts).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Alignment {
            phones: al
                .phones
                .into_iter()
                .map(|p| PhoneSpan { phone: p.phone, word: p.word, alt: p.alt, start: p.start, end: p.end, loglike: p.loglike })
                .collect(),
            frame_shift: al.frame_shift,
            score: al.score,
        })
    }

    /// Векторы текстов (нормированные, косинус = скалярное произведение).
    pub fn embed(&self, texts: &[String]) -> anyhow::Result<synaptixd::Embeddings> {
        let Loaded::Embed(e) = self else { anyhow::bail!("модель не строит эмбеддинги") };
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vectors = e.encode(&refs).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(synaptixd::Embeddings { dim: e.dim(), vectors })
    }

    pub fn transcribe(&self, pcm: &[f32], sample_rate: u32, language: Option<&str>) -> anyhow::Result<synaptixd::Transcript> {
        let Loaded::Asr(t) = self else { anyhow::bail!("модель не распознаёт речь") };
        let mut t = t.lock().unwrap_or_else(|e| e.into_inner());
        let text = t.transcribe_pcm_text(pcm, sample_rate, language).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(synaptixd::Transcript { text: text.trim().to_string(), segments: Vec::new() })
    }

    pub fn generate(&self, messages: &[synaptixd::Message], params: &Params, tools: &[serde_json::Value], on_event: &dyn Fn(synaptixd::Event)) -> anyhow::Result<synaptixd::Generated> {
        use synaptix::facade::chat::{prompt_for_model, StreamParser};
        use synaptix::facade::llm::{GenerationOptions, LlmGeneration, Message};
        let Loaded::Llm(b) = self else { anyhow::bail!("модель не языковая") };
        let (llm, tok) = (&b.llm, &b.tok);
        let max_tokens = params.u32("max_tokens").unwrap_or(16384).max(1) as usize;
        let system = params.text("system_prompt");
        let lead = system.filter(|_| !messages.iter().any(|m| m.role == "system")).map(Message::system);
        // Картинки и видео: эмбеддинги (из кэша или башней зрения), блоки — перед текстом.
        let mut cache = b.media.lock().unwrap_or_else(|e| e.into_inner());
        let parts: Vec<Vec<MediaPart>> = encode_media(b, &mut cache, messages, params);
        let mut order: Vec<String> = Vec::new();
        let with_media = |m: &synaptixd::Message, parts: &[MediaPart], order: &mut Vec<String>| -> String {
            if parts.is_empty() {
                return m.content.clone();
            }
            let mut s = String::new();
            for p in parts {
                match p {
                    MediaPart::Embed(key) => {
                        let e = &cache.iter().find(|(k, _)| k == key).expect("эмбеддинг в кэше").1;
                        s.push_str(&e.prompt_block);
                        s.push('\n');
                        order.push(key.clone());
                    }
                    MediaPart::Text(t) => {
                        s.push_str(t);
                        s.push('\n');
                    }
                }
            }
            s.push_str(&m.content);
            s
        };
        let msgs: Vec<Message> = lead
            .into_iter()
            .chain(messages.iter().zip(parts.iter())
            .map(|(m, parts)| match m.role.as_str() {
                "system" => Message::system(m.content.clone()),
                "assistant" => Message::assistant_turn(
                    m.content.clone(),
                    m.reasoning.clone().filter(|r| !r.is_empty()),
                    m.tool_calls.iter().map(|c| (c.name.clone(), c.arguments.clone())).collect(),
                ),
                "tool" => match &m.name {
                    Some(n) => Message::tool_named(n.clone(), with_media(m, parts, &mut order)),
                    None => Message::tool(with_media(m, parts, &mut order)),
                },
                _ => Message::user(with_media(m, parts, &mut order)),
            }))
            .collect();
        let refs: Vec<&synaptix::facade::llm::MediaEmbedding> =
            order.iter().filter_map(|k| cache.iter().find(|(c, _)| c == k).map(|(_, e)| e)).collect();
        let thinking = params.bool("thinking").unwrap_or(false);
        let tools_opt = (!tools.is_empty()).then_some(tools);
        let prompt = tok.apply_chat_template_reasoning(&msgs, true, thinking, params.text("reasoning"), tools_opt).map_err(|e| anyhow::anyhow!("{e}"))?;
        let prompt = prompt_for_model(tok, thinking, prompt);
        let parser = std::cell::RefCell::new(StreamParser::for_model(tok, thinking, &prompt));
        let reasoning = std::cell::RefCell::new(String::new());
        let ids = tok.encode(&prompt).map_err(|e| anyhow::anyhow!("{e}"))?;
        let window = params.u32("context").map(|v| v as usize).filter(|v| *v > 0).unwrap_or(b.window).clamp(2048, b.window);
        let room = window.saturating_sub(ids.len() + 16);
        if room < 64 {
            anyhow::bail!("промпт ({} токенов) не помещается в контекст {window} токенов — увеличьте размер контекста модели или сократите историю", ids.len());
        }
        let max_tokens = max_tokens.min(room);
        let opts = GenerationOptions {
            max_new_tokens: max_tokens,
            max_seq_len: ids.len() + max_tokens + 16,
            temperature: params.f32("temperature").unwrap_or(0.7),
            top_k: params.u32("top_k").unwrap_or(20) as usize,
            top_p: params.f32("top_p").unwrap_or(0.95),
            min_p: params.f32("min_p").unwrap_or(0.0),
            seed: match params.i64("seed") {
                Some(s) if s >= 0 => s as u64,
                _ => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1,
            },
            repeat_penalty: params.f32("repeat_penalty").unwrap_or(1.0),
            repeat_last_n: 64,
            presence_penalty: params.f32("presence_penalty").unwrap_or(0.0),
            frequency_penalty: 0.0,
        };
        let prof = std::env::var_os("SYNAPTIX_LLM_PROF").is_some();
        synaptix_llm_common::model::set_prefill_prof(prof);
        let mut gen = LlmGeneration::new(llm, opts);
        gen.set_stop_tokens(tok.eos_ids().to_vec());
        let mut text = String::new();
        let mut tokens = 0usize;
        let on_piece = |id: u32, piece: &str| {
            tokens += 1;
            let (body, think, tool) = parser.borrow_mut().feed(id, piece);
            if !body.is_empty() {
                text.push_str(&body);
                on_event(synaptixd::Event::Token { text: body });
            }
            if !think.is_empty() {
                reasoning.borrow_mut().push_str(&think);
                on_event(synaptixd::Event::Thinking { text: think });
            }
            if !tool.is_empty() {
                on_event(synaptixd::Event::ToolText { text: tool });
            }
            !parser.borrow().tool_call_ready()
        };
        let need = ids.len() + max_tokens + 16;
        let mut session = Some(b.session.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(s) = session.as_mut() {
            if s.as_ref().is_none_or(|k| k.ctx_tokens() < need) {
                **s = None;
                let want = need.div_ceil(SESSION_STEP).saturating_mul(SESSION_STEP).clamp(need, window);
                **s = match llm.new_kv_session(want, max_tokens) {
                    Ok(k) => {
                        tracing::info!("generate: кэш контекста на {want} токенов (предел {window})");
                        k
                    }
                    Err(e) => {
                        tracing::warn!("generate: кэш на {want} токенов не создан ({e}) — без кэша");
                        None
                    }
                };
            }
        }
        let t0 = std::time::Instant::now();
        let reused = match session.as_mut().and_then(|s| s.as_mut()) {
            Some(s) if refs.is_empty() => gen.generate_streaming_cached(s, &ids, tok, on_piece).map_err(|e| anyhow::anyhow!("{e}"))?,
            Some(s) if llm.kv_session_media_ok() => gen.generate_streaming_cached_media(s, &ids, tok, &refs, on_piece).map_err(|e| anyhow::anyhow!("{e}"))?,
            _ if !refs.is_empty() => {
                gen.generate_streaming_media(&ids, tok, &refs, on_piece).map_err(|e| anyhow::anyhow!("{e}"))?;
                0
            }
            _ => {
                gen.generate_streaming(&ids, tok, on_piece).map_err(|e| anyhow::anyhow!("{e}"))?;
                0
            }
        };
        tracing::info!("generate: промпт {} токенов (из кэша {reused}), ответ {tokens} за {:.1} с", ids.len(), t0.elapsed().as_secs_f32());
        if prof {
            eprintln!("{}", synaptix_llm_common::model::prefill_prof_report_and_clear());
        }
        let mut parser = parser.into_inner();
        let tail = parser.take_tail();
        if !tail.is_empty() {
            text.push_str(&tail);
            on_event(synaptixd::Event::Token { text: tail });
        }
        let tool_calls: Vec<synaptixd::ToolCall> =
            parser.finish().into_iter().map(|c| synaptixd::ToolCall { name: c.name, arguments: c.arguments_json }).collect();
        Ok(synaptixd::Generated { text, tokens, thinking: reasoning.into_inner(), tool_calls, prompt_tokens: ids.len(), context: window })
    }

    /// `fresh` — проверка настроек: голос прошлых озвучек не берётся и не
    /// запоминается. Возвращает и зерно (у движков, которые его используют).
    pub fn speak(&self, text: &str, params: &Params, fresh: bool) -> anyhow::Result<(Vec<f32>, u32, Option<u64>)> {
        let language = params.text("language");
        let instruct = params.text("voice");
        let (speed, steps) = (params.f32("speed"), params.u32("steps").map(|s| s as usize));
        let Loaded::Tts(p, dev) = self else { anyhow::bail!("модель не синтезирует речь") };
        let mut guard = p.lock().unwrap_or_else(|e| e.into_inner());
        let st = &mut *guard;
        let speed = speed.unwrap_or(1.0) as f64;
        let guidance = params.f32("guidance");
        // Зерно: заданное — повторяемый результат; иначе случайное (с одним и
        // тем же зерном отдельные сочетания текста и образца дают брак).
        // Случайное — в пределах поля «Зерно», чтобы его можно было закрепить.
        let pinned = params.i64("seed").filter(|s| *s >= 0).map(|s| s as u64);
        let seed = pinned.unwrap_or_else(|| {
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
            ((n ^ (n >> 32)) as u32 | 1) as u64
        });
        let ref_audio = params.text("ref_audio").map(str::trim).filter(|s| !s.is_empty());
        let ref_text = params.text("ref_text").unwrap_or("").trim().to_string();
        // Голос для продолжения: фразы одной озвучки и все ответы — одним
        // голосом. Его выбирает опорная фраза ([`anchor_text`]) по описанию и
        // зерну, дальше он держится по уже сказанному: у OmniVoice — референс
        // в кодах, у VibeVoice и VoxCPM — PCM как образец. Другое закреплённое
        // зерно — другой голос, поэтому оно тоже в ключе.
        let key = format!("{}\x1f{}\x1f{}", language.unwrap_or(""), instruct.unwrap_or(""), pinned.map(|s| s.to_string()).unwrap_or_default());
        let akey = anchor_key(&key, seed);
        let anchor = anchor_text(language);
        match &st.engine {
            TtsEngine::Omni(p) => {
                let default_steps = if *dev == DeviceKind::Npu { 16 } else { 32 };
                let gen = synaptix_tts_omnivoice::OmniVoiceGenerationConfig {
                    num_step: steps.unwrap_or(default_steps).clamp(4, 64),
                    guidance_scale: guidance.unwrap_or(2.0),
                    position_temperature: params.f32("position_temperature").unwrap_or(5.0),
                    class_temperature: params.f32("class_temperature").unwrap_or(0.0),
                    seed: Some(seed),
                    ..Default::default()
                };
                let wav = if let Some(path) = ref_audio {
                    // Образец пользователя: клон голоса с записи.
                    let rkey = format!("{path}\x1f{ref_text}");
                    if !st.refs.contains_key(&rkey) {
                        let pr = p.create_voice_clone_prompt(path, &ref_text).map_err(|e| anyhow::anyhow!("образец голоса {path}: {e}"))?;
                        st.refs.insert(rkey.clone(), Voice::Omni(pr));
                    }
                    let prompt = match st.refs.get(&rkey) {
                        Some(Voice::Omni(pr)) => Some(pr),
                        _ => None,
                    };
                    p.synthesize(text, prompt, language, None, speed, None, &gen).map_err(|e| anyhow::anyhow!("{e}"))?
                } else {
                    let mut voice = match st.voices.remove(&key) {
                        Some(Voice::Omni(v)) if !fresh => Some(v),
                        Some(v) => {
                            st.voices.insert(key.clone(), v);
                            None
                        }
                        None => None,
                    };
                    if voice.is_none() {
                        // Голос выбирает опорная запись зерном (скорость — обычная,
                        // чтобы голос не зависел от неё), текст продолжает её.
                        voice = Some(match st.anchors.get(&akey) {
                            Some(Voice::Omni(a)) => a.clone(),
                            _ => {
                                let mut a = None;
                                for s in anchor {
                                    p.synthesize_continuing(s, &mut a, language, instruct, 1.0, &gen, f32::MAX).map_err(|e| anyhow::anyhow!("{e}"))?;
                                }
                                let a = a.ok_or_else(|| anyhow::anyhow!("опорная фраза не озвучена"))?;
                                remember_anchor(&mut st.anchors, akey, Voice::Omni(a.clone()));
                                a
                            }
                        });
                    }
                    // Образец — только опорная запись (0 с дорастания).
                    let r = p.synthesize_continuing(text, &mut voice, language, instruct, speed, &gen, 0.0);
                    if let Some(v) = voice.filter(|_| !fresh) {
                        st.voices.insert(key, Voice::Omni(v));
                    }
                    r.map_err(|e| anyhow::anyhow!("{e}"))?
                };
                Ok((wav, p.output_sample_rate() as u32, Some(seed)))
            }
            TtsEngine::Vibe(p) => {
                use synaptix_tts_vibevoice::pipeline::VoiceSample;
                let cfg = synaptix_tts_vibevoice::GenerationConfig {
                    cfg_scale: guidance.unwrap_or(1.3),
                    ddpm_inference_steps: steps.unwrap_or(20).clamp(1, 200),
                    seed,
                    ..Default::default()
                };
                let sr = p.sample_rate();
                let script = vibe_script(text);
                // Голос без образца пользователя: запомненный по ключу, иначе
                // опорная фраза зерна (она же — начало запоминаемого голоса).
                let mut voice = None;
                let sample = if let Some(path) = ref_audio {
                    Some(VoiceSample::from_path(path).map_err(|e| anyhow::anyhow!("образец голоса {path}: {e}"))?)
                } else {
                    let (pcm, vsr) = pcm_voice(&st.voices, &mut st.anchors, &key, akey, fresh, || {
                        let a = p.synthesize(&vibe_script(&anchor.join(" ")), &[], &cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
                        Ok((a.audio, sr))
                    })?;
                    voice = Some(Voice::Pcm { pcm: pcm.clone(), sr: vsr });
                    Some(VoiceSample::new(pcm, vsr))
                };
                let voices: Vec<VoiceSample> = sample.into_iter().collect();
                let out = p.synthesize(&script, &voices, &cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
                tracing::info!(
                    "vibevoice: образцов {}, токенов {}, звука {:.1} с{}",
                    voices.len(),
                    out.tokens.len(),
                    out.audio.len() as f32 / sr as f32,
                    if out.reached_max { ", обрыв по пределу длины" } else { "" }
                );
                if let Some(v) = voice.filter(|_| !fresh) {
                    st.voices.insert(key, v);
                }
                Ok((out.audio, sr, Some(seed)))
            }
            TtsEngine::Vox(p) => {
                let opts = synaptix_tts_voxcpm::GenerateOptions {
                    n_timesteps: steps.unwrap_or(10).clamp(1, 200),
                    cfg_value: guidance.unwrap_or(2.0),
                    seed,
                    ..Default::default()
                };
                // Описание голоса VoxCPM2 берёт скобками перед текстом: без
                // образца — голос по описанию (voice design), с образцом —
                // манера поверх его тембра. С текстом образца не сочетается.
                let design = instruct.map(str::trim).filter(|d| !d.is_empty());
                let styled = design.map(|d| format!("({d}){text}")).unwrap_or_else(|| text.to_string());
                let mut voice = None;
                let out = if let Some(path) = ref_audio {
                    if ref_text.is_empty() {
                        p.synthesize_with_reference(&styled, path, &opts)
                    } else {
                        // Полный клон: та же запись и образцом тембра, и
                        // началом, которое модель продолжает.
                        p.synthesize_combined(text, &ref_text, path, path, &opts)
                    }
                } else {
                    // Голос без образца пользователя: запомненный по ключу, иначе
                    // опорная фраза зерна (по описанию), и текст — с ней как с образцом.
                    let (pcm, vsr) = pcm_voice(&st.voices, &mut st.anchors, &key, akey, fresh, || {
                        let anchor = anchor.join(" ");
                        let styled_anchor = design.map(|d| format!("({d}){anchor}")).unwrap_or(anchor);
                        let a = p.synthesize(&styled_anchor, &opts).map_err(|e| anyhow::anyhow!("{e}"))?;
                        Ok((a.pcm, a.sample_rate as u32))
                    })?;
                    let r = p.synthesize_with_reference_pcm(&styled, &pcm, vsr, &opts);
                    voice = Some(Voice::Pcm { pcm, sr: vsr });
                    r
                }
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                let sr = out.sample_rate as u32;
                if let Some(v) = voice.filter(|_| !fresh) {
                    st.voices.insert(key, v);
                }
                Ok((out.pcm, sr, Some(seed)))
            }
        }
    }

    /// Отдать видеопамять кэшей модели (KV-сессия, резидентные эксперты MoE)
    /// до `Drop`: он может задержаться, пока жива хоть одна ссылка.
    pub fn release_device_caches(&self) {
        if let Loaded::Llm(b) = self {
            if let Ok(mut s) = b.session.lock() {
                *s = None;
            }
            if let Ok(mut m) = b.media.lock() {
                m.clear();
            }
            b.llm.release_device_caches();
        }
    }

    pub fn warmup(&self) -> anyhow::Result<()> {
        if let Loaded::Ctc(m) = self {
            if m.length_buckets_active() {
                for &n in Wav2Vec2Ctc::length_buckets() {
                    let pcm = noise(n.saturating_sub(1));
                    m.log_probs(&pcm, 16000).map_err(|e| anyhow::anyhow!("{e}"))?;
                }
            }
        }
        Ok(())
    }
}

fn noise(n: usize) -> Vec<f32> {
    let mut x = 12345u32;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((x >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.02
        })
        .collect()
}

/// Ключ эмбеддинга: файл, время его изменения и вид.
fn media_key(path: &Path, kind: &str) -> String {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    format!("{}:{mtime}:{kind}", path.display())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Кадры видео равномерно по длительности (ffmpeg) — для модели без видеовхода.
fn video_frames(path: &Path, n: usize) -> anyhow::Result<Vec<(f64, std::path::PathBuf)>> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()?;
    let dur: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0.0);
    let dir = std::env::temp_dir().join(format!("synaptixd-frames-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let stem = format!("{:x}", md5_like(&media_key(path, "video")));
    let mut frames = Vec::new();
    for i in 0..n {
        let t = if dur > 0.0 { dur * (i as f64 + 0.5) / n as f64 } else { 0.0 };
        let png = dir.join(format!("{stem}-{i}.png"));
        if !png.exists() {
            let ok = std::process::Command::new("ffmpeg")
                .args(["-y", "-v", "error", "-ss", &format!("{t:.2}")])
                .arg("-i")
                .arg(path)
                .args(["-frames:v", "1", "-vf", "scale='min(896,iw)':-2"])
                .arg(&png)
                .status()?
                .success();
            if !ok || !png.exists() {
                continue;
            }
        }
        frames.push((t, png));
        if dur <= 0.0 {
            break;
        }
    }
    if frames.is_empty() {
        anyhow::bail!("не удалось извлечь кадры (нужен ffmpeg)");
    }
    Ok(frames)
}

/// Хэш строки для имён временных кадров (FNV-1a).
fn md5_like(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Эмбеддинги медиа всех сообщений: из кэша, иначе башней зрения (она
/// поднимается один раз и отпускается после — эмбеддинги остаются).
/// Неудача — строкой с причиной, чтобы модель знала о вложении.
fn encode_media(
    b: &LlmSlot,
    cache: &mut Vec<(String, synaptix::facade::llm::MediaEmbedding)>,
    messages: &[synaptixd::Message],
    params: &Params,
) -> Vec<Vec<MediaPart>> {
    use synaptixd::MediaKind;
    let llm = &b.llm;
    let max_tokens = params.u32("max_image_tokens").map(|v| v as usize).filter(|v| *v > 0).or(Some(1024));
    let mut tower: Option<Result<(), String>> = None;
    let mut ensure = |tower: &mut Option<Result<(), String>>| -> Result<(), String> {
        tower
            .get_or_insert_with(|| {
                if !llm.supports_media() {
                    return Err("модель без зрения".into());
                }
                match llm.ensure_media_tower() {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("в модели нет башни зрения".into()),
                    Err(e) => Err(format!("башня зрения не загрузилась: {e}")),
                }
            })
            .clone()
    };
    let mut embed = |key: String, cache: &mut Vec<(String, synaptix::facade::llm::MediaEmbedding)>, tower: &mut Option<Result<(), String>>, f: &dyn Fn() -> Result<synaptix::facade::llm::MediaEmbedding, String>| -> Result<String, String> {
        if cache.iter().any(|(k, _)| *k == key) {
            return Ok(key);
        }
        ensure(tower)?;
        let e = f()?;
        cache.push((key.clone(), e));
        Ok(key)
    };
    let mut out = Vec::with_capacity(messages.len());
    let mut used: Vec<String> = Vec::new();
    for m in messages {
        let mut parts = Vec::new();
        for item in &m.media {
            let path = Path::new(&item.path);
            let name = file_name(path);
            if !path.is_file() {
                parts.push(MediaPart::Text(format!("[вложение «{name}» недоступно: файла нет]")));
                continue;
            }
            match item.kind {
                MediaKind::Image => {
                    let key = media_key(path, "image");
                    match embed(key, cache, &mut tower, &|| llm.encode_image(path, max_tokens).map_err(|e| e.to_string())) {
                        Ok(k) => {
                            used.push(k.clone());
                            parts.push(MediaPart::Embed(k));
                        }
                        Err(e) => parts.push(MediaPart::Text(format!("[изображение «{name}» — не удалось передать модели: {e}]"))),
                    }
                }
                MediaKind::Video => {
                    let key = media_key(path, "video");
                    match embed(key, cache, &mut tower, &|| llm.encode_video(path).map_err(|e| e.to_string())) {
                        Ok(k) => {
                            used.push(k.clone());
                            parts.push(MediaPart::Embed(k));
                        }
                        // Модель не принимает видео (Gemma-4) — кадрами-картинками с таймкодами.
                        Err(video_err) => match video_frames(path, VIDEO_FRAMES) {
                            Ok(frames) => {
                                parts.push(MediaPart::Text(format!("[видео «{name}»: {} кадров по всей длительности]", frames.len())));
                                for (t, png) in frames {
                                    let key = media_key(&png, "image");
                                    match embed(key, cache, &mut tower, &|| llm.encode_image(&png, Some(256)).map_err(|e| e.to_string())) {
                                        Ok(k) => {
                                            parts.push(MediaPart::Text(format!("Кадр {t:.1} с:")));
                                            used.push(k.clone());
                                            parts.push(MediaPart::Embed(k));
                                        }
                                        Err(e) => {
                                            parts.push(MediaPart::Text(format!("[кадры видео «{name}» не переданы модели: {e}]")));
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => parts.push(MediaPart::Text(format!("[видео «{name}» — не удалось передать модели: {video_err}; {e}]"))),
                        },
                    }
                }
            }
        }
        out.push(parts);
    }
    if matches!(tower, Some(Ok(()))) {
        llm.release_media_tower();
    }
    // Бюджет: вытеснять старые, кроме нужных этому запросу.
    let mut total: usize = cache.iter().map(|(_, e)| e.tokens).sum();
    let mut i = 0;
    while total > MEDIA_BUDGET && i < cache.len() {
        if used.contains(&cache[i].0) {
            i += 1;
            continue;
        }
        total -= cache[i].1.tokens;
        cache.remove(i);
    }
    out
}
