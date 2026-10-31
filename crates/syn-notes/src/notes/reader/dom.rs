//! Терпимый разбор XML/HTML в маленькое дерево: книги часто несут
//! невалидный XHTML (`&nbsp;` без DTD, незакрытые `<p>`, `<br>` без `/`),
//! строгий XML-парсер на них падает. Имена тегов и атрибутов — без
//! префикса пространства имён, в нижнем регистре.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Elem(Elem),
    Text(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Elem {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn elems(&self) -> impl Iterator<Item = &Elem> {
        self.children.iter().filter_map(|n| match n {
            Node::Elem(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Elem> {
        self.elems().find(|e| e.name == name)
    }

    /// Первый потомок с именем (поиск в глубину).
    pub fn find(&self, name: &str) -> Option<&Elem> {
        for e in self.elems() {
            if e.name == name {
                return Some(e);
            }
            if let Some(f) = e.find(name) {
                return Some(f);
            }
        }
        None
    }

    /// Все потомки с именем.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Elem>) {
        for e in self.elems() {
            if e.name == name {
                out.push(e);
            }
            e.find_all(name, out);
        }
    }

    /// Весь текст потомков.
    pub fn text(&self) -> String {
        let mut s = String::new();
        fn walk(e: &Elem, s: &mut String) {
            for n in &e.children {
                match n {
                    Node::Text(t) => s.push_str(t),
                    Node::Elem(c) => walk(c, s),
                }
            }
        }
        walk(self, &mut s);
        s
    }
}

const VOID: &[&str] = &["br", "img", "hr", "meta", "link", "input", "col", "area", "base", "wbr", "source", "embed", "param"];
const RAW: &[&str] = &["script", "style"];

/// Разобрать документ; корень — искусственный элемент с пустым именем.
pub fn parse(src: &str) -> Elem {
    let mut stack: Vec<Elem> = vec![Elem::default()];
    let b = src.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    let flush = |stack: &mut Vec<Elem>, s: &str| {
        if !s.is_empty() {
            let t = decode_entities(s);
            let top = stack.last_mut().unwrap();
            if let Some(Node::Text(prev)) = top.children.last_mut() {
                prev.push_str(&t);
            } else {
                top.children.push(Node::Text(t));
            }
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            flush(&mut stack, &src[text_start..i]);
            i = rest.find("-->").map_or(b.len(), |e| i + e + 3);
            text_start = i;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            flush(&mut stack, &src[text_start..i]);
            let end = rest.find("]]>").map_or(b.len(), |e| i + e);
            let raw = &src[i + 9..end.max(i + 9)];
            stack.last_mut().unwrap().children.push(Node::Text(raw.to_string()));
            i = (end + 3).min(b.len());
            text_start = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut stack, &src[text_start..i]);
            i = rest.find('>').map_or(b.len(), |e| i + e + 1);
            text_start = i;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if next == b'/' {
            flush(&mut stack, &src[text_start..i]);
            let end = rest.find('>').map_or(b.len(), |e| i + e);
            let name = local(src[i + 2..end].trim());
            i = (end + 1).min(b.len());
            text_start = i;
            close(&mut stack, &name);
            continue;
        }
        if !(next.is_ascii_alphabetic() || next == b'_') {
            i += 1;
            continue;
        }
        flush(&mut stack, &src[text_start..i]);
        // Тег: имя, атрибуты, конец с учётом кавычек.
        let mut j = i + 1;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        let name = local(&src[i + 1..j]);
        let mut attrs = Vec::new();
        let mut self_close = false;
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                self_close = true;
                j += 1;
                continue;
            }
            let ks = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let key = local(&src[ks..j]);
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut val = String::new();
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vs = j + 1;
                    j = vs;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                    val = decode_entities(&src[vs..j.min(b.len())]);
                    j += 1;
                } else {
                    let vs = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    val = decode_entities(&src[vs..j]);
                }
            }
            if !key.is_empty() {
                attrs.push((key, val));
            }
        }
        i = j.min(b.len());
        text_start = i;
        let el = Elem { name: name.clone(), attrs, children: Vec::new() };
        if self_close || VOID.contains(&name.as_str()) {
            stack.last_mut().unwrap().children.push(Node::Elem(el));
            continue;
        }
        if RAW.contains(&name.as_str()) {
            let close_tag = format!("</{name}");
            let lower = src[i..].to_ascii_lowercase();
            let end = lower.find(&close_tag).map_or(b.len(), |e| i + e);
            let mut el = el;
            el.children.push(Node::Text(src[i..end].to_string()));
            stack.last_mut().unwrap().children.push(Node::Elem(el));
            i = src[end..].find('>').map_or(b.len(), |e| end + e + 1);
            text_start = i;
            continue;
        }
        // Незакрытый <p> закрывается следующим блочным тегом (HTML).
        if matches!(name.as_str(), "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "ul" | "ol" | "table")
            && stack.len() > 1
            && stack.last().is_some_and(|e| e.name == "p")
            && name != "p"
            || (name == "p" && stack.last().is_some_and(|e| e.name == "p"))
        {
            close(&mut stack, "p");
        }
        stack.push(el);
    }
    flush(&mut stack, &src[text_start.min(b.len())..]);
    while stack.len() > 1 {
        let e = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Elem(e));
    }
    stack.pop().unwrap()
}

