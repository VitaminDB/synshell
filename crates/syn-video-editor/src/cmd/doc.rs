//! Документы проекта — Markdown-файлы в его папке (сценарий, диалоги,
//! заметки): агент пишет и перечитывает их, пользователь открывает двойным
//! щелчком в медиатеке и правит.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{arg, req, Args, Command as Cmd, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&WRITE, &READ, &LIST, &DOCUMENT];

/// Папка документов проекта (папка проекта; без файла — нет).
pub fn dir(project: Option<&Path>) -> Option<PathBuf> {
    project.map(crate::store::media_dir)
}

/// Документы проекта: `*.md` в его папке, по имени.
pub fn list_docs(project: Option<&Path>) -> Vec<PathBuf> {
    let Some(d) = dir(project) else { return Vec::new() };
    let mut v: Vec<PathBuf> = std::fs::read_dir(&d)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn doc_path(ed: &Editor, name: &str) -> Result<PathBuf, String> {
    let d = dir(ed.path.as_deref()).ok_or("проект ещё не сохранён — у него нет папки")?;
    let stem: String = name.trim().trim_end_matches(".md").chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '-' } else { c }).collect();
    if stem.trim().is_empty() {
        return Err("пустое имя документа".into());
    }
    Ok(d.join(format!("{}.md", stem.trim())))
}

static WRITE: Cmd = Cmd {
    id: "doc.write",
    category: "Проект",
    title: "Записать документ проекта",
    shortcut: "",
    keywords: "doc document документ сценарий диалоги заметки markdown записать",
    help: "Записать Markdown-документ в папку проекта (`<name>.md`): сценарий, диалоги по сценам, заметки. Пользователь видит его в медиатеке («Документы») и может править — перед следующими шагами перечитай doc.read. `append` — дописать в конец.",
    args: &[req("name", Ty::Str, "имя документа (без .md)"), req("text", Ty::Str, "содержимое (Markdown)"), arg("append", Ty::Bool, "дописать в конец")],
    danger: false,
    run: write,
};

static READ: Cmd = Cmd {
    id: "doc.read",
    category: "Проект",
    title: "Прочитать документ проекта",
    shortcut: "",
    keywords: "doc read прочитать документ сценарий диалоги",
    help: "Прочитать Markdown-документ из папки проекта (с правками пользователя).",
    args: &[req("name", Ty::Str, "имя документа (без .md)")],
    danger: false,
    run: read,
};

static DOCUMENT: Cmd = Cmd {
    id: "document.read",
    category: "Проект",
    title: "Читать документ (PDF)",
    shortcut: "",
    keywords: "pdf документ читать страницы поиск сценарий книга",
    help: "Читать документ (PDF) по частям через службу: без pages и search — обзор (страниц, сканы, токены, оглавление); pages «a-b» — страницы по порядку, текст и картинки на своих местах (до 40 за вызов; ocr — распознать сканы); search — найти слова и страницы. Большой документ во вложении показан обзором — дочитывай им.",
    args: &[req("path", Ty::Str, "путь к документу"), arg("pages", Ty::Str, "страницы «a-b»"), arg("search", Ty::Str, "что найти"), arg("ocr", Ty::Bool, "распознать сканы")],
    danger: false,
    run: |_, a| {
        use syn_assist::tools::Tool;
        let o = syn_assist::builtin::document::DocumentTool.run(&a.all_value())?;
        Ok(Out::with(o.text, json!({"files": o.files})))
    },
};

static LIST: Cmd = Cmd {
    id: "doc.list",
    category: "Проект",
    title: "Документы проекта",
    shortcut: "",
    keywords: "doc list документы проекта",
    help: "Список Markdown-документов в папке проекта.",
    args: &[],
    danger: false,
    run: list,
};

