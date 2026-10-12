use std::time::Duration;

use futures_util::StreamExt;
use thiserror::Error;

pub const MAX_FETCH_BYTES: usize = 1024 * 1024;

pub const TIMEOUT: Duration = Duration::from_secs(15);

pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct RawDoc {
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP client: {0}")]
    Client(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("reading body: {0}")]
    Read(String),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FetchOpts<'a> {
    pub referer: Option<&'a str>,
    pub accept_language: Option<&'a str>,
}

pub async fn fetch_raw(url: &str, referer: Option<&str>) -> Result<RawDoc, FetchError> {
    fetch_raw_with(
        url,
        FetchOpts {
            referer,
            accept_language: None,
        },
    )
    .await
}

pub async fn fetch_raw_with(url: &str, opts: FetchOpts<'_>) -> Result<RawDoc, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| FetchError::Client(e.to_string()))?;

    let mut req = client.get(url);
    if let Some(r) = opts.referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    if let Some(al) = opts.accept_language {
        req = req.header(reqwest::header::ACCEPT_LANGUAGE, al);
        req = req.header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        );
    }
    let response = req
        .send()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let final_url = response.url().to_string();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| FetchError::Read(e.to_string()))?;
        if buf.len() + bytes.len() > MAX_FETCH_BYTES {
            let take = MAX_FETCH_BYTES.saturating_sub(buf.len());
            buf.extend_from_slice(&bytes[..take]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&bytes);
    }

    Ok(RawDoc {
        final_url,
        status,
        content_type,
        body: buf,
        truncated,
    })
}

pub async fn fetch_html_text(url: &str, referer: Option<&str>) -> Result<String, FetchError> {
    fetch_html_text_with(
        url,
        FetchOpts {
            referer,
            accept_language: None,
        },
    )
    .await
}

pub async fn fetch_html_text_with(
    url: &str,
    opts: FetchOpts<'_>,
) -> Result<String, FetchError> {
    let doc = fetch_raw_with(url, opts).await?;
    Ok(String::from_utf8_lossy(&doc.body).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_is_desktop_chrome() {
        assert!(!USER_AGENT.to_lowercase().contains("synthos"));
        assert!(USER_AGENT.contains("Mozilla/5.0"));
        assert!(USER_AGENT.contains("Chrome"));
    }

    #[test]
    fn max_fetch_bytes_is_1mb() {
        assert_eq!(MAX_FETCH_BYTES, 1024 * 1024);
    }

    #[test]
    fn fetch_opts_default_is_all_none() {
        let opts = FetchOpts::default();
        assert!(opts.referer.is_none());
        assert!(opts.accept_language.is_none());
    }
}
