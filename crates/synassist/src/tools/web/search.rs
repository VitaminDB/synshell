use scraper::{Html, Selector};

use super::envelope::SerpHit;
use super::http::{self, FetchOpts};

const DDG_BASE: &str = "https://html.duckduckgo.com/html/";

const DDG_LITE_BASE: &str = "https://lite.duckduckgo.com/lite/";

const DDG_REFERER: &str = "https://html.duckduckgo.com/";
const DDG_LITE_REFERER: &str = "https://lite.duckduckgo.com/";

const BING_BASE: &str = "https://www.bing.com/search";
const BING_REFERER: &str = "https://www.bing.com/";

pub const ENGINE_DDG: &str = "duckduckgo";
pub const ENGINE_BING: &str = "bing";

#[derive(Debug, Clone)]
pub struct SearchHits {
    pub engine: &'static str,
    pub hits: Vec<SerpHit>,
}

#[derive(Debug)]
pub enum SearchError {
    Network(String),
    Challenge,
    Empty,
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "DDG fetch: {e}"),
            Self::Challenge => write!(
                f,
                "Search is unavailable from this host: DuckDuckGo returns a \
                 captcha and the Bing fallback gave nothing. Rephrasing will \
                 NOT help — every search will fail the same way. Do not call \
                 `web search` again in this conversation. Use `web read` with \
                 a concrete URL if you know one, answer from your own \
                 knowledge (say what you are unsure about), or ask the user \
                 for a link."
            ),
            Self::Empty => write!(
                f,
                "No results from any search engine (DuckDuckGo html/lite, \
                 Bing). This host's searches are likely being filtered, so \
                 rephrasing usually does NOT help. Try `web read` with a \
                 concrete URL, answer from your own knowledge (say what you \
                 are unsure about), or ask the user for a link — do not \
                 repeat the search with different wording more than once."
            ),
        }
    }
}

enum EndpointOutcome {
    Hits(Vec<SerpHit>),
    Challenge,
    Empty,
}