fn write(ed: &mut Editor, a: &Args) -> Res {
    let p = doc_path(ed, &a.need_str("name")?)?;
    let text = crate::yo::yofy(&a.need_str("text")?);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let body = if a.bool("append")?.unwrap_or(false) {
        let old = std::fs::read_to_string(&p).unwrap_or_default();
        if old.is_empty() { text } else { format!("{}\n{text}", old.trim_end()) }
    } else {
        text
    };
    std::fs::write(&p, &body).map_err(|e| format!("{}: {e}", p.display()))?;
    ed.revision += 1;
    let mut msg = format!("Документ {} ({} строк)", p.display(), body.lines().count());
    // полнота сцен — только у сценария (у диалогов другие разделы)
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    // сценарий и диалоги под любым именем (агент мог назвать по-своему)
    let is_script = ["сценари", "screenplay", "script"].iter().any(|k| stem.contains(k));
    let thin = if is_script { thin_scenes(&body) } else { Vec::new() };
    if !thin.is_empty() {
        msg.push_str(&format!(
            ". Сцены {} описаны неполно — допиши каждую по шаблону навыка screenplay (поля: Задача, План и камера, Действие, Эмоции, Звук, Реплики) и перезапиши документ",
            thin.join(", ")
        ));
    }
    if is_script {
        if let Some(w) = length_mismatch(&body) {
            msg.push_str(&format!(". {w}"));
        }
        for w in source_mismatch(&body) {
            msg.push_str(&format!(". {w}"));
        }
    }
    let is_dialog = ["диалог", "dialog", "реплик"].iter().any(|k| stem.contains(k));
    if is_dialog {
        let (lat, all) = quoted_latin(&body);
        if all >= 4 && lat * 2 > all {
            msg.push_str(&format!(". Реплики не переведены: {lat} из {all} в «ёлочках» латиницей — переведи все реплики на язык фильма (адаптивный перевод по навыку screenplay) и перезапиши документ"));
        }
    }
    if is_dialog {
        let latin = latin_speakers(&body);
        if !latin.is_empty() {
            msg.push_str(&format!(". Имена говорящих латиницей ({}) при репликах на языке перевода — напиши их на языке перевода, как в сценарии, и перезапиши документ", latin.join(", ")));
        }
        let bad = bad_lines(&body);
        if !bad.is_empty() {
            msg.push_str(&format!(". Реплики не в едином формате «ИМЯ [подача]: «реплика»» ({} строк, например: {}) — приведи все реплики к нему и перезапиши документ", bad.len(), bad.iter().take(2).map(|l| format!("«{}»", l.chars().take(60).collect::<String>())).collect::<Vec<_>>().join(", ")));
        }
    }
    if is_dialog || is_script {
        let src = SOURCE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(src) = src {
            for w in names_and_profanity(&body, &src, is_dialog) {
                msg.push_str(&format!(". {w}"));
            }
        }
    }
    let mixed = mixed_words(&body);
    if !mixed.is_empty() {
        msg.push_str(&format!(". Слова со смесью латиницы и кириллицы: {} — исправь (одна письменность в слове) и перезапиши документ", mixed.join(", ")));
    }
    Ok(Out::with(msg, json!({"path": p, "thin_scenes": thin, "mixed_words": mixed})))
}

/// Сценарий-источник, приложенный пользователем: сцены (заголовки
/// INT./EXT.), страницы без титульной и название. По нему `doc.write`
/// сверяет адаптацию.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub scenes: usize,
    pub pages: usize,
    pub title: String,
    /// Имена говорящих (строки капсом над репликами).
    pub names: Vec<String>,
    /// Грубых ругательств в репликах источника.
    pub profanity: usize,
}

/// Грубые ругательства (основы) в тексте источника.
const STRONG: &[&str] = &["fuck", "shit", "bitch", "asshole", "cunt", "dick", "bastard", "motherf", "damn"];
/// Мягкие замены брани в переводе.
const SOFT: &[&str] = &["блин", "чёрт", "черт", "отвали", "фиг", "блинский", "ёлки", "елки", "проклят"];

