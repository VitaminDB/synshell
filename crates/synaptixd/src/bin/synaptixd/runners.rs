use std::path::Path;

use synaptix_asr_wav2vec2::Wav2Vec2Ctc;
use synaptix_core::dtype::DType;
use synaptixd::{Alignment, CtcInfo, DeviceKind, ModelKind, Params, PhoneSpan};

use crate::devices;

const SESSION_STEP: usize = 16384;

pub struct LlmSlot {
    llm: synaptix::facade::llm::Llm,
    tok: synaptix::facade::llm::LlmTokenizer,
    window: usize,
    session: std::sync::Mutex<Option<synaptix::facade::llm::LlmKvSession>>,
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

/// Синтезатор речи и запомненные голоса: `voices` — референсы, выросшие из
/// первых фраз (ключ: язык + описание голоса), чтобы все фразы одной
/// озвучки и все ответы звучали одним голосом; `refs` — клоны с образцов
/// пользователя (ключ: путь + текст образца).
pub struct TtsState {
    engine: TtsEngine,
    voices: std::collections::HashMap<String, Voice>,
    refs: std::collections::HashMap<String, Voice>,
}

/// Сколько секунд речи копить в референсе голоса из первых фраз.
const VOICE_REF_SEC: f32 = 12.0;

/// VibeVoice читает сценарий «Speaker N: …»; обычный текст — одним голосом.
fn vibe_script(text: &str) -> String {
    let has_speakers = text.lines().any(|l| l.trim_start().to_ascii_lowercase().starts_with("speaker"));
    if has_speakers {
        text.to_string()
    } else {
        format!("Speaker 0: {}", text.trim().replace('\n', " "))
    }
}

/// Дописать сказанное к голосу-референсу, пока он короче `VOICE_REF_SEC`.
fn grow_pcm_voice(voice: &mut Option<Voice>, pcm: &[f32], sr: u32) {
    let cap = (VOICE_REF_SEC * sr as f32) as usize;
    match voice {
        Some(Voice::Pcm { pcm: acc, .. }) => {
            if acc.len() < cap {
                acc.extend_from_slice(pcm);
            }
        }
        Some(Voice::Omni(_)) => {}
        None => *voice = Some(Voice::Pcm { pcm: pcm.to_vec(), sr }),
    }
}

pub enum Loaded {
    Ctc(Box<Wav2Vec2Ctc>),
    Mfa(Box<synaptix_align_mfa::MfaModel>),
    Asr(std::sync::Mutex<synaptix::facade::asr::Transcriber>),
    Llm(Box<LlmSlot>),
    Tts(std::sync::Mutex<TtsState>, DeviceKind),
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
    matches!(kind, ModelKind::Ctc | ModelKind::Mfa | ModelKind::Asr | ModelKind::Llm | ModelKind::Tts)
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
            Ok((Loaded::Llm(Box::new(LlmSlot { llm, tok, window, session: std::sync::Mutex::new(None) })), Some(label)))
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
            let st = TtsState { engine, voices: Default::default(), refs: Default::default() };
            Ok((Loaded::Tts(std::sync::Mutex::new(st), dev), Some(precision_tag(q, dtype))))
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
        let msgs: Vec<Message> = lead
            .into_iter()
            .chain(messages.iter()
            .map(|m| match m.role.as_str() {
                "system" => Message::system(m.content.clone()),
                "assistant" => Message::assistant_turn(
                    m.content.clone(),
                    m.reasoning.clone().filter(|r| !r.is_empty()),
                    m.tool_calls.iter().map(|c| (c.name.clone(), c.arguments.clone())).collect(),
                ),
                "tool" => match &m.name {
                    Some(n) => Message::tool_named(n.clone(), m.content.clone()),
                    None => Message::tool(m.content.clone()),
                },
                _ => Message::user(m.content.clone()),
            }))
            .collect();
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
            Some(s) => gen.generate_streaming_cached(s, &ids, tok, on_piece).map_err(|e| anyhow::anyhow!("{e}"))?,
            None => {
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

    pub fn speak(&self, text: &str, params: &Params) -> anyhow::Result<(Vec<f32>, u32)> {
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
        let seed = params.i64("seed").filter(|s| *s >= 0).map(|s| s as u64).unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1
        });
        let ref_audio = params.text("ref_audio").map(str::trim).filter(|s| !s.is_empty());
        let ref_text = params.text("ref_text").unwrap_or("").trim().to_string();
        // Голос для продолжения: фразы одной озвучки и все ответы — одним
        // голосом. Первая фраза выбирает голос (по описанию или случайно),
        // дальше он держится по уже сказанному: у OmniVoice — референс в
        // кодах, у VibeVoice и VoxCPM — PCM как образец. Описание голоса в
        // промпт больше не идёт — модель читала его вслух.
        let key = format!("{}\x1f{}", language.unwrap_or(""), instruct.unwrap_or(""));
        match &st.engine {
            TtsEngine::Omni(p) => {
                let default_steps = if *dev == DeviceKind::Npu { 16 } else { 32 };
                let gen = synaptix_tts_omnivoice::OmniVoiceGenerationConfig {
                    num_step: steps.unwrap_or(default_steps).clamp(4, 64),
                    guidance_scale: guidance.unwrap_or(2.0),
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
                        Some(Voice::Omni(v)) => Some(v),
                        _ => None,
                    };
                    let r = p.synthesize_continuing(text, &mut voice, language, instruct, speed, &gen, VOICE_REF_SEC);
                    if let Some(v) = voice {
                        st.voices.insert(key, Voice::Omni(v));
                    }
                    r.map_err(|e| anyhow::anyhow!("{e}"))?
                };
                Ok((wav, p.output_sample_rate() as u32))
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
                let sample = if let Some(path) = ref_audio {
                    Some(VoiceSample::from_path(path).map_err(|e| anyhow::anyhow!("образец голоса {path}: {e}"))?)
                } else {
                    match st.voices.get(&key) {
                        Some(Voice::Pcm { pcm, sr }) => Some(VoiceSample::new(pcm.clone(), *sr)),
                        _ => None,
                    }
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
                if ref_audio.is_none() {
                    let mut voice = st.voices.remove(&key);
                    grow_pcm_voice(&mut voice, &out.audio, sr);
                    if let Some(v) = voice {
                        st.voices.insert(key, v);
                    }
                }
                Ok((out.audio, sr))
            }
            TtsEngine::Vox(p) => {
                let opts = synaptix_tts_voxcpm::GenerateOptions {
                    n_timesteps: steps.unwrap_or(10).clamp(1, 200),
                    cfg_value: guidance.unwrap_or(2.0),
                    seed,
                    ..Default::default()
                };
                let out = if let Some(path) = ref_audio {
                    if ref_text.is_empty() {
                        p.synthesize_with_reference(text, path, &opts)
                    } else {
                        p.synthesize_continuation(text, &ref_text, path, &opts)
                    }
                } else {
                    match st.voices.get(&key) {
                        Some(Voice::Pcm { pcm, sr }) => p.synthesize_with_reference_pcm(text, pcm, *sr, &opts),
                        _ => p.synthesize(text, &opts),
                    }
                }
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                let sr = out.sample_rate as u32;
                if ref_audio.is_none() {
                    let mut voice = st.voices.remove(&key);
                    grow_pcm_voice(&mut voice, &out.pcm, sr);
                    if let Some(v) = voice {
                        st.voices.insert(key, v);
                    }
                }
                Ok((out.pcm, sr))
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