pub async fn search(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<SearchHits, SearchError> {
    let primary = try_html_endpoint(query, lang, max_results).await?;
    match primary {
        EndpointOutcome::Hits(hits) if !hits.is_empty() => {
            return Ok(SearchHits { engine: ENGINE_DDG, hits });
        }
        EndpointOutcome::Hits(_) | EndpointOutcome::Empty | EndpointOutcome::Challenge => {}
    }

    let primary_was_challenge = matches!(primary, EndpointOutcome::Challenge);
    let secondary = try_lite_endpoint(query, lang, max_results).await?;
    let ddg_error = match secondary {
        EndpointOutcome::Hits(hits) if !hits.is_empty() => {
            return Ok(SearchHits { engine: ENGINE_DDG, hits });
        }
        EndpointOutcome::Challenge => SearchError::Challenge,
        EndpointOutcome::Hits(_) | EndpointOutcome::Empty => {
            if primary_was_challenge {
                SearchError::Challenge
            } else {
                SearchError::Empty
            }
        }
    };

    match try_bing(query, lang, max_results).await {
        Ok(EndpointOutcome::Hits(hits)) if !hits.is_empty() => {
            log::info!("[web] DDG без результатов ({ddg_error}) — взяли Bing: {} карточек", hits.len());
            Ok(SearchHits { engine: ENGINE_BING, hits })
        }
        Ok(_) => Err(ddg_error),
        Err(e) => {
            log::warn!("[web] Bing fallback не ответил: {e}");
            Err(ddg_error)
        }
    }
}

async fn try_bing(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<EndpointOutcome, SearchError> {
    let url = build_bing_url(query, lang, max_results);
    let html = http::fetch_html_text_with(
        &url,
        FetchOpts {
            referer: Some(BING_REFERER),
            accept_language: Some(accept_language_for(lang)),
        },
    )
    .await
    .map_err(|e| SearchError::Network(e.to_string()))?;

    if detect_bing_no_results(&html) {
        return Ok(EndpointOutcome::Empty);
    }
    let mut hits = parse_bing(&html, max_results);
    hits = dedup_by_origin(hits);
    let hits = renumber(hits, max_results);
    if hits.is_empty() {
        return Ok(EndpointOutcome::Empty);
    }
    if !serp_matches_query(query, &hits) {
        log::warn!(
            "[web] Bing отдал выдачу не по запросу ({} карточек, первая: {:?}) — считаем пустой",
            hits.len(),
            hits.first().map(|h| h.title.as_str()).unwrap_or("")
        );
        return Ok(EndpointOutcome::Empty);
    }
    Ok(EndpointOutcome::Hits(hits))
}

fn serp_matches_query(query: &str, hits: &[SerpHit]) -> bool {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.to_lowercase())
        .filter(|w| !matches!(w.as_str(), "site" | "the" | "and" | "for" | "how" | "или" | "как" | "что"))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if words.is_empty() {
        return true;
    }
    let haystack: String = hits
        .iter()
        .map(|h| format!("{} {} {}", h.title, h.snippet, h.url))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let matched = words.iter().filter(|w| haystack.contains(w.as_str())).count();
    let needed = words.len().div_ceil(2).max(2).min(words.len());
    matched >= needed
}

fn detect_bing_no_results(html: &str) -> bool {
    let Ok(sel) = Selector::parse("#b_results .b_no, li.b_no") else {
        return false;
    };
    Html::parse_document(html).select(&sel).next().is_some()
}

fn build_bing_url(query: &str, lang: &str, max_results: usize) -> String {
    let q_enc = urlencoding::encode(query);
    let count = max_results.clamp(1, 50);
    format!("{BING_BASE}?q={q_enc}&setlang={lang}&count={count}")
}

async fn try_html_endpoint(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<EndpointOutcome, SearchError> {
    let url = build_search_url(query, lang);
    let html = http::fetch_html_text_with(
        &url,
        FetchOpts {
            referer: Some(DDG_REFERER),
            accept_language: Some(accept_language_for(lang)),
        },
    )
    .await
    .map_err(|e| SearchError::Network(e.to_string()))?;

    if detect_challenge(&html) {
        return Ok(EndpointOutcome::Challenge);
    }

    let mut hits = parse_ddg(&html, max_results);
    hits = dedup_by_origin(hits);
    let hits = renumber(hits, max_results);
    if hits.is_empty() {
        Ok(EndpointOutcome::Empty)
    } else {
        Ok(EndpointOutcome::Hits(hits))
    }
}

async fn try_lite_endpoint(
    query: &str,
    lang: &str,
    max_results: usize,
) -> Result<EndpointOutcome, SearchError> {
    let url = build_lite_url(query, lang);
    let html = http::fetch_html_text_with(
        &url,
        FetchOpts {
            referer: Some(DDG_LITE_REFERER),
            accept_language: Some(accept_language_for(lang)),
        },
    )
    .await
    .map_err(|e| SearchError::Network(e.to_string()))?;

    if detect_challenge(&html) {
        return Ok(EndpointOutcome::Challenge);
    }

    let mut hits = parse_ddg_lite(&html, max_results);
    hits = dedup_by_origin(hits);
    let hits = renumber(hits, max_results);
    if hits.is_empty() {
        Ok(EndpointOutcome::Empty)
    } else {
        Ok(EndpointOutcome::Hits(hits))
    }
}

fn build_search_url(query: &str, lang: &str) -> String {
    let q_enc = urlencoding::encode(query);
    let kl = kl_for(lang);
    format!("{DDG_BASE}?q={q_enc}&kl={kl}&ia=web")
}

fn build_lite_url(query: &str, lang: &str) -> String {
    let q_enc = urlencoding::encode(query);
    let kl = kl_for(lang);
    format!("{DDG_LITE_BASE}?q={q_enc}&kl={kl}")
}

fn kl_for(lang: &str) -> String {
    match lang {
        "en" => "us-en".to_string(),
        other => format!("{other}-{other}"),
    }
}

fn accept_language_for(lang: &str) -> &'static str {
    match lang {
        "ru" => "ru-RU,ru;q=0.9,en-US;q=0.5,en;q=0.3",
        "en" => "en-US,en;q=0.9",
        "de" => "de-DE,de;q=0.9,en;q=0.5",
        "fr" => "fr-FR,fr;q=0.9,en;q=0.5",
        "es" => "es-ES,es;q=0.9,en;q=0.5",
        "it" => "it-IT,it;q=0.9,en;q=0.5",
        "pt" => "pt-PT,pt;q=0.9,en;q=0.5",
        "uk" => "uk-UA,uk;q=0.9,ru;q=0.6,en;q=0.3",
        _ => "en-US,en;q=0.9",
    }
}

pub(super) fn detect_challenge(html: &str) -> bool {
    if html.contains("Unfortunately, bots use DuckDuckGo too")
        || html.contains("anomaly_modal")
    {
        return true;
    }
    let doc = Html::parse_document(html);
    let Ok(sel) = Selector::parse(
        r#"form#challenge-form, form[action*="challenge"], div#anomaly-modal__title"#,
    ) else {
        return false;
    };
    doc.select(&sel).next().is_some()
}

fn parse_ddg(html: &str, max: usize) -> Vec<SerpHit> {
    let doc = Html::parse_document(html);

    let Ok(card_sel) = Selector::parse("div.result, div.web-result") else {
        return Vec::new();
    };
    let Ok(title_sel) = Selector::parse("h2.result__title a.result__a, a.result__a, h2 a") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = Selector::parse(
        "a.result__snippet, div.result__snippet, [data-result=\"snippet\"]",
    ) else {
        return Vec::new();
    };

    let mut out: Vec<SerpHit> = Vec::new();
    for card in doc.select(&card_sel) {
        if out.len() >= max {
            break;
        }
        let Some(a) = card.select(&title_sel).next() else {
            continue;
        };
        let title = clean_text(&a.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let raw_href = a.value().attr("href").unwrap_or("");
        let url = unwrap_uddg(raw_href);
        if !is_http_url(&url) {
            continue;
        }
        let snippet = card
            .select(&snippet_sel)
            .next()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SerpHit {
            rank: out.len() + 1,
            title,
            url,
            snippet,
        });
    }
    out
}

fn parse_ddg_lite(html: &str, max: usize) -> Vec<SerpHit> {
    let doc = Html::parse_document(html);

    let Ok(link_sel) = Selector::parse("a.result-link") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = Selector::parse("td.result-snippet") else {
        return Vec::new();
    };

    let links: Vec<_> = doc.select(&link_sel).collect();
    let snippets: Vec<String> = doc
        .select(&snippet_sel)
        .map(|td| clean_text(&td.text().collect::<String>()))
        .collect();

    let mut out: Vec<SerpHit> = Vec::new();
    for (i, a) in links.iter().enumerate() {
        if out.len() >= max {
            break;
        }
        let title = clean_text(&a.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let raw_href = a.value().attr("href").unwrap_or("");
        let url = unwrap_uddg(raw_href);
        if !is_http_url(&url) {
            continue;
        }
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        out.push(SerpHit {
            rank: out.len() + 1,
            title,
            url,
            snippet,
        });
    }
    out
}

fn parse_bing(html: &str, max: usize) -> Vec<SerpHit> {
    let doc = Html::parse_document(html);
    let Ok(card_sel) = Selector::parse("li.b_algo") else {
        return Vec::new();
    };
    let Ok(title_sel) = Selector::parse("h2 a") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = Selector::parse("div.b_caption p, p[class*=\"b_lineclamp\"]") else {
        return Vec::new();
    };

    let mut out: Vec<SerpHit> = Vec::new();
    for card in doc.select(&card_sel) {
        if out.len() >= max {
            break;
        }
        let Some(a) = card.select(&title_sel).next() else {
            continue;
        };
        let title = clean_text(&a.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let url = unwrap_bing_redirect(a.value().attr("href").unwrap_or(""));
        if !is_http_url(&url) {
            continue;
        }
        let snippet = card
            .select(&snippet_sel)
            .next()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SerpHit {
            rank: out.len() + 1,
            title,
            url,
            snippet,
        });
    }
    out
}

fn unwrap_bing_redirect(href: &str) -> String {
    use base64::Engine as _;
    let Ok(u) = url::Url::parse(href) else {
        return href.to_string();
    };
    let is_redirect = u
        .host_str()
        .is_some_and(|h| h.ends_with("bing.com"))
        && u.path().starts_with("/ck/");
    if !is_redirect {
        return href.to_string();
    }
    let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "u") else {
        return href.to_string();
    };
    let payload = v.strip_prefix("a1").unwrap_or(&v);
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .ok()
        .and_then(|b| String::from_utf8(b).ok());
    match decoded {
        Some(target) if is_http_url(&target) => target,
        _ => href.to_string(),
    }
}

fn unwrap_uddg(href: &str) -> String {
    if href.is_empty() {
        return String::new();
    }
    if !href.contains("/l/?") {
        return href.to_string();
    }
    let candidate = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    if let Ok(u) = url::Url::parse(&candidate) {
        if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "uddg") {
            return v.into_owned();
        }
    }
    href.to_string()
}