/// Имя говорящего в строке сценария: строка капсом латиницей (2–30 знаков),
/// с пометками (O.S.), (V.O.), (CONT'D); не заголовок сцены и не переход.
fn speaker_of(line: &str) -> Option<String> {
    let t = line.trim();
    let name = t.split('(').next().unwrap_or("").trim().trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
    let n = name.chars().count();
    if !(2..=30).contains(&n) || is_slug(t) || t.ends_with(':') || t.ends_with('.') && !t.contains('(') {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_uppercase() || c == ' ' || c == '\'' || c == '.' || c == '-') || !name.chars().any(|c| c.is_ascii_uppercase()) {
        return None;
    }
    if ["BEAT", "FADE IN", "FADE OUT", "CUT TO", "THE END", "CONTINUED", "LATER", "INTERCUT", "BLACK", "SUPER", "TITLE", "MONTAGE", "FLASHBACK", "END FLASHBACK", "WRITTEN BY"].iter().any(|w| name.starts_with(w)) {
        return None;
    }
    Some(name.to_string())
}

static SOURCE: std::sync::Mutex<Option<Source>> = std::sync::Mutex::new(None);

/// Запомнить источник по вложениям хода (PDF или текст со сценарием).
pub fn set_source_from(files: &[std::path::PathBuf]) {
    for f in files {
        let pages: Vec<String> = match syn_assist::attach::kind(f) {
            syn_assist::attach::Kind::Pdf => syn_assist::pdf::pages(f).map(|v| v.into_iter().map(|(_, t, _)| t).collect()).unwrap_or_default(),
            syn_assist::attach::Kind::Text => std::fs::read_to_string(f).map(|t| vec![t]).unwrap_or_default(),
            _ => continue,
        };
        if let Some(s) = source_of(&pages) {
            *SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
            return;
        }
    }
}

fn is_slug(line: &str) -> bool {
    let t = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
    ["INT.", "EXT.", "INT/EXT", "EXT/INT", "I/E", "INT ", "EXT ", "ИНТ.", "НАТ.", "ЭКСТ.", "ИНТ ", "НАТ "].iter().any(|p| t.starts_with(p))
}

/// Сцены, страницы (без титульной: первая без сцен) и название (первая
/// строка первой страницы). Нет ни одной сцены — не сценарий.
pub fn source_of(pages: &[String]) -> Option<Source> {
    let scenes: usize = pages.iter().map(|p| p.lines().filter(|l| is_slug(l)).count()).sum();
    if scenes == 0 {
        return None;
    }
    let with_text: Vec<&String> = pages.iter().filter(|p| p.chars().any(char::is_alphanumeric)).collect();
    let title_page = with_text.first().is_some_and(|p| !p.lines().any(is_slug));
    let title = with_text.first().filter(|_| title_page).and_then(|p| p.lines().map(str::trim).find(|l| !l.is_empty())).unwrap_or("").to_string();
    // имена: строки-говорящие, встреченные хотя бы дважды
    let mut counts: Vec<(String, usize)> = Vec::new();
    for p in pages {
        for l in p.lines() {
            if let Some(n) = speaker_of(l) {
                match counts.iter_mut().find(|(k, _)| *k == n) {
                    Some(c) => c.1 += 1,
                    None => counts.push((n, 1)),
                }
            }
        }
    }
    let names = counts.into_iter().filter(|(n, c)| *c >= 2 && *n != title).map(|(n, _)| n).collect();
    let low = pages.join("\n").to_lowercase();
    let profanity = STRONG.iter().map(|w| low.matches(w).count()).sum();
    Some(Source { scenes, pages: with_text.len() - usize::from(title_page), title, names, profanity })
}

/// Русская запись латиницей — для сравнения названия с оригиналом.
fn latin(s: &str) -> String {
    let map = |c: char| -> &'static str {
        match c {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' | 'ё' | 'э' => "e", 'ж' => "zh", 'з' => "z", 'и' | 'й' | 'ы' => "i",
            'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f",
            'х' => "h", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "sch", 'ю' => "yu", 'я' => "ya", _ => "",
        }
    };
    s.to_lowercase().chars().map(|c| if c.is_ascii_alphabetic() { c.to_string() } else { map(c).to_string() }).collect()
}