fn close(stack: &mut Vec<Elem>, name: &str) {
    let Some(at) = stack.iter().rposition(|e| e.name == name) else { return };
    if at == 0 {
        return;
    }
    while stack.len() > at {
        let e = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Elem(e));
    }
}

fn local(s: &str) -> String {
    let s = s.rsplit(':').next().unwrap_or(s);
    s.to_ascii_lowercase()
}

/// `&amp;` `&#1234;` `&#x41;` и частые именованные HTML-сущности.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace()).map(|e| e + 1);
        match end {
            Some(e) if rest.as_bytes()[e] == b';' && e <= 12 => {
                let name = &rest[1..e];
                let ch = if let Some(n) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(n, 16).ok().and_then(char::from_u32)
                } else if let Some(n) = name.strip_prefix('#') {
                    n.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    named(name)
                };
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = &rest[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(n: &str) -> Option<char> {
    static MAP: std::sync::OnceLock<HashMap<&'static str, char>> = std::sync::OnceLock::new();
    let m = MAP.get_or_init(|| {
        [
            ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"'), ("apos", '\''), ("nbsp", '\u{a0}'),
            ("mdash", '—'), ("ndash", '–'), ("hellip", '…'), ("laquo", '«'), ("raquo", '»'), ("bdquo", '„'),
            ("ldquo", '“'), ("rdquo", '”'), ("lsquo", '‘'), ("rsquo", '’'), ("sbquo", '‚'), ("copy", '©'),
            ("reg", '®'), ("trade", '™'), ("deg", '°'), ("middot", '·'), ("bull", '•'), ("shy", '\u{ad}'),
            ("times", '×'), ("divide", '÷'), ("plusmn", '±'), ("sect", '§'), ("para", '¶'), ("euro", '€'),
            ("thinsp", '\u{2009}'), ("ensp", '\u{2002}'), ("emsp", '\u{2003}'), ("zwnj", '\u{200c}'),
            ("zwj", '\u{200d}'), ("minus", '−'), ("prime", '′'), ("Prime", '″'), ("larr", '←'), ("rarr", '→'),
            ("frac12", '½'), ("frac14", '¼'), ("frac34", '¾'), ("sup1", '¹'), ("sup2", '²'), ("sup3", '³'),
            ("iexcl", '¡'), ("iquest", '¿'), ("eacute", 'é'), ("egrave", 'è'), ("agrave", 'à'), ("ccedil", 'ç'),
            ("uuml", 'ü'), ("ouml", 'ö'), ("auml", 'ä'), ("szlig", 'ß'), ("dagger", '†'), ("Dagger", '‡'),
            ("numero", '№'),
        ]
        .into_iter()
        .collect()
    });
    m.get(n).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_html() {
        let d = parse("<?xml version='1.0'?><html><body><p>Один&nbsp;два<br>три<p class=x>четыре &amp; <b>пять</b></body></html>");
        let body = d.find("body").unwrap();
        let ps: Vec<_> = body.elems().filter(|e| e.name == "p").collect();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].text(), "Один\u{a0}дватри");
        assert_eq!(ps[1].attr("class"), Some("x"));
        assert_eq!(ps[1].text(), "четыре & пять");
    }

    #[test]
    fn namespaces_and_cdata() {
        let d = parse(r#"<opf:package xmlns:opf="x"><dc:title>Т</dc:title><x><![CDATA[a<b]]></x><img src="a.png"/></opf:package>"#);
        let p = d.child("package").unwrap();
        assert_eq!(p.child("title").unwrap().text(), "Т");
        assert_eq!(p.child("x").unwrap().text(), "a<b");
        assert_eq!(p.child("img").unwrap().attr("src"), Some("a.png"));
    }
}
