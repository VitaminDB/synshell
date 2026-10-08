//! Типы файлов по shared-mime-info и программы для них (`mimeapps.list`).
//!
//! База читается один раз из `$XDG_DATA_DIRS/mime` (и `~/.local/share/mime`):
//! - `globs2` — шаблоны имён с весами (`*.rs`, `Makefile`, `*.tar.gz`);
//! - `magic` — сигнатуры содержимого (для файлов без расширения или с
//!   расширением, которое подходит к нескольким типам);
//! - `subclasses`, `aliases`, `generic-icons`;
//! - `<тип>.xml` — описания на языке системы (лениво, по запросу).
//!
//! Программы: `mimeapps.list` по спецификации (свой рабочий стол →
//! пользовательский → системные), затем `MimeType=` из .desktop с учётом
//! родительских типов.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::paths;
use crate::xdg::{self, DesktopEntry};

pub const DIRECTORY: &str = "inode/directory";
pub const SYMLINK: &str = "inode/symlink";
pub const OCTET: &str = "application/octet-stream";
pub const TEXT: &str = "text/plain";

struct Glob {
    weight: u32,
    pattern: String,
    mime: String,
    case_sensitive: bool,
}

struct MagicMatch {
    indent: u32,
    offset: usize,
    value: Vec<u8>,
    mask: Option<Vec<u8>>,
    range: usize,
}

struct MagicRule {
    priority: u32,
    mime: String,
    matches: Vec<MagicMatch>,
}

#[derive(Default)]
struct Db {
    /// Точные имена и `*.ext` — быстрый путь.
    literal: HashMap<String, (u32, String)>,
    ext: HashMap<String, Vec<(u32, String)>>,
    /// Остальные шаблоны (`*.tar.gz` тоже тут через ext, а `README*` — здесь).
    other: Vec<Glob>,
    magic: Vec<MagicRule>,
    parents: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
    generic_icons: HashMap<String, String>,
    dirs: Vec<PathBuf>,
}

fn db() -> &'static Db {
    static DB: OnceLock<Db> = OnceLock::new();
    DB.get_or_init(load)
}

fn mime_dirs() -> Vec<PathBuf> {
    let mut v = vec![paths::data_home().join("mime")];
    v.extend(paths::data_dirs().into_iter().map(|d| d.join("mime")));
    v.retain(|d| d.is_dir());
    v.dedup();
    v
}

fn load() -> Db {
    let mut db = Db { dirs: mime_dirs(), ..Default::default() };
    // Первый каталог (пользовательский) важнее — читаем в обратном порядке,
    // чтобы его записи перекрыли системные.
    for dir in db.dirs.clone().iter().rev() {
        if let Ok(t) = std::fs::read_to_string(dir.join("globs2")) {
            for line in t.lines() {
                if line.starts_with('#') {
                    continue;
                }
                let mut it = line.split(':');
                let (Some(w), Some(mime), Some(pat)) = (it.next(), it.next(), it.next()) else { continue };
                let flags = it.next().unwrap_or("");
                if pat == "__NOGLOBS__" {
                    continue;
                }
                let weight = w.parse().unwrap_or(50);
                let cs = flags.split(',').any(|f| f == "cs");
                db.add_glob(weight, pat, mime, cs);
            }
        }
        if let Ok(b) = std::fs::read(dir.join("magic")) {
            parse_magic(&b, &mut db.magic);
        }
        for (file, f) in [("subclasses", 0u8), ("aliases", 1), ("generic-icons", 2)] {
            let Ok(t) = std::fs::read_to_string(dir.join(file)) else { continue };
            for line in t.lines() {
                let (a, b) = match f {
                    2 => match line.split_once(':') {
                        Some(x) => x,
                        None => continue,
                    },
                    _ => match line.split_once(' ') {
                        Some(x) => x,
                        None => continue,
                    },
                };
                match f {
                    0 => db.parents.entry(a.to_string()).or_default().push(b.to_string()),
                    1 => {
                        db.aliases.insert(a.to_string(), b.to_string());
                    }
                    _ => {
                        db.generic_icons.insert(a.to_string(), b.to_string());
                    }
                }
            }
        }
    }
    db.magic.sort_by(|a, b| b.priority.cmp(&a.priority));
    db
}