/// Похожесть строк по общим биграммам (0…1).
fn similar(a: &str, b: &str) -> f64 {
    let grams = |s: &str| -> Vec<(char, char)> { let v: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect(); v.windows(2).map(|w| (w[0], w[1])).collect() };
    let (ga, mut gb) = (grams(a), grams(b));
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let total = ga.len() + gb.len();
    let mut hit = 0;
    for g in &ga {
        if let Some(i) = gb.iter().position(|x| x == g) {
            gb.remove(i);
            hit += 1;
        }
    }
    2.0 * hit as f64 / total as f64
}

/// Сверка адаптации с источником: число сцен, страницы, перевод названия.
pub fn source_mismatch(text: &str) -> Vec<String> {
    let Some(src) = SOURCE.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return Vec::new() };
    source_check(text, &src)
}

pub fn source_check(text: &str, src: &Source) -> Vec<String> {
    let mut out = Vec::new();
    let scenes = text.lines().filter(|l| l.starts_with('#') && l.trim_start_matches('#').trim().to_lowercase().starts_with("сцена ")).count();
    if scenes == 0 {
        out.push(format!("В сценарии нет разделов сцен («### Сцена N — <место, время> — ~<длительность>»), а в источнике {} сцен — распиши каждую сцену отдельным разделом по шаблону навыка screenplay", src.scenes));
    } else if scenes != src.scenes {
        out.push(format!("В сценарии-источнике {} сцен, а в адаптации {scenes} — перенеси все сцены источника в его порядке, ничего не объединяя и не выбрасывая", src.scenes));
    }
    if let Some(l) = text.lines().find(|l| l.to_lowercase().contains("страниц")) {
        let n: Option<usize> = l.split(|c: char| !c.is_ascii_digit()).filter_map(|w| w.parse().ok()).find(|v| (1..2000).contains(v) && l.to_lowercase().contains(&format!("{v} стр")) || l.contains(&format!("~{v} стр")));
        if let Some(n) = n.filter(|n| *n != src.pages) {
            out.push(format!("Страниц в источнике {} (без титульной), а в шапке {n} — исправь и пересчитай длину (≈1 страница = 1 минута)", src.pages));
        }
    }
    let title = text.lines().find(|l| l.starts_with("# ")).unwrap_or("");
    let ru: String = title.split(['(', '—']).next().unwrap_or("").chars().filter(|c| !c.is_ascii()).collect();
    if !src.title.is_empty() && ru.chars().any(char::is_alphabetic) && similar(&latin(&ru), &src.title.to_lowercase()) >= 0.5 {
        out.push(format!("Название «{}» — запись звуков оригинала «{}», а нужно перевести его по смыслу (из сюжета и контекста; игру слов передать или пояснить), оригинал — в скобках", ru.trim().trim_matches(['«', '»', ' ']), src.title));
    }
    out
}

