use synaptixd::{DeviceKind, ModelKind, Params, QuantOption};

use crate::registry::Manifest;

pub const SYSTEM_PROMPT: &str = "Ты — помощник, встроенный в операционную систему. Отвечай по делу, кратко и на языке пользователя.";

pub fn optimal(m: &Manifest) -> Params {
    builtin(m).over(&m.defaults)
}

fn builtin(m: &Manifest) -> Params {
    let arch = m.arch.to_ascii_lowercase();
    let id = m.id.to_ascii_lowercase();
    let size = |w: u32, h: u32| Params { width: Some(w), height: Some(h), seed: Some(-1), ..Default::default() };
    match m.kind {
        ModelKind::Llm => llm(m),
        ModelKind::Tts => match arch.as_str() {
            "vibevoice" => Params { steps: Some(20), speed: Some(1.0), ..Default::default() },
            "voxcpm" | "voxcpm2" => Params { steps: Some(10), speed: Some(1.0), ..Default::default() },
            _ => Params { speed: Some(1.0), ..Default::default() },
        },
        ModelKind::Image => match arch.as_str() {
            "sdxl" => Params { steps: Some(30), guidance: Some(5.0), ..size(1024, 1024) },
            "flux" => Params { steps: Some(28), guidance: Some(3.5), ..size(1024, 1024) },
            "flux2" => Params { guidance: Some(4.0), ..size(1024, 1024) },
            "qwen_image" => Params { guidance: Some(4.0), negative_prompt: Some(" ".into()), ..size(1024, 1024) },
            "qwen_image21" => Params { guidance: Some(1.0), ..size(1024, 1024) },
            _ => size(1024, 1024),
        },
        ModelKind::Video => match arch.as_str() {
            "minimax_h3" | "h3" => Params { steps: Some(20), guidance: Some(5.0), duration: Some(5.0), fps: Some(24.0), ..size(1344, 768) },
            _ => Params { duration: Some(10.0), fps: Some(24.0), ..size(1024, 576) },
        },
        ModelKind::Music => match arch.as_str() {
            "yue2" => Params { duration: Some(60.0), seed: Some(-1), ..Default::default() },
            _ => {
                let (steps, guidance) = if id.contains("turbo") {
                    (8, 1.0)
                } else if id.contains("sft") {
                    (50, 7.0)
                } else {
                    (32, 7.0)
                };
                Params { steps: Some(steps), guidance: Some(guidance), duration: Some(30.0), seed: Some(-1), ..Default::default() }
            }
        },
        ModelKind::Asr | ModelKind::Ctc | ModelKind::Mfa | ModelKind::Embed => Params::default(),
    }
}

fn llm(m: &Manifest) -> Params {
    let mut p = Params {
        system_prompt: Some(SYSTEM_PROMPT.into()),
        max_tokens: Some(1024),
        seed: Some(-1),
        ..Default::default()
    };
    let Some(v) = m.variants.first() else { return p };
    let profile = synaptix::facade::sampling::sampling_profile(&m.path(v));
    if let Some(s) = profile.pick("", false) {
        p.temperature = Some(s.temperature);
        p.top_p = Some(s.top_p);
        p.top_k = Some(s.top_k as u32);
        p.min_p = Some(s.min_p);
        p.repeat_penalty = Some(s.repetition_penalty);
        p.presence_penalty = Some(s.presence_penalty);
    }
    p
}

pub fn quant_options(m: &Manifest) -> Vec<QuantOption> {
    let devices = m.devices();
    let mut out = vec![QuantOption { id: "auto".into(), devices: devices.clone() }];
    if m.kind == ModelKind::Llm {
        out.push(QuantOption { id: "int8".into(), devices: devices.clone() });
        if devices.contains(&DeviceKind::Cuda) {
            out.push(QuantOption { id: "bf16".into(), devices: vec![DeviceKind::Cuda] });
        }
        return out;
    }
    let mut precisions: Vec<&str> = Vec::new();
    for v in &m.variants {
        if !v.precision.is_empty() && !precisions.contains(&v.precision.as_str()) {
            precisions.push(&v.precision);
        }
    }
    if precisions.len() > 1 {
        for p in precisions {
            let devs = m.variants.iter().filter(|v| v.precision == p).flat_map(|v| v.devices.iter().copied()).fold(Vec::new(), |mut acc, d| {
                if !acc.contains(&d) {
                    acc.push(d);
                }
                acc
            });
            out.push(QuantOption { id: p.to_string(), devices: devs });
        }
    }
    out
}