impl Db {
    fn add_glob(&mut self, weight: u32, pat: &str, mime: &str, cs: bool) {
        let special = |s: &str| s.contains(['*', '?', '['].as_ref());
        if !special(pat) {
            let key = if cs { pat.to_string() } else { pat.to_lowercase() };
            let e = self.literal.entry(key).or_insert((0, String::new()));
            if weight >= e.0 {
                *e = (weight, mime.to_string());
            }
        } else if let Some(ext) = pat.strip_prefix("*.").filter(|e| !special(e)) {
            let key = if cs { ext.to_string() } else { ext.to_lowercase() };
            self.ext.entry(key).or_default().push((weight, mime.to_string()));
        } else {
            self.other.push(Glob { weight, pattern: pat.to_string(), mime: mime.to_string(), case_sensitive: cs });
        }
    }

    fn unalias<'a>(&'a self, m: &'a str) -> &'a str {
        self.aliases.get(m).map(String::as_str).unwrap_or(m)
    }

    /// Кандидаты по имени: самые тяжёлые шаблоны, самое длинное расширение.
    fn by_name(&self, name: &str) -> Vec<String> {
        let lower = name.to_lowercase();
        if let Some((_, m)) = self.literal.get(name).or_else(|| self.literal.get(&lower)) {
            return vec![m.clone()];
        }
        // Самое длинное подходящее расширение: `a.tar.gz` → `tar.gz`, затем `gz`.
        let mut rest = name;
        while let Some(i) = rest.find('.') {
            rest = &rest[i + 1..];
            let hits = self.ext.get(rest).or_else(|| self.ext.get(&rest.to_lowercase()));
            if let Some(hits) = hits {
                let max = hits.iter().map(|h| h.0).max().unwrap_or(0);
                let mut v: Vec<String> = hits.iter().filter(|h| h.0 == max).map(|h| h.1.clone()).collect();
                v.dedup();
                return v;
            }
        }
        let mut best: Vec<(u32, &str)> = Vec::new();
        for g in &self.other {
            let ok = if g.case_sensitive { glob_match(&g.pattern, name) } else { glob_match(&g.pattern.to_lowercase(), &lower) };
            if ok {
                best.push((g.weight, &g.mime));
            }
        }
        let max = best.iter().map(|b| b.0).max().unwrap_or(0);
        best.into_iter().filter(|b| b.0 == max).map(|b| b.1.to_string()).collect()
    }

    fn by_magic(&self, data: &[u8], min_priority: u32) -> Option<(u32, &str)> {
        self.magic
            .iter()
            .filter(|r| r.priority >= min_priority)
            .find(|r| rule_matches(&r.matches, data))
            .map(|r| (r.priority, r.mime.as_str()))
    }
}

