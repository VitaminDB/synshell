pub mod action;
pub mod envelope;
pub mod http;
pub mod read;
pub mod search;

use serde_json::{json, Value};

use self::action::{parse_args, parse_value, ArgError, WebArgs};
use super::{Danger, Tool, ToolOutput, ToolResult};

pub struct WebTool;

impl WebTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebTool {
    fn name(&self) -> &'static str {
        "web"
    }

    fn group(&self) -> &'static str {
        "web"
    }

    fn description(&self) -> String {
        "Поиск и чтение веб-страниц в реальном времени / Search and read web pages in real time. \
         ALWAYS CALL this tool when the question needs current or fresh data that can't be in \
         your weights: weather, currency and crypto rates, quotes and prices, news and events, \
         sports results, schedules (flights, trains, movies), service status, software releases, \
         charts, league tables, elections, facts after your knowledge cutoff, and anything like \
         \"now / today / this week\". Do NOT refuse citing the cutoff and don't send the user to \
         third-party sites — you have `web`, go get the answer. \
         action=search — search via DuckDuckGo (Bing fallback), returns the top-N results \
         (title, url, snippet); use a short query with place/date/units (\"погода Алматы сегодня\", \
         \"USD KZT exchange rate\", \"Bitcoin price USD\"). \
         action=read — fetch a page by URL and extract the article content as Markdown \
         (Mozilla Readability + htmd; nav/footer/ads are filtered automatically). \
         Typical workflow: action=search first, then action=read for the 1-2 most relevant URLs \
         from the results. Don't guess URLs — always search first."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "read"],
                    "description": "Что сделать / What to do: `search` — search the web; `read` — read a specific URL and return Markdown."
                },
                "query": {
                    "type": "string",
                    "description": "Обязателен для action=search. Natural-language search query."
                },
                "url": {
                    "type": "string",
                    "description": "Обязателен для action=read. HTTP(S) URL of the page to read."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Только для action=search. How many results to return (1..=20). Default 10.",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 10
                },
                "lang": {
                    "type": "string",
                    "description": "Только для action=search. Preferred language (e.g. \"ru\", \"en\"). Default \"ru\".",
                    "default": "ru"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn danger(&self, _args: &Value) -> Danger {
        Danger::Safe
    }

    fn summary(&self, args: &Value) -> String {
        match parse_value(args) {
            Ok(WebArgs::Search { query, .. }) => format!("Поиск: {query}"),
            Ok(WebArgs::Read { url }) => format!("Чтение: {url}"),
            Err(_) => "Интернет".to_string(),
        }
    }

    fn run(&self, args: &Value) -> ToolResult {
        let parsed = parse_value(args).map_err(|e| e.to_string())?;
        block_on(execute(parsed)).map(ToolOutput::text)
    }
}

pub async fn run(args_json: &str) -> Result<String, ArgError> {
    let parsed = parse_args(args_json)?;
    Ok(execute(parsed).await)
}

async fn execute(parsed: WebArgs) -> String {
    match parsed {
        WebArgs::Search {
            query,
            max_results,
            lang,
        } => match search::search(&query, &lang, max_results).await {
            Ok(found) => envelope::format_search_envelope(&query, &lang, found.engine, &found.hits),
            Err(e) => envelope::format_search_error(&query, &lang, &e.to_string()),
        },
        WebArgs::Read { url } => match read::read(&url).await {
            Ok(doc) => envelope::format_read_envelope(&doc),
            Err(e) => envelope::format_read_error(&url, &e.to_string()),
        },
    }
}

fn block_on<F>(fut: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    let go = move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|rt| rt.block_on(fut))
            .map_err(|e| format!("tokio runtime: {e}"))
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|s| {
            s.spawn(go)
                .join()
                .unwrap_or_else(|_| Err("web: поток выполнения упал".to_string()))
        })
    } else {
        go()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn run_bad_json_returns_err() {
        let err = run("not json").await.unwrap_err();
        assert!(matches!(err, ArgError::BadArgs(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_missing_action_returns_err() {
        let err = run(r#"{"query": "x"}"#).await.unwrap_err();
        assert!(matches!(err, ArgError::MissingField("action")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_search_missing_query() {
        let err = run(r#"{"action": "search"}"#).await.unwrap_err();
        assert!(matches!(err, ArgError::MissingField("query")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_read_missing_url() {
        let err = run(r#"{"action": "read"}"#).await.unwrap_err();
        assert!(matches!(err, ArgError::MissingField("url")));
    }

    #[test]
    fn tool_meta() {
        let t = WebTool::new();
        assert_eq!(t.name(), "web");
        assert_eq!(t.group(), "web");
        assert_eq!(t.danger(&json!({})), Danger::Safe);
        let s = super::super::schema(&t);
        assert_eq!(s["function"]["name"], "web");
        assert_eq!(s["function"]["parameters"]["required"][0], "action");
    }

    #[test]
    fn tool_summary() {
        let t = WebTool::new();
        assert_eq!(t.summary(&json!({"action": "search", "query": "rust"})), "Поиск: rust");
        assert_eq!(
            t.summary(&json!({"action": "read", "url": "https://e.x/"})),
            "Чтение: https://e.x/"
        );
        assert_eq!(t.summary(&json!({})), "Интернет");
    }

    #[test]
    fn tool_run_bad_args_is_err() {
        let t = WebTool::new();
        let err = t.run(&json!({"action": "search"})).unwrap_err();
        assert!(err.contains("query"), "{err}");
        assert!(t.run(&json!({"action": "crawl"})).is_err());
    }

    #[test]
    fn tool_run_read_bad_url_is_envelope_error() {
        let t = WebTool::new();
        let out = t.run(&json!({"action": "read", "url": "not a url"})).unwrap();
        assert!(out.text.contains("GET not a url"), "{}", out.text);
        assert!(out.text.contains("--- error ---"), "{}", out.text);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_run_inside_runtime_does_not_panic() {
        let t = WebTool::new();
        let out = t.run(&json!({"action": "read", "url": "not a url"})).unwrap();
        assert!(out.text.contains("--- error ---"), "{}", out.text);
    }

    #[test]
    #[ignore]
    fn smoke_tool_search_live() {
        let t = WebTool::new();
        let out = t
            .run(&json!({"action": "search", "query": "synaptix rust", "max_results": 5}))
            .unwrap();
        eprintln!("{}", out.text);
        assert!(out.text.contains("--- results ---"), "{}", out.text);
    }
}
