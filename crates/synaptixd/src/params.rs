use serde::{Deserialize, Serialize};

use crate::ModelKind;

macro_rules! params {
    ($($field:ident: $ty:ty),* $(,)?) => {
        #[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
        #[serde(default)]
        pub struct Params {
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<$ty>,
            )*
        }

        impl Params {
            pub fn over(&self, top: &Params) -> Params {
                Params { $($field: top.$field.clone().or_else(|| self.$field.clone()),)* }
            }

            pub fn is_empty(&self) -> bool {
                true $(&& self.$field.is_none())*
            }
        }
    };
}

params! {
    system_prompt: String,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    top_k: u32,
    min_p: f32,
    repeat_penalty: f32,
    presence_penalty: f32,
    seed: i64,
    negative_prompt: String,
    steps: u32,
    guidance: f32,
    width: u32,
    height: u32,
    duration: f32,
    fps: f32,
    language: String,
    voice: String,
    speed: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ParamKey {
    SystemPrompt,
    MaxTokens,
    Temperature,
    TopP,
    TopK,
    MinP,
    RepeatPenalty,
    PresencePenalty,
    Seed,
    NegativePrompt,
    Steps,
    Guidance,
    Width,
    Height,
    Duration,
    Fps,
    Language,
    Voice,
    Speed,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ParamType {
    Text { multiline: bool },
    Int { min: f64, max: f64, step: f64 },
    Float { min: f64, max: f64, step: f64, decimals: u8 },
}

#[derive(Clone, PartialEq, Debug)]
pub enum ParamValue {
    Text(String),
    Num(f64),
}

impl ParamValue {
    pub fn display(&self, ty: ParamType) -> String {
        match (self, ty) {
            (ParamValue::Text(s), _) => s.clone(),
            (ParamValue::Num(v), ParamType::Float { decimals, .. }) => {
                let s = format!("{v:.*}", decimals as usize);
                if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
            }
            (ParamValue::Num(v), _) => format!("{}", v.round() as i64),
        }
    }
}

impl ParamKey {
    pub fn id(self) -> &'static str {
        match self {
            ParamKey::SystemPrompt => "system_prompt",
            ParamKey::MaxTokens => "max_tokens",
            ParamKey::Temperature => "temperature",
            ParamKey::TopP => "top_p",
            ParamKey::TopK => "top_k",
            ParamKey::MinP => "min_p",
            ParamKey::RepeatPenalty => "repeat_penalty",
            ParamKey::PresencePenalty => "presence_penalty",
            ParamKey::Seed => "seed",
            ParamKey::NegativePrompt => "negative_prompt",
            ParamKey::Steps => "steps",
            ParamKey::Guidance => "guidance",
            ParamKey::Width => "width",
            ParamKey::Height => "height",
            ParamKey::Duration => "duration",
            ParamKey::Fps => "fps",
            ParamKey::Language => "language",
            ParamKey::Voice => "voice",
            ParamKey::Speed => "speed",
        }
    }

    pub fn ty(self) -> ParamType {
        let int = |min: f64, max: f64, step: f64| ParamType::Int { min, max, step };
        let float = |min: f64, max: f64, step: f64, decimals: u8| ParamType::Float { min, max, step, decimals };
        match self {
            ParamKey::SystemPrompt | ParamKey::NegativePrompt | ParamKey::Voice => ParamType::Text { multiline: true },
            ParamKey::Language => ParamType::Text { multiline: false },
            ParamKey::MaxTokens => int(1.0, 131072.0, 64.0),
            ParamKey::Temperature => float(0.0, 2.0, 0.05, 2),
            ParamKey::TopP => float(0.0, 1.0, 0.05, 2),
            ParamKey::TopK => int(0.0, 1000.0, 1.0),
            ParamKey::MinP => float(0.0, 1.0, 0.01, 2),
            ParamKey::RepeatPenalty => float(0.5, 2.0, 0.05, 2),
            ParamKey::PresencePenalty => float(-2.0, 2.0, 0.1, 2),
            ParamKey::Seed => int(-1.0, u32::MAX as f64, 1.0),
            ParamKey::Steps => int(1.0, 200.0, 1.0),
            ParamKey::Guidance => float(0.0, 30.0, 0.5, 2),
            ParamKey::Width | ParamKey::Height => int(64.0, 4096.0, 64.0),
            ParamKey::Duration => float(0.5, 600.0, 0.5, 1),
            ParamKey::Fps => float(1.0, 120.0, 1.0, 0),
            ParamKey::Speed => float(0.25, 4.0, 0.05, 2),
        }
    }

    pub fn get(self, p: &Params) -> Option<ParamValue> {
        let t = |v: &Option<String>| v.clone().map(ParamValue::Text);
        let n = |v: Option<f64>| v.map(ParamValue::Num);
        match self {
            ParamKey::SystemPrompt => t(&p.system_prompt),
            ParamKey::NegativePrompt => t(&p.negative_prompt),
            ParamKey::Language => t(&p.language),
            ParamKey::Voice => t(&p.voice),
            ParamKey::MaxTokens => n(p.max_tokens.map(f64::from)),
            ParamKey::Temperature => n(p.temperature.map(f64::from)),
            ParamKey::TopP => n(p.top_p.map(f64::from)),
            ParamKey::TopK => n(p.top_k.map(f64::from)),
            ParamKey::MinP => n(p.min_p.map(f64::from)),
            ParamKey::RepeatPenalty => n(p.repeat_penalty.map(f64::from)),
            ParamKey::PresencePenalty => n(p.presence_penalty.map(f64::from)),
            ParamKey::Seed => n(p.seed.map(|v| v as f64)),
            ParamKey::Steps => n(p.steps.map(f64::from)),
            ParamKey::Guidance => n(p.guidance.map(f64::from)),
            ParamKey::Width => n(p.width.map(f64::from)),
            ParamKey::Height => n(p.height.map(f64::from)),
            ParamKey::Duration => n(p.duration.map(f64::from)),
            ParamKey::Fps => n(p.fps.map(f64::from)),
            ParamKey::Speed => n(p.speed.map(f64::from)),
        }
    }

    pub fn set(self, p: &mut Params, v: Option<ParamValue>) {
        let text = |v: &Option<ParamValue>| match v {
            Some(ParamValue::Text(s)) => Some(s.clone()),
            Some(ParamValue::Num(n)) => Some(n.to_string()),
            None => None,
        };
        let num = |v: &Option<ParamValue>| match v {
            Some(ParamValue::Num(n)) => Some(*n),
            Some(ParamValue::Text(s)) => s.trim().replace(',', ".").parse().ok(),
            None => None,
        };
        let (lo, hi) = match self.ty() {
            ParamType::Int { min, max, .. } | ParamType::Float { min, max, .. } => (min, max),
            ParamType::Text { .. } => (f64::MIN, f64::MAX),
        };
        let n = num(&v).map(|x| x.clamp(lo, hi));
        let u = n.map(|x| x.round() as u32);
        let f = n.map(|x| x as f32);
        match self {
            ParamKey::SystemPrompt => p.system_prompt = text(&v),
            ParamKey::NegativePrompt => p.negative_prompt = text(&v),
            ParamKey::Language => p.language = text(&v),
            ParamKey::Voice => p.voice = text(&v),
            ParamKey::MaxTokens => p.max_tokens = u,
            ParamKey::Temperature => p.temperature = f,
            ParamKey::TopP => p.top_p = f,
            ParamKey::TopK => p.top_k = u,
            ParamKey::MinP => p.min_p = f,
            ParamKey::RepeatPenalty => p.repeat_penalty = f,
            ParamKey::PresencePenalty => p.presence_penalty = f,
            ParamKey::Seed => p.seed = n.map(|x| x.round() as i64),
            ParamKey::Steps => p.steps = u,
            ParamKey::Guidance => p.guidance = f,
            ParamKey::Width => p.width = u,
            ParamKey::Height => p.height = u,
            ParamKey::Duration => p.duration = f,
            ParamKey::Fps => p.fps = f,
            ParamKey::Speed => p.speed = f,
        }
    }
}

impl ModelKind {
    pub fn params(self) -> &'static [ParamKey] {
        use ParamKey::*;
        match self {
            ModelKind::Llm => &[SystemPrompt, MaxTokens, Temperature, TopP, TopK, MinP, RepeatPenalty, PresencePenalty, Seed],
            ModelKind::Image => &[NegativePrompt, Steps, Guidance, Width, Height, Seed],
            ModelKind::Video => &[NegativePrompt, Steps, Guidance, Width, Height, Duration, Fps, Seed],
            ModelKind::Music => &[Steps, Guidance, Duration, Seed],
            ModelKind::Tts => &[Language, Voice, Speed, Steps],
            ModelKind::Asr => &[Language],
            ModelKind::Ctc | ModelKind::Mfa | ModelKind::Embed => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layering() {
        let base = Params { temperature: Some(1.0), top_k: Some(64), ..Default::default() };
        let user = Params { temperature: Some(0.4), ..Default::default() };
        let call = Params { max_tokens: Some(10), ..Default::default() };
        let eff = base.over(&user).over(&call);
        assert_eq!(eff.temperature, Some(0.4));
        assert_eq!(eff.top_k, Some(64));
        assert_eq!(eff.max_tokens, Some(10));
        assert!(Params::default().is_empty());
    }

    #[test]
    fn keys_roundtrip() {
        let mut p = Params::default();
        ParamKey::Temperature.set(&mut p, Some(ParamValue::Text("0,7".into())));
        ParamKey::Steps.set(&mut p, Some(ParamValue::Num(1000.0)));
        assert_eq!(p.temperature, Some(0.7));
        assert_eq!(p.steps, Some(200));
        assert_eq!(ParamKey::Temperature.get(&p).unwrap().display(ParamKey::Temperature.ty()), "0.7");
        assert!(serde_json::to_string(&p).unwrap().contains("\"steps\":200"));
    }
}