/// Простой glob: `*`, `?`, `[abc]`.
pub fn glob_match(pat: &str, s: &str) -> bool {
    fn rec(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => (0..=s.len()).any(|i| rec(&p[1..], &s[i..])),
            Some('?') => !s.is_empty() && rec(&p[1..], &s[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|&c| c == ']') else { return false };
                let set = &p[1..end];
                !s.is_empty() && set.contains(&s[0]) && rec(&p[end + 1..], &s[1..])
            }
            Some(c) => s.first() == Some(c) && rec(&p[1..], &s[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    rec(&p, &s)
}

fn parse_magic(b: &[u8], out: &mut Vec<MagicRule>) {
    if !b.starts_with(b"MIME-Magic\0\n") {
        return;
    }
    let mut i = 12;
    let mut cur: Option<MagicRule> = None;
    let num = |b: &[u8], i: &mut usize| -> usize {
        let mut n = 0usize;
        while *i < b.len() && b[*i].is_ascii_digit() {
            n = n * 10 + (b[*i] - b'0') as usize;
            *i += 1;
        }
        n
    };
    while i < b.len() {
        if b[i] == b'[' {
            if let Some(r) = cur.take() {
                out.push(r);
            }
            let Some(end) = b[i..].iter().position(|&c| c == b'\n').map(|e| i + e) else { break };
            let head = String::from_utf8_lossy(&b[i + 1..end - 1]).to_string();
            let (prio, mime) = head.split_once(':').unwrap_or(("50", ""));
            cur = Some(MagicRule { priority: prio.parse().unwrap_or(50), mime: mime.to_string(), matches: Vec::new() });
            i = end + 1;
            continue;
        }
        // [indent]>offset=LLvalue[&mask][~word][+range]\n
        let indent = if b[i] == b'>' { 0 } else { num(b, &mut i) as u32 };
        if i >= b.len() || b[i] != b'>' {
            break;
        }
        i += 1;
        let offset = num(b, &mut i);
        if i + 3 > b.len() || b[i] != b'=' {
            break;
        }
        let len = u16::from_be_bytes([b[i + 1], b[i + 2]]) as usize;
        i += 3;
        if i + len > b.len() {
            break;
        }
        let value = b[i..i + len].to_vec();
        i += len;
        let mut mask = None;
        let mut range = 1;
        while i < b.len() && b[i] != b'\n' {
            match b[i] {
                b'&' => {
                    if i + 1 + len > b.len() {
                        break;
                    }
                    mask = Some(b[i + 1..i + 1 + len].to_vec());
                    i += 1 + len;
                }
                b'~' => {
                    i += 1;
                    num(b, &mut i);
                }
                b'+' => {
                    i += 1;
                    range = num(b, &mut i).max(1);
                }
                _ => {
                    // Неизвестное расширение формата — до конца строки.
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                }
            }
        }
        i += 1;
        if let Some(r) = cur.as_mut() {
            r.matches.push(MagicMatch { indent, offset, value, mask, range });
        }
    }
    if let Some(r) = cur.take() {
        out.push(r);
    }
}

fn one_matches(m: &MagicMatch, data: &[u8]) -> bool {
    let n = m.value.len();
    for start in m.offset..m.offset + m.range {
        if start + n > data.len() {
            return false;
        }
        let win = &data[start..start + n];
        let ok = match &m.mask {
            Some(mask) => win.iter().zip(&m.value).zip(mask).all(|((d, v), k)| d & k == v & k),
            None => win == m.value.as_slice(),
        };
        if ok {
            return true;
        }
    }
    false
}

/// Дерево условий: строка с отступом `n` — дочерняя к ближайшей выше с
/// отступом `n-1`; правило срабатывает, если сработала ветка до листа.
fn rule_matches(ms: &[MagicMatch], data: &[u8]) -> bool {
    fn sub(ms: &[MagicMatch], i: usize, data: &[u8]) -> bool {
        if !one_matches(&ms[i], data) {
            return false;
        }
        let level = ms[i].indent;
        let mut j = i + 1;
        let mut has_children = false;
        while j < ms.len() && ms[j].indent > level {
            if ms[j].indent == level + 1 {
                has_children = true;
                if sub(ms, j, data) {
                    return true;
                }
            }
            j += 1;
        }
        !has_children
    }
    let mut i = 0;
    while i < ms.len() {
        if ms[i].indent == 0 && sub(ms, i, data) {
            return true;
        }
        i += 1;
    }
    false
}

fn read_head(path: &Path, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let mut read = 0;
    while read < n {
        match f.read(&mut buf[read..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => read += k,
        }
    }
    buf.truncate(read);
    buf
}

/// Похоже ли содержимое на текст (нет NUL и мало управляющих символов).
pub fn looks_like_text(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    if data.contains(&0) {
        return false;
    }
    let bad = data.iter().filter(|&&c| c < 0x20 && !matches!(c, b'\n' | b'\r' | b'\t' | 0x0c | 0x1b)).count();
    bad * 20 < data.len() && (std::str::from_utf8(data).is_ok() || bad == 0)
}

/// Тип по имени, без чтения файла (`None` — нужно смотреть содержимое).
pub fn guess_by_name(name: &str) -> Option<String> {
    let v = db().by_name(name);
    (v.len() == 1).then(|| db().unalias(&v[0]).to_string())
}

/// Тип обычного файла: имя, при неоднозначности — содержимое.
pub fn detect_file(path: &Path) -> String {
    let d = db();
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let by_name = d.by_name(&name);
    if by_name.len() == 1 {
        return d.unalias(&by_name[0]).to_string();
    }
    let head = read_head(path, 4096);
    if head.is_empty() {
        return by_name.first().map(|m| d.unalias(m).to_string()).unwrap_or_else(|| {
            if std::fs::metadata(path).map(|m| m.len() == 0).unwrap_or(false) {
                "application/x-zerosize".into()
            } else {
                OCTET.into()
            }
        });
    }
    if let Some((_, m)) = d.by_magic(&head, 0) {
        // Сигнатура и шаблоны имени спорят — берём совпавший с именем.
        if by_name.is_empty() || by_name.iter().any(|n| n == m || is_a(m, n) || is_a(n, m)) {
            return d.unalias(m).to_string();
        }
    }
    if let Some(m) = by_name.first() {
        return d.unalias(m).to_string();
    }
    if looks_like_text(&head) {
        TEXT.into()
    } else {
        OCTET.into()
    }
}

/// Тип пути: каталог, ссылка (по цели), файл.
pub fn detect(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => DIRECTORY.into(),
        Ok(m) if m.file_type().is_file() => detect_file(path),
        Ok(_) => {
            use std::os::unix::fs::FileTypeExt;
            let ft = std::fs::metadata(path).map(|m| m.file_type()).ok();
            match ft {
                Some(t) if t.is_block_device() => "inode/blockdevice".into(),
                Some(t) if t.is_char_device() => "inode/chardevice".into(),
                Some(t) if t.is_fifo() => "inode/fifo".into(),
                Some(t) if t.is_socket() => "inode/socket".into(),
                _ => OCTET.into(),
            }
        }
        Err(_) => "inode/symlink".into(),
    }
}

/// Родители типа (прямые), включая неявные: `text/*` → `text/plain`,
/// всё → `application/octet-stream`.
pub fn parents(mime: &str) -> Vec<String> {
    let d = db();
    let mut v = d.parents.get(d.unalias(mime)).cloned().unwrap_or_default();
    if mime.starts_with("text/") && mime != TEXT && !v.iter().any(|p| p == TEXT) {
        v.push(TEXT.into());
    }
    if mime != OCTET && !mime.starts_with("inode/") && !v.iter().any(|p| p == OCTET) {
        v.push(OCTET.into());
    }
    v
}

/// Тип с предками в порядке близости (сам тип первым).
pub fn ancestry(mime: &str) -> Vec<String> {
    let mut out = vec![db().unalias(mime).to_string()];
    let mut i = 0;
    while i < out.len() {
        for p in parents(&out[i].clone()) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        i += 1;
    }
    // Октет-поток — самый общий, всегда последним.
    if let Some(pos) = out.iter().position(|m| m == OCTET) {
        let o = out.remove(pos);
        out.push(o);
    }
    out
}

pub fn is_a(mime: &str, ancestor: &str) -> bool {
    mime == ancestor || ancestry(mime).iter().any(|m| m == ancestor)
}

/// Имена значков для типа по порядку предпочтения.
pub fn icon_names(mime: &str) -> Vec<String> {
    let d = db();
    let mut v = vec![mime.replace('/', "-")];
    if let Some(g) = d.generic_icons.get(mime) {
        v.push(g.clone());
    }
    for p in ancestry(mime).into_iter().skip(1) {
        if p == OCTET {
            continue;
        }
        v.push(p.replace('/', "-"));
        if let Some(g) = d.generic_icons.get(&p) {
            v.push(g.clone());
        }
    }
    let media = mime.split('/').next().unwrap_or("");
    v.push(format!("{media}-x-generic"));
    if mime == DIRECTORY {
        v.insert(0, "folder".into());
    }
    v.push(if mime == DIRECTORY { "folder".into() } else { "application-x-generic".into() });
    v.push("text-x-generic".into());
    v.dedup();
    v
}

/// Путь к значку типа в текущей теме.
pub fn icon_path(mime: &str) -> Option<PathBuf> {
    static CACHE: OnceLock<RwLock<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    if let Some(v) = c.read().ok().and_then(|c| c.get(mime).cloned()) {
        return v;
    }
    let found = icon_names(mime).iter().find_map(|n| xdg::lookup_icon(n));
    if let Ok(mut c) = c.write() {
        c.insert(mime.to_string(), found.clone());
    }
    found
}

/// Описание типа на языке системы («Документ Markdown»).
pub fn description(mime: &str) -> String {
    static CACHE: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    if let Some(v) = c.read().ok().and_then(|c| c.get(mime).cloned()) {
        return v;
    }
    let found = read_comment(mime).unwrap_or_else(|| mime.to_string());
    if let Ok(mut c) = c.write() {
        c.insert(mime.to_string(), found.clone());
    }
    found
}

fn read_comment(mime: &str) -> Option<String> {
    let langs = lang_keys();
    for dir in &db().dirs {
        let Ok(t) = std::fs::read_to_string(dir.join(format!("{mime}.xml"))) else { continue };
        let mut plain = None;
        let mut best: Option<(usize, String)> = None;
        for line in t.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("<comment") else { continue };
            let Some((attrs, body)) = rest.split_once('>') else { continue };
            let text = body.trim_end_matches("</comment>");
            let text = xml_unescape(text);
            match attrs.split("xml:lang=\"").nth(1).and_then(|s| s.split('"').next()) {
                None => plain = Some(text),
                Some(l) => {
                    if let Some(rank) = langs.iter().position(|x| x == l) {
                        if best.as_ref().map(|b| rank < b.0).unwrap_or(true) {
                            best = Some((rank, text));
                        }
                    }
                }
            }
        }
        if let Some((_, t)) = best {
            return Some(t);
        }
        if plain.is_some() {
            return plain;
        }
    }
    None
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn lang_keys() -> Vec<String> {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    let base = lang.split(['.', '@']).next().unwrap_or("").to_string();
    let mut v = Vec::new();
    if !base.is_empty() {
        v.push(base.replace('_', "-"));
        if let Some((l, _)) = base.split_once('_') {
            v.push(l.to_string());
        }
    }
    v
}

// ---------------------------------------------------------------- mimeapps

#[derive(Default, Clone)]
struct Assoc {
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, HashSet<String>>,
}

fn mimeapps_files() -> Vec<PathBuf> {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    let mut dirs = vec![paths::xdg_config_home()];
    dirs.extend(
        std::env::var("XDG_CONFIG_DIRS")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into())
            .split(':')
            .map(PathBuf::from),
    );
    let mut files = Vec::new();
    for d in &dirs {
        for de in &desktops {
            files.push(d.join(format!("{de}-mimeapps.list")));
        }
        files.push(d.join("mimeapps.list"));
    }
    let mut data = vec![paths::data_home()];
    data.extend(paths::data_dirs());
    for d in data {
        let a = d.join("applications");
        for de in &desktops {
            files.push(a.join(format!("{de}-mimeapps.list")));
        }
        files.push(a.join("mimeapps.list"));
    }
    files
}

fn load_assoc() -> Assoc {
    let mut a = Assoc::default();
    for f in mimeapps_files() {
        let Ok(t) = std::fs::read_to_string(&f) else { continue };
        let mut group = "";
        for line in t.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                group = match line {
                    "[Default Applications]" => "d",
                    "[Added Associations]" => "a",
                    "[Removed Associations]" => "r",
                    _ => "",
                };
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, apps) = (k.trim().to_string(), v.split(';').map(str::trim).filter(|s| !s.is_empty()));
            match group {
                // Раньше прочитанный файл важнее: дописываем в конец.
                "d" => a.defaults.entry(k).or_default().extend(apps.map(String::from)),
                "a" => a.added.entry(k).or_default().extend(apps.map(String::from)),
                "r" => a.removed.entry(k).or_default().extend(apps.map(String::from)),
                _ => {}
            }
        }
    }
    a
}

