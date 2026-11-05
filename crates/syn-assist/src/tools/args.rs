use serde_json::{Number, Value};

fn trim_strings(v: &mut Value) {
    match v {
        Value::String(s) => {
            let t = s.trim().to_string();
            *s = t;
        }
        Value::Array(a) => a.iter_mut().for_each(trim_strings),
        Value::Object(o) => o.values_mut().for_each(trim_strings),
        _ => {}
    }
}

pub fn normalize(raw: &str) -> String {
    let trimmed = raw.trim();
    match serde_json::from_str::<Value>(trimmed) {
        Ok(mut v) => {
            trim_strings(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| trimmed.to_string())
        }
        Err(_) => trimmed.to_string(),
    }
}

pub fn coerce_to_schema(v: &mut Value, schema: &Value) -> bool {
    if let Value::Object(map) = v {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else { return false };
        let mut changed = false;
        for (k, val) in map.iter_mut() {
            if let Some(s) = props.get(k) {
                changed |= coerce_to_schema(val, s);
            }
        }
        return changed;
    }
    if let Value::Array(items) = v {
        let Some(s) = schema.get("items") else { return false };
        return items.iter_mut().fold(false, |changed, it| coerce_to_schema(it, s) | changed);
    }
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(|t| t.as_str()).collect(),
        _ => Vec::new(),
    };
    let allows = |t: &str| types.contains(&t);
    let next = match &*v {
        Value::String(s) if !types.is_empty() && !allows("string") => {
            let t = s.trim();
            let as_bool = || match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Some(Value::Bool(true)),
                "false" | "no" | "off" | "0" => Some(Value::Bool(false)),
                _ => None,
            };
            (if allows("boolean") { as_bool() } else { None })
                .or_else(|| if allows("integer") { t.parse::<i64>().ok().map(Value::from) } else { None })
                .or_else(|| if allows("number") { t.parse::<f64>().ok().and_then(Number::from_f64).map(Value::Number) } else { None })
        }
        Value::Number(n) if allows("boolean") && !allows("integer") && !allows("number") => match n.as_i64() {
            Some(0) => Some(Value::Bool(false)),
            Some(1) => Some(Value::Bool(true)),
            _ => None,
        },
        _ => None,
    };
    match next {
        Some(n) => {
            *v = n;
            true
        }
        None => false,
    }
}

pub fn canonical_name<'a>(name: &'a str, keys: &[&'a str]) -> &'a str {
    if keys.contains(&name) {
        return name;
    }
    name.split('.').find(|seg| keys.contains(seg)).unwrap_or(name)
}

pub fn parse(raw: &str, schema: &Value) -> Result<Value, String> {
    let normalized = normalize(raw);
    if normalized.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let mut v: Value = serde_json::from_str(&normalized).map_err(|e| format!("аргументы не JSON: {e}"))?;
    coerce_to_schema(&mut v, schema);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_strings_by_schema() {
        let schema = json!({"type":"object","properties":{"on":{"type":"boolean"},"n":{"type":"integer"},"s":{"type":"string"},"items":{"type":"array","items":{"type":"object","properties":{"b":{"type":"boolean"}}}}}});
        let v = parse(r#"{"on":"True","n":" 30 ","s":"true","items":[{"b":"no"}]}"#, &schema).unwrap();
        assert_eq!(v, json!({"on":true,"n":30,"s":"true","items":[{"b":false}]}));
    }

    #[test]
    fn empty_args_and_names() {
        assert!(parse(r#"{"command":"ls"#, &json!({})).is_err());
        assert_eq!(parse("  ", &json!({})).unwrap(), json!({}));
        let keys = ["bash", "web"];
        assert_eq!(canonical_name("bash.run", &keys), "bash");
        assert_eq!(canonical_name("functions.web", &keys), "web");
        assert_eq!(canonical_name("other", &keys), "other");
    }
}