/// Длительность в секундах из записи «~2 мин», «~0,5 мин», «~16 с», «15–17 минут»
/// (диапазон — его границы).
fn durations(s: &str) -> Option<(f64, f64)> {
    let low = s.to_lowercase().replace(',', ".");
    let chars: Vec<char> = low.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let a: f64 = chars[i..j].iter().collect::<String>().parse().ok()?;
            let mut b = a;
            let mut k = j;
            // диапазон «15–17», «15-17»
            if k < chars.len() && matches!(chars[k], '–' | '-' | '—') && k + 1 < chars.len() && chars[k + 1].is_ascii_digit() {
                let mut m = k + 1;
                while m < chars.len() && (chars[m].is_ascii_digit() || chars[m] == '.') {
                    m += 1;
                }
                b = chars[k + 1..m].iter().collect::<String>().parse().unwrap_or(a);
                k = m;
            }
            let rest: String = chars[k..].iter().take(8).collect::<String>().trim_start().to_string();
            if rest.starts_with("мин") {
                return Some((a * 60.0, b * 60.0));
            }
            if rest.starts_with("с ") || rest.starts_with("сек") || rest == "с" || rest.starts_with("с)") || rest.starts_with("с,") || rest.starts_with("с;") {
                return Some((a, b));
            }
            i = k.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Сумма длительностей сцен против длины в шапке сценария («Длина: …»).
pub fn length_mismatch(text: &str) -> Option<String> {
    let header = text.lines().find(|l| l.trim_start_matches(['*', '#', ' ', '-']).to_lowercase().starts_with("длина")).and_then(durations)?;
    let mut sum = (0.0, 0.0);
    let mut scenes = 0;
    for l in text.lines() {
        let t = l.trim_start_matches('#').trim();
        if l.starts_with('#') && t.to_lowercase().starts_with("сцена ") {
            if let Some((a, b)) = durations(t) {
                sum = (sum.0 + a, sum.1 + b);
                scenes += 1;
            }
        }
    }
    if scenes == 0 {
        return None;
    }
    let mid = (sum.0 + sum.1) / 2.0;
    if mid < header.0 * 0.8 || mid > header.1 * 1.2 {
        let m = |v: f64| format!("{:.1} мин", v / 60.0);
        return Some(format!("Длина нечестная: в шапке {}–{}, а сумма длительностей {scenes} сцен — {} — пересчитай: длительность сцены по её действию и речи, сумма сцен = длина в шапке", m(header.0), m(header.1), m(mid)));
    }
    None
}

/// Метки говорящих латиницей («NAME:», «NAME [указание]:») в документе с
/// кириллическими репликами.
pub fn latin_speakers(text: &str) -> Vec<String> {
    if !text.chars().any(|c| ('\u{400}'..='\u{4FF}').contains(&c)) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let t = l.trim().trim_start_matches(['-', '*', ' ']);
        let label: String = t.chars().take_while(|c| c.is_alphabetic() || *c == ' ' || *c == '.' || *c == '\'').collect();
        let rest = &t[label.len()..];
        let label = label.trim();
        if label.chars().count() >= 2 && label.chars().all(|c| c.is_ascii_uppercase() || c == ' ' || c == '.' || c == '\'') && (rest.starts_with(':') || rest.trim_start().starts_with('[') || rest.trim_start().starts_with('(')) && rest.contains(':') {
            if !out.iter().any(|o| o == label) {
                out.push(label.to_string());
            }
            if out.len() >= 6 {
                break;
            }
        }
    }
    out
}

/// Метки говорящих кириллицей капсом в начале строк (`ИМЯ [..]:`, `**ИМЯ:**`, `3. ИМЯ:`).
pub fn cyr_speakers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let t = l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ' || c == '*' || c == '-');
        let label: String = t.chars().take_while(|c| c.is_alphabetic() && c.is_uppercase() || *c == ' ' || *c == '-').collect();
        let label = label.trim();
        let rest = t[t.char_indices().nth(label.chars().count()).map(|(i, _)| i).unwrap_or(t.len())..].trim_start().trim_start_matches('*');
        if label.chars().count() >= 2 && label.chars().any(|c| ('\u{400}'..='\u{4FF}').contains(&c)) && (rest.starts_with(':') || rest.starts_with('[') || rest.starts_with('(')) {
            for w in label.split_whitespace() {
                if w.chars().count() >= 3 && !out.iter().any(|o| o == w) {
                    out.push(w.to_string());
                }
            }
        }
    }
    out
}