static ASSOC: RwLock<Option<Assoc>> = RwLock::new(None);

fn assoc() -> Assoc {
    if let Some(a) = ASSOC.read().ok().and_then(|g| g.clone()) {
        return a;
    }
    let a = load_assoc();
    if let Ok(mut g) = ASSOC.write() {
        *g = Some(a.clone());
    }
    a
}

/// Прежние имена программ synshell → нынешние (записи в `mimeapps.list`
/// остались от syndesktop: без замены тип уходил первой попавшейся программе).
const LEGACY_IDS: &[(&str, &str)] = &[("syndesktop-files", "synfiles")];

fn strip_desktop(id: &str) -> &str {
    let id = id.strip_suffix(".desktop").unwrap_or(id);
    LEGACY_IDS.iter().find(|(old, _)| *old == id).map(|(_, new)| *new).unwrap_or(id)
}

/// Заменить прежние имена программ synshell в `~/.config/mimeapps.list` —
/// его читают и сторонние программы (`xdg-open`), которым замена в
/// [`strip_desktop`] не видна.
pub fn migrate_legacy_ids() {
    let path = paths::xdg_config_home().join("mimeapps.list");
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let mut out = text.clone();
    for (old, new) in LEGACY_IDS {
        out = out.replace(&format!("{old}.desktop"), &format!("{new}.desktop"));
    }
    if out != text && std::fs::write(&path, out).is_ok() {
        reload_assoc();
    }
}

