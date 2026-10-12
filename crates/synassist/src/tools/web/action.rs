use serde::Deserialize;

pub(super) const DEFAULT_MAX_RESULTS: usize = 10;
pub(super) const MAX_RESULTS_HARD_CAP: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebAction {
    Search,
    Read,
}

#[derive(Debug, Deserialize)]
struct RawArgs {
    #[serde(default)]
    action: Option<WebAction>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Clone)]
pub enum WebArgs {
    Search {
        query: String,
        max_results: usize,
        lang: String,
    },
    Read {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    BadArgs(String),
    MissingField(&'static str),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgError::BadArgs(e) => write!(f, "некорректные аргументы: {e}"),
            ArgError::MissingField(k) => write!(f, "нужен параметр «{k}»"),
        }
    }
}

pub fn parse_value(args: &serde_json::Value) -> Result<WebArgs, ArgError> {
    let raw: RawArgs =
        serde_json::from_value(args.clone()).map_err(|e| ArgError::BadArgs(e.to_string()))?;
    validate(raw)
}

pub fn parse_args(args_json: &str) -> Result<WebArgs, ArgError> {
    let raw: RawArgs =
        serde_json::from_str(args_json).map_err(|e| ArgError::BadArgs(e.to_string()))?;
    validate(raw)
}

fn validate(raw: RawArgs) -> Result<WebArgs, ArgError> {
    let action = raw.action.ok_or(ArgError::MissingField("action"))?;

    match action {
        WebAction::Search => {
            let query = raw
                .query
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(ArgError::MissingField("query"))?
                .to_string();
            let max_results = raw
                .max_results
                .unwrap_or(DEFAULT_MAX_RESULTS)
                .clamp(1, MAX_RESULTS_HARD_CAP);
            let lang = raw
                .lang
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("ru")
                .to_string();
            Ok(WebArgs::Search {
                query,
                max_results,
                lang,
            })
        }
        WebAction::Read => {
            let url = raw
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(ArgError::MissingField("url"))?
                .to_string();
            Ok(WebArgs::Read { url })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_minimal() {
        let args = parse_args(r#"{"action":"search","query":"rust async"}"#).unwrap();
        match args {
            WebArgs::Search { query, max_results, lang } => {
                assert_eq!(query, "rust async");
                assert_eq!(max_results, DEFAULT_MAX_RESULTS);
                assert_eq!(lang, "ru");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn parse_read_minimal() {
        let args = parse_args(r#"{"action":"read","url":"https://e.x/p"}"#).unwrap();
        match args {
            WebArgs::Read { url } => assert_eq!(url, "https://e.x/p"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn bad_json_is_bad_args() {
        let err = parse_args("not json").unwrap_err();
        assert!(matches!(err, ArgError::BadArgs(_)));
    }

    #[test]
    fn missing_action_field() {
        let err = parse_args(r#"{"query":"x"}"#).unwrap_err();
        assert!(matches!(err, ArgError::MissingField("action")));
    }

    #[test]
    fn unknown_action_value() {
        let err = parse_args(r#"{"action":"crawl","query":"x"}"#).unwrap_err();
        assert!(matches!(err, ArgError::BadArgs(_)));
    }

    #[test]
    fn search_without_query_is_missing_field() {
        let err = parse_args(r#"{"action":"search"}"#).unwrap_err();
        assert!(matches!(err, ArgError::MissingField("query")));
    }

    #[test]
    fn search_with_whitespace_only_query() {
        let err = parse_args(r#"{"action":"search","query":"   "}"#).unwrap_err();
        assert!(matches!(err, ArgError::MissingField("query")));
    }

    #[test]
    fn read_without_url_is_missing_field() {
        let err = parse_args(r#"{"action":"read"}"#).unwrap_err();
        assert!(matches!(err, ArgError::MissingField("url")));
    }

    #[test]
    fn read_with_whitespace_only_url() {
        let err = parse_args(r#"{"action":"read","url":"  "}"#).unwrap_err();
        assert!(matches!(err, ArgError::MissingField("url")));
    }

    #[test]
    fn max_results_clamp_low() {
        let args = parse_args(r#"{"action":"search","query":"x","max_results":0}"#).unwrap();
        match args {
            WebArgs::Search { max_results, .. } => assert_eq!(max_results, 1),
            _ => unreachable!(),
        }
    }

    #[test]
    fn max_results_clamp_high() {
        let args = parse_args(r#"{"action":"search","query":"x","max_results":100}"#).unwrap();
        match args {
            WebArgs::Search { max_results, .. } => assert_eq!(max_results, MAX_RESULTS_HARD_CAP),
            _ => unreachable!(),
        }
    }

    #[test]
    fn lang_default_is_ru() {
        let args = parse_args(r#"{"action":"search","query":"x"}"#).unwrap();
        match args {
            WebArgs::Search { lang, .. } => assert_eq!(lang, "ru"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn lang_custom() {
        let args = parse_args(r#"{"action":"search","query":"x","lang":"en"}"#).unwrap();
        match args {
            WebArgs::Search { lang, .. } => assert_eq!(lang, "en"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn read_ignores_max_results_and_lang() {
        let args = parse_args(
            r#"{"action":"read","url":"https://e.x/","max_results":5,"lang":"en"}"#,
        )
        .unwrap();
        assert!(matches!(args, WebArgs::Read { .. }));
    }
}