/// Строки реплик не в формате `ИМЯ [подача]: «реплика»` (подача и пометки
/// в [] или () — по желанию).
pub fn bad_lines(text: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for l in text.lines() {
        let t = l.trim();
        let label: String = t.chars().take_while(|c| c.is_alphabetic() && c.is_uppercase() || *c == ' ' || *c == '-').collect();
        if label.trim().chars().count() < 2 || !label.chars().any(|c| ('\u{400}'..='\u{4FF}').contains(&c)) {
            // не начинается с имени капсом — реплика ли это? (**ИМЯ**, «1. ИМЯ», «ИМЯ —»)
            let stripped = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ' || c == '*');
            let lab2: String = stripped.chars().take_while(|c| c.is_alphabetic() && c.is_uppercase() || *c == ' ').collect();
            if stripped.len() != t.len() && lab2.trim().chars().count() >= 2 && lab2.chars().any(|c| ('\u{400}'..='\u{4FF}').contains(&c)) && (stripped.contains('«') || stripped.contains(':') || stripped.contains('—')) {
                bad.push(t.to_string());
            }
            continue;
        }
        let mut rest = t[label.len()..].trim_start();
        // подача и пометки
        while rest.starts_with('[') || rest.starts_with('(') {
            let close = if rest.starts_with('[') { ']' } else { ')' };
            match rest.find(close) {
                Some(i) => rest = rest[i + close.len_utf8()..].trim_start(),
                None => break,
            }
        }
        let ok = rest.starts_with(':') && {
            let r = rest[1..].trim();
            r.starts_with('«') && r.ends_with('»')
        };
        if !ok && (rest.starts_with(':') || rest.starts_with('—') || rest.starts_with('*')) {
            bad.push(t.to_string());
        }
    }
    bad
}

/// Имена говорящих против источника и смягчённая брань.
pub fn names_and_profanity(text: &str, src: &Source, is_dialog: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut wrong = Vec::new();
    for ru in cyr_speakers(text) {
        let lat = latin(&ru);
        let Some((best, score)) = src.names.iter().map(|n| (n, similar(&lat, &n.to_lowercase()))).max_by(|a, b| a.1.total_cmp(&b.1)) else { continue };
        let mut a: Vec<char> = lat.chars().collect();
        let mut b: Vec<char> = best.to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
        a.sort_unstable();
        b.sort_unstable();
        let permuted = a == b && lat != best.to_lowercase().replace(' ', "");
        if permuted || (0.4..0.85).contains(&score) {
            wrong.push(format!("«{ru}» (в источнике {best})"));
        }
    }
    if !wrong.is_empty() {
        out.push(format!("Имена не совпадают с источником: {} — имя человека транскрипцией по произношению оригинала, без перестановки и замены букв, одинаково во всех документах; роль без имени (водитель, тренер и т. п.) — переводом; исправь и перезапиши", wrong.join(", ")));
    }
    if is_dialog && src.profanity > 0 {
        let low = text.to_lowercase();
        let mut soft: Vec<&str> = SOFT.iter().copied().filter(|w| low.contains(*w)).collect();
        if cut_sends(&low) {
            soft.push("недосказанный посыл");
        }
        if !soft.is_empty() {
            out.push(format!("В источнике грубая брань ({} мест), а в переводе мягкие замены ({}) — брань переводится не слабее оригинала (а если пользователь разрешил мат — матом); исправь и перезапиши", src.profanity, soft.join(", ")));
        }
    }
    out
}

/// Посыл без продолжения («иди ты!», «пошёл ты…», «катись») — смягчённый
/// перевод грубого посыла оригинала.
pub fn cut_sends(low: &str) -> bool {
    const SENDS: &[&str] = &["иди ты", "пошёл ты", "пошел ты", "пошла ты", "идите вы", "катись", "иди к чёрту", "иди к черту", "иди в ад"];
    const STRONG_RU: &[&str] = &["нах", "хуй", "хуе", "хер", "пизд", "ебан", "ебл", "ёб", "еб", "жоп", "бляд", "сук"];
    for s in SENDS {
        let mut from = 0;
        while let Some(i) = low[from..].find(s).map(|i| i + from) {
            let tail: String = low[i + s.len()..].chars().take(24).collect();
            let next: String = tail.split(|c: char| c == '!' || c == '.' || c == '»' || c == '?' || c == '\n').next().unwrap_or("").to_string();
            if !STRONG_RU.iter().any(|m| next.contains(m)) {
                return true;
            }
            from = i + s.len();
        }
    }
    false
}