/// Команды, которыми файл или каталог открывается программой по умолчанию.
/// Пусто — подходящей программы нет.
pub fn open_commands(path: &Path) -> Vec<String> {
    let mime = detect(path);
    default_app(&mime).map(|e| e.commands_for(&[path.to_path_buf()])).unwrap_or_default()
}

/// Программы для типа: сначала по умолчанию, затем остальные подходящие
/// (с учётом родительских типов), без повторов.
pub fn apps_for(mime: &str) -> Vec<DesktopEntry> {
    let a = assoc();
    let all = xdg::apps();
    let by_id = |id: &str| all.iter().find(|e| e.id == strip_desktop(id)).cloned();
    let mut out: Vec<DesktopEntry> = Vec::new();
    let push = |e: DesktopEntry, out: &mut Vec<DesktopEntry>| {
        if !out.iter().any(|x| x.id == e.id) {
            out.push(e);
        }
    };
    for m in ancestry(mime) {
        let removed = a.removed.get(&m).cloned().unwrap_or_default();
        let ok = |id: &str| !removed.contains(id) && !removed.contains(&format!("{id}.desktop"));
        for id in a.defaults.get(&m).into_iter().flatten().chain(a.added.get(&m).into_iter().flatten()) {
            if ok(strip_desktop(id)) {
                if let Some(e) = by_id(id) {
                    push(e, &mut out);
                }
            }
        }
        for e in all.iter().filter(|e| e.mime_types.iter().any(|t| t == &m) && ok(&e.id)) {
            push(e.clone(), &mut out);
        }
        // От октет-потока подходят все подряд — не предлагать.
        if m == OCTET {
            break;
        }
    }
    out
}

