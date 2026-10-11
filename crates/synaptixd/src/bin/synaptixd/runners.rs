use std::path::Path;

use synaptix_asr_wav2vec2::Wav2Vec2Ctc;
use synaptix_core::dtype::DType;
use synaptixd::{Alignment, CtcInfo, DeviceKind, ModelKind, PhoneSpan};

use crate::devices;

pub enum Loaded {
    Ctc(Box<Wav2Vec2Ctc>),
    Mfa(Box<synaptix_align_mfa::MfaModel>),
    Asr(std::sync::Mutex<synaptix::facade::asr::Transcriber>),
    Llm(Box<(synaptix::facade::llm::Llm, synaptix::facade::llm::LlmTokenizer)>),
    Tts(std::sync::Mutex<synaptix_tts_omnivoice::OmniVoicePipeline>, DeviceKind),
}

pub struct CtcResult {
    pub frames: usize,
    pub vocab: usize,
    pub frame_sec: f32,
    pub logprobs: Vec<f32>,
    pub hidden: Option<Vec<f32>>,
    pub hidden_dim: usize,
}

pub fn load(kind: ModelKind, arch: &str, path: &Path, dev: DeviceKind) -> anyhow::Result<Loaded> {
    match kind {
        ModelKind::Ctc => {
            let device = devices::device(dev);
            let dtype = if dev == DeviceKind::Cpu { DType::F32 } else { DType::F16 };
            let m = Wav2Vec2Ctc::load(path, &device, dtype).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Loaded::Ctc(Box::new(m)))
        }
        ModelKind::Mfa => {
            let m = synaptix_align_mfa::MfaModel::load(path).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Loaded::Mfa(Box::new(m)))
        }
        ModelKind::Asr => {
            use synaptix::facade::asr::{AsrConfig, AsrModelKind, ComputeDType, Transcriber};
            let device = devices::device(dev);
            let compute = if dev == DeviceKind::Cpu { ComputeDType::F32 } else { ComputeDType::F16 };
            let t = Transcriber::load(AsrConfig {
                kind: AsrModelKind::Whisper,
                model_path: path.to_path_buf(),
                language: None,
                device,
                storage_dtype: compute.to_dtype(),
                compute_dtype: compute,
            })
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Loaded::Asr(std::sync::Mutex::new(t)))
        }
        ModelKind::Llm => {
            use synaptix::facade::llm::{load_llm, optimal_profile_for_device};
            let device = devices::device(dev);
            let precision = optimal_profile_for_device(path, device).policy.to_precision().map_err(|e| anyhow::anyhow!("{e}"))?;
            let max_seq = if dev == DeviceKind::Npu { 2048 } else { 8192 };
            let (llm, tok) = load_llm(path, device, precision, Some(max_seq)).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Loaded::Llm(Box::new((llm, tok))))
        }
        ModelKind::Tts => {
            let device = devices::device(dev);
            let dtype = match dev {
                DeviceKind::Cpu => DType::F32,
                DeviceKind::Npu => DType::F16,
                _ => DType::BF16,
            };
            let p = synaptix_tts_omnivoice::OmniVoicePipeline::from_syn(path, device, dtype).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Loaded::Tts(std::sync::Mutex::new(p), dev))
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

    pub fn generate(
        &self,
        messages: &[synaptixd::Message],
        max_tokens: usize,
        temperature: Option<f32>,
        on_token: &dyn Fn(&str),
    ) -> anyhow::Result<synaptixd::Generated> {
        use synaptix::facade::llm::{GenerationOptions, LlmGeneration, Message};
        let Loaded::Llm(b) = self else { anyhow::bail!("модель не языковая") };
        let (llm, tok) = (&b.0, &b.1);
        let msgs: Vec<Message> = messages
            .iter()
            .map(|m| match m.role.as_str() {
                "system" => Message::system(m.content.clone()),
                "assistant" => Message::assistant(m.content.clone()),
                "tool" => Message::tool(m.content.clone()),
                _ => Message::user(m.content.clone()),
            })
            .collect();
        let prompt = tok.apply_chat_template_ex_tools(&msgs, true, false, None).map_err(|e| anyhow::anyhow!("{e}"))?;
        let ids = tok.encode(&prompt).map_err(|e| anyhow::anyhow!("{e}"))?;
        let opts = GenerationOptions {
            max_new_tokens: max_tokens.max(1),
            max_seq_len: ids.len() + max_tokens.max(1) + 16,
            temperature: temperature.unwrap_or(0.7),
            top_k: 20,
            top_p: 0.95,
            min_p: 0.0,
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1,
            repeat_penalty: 1.0,
            repeat_last_n: 64,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
        };
        let prof = std::env::var_os("SYNAPTIX_LLM_PROF").is_some();
        synaptix_llm_common::model::set_prefill_prof(prof);
        let mut gen = LlmGeneration::new(llm, opts);
        gen.set_stop_tokens(tok.eos_ids().to_vec());
        let mut text = String::new();
        let mut tokens = 0usize;
        gen.generate_streaming(&ids, tok, |_, piece| {
            tokens += 1;
            text.push_str(piece);
            on_token(piece);
            true
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        if prof {
            eprintln!("{}", synaptix_llm_common::model::prefill_prof_report_and_clear());
        }
        Ok(synaptixd::Generated { text, tokens })
    }

    pub fn speak(
        &self,
        text: &str,
        language: Option<&str>,
        instruct: Option<&str>,
        speed: Option<f32>,
        steps: Option<usize>,
    ) -> anyhow::Result<(Vec<f32>, u32)> {
        let Loaded::Tts(p, dev) = self else { anyhow::bail!("модель не синтезирует речь") };
        let p = p.lock().unwrap_or_else(|e| e.into_inner());
        let default_steps = if *dev == DeviceKind::Npu { 16 } else { 32 };
        let gen = synaptix_tts_omnivoice::OmniVoiceGenerationConfig {
            num_step: steps.unwrap_or(default_steps).clamp(4, 64),
            ..Default::default()
        };
        let wav = p
            .synthesize(text, None, language, instruct, speed.unwrap_or(1.0) as f64, None, &gen)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok((wav, p.output_sample_rate() as u32))
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
