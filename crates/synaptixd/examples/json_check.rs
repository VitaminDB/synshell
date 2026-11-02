//! Проверка вывода по JSON-схеме через службу: N генераций подряд по схеме
//! с вложенными массивами и enum — все разбираются и проходят схему; скорость
//! против обычной генерации того же запроса.
//!
//! `cargo run --example json_check -- <модель> [N]` (сокет — SYNAPTIXD_SOCKET).

use serde_json::{json, Value};
use synaptixd::{Client, Message, Params};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topic": {"type": "string", "minLength": 3},
            "level": {"enum": ["A1", "A2", "B1", "B2", "C1", "C2"]},
            "cards": {
                "type": "array", "minItems": 3, "maxItems": 5,
                "items": {
                    "type": "object",
                    "properties": {
                        "word": {"type": "string", "minLength": 1},
                        "pos": {"enum": ["noun", "verb", "adjective", "adverb", "phrase"]},
                        "translation_ru": {"type": "string"},
                        "ipa": {"type": "string"},
                        "examples": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 2},
                        "difficulty": {"type": "integer", "minimum": 1, "maximum": 5}
                    },
                    "required": ["word", "pos", "translation_ru", "examples", "difficulty"]
                }
            },
            "quiz": {
                "type": "array", "minItems": 2, "maxItems": 3,
                "items": {
                    "type": "object",
                    "properties": {
                        "question": {"type": "string"},
                        "options": {"type": "array", "items": {"type": "string"}, "minItems": 3, "maxItems": 4},
                        "answer": {"type": "integer", "minimum": 0, "maximum": 3}
                    },
                    "required": ["question", "options", "answer"]
                }
            }
        },
        "required": ["topic", "level", "cards", "quiz"]
    })
}

/// Проверка значения по подмножеству JSON-схемы, которое использует пример.
fn check(v: &Value, s: &Value, path: &str) -> Result<(), String> {
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        return if e.contains(v) { Ok(()) } else { Err(format!("{path}: {v} не из enum")) };
    }
    match s.get("type").and_then(Value::as_str) {
        Some("object") => {
            let o = v.as_object().ok_or(format!("{path}: не объект"))?;
            let props = s["properties"].as_object().unwrap();
            for r in s["required"].as_array().into_iter().flatten() {
                if !o.contains_key(r.as_str().unwrap()) {
                    return Err(format!("{path}: нет поля {r}"));
                }
            }
            for (k, x) in o {
                let ps = props.get(k).ok_or(format!("{path}: лишнее поле {k}"))?;
                check(x, ps, &format!("{path}.{k}"))?;
            }
        }
        Some("array") => {
            let a = v.as_array().ok_or(format!("{path}: не массив"))?;
            let min = s.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize;
            let max = s.get("maxItems").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
            if a.len() < min || a.len() > max {
                return Err(format!("{path}: {} элементов вне [{min}, {max}]", a.len()));
            }
            for (i, x) in a.iter().enumerate() {
                check(x, &s["items"], &format!("{path}[{i}]"))?;
            }
        }
        Some("string") => {
            let t = v.as_str().ok_or(format!("{path}: не строка"))?;
            let min = s.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
            if t.chars().count() < min {
                return Err(format!("{path}: короче {min}"));
            }
        }
        Some("integer") => {
            let n = v.as_i64().ok_or(format!("{path}: не целое"))?;
            let lo = s.get("minimum").and_then(Value::as_i64).unwrap_or(i64::MIN);
            let hi = s.get("maximum").and_then(Value::as_i64).unwrap_or(i64::MAX);
            if n < lo || n > hi {
                return Err(format!("{path}: {n} вне [{lo}, {hi}]"));
            }
        }
        _ => {}
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = args.get(1).cloned().unwrap_or_else(|| "qwen3-6-35b-a3b".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let thinking = std::env::var_os("THINKING").is_some();
    let mut c = Client::connect().expect("служба");
    let topics = ["travel", "job interview", "cooking", "weather", "health", "shopping", "school", "sports", "music", "family"];
    let sch = schema();
    let base = Params::new().with("max_tokens", 1500i64).with("temperature", 0.8).with("thinking", thinking);
    let msgs = |t: &str| {
        vec![
            Message::system("You are an English teacher for Russian-speaking learners."),
            Message::user(format!("Make a vocabulary lesson on the topic «{t}» for level B1: 3-5 word cards and a short quiz.")),
        ]
    };
    // Прогрев (загрузка модели, построение словаря грамматики).
    let _ = c.generate_json(&model, msgs("warmup"), &sch, &base.clone().with("max_tokens", 8i64), |_| {});
    let (mut ok, mut tok_json, mut ms_json) = (0usize, 0usize, 0u64);
    for i in 0..n {
        let t = topics[i % topics.len()];
        match c.generate_json(&model, msgs(t), &sch, &base, |_| {}) {
            Ok((v, g)) => {
                tok_json += g.tokens;
                ms_json += g.decode_ms;
                match check(&v, &sch, "$") {
                    Ok(()) => {
                        ok += 1;
                        println!("{i:2} ok   {:4} ток  {:5.1} ток/с  {}", g.tokens, g.tokens as f64 * 1e3 / g.decode_ms.max(1) as f64, v["cards"].as_array().map(|a| a.iter().map(|c| c["word"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join(", ")).unwrap_or_default());
                    }
                    Err(e) => println!("{i:2} СХЕМА: {e}\n{}", g.text),
                }
            }
            Err(e) => println!("{i:2} ОШИБКА: {e}"),
        }
    }
    // Тот же запрос без грамматики (только просьба в промпте) — скорость и
    // сколько из них сами по себе валидны.
    let m = n.min(5);
    let (mut tok_free, mut ms_free, mut ok_free) = (0usize, 0u64, 0usize);
    let hint = format!("Respond with a single JSON value that conforms to this JSON Schema — no code fences, no text before or after it.\nJSON Schema:\n{sch}");
    for i in 0..m {
        let mut mm = msgs(topics[i % topics.len()]);
        mm[0].content.push_str("\n\n");
        mm[0].content.push_str(&hint);
        let g = c.generate_with(&model, mm, &base, |_| {}).expect("генерация");
        tok_free += g.tokens;
        ms_free += g.decode_ms;
        if serde_json::from_str::<Value>(g.text.trim()).ok().is_some_and(|v| check(&v, &sch, "$").is_ok()) {
            ok_free += 1;
        }
    }
    let rate = |t: usize, ms: u64| t as f64 * 1e3 / ms.max(1) as f64;
    println!("\nпо схеме: {ok}/{n} валидны, декод {:.1} ток/с", rate(tok_json, ms_json));
    println!("без грамматики: {ok_free}/{m} валидны сами, декод {:.1} ток/с", rate(tok_free, ms_free));
    println!("замедление декода: {:+.1} %", (rate(tok_free, ms_free) / rate(tok_json, ms_json) - 1.0) * 100.0);
}