/// Программа по умолчанию для типа.
pub fn default_app(mime: &str) -> Option<DesktopEntry> {
    apps_for(mime).into_iter().find(|e| e.takes_files() || mime == DIRECTORY)
}

/// Сделать программу умолчанием для типа (`~/.config/mimeapps.list`).
pub fn set_default_app(mime: &str, app_id: &str) -> std::io::Result<()> {
    set_default_apps(&[(mime.to_string(), Some(app_id.to_string()))])
}

/// Программа по умолчанию, выбранная пользователем (`~/.config/mimeapps.list`),
/// а не пришедшая из системных списков.
pub fn user_default(mime: &str) -> Option<String> {
    let text = std::fs::read_to_string(paths::xdg_config_home().join("mimeapps.list")).ok()?;
    let mut in_default = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_default = t == "[Default Applications]";
        } else if in_default {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == mime {
                    return v.split(';').map(str::trim).find(|s| !s.is_empty()).map(|s| strip_desktop(s).to_string());
                }
            }
        }
    }
    None
}

/// Записать умолчания для нескольких типов одним проходом: `Some(id)` —
/// программа, `None` — убрать свою запись (снова решают системные списки).
pub fn set_default_apps(entries: &[(String, Option<String>)]) -> std::io::Result<()> {
    let path = paths::xdg_config_home().join("mimeapps.list");
    let out = rewrite_defaults(&std::fs::read_to_string(&path).unwrap_or_default(), entries);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, out)?;
    reload_assoc();
    Ok(())
}

