//! Шаблоны Fusion (`.setting`, DaVinci Resolve): разбор таблиц Lua в дерево
//! и извлечение того, что уже умеет композитор — тексты TextPlus (строка,
//! шрифт, начертание, размер, цвет, положение). Полное исполнение графа
//! (Merge, Transform, маски, ключи BezierSpline) — этап «Шаблоны».

use std::path::{Path, PathBuf};

/// Значение таблицы Lua.
#[derive(Clone, Debug, PartialEq)]
pub enum Lua {
    Str(String),
    Num(f64),
    Bool(bool),
    Nil,
    /// Таблица: тип перед `{` (`TextPlus`, `Input`, `ordered()`), поля по порядку
    /// (ключ — имя, `["строка"]` или `None` у элементов списка).
    Table { ty: String, items: Vec<(Option<String>, Lua)> },
}

impl Lua {
    pub fn get(&self, key: &str) -> Option<&Lua> {
        match self {
            Lua::Table { items, .. } => items.iter().find(|(k, _)| k.as_deref() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn ty(&self) -> &str {
        match self {
            Lua::Table { ty, .. } => ty,
            _ => "",
        }
    }
    pub fn items(&self) -> &[(Option<String>, Lua)] {
        match self {
            Lua::Table { items, .. } => items,
            _ => &[],
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Lua::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Lua::Str(s) => Some(s),
            _ => None,
        }
    }
    /// `Input { Value = … }` → значение.
    pub fn input(&self, name: &str) -> Option<&Lua> {
        self.get("Inputs")?.get(name)?.get("Value")
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"--") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
    }
    fn peek(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }
    fn ident(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_' || self.s[self.i] == b'.') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }
    fn string(&mut self) -> Result<String, String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() && self.s[self.i] != q {
            if self.s[self.i] == b'\\' && self.i + 1 < self.s.len() {
                self.i += 1;
                out.push(match self.s[self.i] {
                    b'n' => b'\n',
                    b't' => b'\t',
                    c => c,
                });
            } else {
                out.push(self.s[self.i]);
            }
            self.i += 1;
        }
        if self.i >= self.s.len() {
            return Err("незакрытая строка".into());
        }
        self.i += 1;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
    fn table(&mut self, ty: String) -> Result<Lua, String> {
        self.i += 1; // {
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b'}' => {
                    self.i += 1;
                    return Ok(Lua::Table { ty, items });
                }
                0 => return Err("незакрытая таблица".into()),
                b',' | b';' => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            // ключ: name = …, ["str"] = …, [1] = …
            let save = self.i;
            let mut key = None;
            if self.peek() == b'[' {
                self.i += 1;
                self.ws();
                let k = if matches!(self.peek(), b'"' | b'\'') { self.string()? } else { self.ident() };
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    self.ws();
                    if self.peek() == b'=' {
                        self.i += 1;
                        key = Some(k);
                    } else {
                        self.i = save;
                    }
                } else {
                    self.i = save;
                }
            } else if self.peek().is_ascii_alphabetic() || self.peek() == b'_' {
                let k = self.ident();
                self.ws();
                if self.peek() == b'=' && self.s.get(self.i + 1) != Some(&b'=') {
                    self.i += 1;
                    key = Some(k);
                } else {
                    self.i = save;
                }
            }
            let v = self.value()?;
            items.push((key, v));
        }
    }
    fn value(&mut self) -> Result<Lua, String> {
        self.ws();
        match self.peek() {
            b'{' => self.table(String::new()),
            b'"' | b'\'' => Ok(Lua::Str(self.string()?)),
            b'-' | b'0'..=b'9' | b'.' => {
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || matches!(self.s[self.i], b'.' | b'-' | b'+')) {
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                Ok(t.parse().map(Lua::Num).unwrap_or(Lua::Str(t)))
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let id = self.ident();
                match id.as_str() {
                    "true" => return Ok(Lua::Bool(true)),
                    "false" => return Ok(Lua::Bool(false)),
                    "nil" => return Ok(Lua::Nil),
                    _ => {}
                }
                self.ws();
                // ordered() { … }, Type { … }, Type "строка"
                let mut ty = id;
                if self.peek() == b'(' {
                    while self.i < self.s.len() && self.s[self.i] != b')' {
                        self.i += 1;
                    }
                    self.i += 1;
                    ty.push_str("()");
                    self.ws();
                }
                match self.peek() {
                    b'{' => self.table(ty),
                    b'"' | b'\'' => Ok(Lua::Table { ty, items: vec![(None, Lua::Str(self.string()?))] }),
                    _ => Ok(Lua::Str(ty)),
                }
            }
            c => Err(format!("неожиданный символ «{}» на {}", c as char, self.i)),
        }
    }
}

pub fn parse(text: &str) -> Result<Lua, String> {
    let mut p = P { s: text.as_bytes(), i: 0 };
    p.value()
}

/// Текст шаблона: строка, шрифт, начертание, размер (доля высоты кадра), цвет, центр.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayer {
    pub name: String,
    pub text: String,
    pub font: String,
    pub style: String,
    pub size: f64,
    pub color: String,
    pub center: (f64, f64),
}

