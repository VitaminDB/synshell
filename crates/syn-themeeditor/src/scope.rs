//! Перенос таблицы стилей в область: каждый селектор получает префикс
//! (`.panel` → `.te-scope-shell .panel`), а `:root` становится самой областью
//! (`.te-scope-shell { --bg: … }` — переменные действуют только внутри неё,
//! syngui ищет `var()` у предков). Так предпросмотр в окне редактора
//! рисуется темой-черновиком, а само окно — темой системы.

/// Перенести `src` в область `scope` (селектор, например `.te-scope-shell`).
pub fn scope_mss(src: &str, scope: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len() + src.len() / 8);
    let mut i = 0;
    while i < b.len() {
        // Пробелы и комментарии между правилами — как есть.
        if b[i].is_ascii_whitespace() {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        if src[i..].starts_with("/*") {
            let end = src[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len());
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        // Заголовок правила — до `{` (вне строк и комментариев).
        let Some(open) = find_open(src, i) else {
            out.push_str(&src[i..]);
            break;
        };
        let close = match_block(src, open);
        let head = src[i..open].trim();
        let body = &src[open..close];
        if head.starts_with('@') {
            // @keyframes и прочие @-правила — без изменений.
            out.push_str(head);
            out.push(' ');
            out.push_str(body);
        } else {
            let sels: Vec<String> = split_top(head, ',')
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if s == ":root" {
                        scope.to_string()
                    } else if let Some(rest) = s.strip_prefix(":root") {
                        format!("{scope}{rest}")
                    } else {
                        format!("{scope} {s}")
                    }
                })
                .collect();
            out.push_str(&sels.join(", "));
            out.push(' ');
            out.push_str(body);
        }
        i = close;
    }
    out
}

/// Позиция `{` заголовка, начиная с `from`.
fn find_open(s: &str, from: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'{' => return Some(i),
            b'"' | b'\'' => i = skip_string(s, i),
            b'/' if s[i..].starts_with("/*") => i = s[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len()),
            _ => i += 1,
        }
    }
    None
}

/// Конец блока (позиция после парной `}`), начиная с `{` в `open`.
fn match_block(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'"' | b'\'' => {
                i = skip_string(s, i);
                continue;
            }
            b'/' if s[i..].starts_with("/*") => {
                i = s[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len());
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn skip_string(s: &str, start: usize) -> usize {
    let b = s.as_bytes();
    let q = b[start];
    let mut i = start + 1;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// Разбить по `sep` вне скобок.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut v = Vec::new();
    let mut depth = 0;
    let mut last = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if c == sep && depth == 0 => {
                v.push(&s[last..i]);
                last = i + c.len_utf8();
            }
            _ => {}
        }
    }
    v.push(&s[last..]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_rules_and_root() {
        let src = "/* c */\n:root { --bg: #000; }\n.a, Text:hover { color: var(--bg); }\n@keyframes k { from { opacity: 0; } to { opacity: 1; } }\n.n { &:hover { x: 1; } }\n";
        let out = scope_mss(src, ".s");
        assert!(out.contains(".s { --bg: #000; }"), "{out}");
        assert!(out.contains(".s .a, .s Text:hover {"), "{out}");
        assert!(out.contains("@keyframes k { from { opacity: 0; } to { opacity: 1; } }"), "{out}");
        assert!(out.contains(".s .n { &:hover { x: 1; } }"), "{out}");
        assert!(out.contains("/* c */"));
    }

    #[test]
    fn shell_stylesheet_parses_after_scoping() {
        let src = include_str!("../../synshell-ui/styles/shell.mss");
        let out = scope_mss(src, ".te-scope-shell");
        let (_, warnings) = syngui::mss::MssParser::new(&out).parse().expect("разбор");
        let (_, base_warnings) = syngui::mss::MssParser::new(src).parse().expect("разбор");
        assert_eq!(warnings.len(), base_warnings.len(), "{:?}", warnings.iter().take(3).collect::<Vec<_>>());
    }
}