/// Текст `mimeapps.list` с заменёнными строками `[Default Applications]`.
fn rewrite_defaults(text: &str, entries: &[(String, Option<String>)]) -> String {
    let line_for = |mime: &str| -> Option<String> {
        entries.iter().rev().find(|(m, _)| m == mime).and_then(|(m, id)| id.as_ref().map(|id| format!("{m}={id}.desktop;")))
    };
    let mut written: HashSet<String> = HashSet::new();
    let mut out = String::new();
    let mut in_default = false;
    let mut seen_default = false;
    let flush_new = |out: &mut String, written: &mut HashSet<String>| {
        for (m, _) in entries {
            if written.insert(m.clone()) {
                if let Some(l) = line_for(m) {
                    out.push_str(&l);
                    out.push('\n');
                }
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_default {
                // Новые строки — перед пустыми строками, отделяющими секции.
                let mut blanks = 0;
                while out.ends_with("\n\n") {
                    out.pop();
                    blanks += 1;
                }
                flush_new(&mut out, &mut written);
                out.push_str(&"\n".repeat(blanks));
            }
            in_default = t == "[Default Applications]";
            seen_default |= in_default;
        } else if in_default {
            if let Some((k, _)) = t.split_once('=') {
                if let Some((m, _)) = entries.iter().find(|(m, _)| m == k.trim()) {
                    if written.insert(m.clone()) {
                        if let Some(l) = line_for(m) {
                            out.push_str(&l);
                            out.push('\n');
                        }
                    }
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_default {
        flush_new(&mut out, &mut written);
    } else if !seen_default && entries.iter().any(|(_, id)| id.is_some()) {
        // Секции умолчаний ещё нет (если была — новые строки дописаны при выходе из неё).
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("[Default Applications]\n");
        flush_new(&mut out, &mut written);
    }
    out
}

/// Все типы, которые знают установленные программы или списки умолчаний.
pub fn known_types() -> Vec<String> {
    let a = assoc();
    let mut set: HashSet<String> = a.defaults.keys().cloned().collect();
    for e in xdg::apps().iter() {
        set.extend(e.mime_types.iter().filter(|m| m.contains('/')).cloned());
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

/// Перечитать `mimeapps.list` (после внешней правки).
pub fn reload_assoc() {
    if let Ok(mut g) = ASSOC.write() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_defaults_keeps_other_lines() {
        let e = |m: &str, id: Option<&str>| (m.to_string(), id.map(String::from));
        let text = "[Default Applications]\ninode/directory=org.kde.filelight.desktop;\ntext/plain=kate.desktop;\n\n[Added Associations]\nimage/png=gwenview.desktop;\n";
        let out = rewrite_defaults(text, &[e("inode/directory", Some("synfiles")), e("text/plain", None), e("video/mp4", Some("mpv"))]);
        assert_eq!(
            out,
            "[Default Applications]\ninode/directory=synfiles.desktop;\nvideo/mp4=mpv.desktop;\n\n[Added Associations]\nimage/png=gwenview.desktop;\n"
        );
        assert_eq!(rewrite_defaults("", &[e("a/b", Some("x"))]), "[Default Applications]\na/b=x.desktop;\n");
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.tar.gz", "a.tar.gz"));
        assert!(glob_match("README*", "README.md"));
        assert!(glob_match("[Mm]akefile", "makefile"));
        assert!(!glob_match("*.rs", "a.rsx"));
    }

    #[test]
    fn magic_tree() {
        let mut v = Vec::new();
        let mut b = b"MIME-Magic\0\n[50:image/png]\n>0=\x00\x04\x89PNG\n".to_vec();
        b.extend_from_slice(b"[40:x/child]\n>0=\x00\x02AB\n1>4=\x00\x01C\n");
        parse_magic(&b, &mut v);
        assert_eq!(v.len(), 2);
        assert!(rule_matches(&v[0].matches, b"\x89PNG...."));
        assert!(rule_matches(&v[1].matches, b"ABxxC"));
        assert!(!rule_matches(&v[1].matches, b"ABxxD"));
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    /// Проверка на базе системы (если shared-mime-info установлен).
    #[test]
    fn detect_real_files() {
        if !Path::new("/usr/share/mime/globs2").exists() {
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(detect(root), DIRECTORY);
        assert_eq!(detect(&root.join("Cargo.toml")), "application/toml");
        assert_eq!(detect(&root.join("src/lib.rs")), "text/rust");
        assert!(is_a("text/rust", TEXT));
        assert_eq!(detect(&root.join("../../docs/themes.jpg")), "image/jpeg");
        assert!(!description("image/jpeg").is_empty());
        eprintln!("jpeg: {} | {:?}", description("image/jpeg"), icon_names("text/rust"));
        let apps: Vec<String> = apps_for("text/plain").into_iter().map(|e| e.id).collect();
        eprintln!("text apps: {apps:?}; dir default: {:?}", default_app(DIRECTORY).map(|e| e.id));
    }
}

