use std::path::Path;

use synaptix_asr_wav2vec2::Wav2Vec2Ctc;
use synaptix_core::dtype::DType;
use synaptixd::{Alignment, CtcInfo, DeviceKind, ModelKind, PhoneSpan};

use crate::devices;

pub enum Loaded {
    Ctc(Box<Wav2Vec2Ctc>),
    Mfa(Box<synaptix_align_mfa::MfaModel>),
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