/// Реплики в «ёлочках»: (сколько латиницей, сколько всего).
pub fn quoted_latin(text: &str) -> (usize, usize) {
    let (mut lat, mut all) = (0, 0);
    for part in text.split('«').skip(1) {
        let q = part.split('»').next().unwrap_or("");
        let letters: Vec<char> = q.chars().filter(|c| c.is_alphabetic()).collect();
        if letters.is_empty() {
            continue;
        }
        all += 1;
        if letters.iter().filter(|c| c.is_ascii_alphabetic()).count() * 2 > letters.len() {
            lat += 1;
        }
    }
    (lat, all)
}

/// Слова, где смешаны латиница и кириллица («Арrogant»).
pub fn mixed_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in text.split(|c: char| !c.is_alphanumeric()) {
        let lat = w.chars().any(|c| c.is_ascii_alphabetic());
        let cyr = w.chars().any(|c| ('\u{400}'..='\u{4FF}').contains(&c));
        if lat && cyr && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
            if out.len() >= 8 {
                break;
            }
        }
    }
    out
}

/// Разделы сцен («### Сцена N …») без обязательных полей сценария.
pub fn thin_scenes(text: &str) -> Vec<String> {
    const FIELDS: &[&str] = &["план", "действие", "звук", "реплик"];
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let check = |c: Option<(String, String)>, out: &mut Vec<String>| {
        if let Some((name, body)) = c {
            let low = body.to_lowercase();
            if FIELDS.iter().filter(|f| low.contains(*f)).count() < 3 {
                out.push(name);
            }
        }
    };
    for line in text.lines() {
        let t = line.trim_start_matches('#').trim();
        if line.starts_with('#') && t.to_lowercase().starts_with("сцена ") {
            check(cur.take(), &mut out);
            let n: String = t[t.find(' ').map(|i| i + 1).unwrap_or(0)..].chars().take_while(|c| c.is_ascii_digit()).collect();
            cur = Some((if n.is_empty() { t.chars().take(20).collect() } else { n }, String::new()));
        } else if line.starts_with('#') {
            check(cur.take(), &mut out);
        } else if let Some((_, b)) = cur.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    check(cur, &mut out);
    out
}

#[cfg(test)]
mod length_tests {
    #[test]
    fn honest_length_and_mixed_words() {
        let bad = "# Фильм\n**Длина:** ~15–17 минут\n### Сцена 1 — КУХНЯ — ~2 мин\n### Сцена 2 — ДВОР — ~3 мин\n";
        assert!(super::length_mismatch(bad).is_some());
        let ok = "# Фильм\n**Длина:** ~5 минут\n### Сцена 1 — КУХНЯ — ~2 мин\n### Сцена 2 — ДВОР — ~3 мин\n";
        assert!(super::length_mismatch(ok).is_none());
        let secs = "Длина: ~1 мин\n### Сцена 1 (~30 с)\n### Сцена 2 (~25 с)\n";
        assert!(super::length_mismatch(secs).is_none());
        assert_eq!(super::mixed_words("Он Арrogant и спокоен, Ndumiso"), vec!["Арrogant".to_string()]);
        assert_eq!(super::latin_speakers("ANNA [тихо]: «Привет»\nБОРИС: «Да»\nPAUL: «Ну»"), vec!["ANNA".to_string(), "PAUL".to_string()]);
        assert!(super::latin_speakers("ANNA: Hello").is_empty());
        // источник: титульная страница не считается, сцены по INT./EXT.
        let pages = vec!["TOGO MAKUNDA WENI\nWritten by\nAuthor".to_string(), "INT. HOUSE - DAY 1 1\ntext\nEXT. STREET - NIGHT2 2\nmore".to_string(), "3.\nINT. CAR - DAY 3".to_string()];
        let src = super::source_of(&pages).unwrap();
        assert_eq!((src.scenes, src.pages, src.title.as_str()), (3, 2, "TOGO MAKUNDA WENI"));
        let adapted = "# «Того Макунда Вени» (ориг.: TOGO MAKUNDA WENI)\n**Длина:** ~2 мин · 2 сцены · ~5 страниц\n### Сцена 1 — ~1 мин\n### Сцена 2 — ~1 мин\n";
        let w = super::source_check(adapted, &src);
        assert_eq!(w.len(), 3, "{w:?}");
        let good = "# «Дорога домой» (ориг.: TOGO MAKUNDA WENI)\n**Длина:** ~2 мин · 3 сцены · ~2 страниц\n### Сцена 1\n### Сцена 2\n### Сцена 3\n";
        assert!(super::source_check(good, &src).is_empty(), "{:?}", super::source_check(good, &src));
        assert!(super::source_check("# «Дорога домой»\nпросто пересказ без сцен", &src).iter().any(|w| w.contains("нет разделов сцен")));
        // имена и брань
        let pages = vec!["INT. ROOM - DAY 1\nKOFI\nGet out, you bitch!\nAMARA (O.S.)\nNo.\nINT. HALL - NIGHT 2\nKOFI\nFuck.\nAMARA\nWhy?".to_string()];
        let src = super::source_of(&pages).unwrap();
        assert_eq!(src.names, vec!["KOFI".to_string(), "AMARA".to_string()]);
        assert_eq!(src.profanity, 2);
        let d = "КОФИ [зло]: «Пошла вон, сука!»\nАРАМА: «Нет.»\nКОФИ: «Блин.»";
        let w = super::names_and_profanity(d, &src, true);
        assert!(w.iter().any(|x| x.contains("АРАМА")), "{w:?}");
        assert!(w.iter().any(|x| x.contains("блин")), "{w:?}");
        assert!(super::names_and_profanity("КОФИ: «Сука.»\nАМАРА: «Нет.»", &src, true).is_empty());
        assert!(super::cut_sends("кофи: «иди ты!»"));
        assert_eq!(super::quoted_latin("КОФИ: «Get out!»\nАМАРА: «Нет.»\nКОФИ: «Why?»"), (2, 3));
        assert!(super::cut_sends("кофи: «может быть… иди ты»"));
        assert!(!super::cut_sends("кофи: «иди ты нахуй!»"));
        assert!(!super::cut_sends("кофи: «пошёл ты на хуй, сука!»"));
        // формат реплик
        assert!(super::bad_lines("КОФИ [зло]: «Пошла вон!»\nАМАРА (за кадром): «Нет.»").is_empty());
        assert_eq!(super::bad_lines("**КОФИ:** — Пошла вон!\n3. АМАРА: «Нет.»\nАМАРА — да").len(), 3);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thin() {
        let t = "# X\n## Сцены\n### Сцена 1 — А\n- Задача: x\n### Сцена 2 — Б\n- План и камера: x\n- Действие: y\n- Звук: z\n- Реплики: w\n";
        assert_eq!(super::thin_scenes(t), vec!["1".to_string()]);
    }
}

fn read(ed: &mut Editor, a: &Args) -> Res {
    let p = doc_path(ed, &a.need_str("name")?)?;
    let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Out::with(t, json!({"path": p})))
}

fn list(ed: &mut Editor, _a: &Args) -> Res {
    let v = list_docs(ed.path.as_deref());
    if v.is_empty() {
        return Ok(Out::text("Документов нет — doc.write".to_string()));
    }
    let names: Vec<String> = v.iter().filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
    Ok(Out::with(names.join("\n"), json!({"docs": v})))
}

#[cfg(test)]
mod real_check {
    /// `SYN_SRC=текст_источника.txt SYN_DOC=диалоги.md cargo test -p syn-video-editor real_check -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_check() {
        let src = std::fs::read_to_string(std::env::var("SYN_SRC").unwrap()).unwrap();
        let pages: Vec<String> = src.split("--- страница").map(str::to_string).collect();
        let s = super::source_of(&pages).unwrap();
        println!("источник: сцен {}, имена {:?}, брани {}", s.scenes, s.names, s.profanity);
        let d = std::fs::read_to_string(std::env::var("SYN_DOC").unwrap()).unwrap();
        for w in super::names_and_profanity(&d, &s, true) {
            println!("ЗАМЕЧАНИЕ: {w}");
        }
        let bad = super::bad_lines(&d);
        println!("строк не в формате: {} {:?}", bad.len(), bad.iter().take(2).collect::<Vec<_>>());
    }
}
