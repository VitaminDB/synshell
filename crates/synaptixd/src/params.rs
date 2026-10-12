use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl ParamValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Int(v) => Some(*v as f64),
            ParamValue::Float(v) => Some(*v),
            ParamValue::Text(s) => s.trim().replace(',', ".").parse().ok(),
            ParamValue::Bool(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            ParamValue::Text(s) => match s.trim() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            },
            ParamValue::Int(v) => Some(*v != 0),
            ParamValue::Float(_) => None,
        }
    }

    pub fn display(&self, kind: &ParamKind) -> String {
        match (self, kind) {
            (ParamValue::Text(s), _) => s.clone(),
            (ParamValue::Bool(b), _) => if *b { "да" } else { "нет" }.into(),
            (v, ParamKind::Float { decimals, .. }) => {
                let s = format!("{:.*}", *decimals as usize, v.as_f64().unwrap_or(0.0));
                if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
            }
            (v, _) => format!("{}", v.as_f64().unwrap_or(0.0).round() as i64),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    Text {
        #[serde(default)]
        multiline: bool,
    },
    Int {
        min: i64,
        max: i64,
        step: i64,
    },
    Float {
        min: f64,
        max: f64,
        step: f64,
        decimals: u8,
    },
    Bool,
    Choice {
        options: Vec<String>,
    },
    /// Путь к файлу; `patterns` — MIME-типы или маски для диалога выбора.
    File {
        #[serde(default)]
        patterns: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ParamSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub hint: String,
    pub kind: ParamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
}

impl ParamSpec {
    pub fn normalize(&self, v: &ParamValue) -> Option<ParamValue> {
        match &self.kind {
            ParamKind::Text { .. } | ParamKind::File { .. } => Some(ParamValue::Text(match v {
                ParamValue::Text(s) => s.clone(),
                other => other.display(&self.kind),
            })),
            ParamKind::Int { min, max, .. } => v.as_f64().map(|x| ParamValue::Int((x.round() as i64).clamp(*min, *max))),
            ParamKind::Float { min, max, .. } => v.as_f64().map(|x| ParamValue::Float(x.clamp(*min, *max))),
            ParamKind::Bool => v.as_bool().map(ParamValue::Bool),
            ParamKind::Choice { options } => {
                let s = match v {
                    ParamValue::Text(s) => s.clone(),
                    other => other.display(&ParamKind::Int { min: 0, max: 0, step: 1 }),
                };
                options.iter().find(|o| o.eq_ignore_ascii_case(s.trim())).map(|o| ParamValue::Text(o.clone()))
            }
        }
    }

    pub fn parse(&self, text: &str) -> Option<ParamValue> {
        self.normalize(&ParamValue::Text(text.to_string()))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(transparent)]
pub struct Params(pub BTreeMap<String, ParamValue>);

impl Params {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: &str, v: impl Into<ParamValue>) -> Self {
        self.0.insert(key.to_string(), v.into());
        self
    }

    pub fn from_schema(schema: &[ParamSpec]) -> Self {
        Params(schema.iter().filter_map(|s| s.default.clone().map(|d| (s.key.clone(), d))).collect())
    }

    pub fn over(&self, top: &Params) -> Params {
        let mut out = self.clone();
        for (k, v) in &top.0 {
            out.0.insert(k.clone(), v.clone());
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&ParamValue> {
        self.0.get(key)
    }

    pub fn set(&mut self, key: &str, v: Option<ParamValue>) {
        match v {
            Some(v) => {
                self.0.insert(key.to_string(), v);
            }
            None => {
                self.0.remove(key);
            }
        }
    }

    pub fn sanitize(&self, schema: &[ParamSpec]) -> Params {
        Params(self.0.iter().filter_map(|(k, v)| schema.iter().find(|s| &s.key == k).and_then(|s| s.normalize(v)).map(|v| (k.clone(), v))).collect())
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(ParamValue::as_f64).map(|v| v as f32)
    }

    pub fn u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(ParamValue::as_f64).filter(|v| *v >= 0.0).map(|v| v.round() as u32)
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(ParamValue::as_f64).map(|v| v.round() as i64)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(ParamValue::as_str)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.str(key).map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(ParamValue::as_bool)
    }
}

impl From<bool> for ParamValue {
    fn from(v: bool) -> Self {
        ParamValue::Bool(v)
    }
}

impl From<i64> for ParamValue {
    fn from(v: i64) -> Self {
        ParamValue::Int(v)
    }
}

impl From<u32> for ParamValue {
    fn from(v: u32) -> Self {
        ParamValue::Int(v as i64)
    }
}

impl From<f64> for ParamValue {
    fn from(v: f64) -> Self {
        ParamValue::Float(v)
    }
}

impl From<f32> for ParamValue {
    fn from(v: f32) -> Self {
        ParamValue::Float(v as f64)
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        ParamValue::Text(v.to_string())
    }
}

impl From<String> for ParamValue {
    fn from(v: String) -> Self {
        ParamValue::Text(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> ParamSpec {
        ParamSpec { key: "temperature".into(), label: "Т".into(), hint: String::new(), kind: ParamKind::Float { min: 0.0, max: 2.0, step: 0.05, decimals: 2 }, default: Some(ParamValue::Float(1.0)) }
    }

    #[test]
    fn layering_and_sanitize() {
        let schema = vec![temp()];
        let base = Params::from_schema(&schema);
        let user = Params::new().with("temperature", 5.0).with("unknown", 1i64);
        let eff = base.over(&user.sanitize(&schema));
        assert_eq!(eff.f32("temperature"), Some(2.0));
        assert!(eff.get("unknown").is_none());
        assert_eq!(temp().parse("0,7"), Some(ParamValue::Float(0.7)));
    }

    #[test]
    fn serde_forms() {
        let p: Params = serde_json::from_str(r#"{"temperature":0.3,"steps":20,"thinking":true,"voice":"low"}"#).unwrap();
        assert_eq!(p.u32("steps"), Some(20));
        assert_eq!(p.bool("thinking"), Some(true));
        assert_eq!(p.f32("temperature"), Some(0.3));
        let spec: ParamSpec = serde_json::from_str(&serde_json::to_string(&temp()).unwrap()).unwrap();
        assert_eq!(spec, temp());
    }
}