fn dedup_by_origin(hits: Vec<SerpHit>) -> Vec<SerpHit> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let key = url::Url::parse(&h.url)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|host| format!("{}{}", host.to_ascii_lowercase(), u.path()))
            })
            .unwrap_or_else(|| h.url.clone());
        if seen.insert(key) {
            out.push(h);
        }
    }
    out
}

fn renumber(mut hits: Vec<SerpHit>, max: usize) -> Vec<SerpHit> {
    hits.truncate(max);
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i + 1;
    }
    hits
}

fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_http_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bing_redirect_unwraps_base64url_target() {
        let href = "https://www.bing.com/ck/a?!&&p=abc&u=a1aHR0cHM6Ly93d3cubWkuY29tL2dsb2JhbC9wcm9kdWN0LWxpc3QvcmVkbWkv&ntb=1";
        assert_eq!(
            unwrap_bing_redirect(href),
            "https://www.mi.com/global/product-list/redmi/"
        );
        assert_eq!(unwrap_bing_redirect("https://a.b/c"), "https://a.b/c");
        assert_eq!(unwrap_bing_redirect("javascript:void(0)"), "javascript:void(0)");
    }

    #[test]
    fn bing_parser_reads_cards_and_unwraps_links() {
        let html = r#"<html><body><ol id="b_results">
            <li class="b_algo"><div class="b_tpcn"><a class="tilk" href="https://www.bing.com/ck/a?p=1&u=a1aHR0cHM6Ly93d3cubWkuY29tL2dsb2JhbC9wcm9kdWN0LWxpc3QvcmVkbWkv">mi.com</a></div>
              <h2><a href="https://www.bing.com/ck/a?p=1&u=a1aHR0cHM6Ly93d3cubWkuY29tL2dsb2JhbC9wcm9kdWN0LWxpc3QvcmVkbWkv">Redmi <strong>Series</strong> | Xiaomi</a></h2>
              <div class="b_caption"><p class="b_lineclamp2">View  Xiaomi Redmi Series.</p></div></li>
            <li class="b_algo"><h2><a href="https://xdaforums.com/t/redmi-9c.123/">Redmi 9C unlock</a></h2>
              <div class="b_caption"><p>Thread about unlocking.</p></div></li>
            <li class="b_ad"><h2><a href="https://ads.example/">Ad</a></h2></li>
        </ol></body></html>"#;
        let hits = parse_bing(html, 10);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].url, "https://www.mi.com/global/product-list/redmi/");
        assert_eq!(hits[0].title, "Redmi Series | Xiaomi");
        assert_eq!(hits[0].snippet, "View Xiaomi Redmi Series.");
        assert_eq!(hits[1].url, "https://xdaforums.com/t/redmi-9c.123/");
        assert_eq!(hits[1].rank, 2);
    }

    #[test]
    fn bing_no_results_page_is_detected() {
        let html = r#"<ol id="b_results"><li class="b_no"><h1>There are no results for <strong>foo</strong></h1></li>
            <li class="b_algo"><h2><a href="https://www.mi.com/global/">Xiaomi Global</a></h2></li></ol>"#;
        assert!(detect_bing_no_results(html));
        assert!(!detect_bing_no_results(r#"<ol id="b_results"><li class="b_algo"><h2><a href="https://a.b/">A</a></h2></li></ol>"#));
    }

    #[test]
    fn bing_no_results_text_in_script_is_not_detected() {
        let html = r#"<html><head><script>var L={"searchNoResult":"There are no results for this question"};</script></head>
            <body><ol id="b_results"><li class="b_algo"><h2><a href="https://rust-lang.org/">Rust Programming Language</a></h2></li></ol></body></html>"#;
        assert!(!detect_bing_no_results(html));
        assert_eq!(parse_bing(html, 5).len(), 1);
    }

    #[test]
    fn bing_junk_serp_is_rejected_by_relevance() {
        let junk = vec![
            SerpHit { rank: 1, title: "LOS 10 MEJORES hoteles de lujo en Punta Cana".into(), url: "https://www.tripadvisor.es/x".into(), snippet: String::new() },
            SerpHit { rank: 2, title: "Xiaomi Global".into(), url: "https://www.mi.com/global/".into(), snippet: "Official site".into() },
        ];
        assert!(!serp_matches_query("postmarketOS Redmi 9C Helio G35 support", &junk));
        assert!(!serp_matches_query("Xiaomi unlock bootloader official guide", &junk));
        let good = vec![SerpHit {
            rank: 1,
            title: "How to unlock the bootloader on Redmi 9C".into(),
            url: "https://xdaforums.com/t/redmi-9c-unlock".into(),
            snippet: "Mi Unlock tool, 168 hours wait".into(),
        }];
        assert!(serp_matches_query("Redmi 9C unlock bootloader", &good));
        assert!(serp_matches_query("redmi", &good));
    }

    #[test]
    fn bing_url_carries_query_lang_and_count() {
        let u = build_bing_url("redmi 9c", "ru", 10);
        assert!(u.starts_with("https://www.bing.com/search?q=redmi%209c"), "{u}");
        assert!(u.contains("setlang=ru"), "{u}");
        assert!(u.contains("count=10"), "{u}");
    }

    #[test]
    fn build_url_encodes_cyrillic_and_locale() {
        let u = build_search_url("руст асинхронный", "ru");
        assert!(u.starts_with("https://html.duckduckgo.com/html/?q="), "{u}");
        assert!(u.contains("&kl=ru-ru"), "{u}");
        assert!(u.contains("&ia=web"), "{u}");
        assert!(u.contains("%D1%80%D1%83%D1%81%D1%82"), "{u}");
    }

    #[test]
    fn build_url_english_uses_us_en() {
        let u = build_search_url("rust async", "en");
        assert!(u.contains("&kl=us-en"), "{u}");
    }

    #[test]
    fn build_lite_url_uses_lite_host() {
        let u = build_lite_url("rust async", "ru");
        assert!(u.starts_with("https://lite.duckduckgo.com/lite/?q="), "{u}");
        assert!(u.contains("&kl=ru-ru"), "{u}");
    }

    #[test]
    fn accept_language_known_langs() {
        assert!(accept_language_for("ru").starts_with("ru-RU,"));
        assert!(accept_language_for("en").starts_with("en-US,"));
        assert!(accept_language_for("de").starts_with("de-DE,"));
        assert!(accept_language_for("xx").starts_with("en-US,"));
    }

    #[test]
    fn parse_minimal_card() {
        let html = r#"<!doctype html><html><body>
            <div class="result">
              <h2 class="result__title">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fpage&amp;rut=abc">Example Page</a>
              </h2>
              <a class="result__snippet">A page about examples.</a>
            </div>
        </body></html>"#;
        let hits = parse_ddg(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Example Page");
        assert_eq!(hits[0].url, "https://example.org/page");
        assert!(hits[0].snippet.contains("examples"));
    }

    #[test]
    fn parse_clamps_to_max() {
        let mut html = String::from("<html><body>");
        for i in 0..5 {
            html.push_str(&format!(
                r#"<div class="result">
                    <h2 class="result__title"><a class="result__a" href="https://e{}.x/p">Title {}</a></h2>
                    <a class="result__snippet">Snippet {}</a>
                </div>"#,
                i, i, i
            ));
        }
        html.push_str("</body></html>");
        let hits = parse_ddg(&html, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Title 0");
        assert_eq!(hits[2].title, "Title 2");
    }

    #[test]
    fn parse_skips_card_without_title() {
        let html = r#"<html><body>
            <div class="result"><a class="result__a" href="https://e.x/p"></a></div>
            <div class="result">
                <h2><a class="result__a" href="https://e.x/p2">Real Title</a></h2>
            </div>
        </body></html>"#;
        let hits = parse_ddg(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Real Title");
    }

    #[test]
    fn parse_empty_html_returns_empty() {
        assert!(parse_ddg("", 10).is_empty());
        assert!(parse_ddg("<html></html>", 10).is_empty());
    }

    #[test]
    fn parse_lite_minimal_card() {
        let html = r#"<!doctype html><html><body><table>
            <tr><td><a class="result-link" rel="nofollow" href="https://example.org/page">Example Page</a></td></tr>
            <tr><td class="result-snippet">A page about examples.</td></tr>
            <tr><td><span class="link-text">example.org</span></td></tr>
        </table></body></html>"#;
        let hits = parse_ddg_lite(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Example Page");
        assert_eq!(hits[0].url, "https://example.org/page");
        assert!(hits[0].snippet.contains("examples"));
    }

    #[test]
    fn parse_lite_unwraps_uddg() {
        let html = r#"<html><body><table>
            <tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.b%2Fc">Title</a></td></tr>
            <tr><td class="result-snippet">Snip</td></tr>
        </table></body></html>"#;
        let hits = parse_ddg_lite(html, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://a.b/c");
    }

    #[test]
    fn parse_lite_clamps_to_max() {
        let mut html = String::from("<html><body><table>");
        for i in 0..5 {
            html.push_str(&format!(
                r#"<tr><td><a class="result-link" href="https://e{i}.x/p">Title {i}</a></td></tr>
                   <tr><td class="result-snippet">Snippet {i}</td></tr>"#
            ));
        }
        html.push_str("</table></body></html>");
        let hits = parse_ddg_lite(&html, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Title 0");
        assert_eq!(hits[2].title, "Title 2");
        assert_eq!(hits[2].snippet, "Snippet 2");
    }

    #[test]
    fn parse_lite_empty_html_returns_empty() {
        assert!(parse_ddg_lite("", 10).is_empty());
        assert!(parse_ddg_lite("<html></html>", 10).is_empty());
    }

    #[test]
    fn detect_challenge_form_id() {
        let html = r#"<html><body>
            <form id="challenge-form" action="/challenge" method="post"></form>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_action_substring() {
        let html = r#"<html><body>
            <form action="/anti-bot/challenge?id=1"></form>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_text_phrase() {
        let html = r#"<html><body>
            <p>Unfortunately, bots use DuckDuckGo too — please prove you are human.</p>
        </body></html>"#;
        assert!(detect_challenge(html));
    }

    #[test]
    fn detect_challenge_normal_serp_is_negative() {
        let html = r#"<html><body>
            <div class="result">
                <h2><a class="result__a" href="https://e.x/p">Title</a></h2>
            </div>
        </body></html>"#;
        assert!(!detect_challenge(html));
    }

    #[test]
    fn search_error_display_messages() {
        assert!(SearchError::Network("conn".into()).to_string().contains("DDG fetch"));
        let challenge = SearchError::Challenge.to_string();
        assert!(challenge.contains("captcha"), "{challenge}");
        assert!(challenge.contains("NOT help"), "{challenge}");
        let empty = SearchError::Empty.to_string();
        assert!(empty.contains("No results"), "{empty}");
        assert!(empty.contains("does NOT help"), "{empty}");
    }

    #[test]
    fn unwrap_uddg_protocol_relative() {
        assert_eq!(
            unwrap_uddg("//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.b%2F&rut=x"),
            "https://a.b/"
        );
    }

    #[test]
    fn unwrap_uddg_path_only() {
        assert_eq!(
            unwrap_uddg("/l/?uddg=https%3A%2F%2Fc.d%2Fe&rut=y"),
            "https://c.d/e"
        );
    }

    #[test]
    fn unwrap_uddg_passthrough() {
        assert_eq!(
            unwrap_uddg("https://target.example/path"),
            "https://target.example/path"
        );
    }

    #[test]
    fn dedup_collapses_same_path_with_query_diff() {
        let hits = vec![
            SerpHit {
                rank: 1,
                title: "a".into(),
                url: "https://example.com/p?x=1".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 2,
                title: "b".into(),
                url: "https://example.com/p?x=2".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 3,
                title: "c".into(),
                url: "https://OTHER.com/p".into(),
                snippet: String::new(),
            },
        ];
        let out = dedup_by_origin(hits);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "a");
        assert_eq!(out[1].title, "c");
    }

    #[test]
    fn dedup_handles_invalid_url_as_unique_key() {
        let hits = vec![
            SerpHit {
                rank: 1,
                title: "a".into(),
                url: "not a url".into(),
                snippet: String::new(),
            },
            SerpHit {
                rank: 2,
                title: "b".into(),
                url: "not a url".into(),
                snippet: String::new(),
            },
        ];
        let out = dedup_by_origin(hits);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn renumber_truncates_and_resets_ranks() {
        let hits = vec![
            SerpHit { rank: 5, title: "a".into(), url: "u1".into(), snippet: String::new() },
            SerpHit { rank: 7, title: "b".into(), url: "u2".into(), snippet: String::new() },
            SerpHit { rank: 9, title: "c".into(), url: "u3".into(), snippet: String::new() },
        ];
        let out = renumber(hits, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].rank, 1);
        assert_eq!(out[1].rank, 2);
    }

    #[test]
    fn clean_text_collapses_whitespace() {
        assert_eq!(clean_text("  hello   world\n\t!  "), "hello world !");
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg() {
        let found = super::search("rust async", "en", 10)
            .await
            .expect("DDG должен ответить");
        let hits = found.hits;
        eprintln!("engine={} hits: {}", found.engine, hits.len());
        for h in &hits {
            eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
        }
        assert!(
            hits.len() >= 3,
            "ожидаемо минимум 3 результата на популярный запрос, получили: {}",
            hits.len()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg_ru() {
        let found = super::search("новости сегодня главные события", "ru", 10)
            .await
            .expect("DDG должен ответить на русский запрос");
        let hits = found.hits;
        eprintln!("engine={} ru hits: {}", found.engine, hits.len());
        for h in &hits {
            eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
        }
        assert!(
            hits.len() >= 3,
            "ожидаемо минимум 3 результата на новости, получили: {}",
            hits.len()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn smoke_search_real_ddg_lite() {
        let outcome = super::try_lite_endpoint("rust async", "en", 10)
            .await
            .expect("lite DDG должен ответить");
        match outcome {
            super::EndpointOutcome::Hits(hits) => {
                eprintln!("lite hits: {}", hits.len());
                for h in &hits {
                    eprintln!("  {}. {} → {}", h.rank, h.title, h.url);
                }
                assert!(hits.len() >= 3, "минимум 3 хита, получили: {}", hits.len());
            }
            super::EndpointOutcome::Empty => panic!("lite вернул пустой SERP"),
            super::EndpointOutcome::Challenge => panic!("lite ловит captcha — IP-block"),
        }
    }
}
