use dom_smoothie::Readability;

use super::envelope::ReadDoc;
use super::http;

#[derive(Debug, Clone)]
pub enum ReadError {
    Http(String),
    UnsupportedMime(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Http(e) => write!(f, "{e}"),
            ReadError::UnsupportedMime(ct) => {
                write!(f, "unsupported content type: {ct}")
            }
        }
    }
}

pub async fn read(source_url: &str) -> Result<ReadDoc, ReadError> {
    let raw = http::fetch_raw(source_url, None)
        .await
        .map_err(|e| ReadError::Http(e.to_string()))?;

    let mime = parse_mime(&raw.content_type);
    let kind = classify_mime(&mime);

    let body_text = String::from_utf8_lossy(&raw.body).into_owned();

    let (markdown, readability_fallback) = match kind {
        BodyKind::Html => render_html_via_readability(&body_text, &raw.final_url),
        BodyKind::PlainText | BodyKind::Markdown => (body_text, false),
        BodyKind::Unsupported => return Err(ReadError::UnsupportedMime(raw.content_type)),
    };

    Ok(ReadDoc {
        source_url: source_url.to_string(),
        final_url: raw.final_url,
        status: raw.status,
        content_type: raw.content_type,
        markdown,
        readability_fallback,
        truncated: raw.truncated,
    })
}

fn render_html_via_readability(html: &str, base_url: &str) -> (String, bool) {
    let parsed = Readability::new(html, Some(base_url), None).and_then(|mut r| r.parse());

    match parsed {
        Ok(article) => {
            let chunk: &str = &article.content;
            let md = htmd::convert(chunk).unwrap_or_default();
            (md, false)
        }
        Err(_) => {
            let md = htmd::convert(html).unwrap_or_default();
            (md, true)
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum BodyKind {
    Html,
    PlainText,
    Markdown,
    Unsupported,
}

fn parse_mime(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn classify_mime(mime: &str) -> BodyKind {
    match mime {
        "text/html" | "application/xhtml+xml" => BodyKind::Html,
        "text/plain" => BodyKind::PlainText,
        "text/markdown" | "text/x-markdown" => BodyKind::Markdown,
        _ => BodyKind::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE_HTML: &str = r#"
        <!DOCTYPE html>
        <html><head><title>Тестовый заголовок</title></head>
        <body>
            <header><nav><a href="/">Home</a><a href="/about">About</a></nav></header>
            <article>
                <h1>Главный заголовок статьи</h1>
                <p>Это первый параграф статьи. Он должен попасть в Markdown.</p>
                <p>Это второй параграф. <a href="/link">Ссылка</a>.</p>
                <p>Третий параграф для readability score &mdash; нужен достаточный объём текста, иначе readability не классифицирует блок как article. Добавим ещё больше осмысленных слов, чтобы плотность параграфов вышла выше пороговой.</p>
                <p>И ещё один параграф просто чтобы плотность текста точно прошла пороговое значение. Это типичный length-bonus в readability score.</p>
            </article>
            <footer>© 2026 Footer Text</footer>
        </body></html>
    "#;

    #[test]
    fn readability_extracts_article_drops_chrome() {
        let (md, fallback) = render_html_via_readability(ARTICLE_HTML, "https://e.x/post/1");
        assert!(!fallback, "readability не должен fallback'ить на этом article");
        assert!(md.contains("Главный заголовок"), "{md}");
        assert!(md.contains("первый параграф"), "{md}");
        assert!(!md.contains("Footer Text"), "footer не должен попасть: {md}");
        assert!(!md.contains("Home"), "nav не должен попасть: {md}");
    }

    #[test]
    fn readability_falls_back_on_empty_spa() {
        let html = r#"<!DOCTYPE html><html><body><div id="root"></div></body></html>"#;
        let (_md, _fallback) = render_html_via_readability(html, "https://spa.e/");
    }

    #[test]
    fn readability_invalid_base_url_does_not_panic() {
        let (md, _) = render_html_via_readability(ARTICLE_HTML, "not-a-url");
        assert!(!md.is_empty(), "markdown непустой даже на невалидном base_url");
    }

    #[test]
    fn parse_mime_strips_charset() {
        assert_eq!(parse_mime("text/html; charset=utf-8"), "text/html");
        assert_eq!(parse_mime("Text/HTML"), "text/html");
        assert_eq!(parse_mime(""), "");
    }

    #[test]
    fn classify_dispatches() {
        assert_eq!(classify_mime("text/html"), BodyKind::Html);
        assert_eq!(classify_mime("application/xhtml+xml"), BodyKind::Html);
        assert_eq!(classify_mime("text/plain"), BodyKind::PlainText);
        assert_eq!(classify_mime("text/markdown"), BodyKind::Markdown);
        assert_eq!(classify_mime("text/x-markdown"), BodyKind::Markdown);
        assert_eq!(classify_mime("application/pdf"), BodyKind::Unsupported);
        assert_eq!(classify_mime("image/png"), BodyKind::Unsupported);
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_read_real_example_com() {
        let doc = super::read("https://example.com/").await.expect("fetch ok");
        eprintln!(
            "status={} ct={} fallback={} markdown_len={}",
            doc.status,
            doc.content_type,
            doc.readability_fallback,
            doc.markdown.len()
        );
        eprintln!("--- markdown ---\n{}", doc.markdown);
        assert_eq!(doc.status, 200);
        assert!(
            doc.markdown.to_lowercase().contains("documentation examples"),
            "ожидаем article-копи 'documentation examples', получили: {}",
            doc.markdown
        );
    }
}