/// Все инструменты шаблона (с заходом в MacroOperator/GroupOperator).
fn tools<'a>(root: &'a Lua, out: &mut Vec<(&'a str, &'a Lua)>) {
    if let Some(t) = root.get("Tools") {
        for (k, v) in t.items() {
            if let Some(k) = k {
                out.push((k.as_str(), v));
                tools(v, out);
            }
        }
    }
}

pub fn text_layers(root: &Lua) -> Vec<TextLayer> {
    let mut all = Vec::new();
    tools(root, &mut all);
    let mut v = Vec::new();
    for (name, t) in all.iter().filter(|(_, t)| t.ty() == "TextPlus") {
        let text = match t.input("StyledText") {
            Some(Lua::Str(s)) => s.clone(),
            Some(tbl @ Lua::Table { .. }) => tbl.get("Value").and_then(Lua::str).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        if text.trim().is_empty() {
            continue;
        }
        let c = |k: &str, d: f64| t.input(k).and_then(Lua::num).unwrap_or(d);
        let (r, g, b) = (c("Red1", 1.0), c("Green1", 1.0), c("Blue1", 1.0));
        let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let center = t.input("Center").map(|c| {
            let n: Vec<f64> = c.items().iter().filter_map(|(_, v)| v.num()).collect();
            (n.first().copied().unwrap_or(0.5), n.get(1).copied().unwrap_or(0.5))
        });
        v.push(TextLayer {
            name: name.to_string(),
            text,
            font: t.input("Font").and_then(Lua::str).unwrap_or("").to_string(),
            style: t.input("Style").and_then(Lua::str).unwrap_or("").to_string(),
            size: c("Size", 0.08),
            color: format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b)),
            center: center.unwrap_or((0.5, 0.5)),
        });
    }
    v
}

/// Библиотека шаблонов: `~/Storage/Видеомонтаж/DaVinci Resolve` (или `SYN_VIDEO_EDITOR_TEMPLATES`).
pub fn library() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_VIDEO_EDITOR_TEMPLATES") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Storage/Видеомонтаж/DaVinci Resolve")
}

/// Все `.setting` в папке (рекурсивно): (категория — путь папки, имя, путь).
pub fn scan(root: &Path) -> Vec<(String, String, PathBuf)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "setting") {
                let cat = p.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.display().to_string()).unwrap_or_default();
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                out.push((cat, name, p));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_textplus() {
        let s = r#"{ Tools = ordered() { T = TextPlus { Inputs = { StyledText = Input { Value = "Привет", }, Font = Input { Value = "Uni Sans", }, Size = Input { Value = 0.07, }, Red1 = Input { Value = 1, }, Green1 = Input { Value = 0, }, Blue1 = Input { Value = 0, }, ["Gamut.X"] = Input { Value = FuID { "SLog2" }, }, Center = Input { Value = { 0.5, 0.3 }, }, }, ViewInfo = OperatorInfo { Pos = { 1, -2 } }, }, }, }"#;
        let root = parse(s).unwrap();
        let t = text_layers(&root);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "Привет");
        assert_eq!(t[0].font, "Uni Sans");
        assert_eq!(t[0].color, "#ff0000");
        assert_eq!(t[0].center, (0.5, 0.3));
    }

    #[test]
    fn parses_library_template() {
        let p = library().join("Титры/Заголовки/Minimal_Title_01.setting");
        let Ok(text) = std::fs::read_to_string(&p) else { return };
        let root = parse(&text).unwrap();
        let t = text_layers(&root);
        assert!(t.iter().any(|l| l.text == "DAVINCI"), "{t:?}");
    }
}
