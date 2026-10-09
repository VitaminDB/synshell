//! Пакеты: pacman (репозитории, установленные, обновления) и AUR (RPC v5,
//! сборка makepkg). Для приложения установки программ `synpkg`.
//!
//! Чтение — `pacman` с `LC_ALL=C` и разбор вывода (без libalpm: одинаково на
//! десктопе и телефоне, без линковки). AUR — `curl` (уважает `http_proxy`,
//! как у телефона за прокси хоста), исходники — архив snapshot (git не
//! нужен). Изменения — задания ([`Job`]) с потоковым логом: pacman от root
//! напрямую, иначе через `pkexec`; makepkg никогда от root — от
//! `build_user`.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use crate::util::which;
use synshell_tr::{n_, t};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    /// Репозиторий pacman (`core`, `extra`, `alarm`…).
    Repo(String),
    Aur,
    /// Установлен, но ни в одном репозитории (собран сам).
    Local,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Repo(r) => r.clone(),
            Source::Aur => "AUR".into(),
            Source::Local => t!("локальный").into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: Source,
    /// Установленная версия.
    pub installed: Option<String>,
    /// AUR: голоса и популярность.
    pub votes: Option<u32>,
    pub popularity: Option<f64>,
    pub out_of_date: bool,
    /// Метапакет: своих файлов нет, только зависимости ([`is_meta`]).
    pub meta: bool,
}

/// Подробности о пакете.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Details {
    pub fields: Vec<(String, String)>,
    pub depends: Vec<String>,
    pub make_depends: Vec<String>,
    pub optional: Vec<String>,
    pub url: Option<String>,
    pub files: Vec<String>,
}

fn pacman(args: &[&str]) -> Option<String> {
    let out = Command::new("pacman").args(args).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn curl_json(url: &str) -> Option<serde_json::Value> {
    let out = Command::new("curl").args(["-sfL", "--max-time", "20", url]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Установленные: имя → версия.
pub fn installed_map() -> HashMap<String, String> {
    pacman(&["-Q"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, v)| (n.to_string(), v.to_string())))
        .collect()
}

/// Установленные не из репозиториев (AUR и собранные вручную).
pub fn foreign() -> HashMap<String, String> {
    pacman(&["-Qm"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, v)| (n.to_string(), v.to_string())))
        .collect()
}

/// Синхронизированные базы: имя → (репозиторий, версия) из `pacman -Sl`;
/// пакет в нескольких репозиториях — по первому (порядок pacman.conf, как у pacman).
pub fn sync_index() -> HashMap<String, (String, String)> {
    let mut sync = HashMap::new();
    for l in pacman(&["-Sl"]).unwrap_or_default().lines() {
        let mut p = l.split_whitespace();
        if let (Some(repo), Some(name), Some(ver)) = (p.next(), p.next(), p.next()) {
            sync.entry(name.to_string()).or_insert((repo.to_string(), ver.to_string()));
        }
    }
    sync
}

/// Разбор `pacman -Ss`: `repo/name version [installed]` + строка описания.
pub fn parse_ss(text: &str, installed: &HashMap<String, String>) -> Vec<Pkg> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(head) = lines.next() {
        if head.starts_with(' ') || head.trim().is_empty() {
            continue;
        }
        let mut parts = head.split_whitespace();
        let (Some(rn), Some(ver)) = (parts.next(), parts.next()) else { continue };
        let Some((repo, name)) = rn.split_once('/') else { continue };
        let desc = match lines.peek() {
            Some(l) if l.starts_with(' ') => lines.next().unwrap().trim().to_string(),
            _ => String::new(),
        };
        out.push(Pkg {
            name: name.to_string(),
            version: ver.to_string(),
            description: desc,
            source: Source::Repo(repo.to_string()),
            installed: installed.get(name).cloned(),
            votes: None,
            popularity: None,
            out_of_date: false,
            meta: false,
        });
    }
    out
}

fn aur_pkg(v: &serde_json::Value, installed: &HashMap<String, String>) -> Option<Pkg> {
    let name = v.get("Name")?.as_str()?.to_string();
    Some(Pkg {
        installed: installed.get(&name).cloned(),
        version: v.get("Version").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        description: v.get("Description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        source: Source::Aur,
        votes: v.get("NumVotes").and_then(|x| x.as_u64()).map(|x| x as u32),
        popularity: v.get("Popularity").and_then(|x| x.as_f64()),
        out_of_date: v.get("OutOfDate").is_some_and(|x| !x.is_null()),
        meta: false,
        name,
    })
}

/// Поиск: репозитории и (если `aur`) AUR. Сначала точные совпадения имени,
/// затем установленные, затем по популярности.
pub fn search(query: &str, aur: bool) -> Vec<Pkg> {
    let q = query.trim();
    if q.len() < 2 {
        return Vec::new();
    }
    let installed = installed_map();
    let mut out = parse_ss(&pacman(&["-Ss", q]).unwrap_or_default(), &installed);
    if aur {
        let repo_names: BTreeSet<String> = out.iter().map(|p| p.name.clone()).collect();
        out.extend(aur_query(q, &installed).into_iter().filter(|p| !repo_names.contains(&p.name)));
    }
    sort_found(&mut out, q);
    out
}

/// Поиск только в AUR (имя и описание), по той же сортировке.
pub fn search_aur(query: &str) -> Vec<Pkg> {
    let q = query.trim();
    if q.len() < 2 {
        return Vec::new();
    }
    let mut out = aur_query(q, &installed_map());
    sort_found(&mut out, q);
    out
}

fn aur_query(q: &str, installed: &HashMap<String, String>) -> Vec<Pkg> {
    let url = format!("https://aur.archlinux.org/rpc/v5/search/{}?by=name-desc", urlencode(q));
    let Some(j) = curl_json(&url) else { return Vec::new() };
    j.get("results").and_then(|r| r.as_array()).into_iter().flatten().filter_map(|v| aur_pkg(v, installed)).collect()
}

/// Сначала точные совпадения имени, затем начинающиеся с запроса, установленные, популярные.
fn sort_found(out: &mut Vec<Pkg>, q: &str) {
    let ql = q.to_lowercase();
    out.sort_by(|a, b| {
        let key = |p: &Pkg| {
            let exact = p.name == ql;
            let prefix = p.name.starts_with(&ql);
            (!exact, !prefix, p.installed.is_none(), std::cmp::Reverse((p.popularity.unwrap_or(0.0) * 1000.0) as i64))
        };
        key(a).cmp(&key(b)).then_with(|| a.name.cmp(&b.name))
    });
    out.truncate(200);
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Все установленные с описаниями (`pacman -Qi`, один вызов); репозиторий —
/// из [`sync_index`].
pub fn installed() -> Vec<Pkg> {
    let foreign = foreign();
    let sync = sync_index();
    let text = pacman(&["-Qi"]).unwrap_or_default();
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let f = parse_fields(block);
        let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let name = get("Name");
        if name.is_empty() {
            continue;
        }
        let version = get("Version");
        out.push(Pkg {
            source: if foreign.contains_key(&name) { Source::Local } else { Source::Repo(sync.get(&name).map(|(r, _)| r.clone()).unwrap_or_default()) },
            installed: Some(version.clone()),
            description: get("Description"),
            votes: None,
            popularity: None,
            out_of_date: false,
            meta: is_meta(&name, &get("Installed Size"), &get("Depends On")),
            version,
            name,
        });
    }
    out
}

/// Размер из `pacman -Qi/-Si` («27.26 KiB») в КиБ.
fn size_kib(s: &str) -> Option<f64> {
    let (n, unit) = s.trim().split_once(' ')?;
    let n: f64 = n.parse().ok()?;
    Some(match unit {
        "B" => n / 1024.0,
        "KiB" => n,
        "MiB" => n * 1024.0,
        "GiB" => n * 1024.0 * 1024.0,
        _ => return None,
    })
}

/// Метапакет: зависимости есть, своих файлов нет (0 байт); `*-meta` с парой
/// КиБ лицензий и документации — тоже. `size` и `depends` — поля `pacman -Qi/-Si`.
pub fn is_meta(name: &str, size: &str, depends: &str) -> bool {
    if depends.trim().is_empty() || depends.trim() == "None" {
        return false;
    }
    match size_kib(size) {
        Some(k) => k == 0.0 || (name.ends_with("-meta") && k < 64.0),
        None => false,
    }
}

/// Метапакеты синхронизированных баз (`pacman -Si`, один вызов): по имени,
/// пакет в нескольких репозиториях — по первому.
pub fn metapackages() -> Vec<Pkg> {
    let installed = installed_map();
    let text = pacman(&["-Si"]).unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let f = parse_fields(block);
        let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let name = get("Name");
        if name.is_empty() || !is_meta(&name, &get("Installed Size"), &get("Depends On")) || !seen.insert(name.clone()) {
            continue;
        }
        out.push(Pkg {
            installed: installed.get(&name).cloned(),
            version: get("Version"),
            description: get("Description"),
            source: Source::Repo(get("Repository")),
            votes: None,
            popularity: None,
            out_of_date: false,
            meta: true,
            name,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Группа пакетов pacman (`gnome`, `xfce4`…): общее имя набора пакетов.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub members: Vec<String>,
}

/// Группы синхронизированных баз (`pacman -Sgg`) по имени.
pub fn groups() -> Vec<Group> {
    parse_groups(&pacman(&["-Sgg"]).unwrap_or_default())
}

/// Разбор `pacman -Sgg`: строки «группа пакет».
pub fn parse_groups(text: &str) -> Vec<Group> {
    let mut map: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for l in text.lines() {
        if let Some((g, p)) = l.trim().split_once(' ') {
            let v = map.entry(g.to_string()).or_default();
            if !v.iter().any(|x| x == p.trim()) {
                v.push(p.trim().to_string());
            }
        }
    }
    map.into_iter().map(|(name, members)| Group { name, members }).collect()
}

/// «Ключ : значение» с продолжениями строк.
pub fn parse_fields(block: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        if line.starts_with(' ') {
            if let Some(last) = out.last_mut() {
                last.1.push('\n');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(" : ") {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn list_field(v: &str) -> Vec<String> {
    if v == "None" {
        return Vec::new();
    }
    v.split_whitespace().map(String::from).collect()
}

/// Подробности: `pacman -Si` (или `-Qi` для установленного без репозитория),
/// AUR — RPC info.
pub fn details(p: &Pkg) -> Details {
    match &p.source {
        Source::Aur => {
            let url = format!("https://aur.archlinux.org/rpc/v5/info?arg[]={}", urlencode(&p.name));
            let Some(j) = curl_json(&url) else { return Details::default() };
            let Some(v) = j.get("results").and_then(|r| r.as_array()).and_then(|a| a.first()) else { return Details::default() };
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
            let arr = |k: &str| v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let mut fields = vec![(t!("Версия").into(), p.version.clone())];
            if let Some(m) = s("Maintainer") {
                fields.push((t!("Сопровождающий").into(), m));
            }
            if let Some(n) = v.get("NumVotes").and_then(|x| x.as_u64()) {
                fields.push((t!("Голоса").into(), n.to_string()));
            }
            if let Some(l) = v.get("License").and_then(|x| x.as_array()) {
                fields.push((t!("Лицензия").into(), l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
            }
            Details { fields, depends: arr("Depends"), make_depends: arr("MakeDepends"), optional: arr("OptDepends"), url: s("URL"), files: Vec::new() }
        }
        _ => {
            let text = pacman(&["-Si", &p.name]).filter(|t| !t.trim().is_empty()).or_else(|| pacman(&["-Qi", &p.name])).unwrap_or_default();
            let f = parse_fields(&text);
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            let names = [
                ("Version", t!("Версия")),
                ("Repository", t!("Репозиторий")),
                ("Licenses", t!("Лицензия")),
                ("Download Size", t!("Загрузка")),
                ("Installed Size", t!("Размер")),
                ("Packager", t!("Сборщик")),
                ("Build Date", t!("Собран")),
                ("Install Date", t!("Установлен")),
                ("Required By", t!("Нужен для")),
            ];
            let fields = names.iter().filter_map(|(k, ru)| Some((ru.to_string(), get(k)).clone()).filter(|(_, v)| !v.is_empty() && v != "None")).collect();
            let files = if p.installed.is_some() {
                pacman(&["-Qlq", &p.name]).unwrap_or_default().lines().filter(|l| !l.ends_with('/')).map(String::from).collect()
            } else {
                Vec::new()
            };
            Details {
                fields,
                depends: list_field(&get("Depends On")),
                make_depends: Vec::new(),
                optional: get("Optional Deps").lines().filter(|l| l.trim() != "None").map(|l| l.trim().to_string()).collect(),
                url: Some(get("URL")).filter(|u| !u.is_empty() && u != "None"),
                files,
            }
        }
    }
}

// ─── Каталог по категориям ──────────────────────────────────────────────────

/// Программа каталога: пакет репозитория с названием и категориями из
/// AppStream (`archlinux-appstream-data`).
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogApp {
    pub pkg: Pkg,
    /// Название программы (по-русски, если есть перевод).
    pub title: String,
    /// Категории freedesktop (`AudioVideo`, `Game`…).
    pub categories: Vec<String>,
    /// Имя значка из темы (`<icon type="stock">`).
    pub icon: Option<String>,
    /// Идентификатор AppStream (`org.gimp.GIMP`).
    pub id: String,
    /// Абзацы полного описания.
    pub description: Vec<String>,
    /// Адреса снимков экрана.
    pub screenshots: Vec<String>,
    /// Значки из кэша каталога (JPEG XL): (ширина, путь).
    pub cached_icons: Vec<(u32, PathBuf)>,
}

impl CatalogApp {
    /// Кэшированный значок ближайшего к `size` размера (не меньше, если есть).
    pub fn cached_icon(&self, size: u32) -> Option<&PathBuf> {
        let mut v: Vec<&(u32, PathBuf)> = self.cached_icons.iter().collect();
        v.sort_by_key(|(w, _)| (*w < size, w.abs_diff(size)));
        v.first().map(|(_, p)| p)
    }
}

/// Каталог AppStream: `/usr/share/swcatalog/xml/*.xml.gz` (и старый путь
/// `/usr/share/app-info/xmls`). Только пакеты, которые есть в синхронизированных
/// базах pacman этой архитектуры, — версия и репозиторий из `pacman -Sl`.
/// Пусто, если данных AppStream нет.
pub fn catalog() -> Vec<CatalogApp> {
    let installed = installed_map();
    let sync = sync_index();
    let mut files: Vec<PathBuf> = ["/usr/share/swcatalog/xml", "/usr/share/app-info/xmls"]
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".xml.gz") || p.extension().is_some_and(|e| e == "xml"))
        .collect();
    files.sort();
    let mut out: Vec<CatalogApp> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for f in files {
        let Ok(raw) = std::fs::read(&f) else { continue };
        let text = if f.extension().is_some_and(|e| e == "gz") {
            let mut s = String::new();
            if std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&raw[..]), &mut s).is_err() {
                continue;
            }
            s
        } else {
            String::from_utf8_lossy(&raw).into_owned()
        };
        // Значки лежат в /usr/share/swcatalog/icons/archlinux-arch-<репозиторий>/<W>x<W>/.
        let stem = f.file_name().map(|n| n.to_string_lossy().split('.').next().unwrap_or("").to_string()).unwrap_or_default();
        let icon_dir = PathBuf::from(format!("/usr/share/swcatalog/icons/archlinux-arch-{stem}"));
        for c in parse_appstream(&text) {
            let Some((repo, ver)) = sync.get(&c.pkgname) else { continue };
            let cached_icons: Vec<(u32, PathBuf)> = c.cached_icons.iter().map(|(w, n)| (*w, icon_dir.join(format!("{w}x{w}")).join(n))).collect();
            if let Some(&i) = index.get(&c.pkgname) {
                // Несколько программ в одном пакете — один пункт, категории вместе.
                for cat in c.categories {
                    if !out[i].categories.contains(&cat) {
                        out[i].categories.push(cat);
                    }
                }
                if out[i].screenshots.is_empty() {
                    out[i].screenshots = c.screenshots;
                }
                continue;
            }
            index.insert(c.pkgname.clone(), out.len());
            out.push(CatalogApp {
                pkg: Pkg {
                    installed: installed.get(&c.pkgname).cloned(),
                    version: ver.clone(),
                    description: c.summary,
                    source: Source::Repo(repo.clone()),
                    votes: None,
                    popularity: None,
                    out_of_date: false,
                    meta: false,
                    name: c.pkgname,
                },
                title: c.name,
                categories: c.categories,
                icon: c.icon,
                id: c.id,
                description: c.description,
                screenshots: c.screenshots,
                cached_icons,
            });
        }
    }
    out.sort_by_cached_key(|a| a.title.to_lowercase());
    out
}

/// Компонент AppStream (только нужные поля).
#[derive(Debug, Default, PartialEq)]
pub struct AppStreamComponent {
    pub id: String,
    pub pkgname: String,
    pub name: String,
    pub summary: String,
    /// Абзацы описания (`<p>`, пункты `<li>` — с «• »).
    pub description: Vec<String>,
    pub categories: Vec<String>,
    pub icon: Option<String>,
    /// Значки из кэша каталога: (ширина, имя файла).
    pub cached_icons: Vec<(u32, String)>,
    /// Адреса снимков экрана (`<image type="source">`), основной — первым.
    pub screenshots: Vec<String>,
}

/// Разбор XML AppStream: компоненты `desktop-application`/`console-application`
/// с `<pkgname>`. Название, описание и сводка — `xml:lang="ru"`, иначе без языка.
/// Свой маленький разборщик тегов: формат плоский, вложенные `<name>`
/// (`<developer>`) отсекаются по глубине, разметка внутри абзацев (`<em>`,
/// `<code>`) — просто текст абзаца.
pub fn parse_appstream(xml: &str) -> Vec<AppStreamComponent> {
    #[derive(Default)]
    struct Cur {
        c: AppStreamComponent,
        app: bool,
        name_ru: Option<String>,
        summary_ru: Option<String>,
        desc_ru: Vec<String>,
        /// Язык открытого `<description>`: `Some(None)` — без языка, `Some(Some("ru"))`, иначе пропуск.
        desc_lang: Option<Option<String>>,
        /// Открытый `<screenshot>` — основной (`type="default"`).
        shot_default: bool,
    }
    let mut out = Vec::new();
    let mut cur: Option<Cur> = None;
    // Глубина внутри компонента (1 — прямые дети) и открытый интересный тег.
    let mut depth = 0usize;
    // (тег, глубина, xml:lang, type, width)
    let mut open: Option<(String, usize, Option<String>, Option<String>, Option<String>)> = None;
    let mut text = String::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if open.is_some() {
            text.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|i| &rest[i + 3..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if name == "component" {
                if let Some(c) = cur.take() {
                    let mut comp = c.c;
                    if let Some(n) = c.name_ru {
                        comp.name = n;
                    }
                    if let Some(s) = c.summary_ru {
                        comp.summary = s;
                    }
                    if !c.desc_ru.is_empty() {
                        comp.description = c.desc_ru;
                    }
                    if c.app && !comp.pkgname.is_empty() && !comp.name.is_empty() {
                        out.push(comp);
                    }
                }
                continue;
            }
            let Some(c) = cur.as_mut() else { continue };
            if open.as_ref().is_some_and(|o| o.0 == name && o.1 == depth) {
                let (t, _, lang, ty, width) = open.take().unwrap();
                let v = xml_unescape(&text.split_whitespace().collect::<Vec<_>>().join(" "));
                text.clear();
                match (t.as_str(), lang.as_deref()) {
                    ("id", _) => c.c.id = v,
                    ("pkgname", None) => c.c.pkgname = v,
                    ("name", None) => c.c.name = v,
                    ("name", Some("ru")) => c.name_ru = Some(v),
                    ("summary", None) => c.c.summary = v,
                    ("summary", Some("ru")) => c.summary_ru = Some(v),
                    ("p" | "li", _) if !v.is_empty() => {
                        let v = if t == "li" { format!("• {v}") } else { v };
                        match c.desc_lang.as_ref() {
                            Some(None) => c.c.description.push(v),
                            Some(Some(l)) if l == "ru" => c.desc_ru.push(v),
                            _ => {}
                        }
                    }
                    ("category", _) if !v.is_empty() && !c.c.categories.contains(&v) => c.c.categories.push(v),
                    ("icon", _) if ty.as_deref() == Some("stock") && c.c.icon.is_none() => c.c.icon = Some(v),
                    ("icon", _) if ty.as_deref() == Some("cached") && !v.is_empty() => {
                        c.c.cached_icons.push((width.and_then(|w| w.parse().ok()).unwrap_or(64), v));
                    }
                    ("image", _) if ty.as_deref() == Some("source") && v.starts_with("http") && !c.c.screenshots.contains(&v) => {
                        if c.shot_default {
                            c.c.screenshots.insert(0, v);
                        } else {
                            c.c.screenshots.push(v);
                        }
                    }
                    _ => {}
                }
            }
            if name == "description" && depth == 1 {
                c.desc_lang = None;
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        if name == "component" {
            let ty = xml_attr(attrs, "type").unwrap_or_default();
            cur = Some(Cur { app: ty == "desktop-application" || ty == "console-application" || ty == "desktop", ..Default::default() });
            depth = 0;
            open = None;
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        if self_closing {
            continue;
        }
        depth += 1;
        if open.is_some() {
            // Разметка внутри абзаца — текст копится дальше.
            continue;
        }
        match (name, depth) {
            ("description", 1) => {
                c.desc_lang = match xml_attr(attrs, "xml:lang") {
                    None => Some(None),
                    Some(l) if l == "ru" => Some(Some(l)),
                    Some(_) => Some(Some(String::new())),
                };
            }
            ("screenshot", 2) => c.shot_default = xml_attr(attrs, "type").as_deref() == Some("default"),
            _ => {}
        }
        // Прямые дети компонента, <category> внутри <categories>, абзацы описания, картинки снимков.
        let wanted = match name {
            "id" | "pkgname" | "name" | "summary" | "icon" => depth == 1,
            "category" => depth == 2,
            "p" => depth == 2 && c.desc_lang.is_some(),
            "li" => depth == 3 && c.desc_lang.is_some(),
            "image" => depth == 3,
            _ => false,
        };
        if wanted {
            open = Some((name.to_string(), depth, xml_attr(attrs, "xml:lang"), xml_attr(attrs, "type"), xml_attr(attrs, "width")));
            text.clear();
        }
    }
    out
}

fn xml_attr(attrs: &str, key: &str) -> Option<String> {
    let i = attrs.find(&format!("{key}="))?;
    let v = &attrs[i + key.len() + 1..];
    let q = v.chars().next()?;
    let v = &v[1..];
    Some(v[..v.find(q)?].to_string())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Доступное обновление.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub name: String,
    pub old: String,
    pub new: String,
    /// Откуда новая версия: репозиторий pacman или AUR.
    pub source: Source,
}

/// Обновления: `checkupdates` (своя копия базы, без root) или `pacman -Qu`
/// по имеющейся базе; репозиторий — из [`sync_index`]; AUR — версии RPC
/// против установленных.
pub fn updates(aur: bool) -> Vec<Update> {
    let text = if which("checkupdates") {
        Command::new("checkupdates").env("LC_ALL", "C").stderr(Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        pacman(&["-Qu"])
    }
    .unwrap_or_default();
    let sync = sync_index();
    let mut out: Vec<Update> = text
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 4 && p[2] == "->").then(|| Update { name: p[0].into(), old: p[1].into(), new: p[3].into(), source: Source::Repo(sync.get(p[0]).map(|(r, _)| r.clone()).unwrap_or_default()) })
        })
        .collect();
    if aur {
        out.extend(aur_updates());
    }
    out
}

/// Обновления пакетов AUR: версии RPC против установленных не из репозиториев
/// (собранные вручную и отсутствующие в AUR не попадают).
pub fn aur_updates() -> Vec<Update> {
    let foreign = foreign();
    let mut out = Vec::new();
    let names: Vec<&String> = foreign.keys().collect();
    // Запрос RPC — порциями: длина адреса ограничена.
    for chunk in names.chunks(150) {
        let args: String = chunk.iter().map(|n| format!("arg[]={}", urlencode(n))).collect::<Vec<_>>().join("&");
        let Some(j) = curl_json(&format!("https://aur.archlinux.org/rpc/v5/info?{args}")) else { continue };
        for v in j.get("results").and_then(|r| r.as_array()).into_iter().flatten() {
            let (Some(n), Some(new)) = (v.get("Name").and_then(|x| x.as_str()), v.get("Version").and_then(|x| x.as_str())) else { continue };
            if let Some(old) = foreign.get(n) {
                if vercmp(new, old) == std::cmp::Ordering::Greater {
                    out.push(Update { name: n.into(), old: old.clone(), new: new.into(), source: Source::Aur });
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Сравнение версий как у pacman (`vercmp`); без него — по строке.
pub fn vercmp(a: &str, b: &str) -> std::cmp::Ordering {
    if let Ok(o) = Command::new("vercmp").args([a, b]).output() {
        if let Ok(n) = String::from_utf8_lossy(&o.stdout).trim().parse::<i32>() {
            return n.cmp(&0);
        }
    }
    a.cmp(b)
}

// ─── Задания ────────────────────────────────────────────────────────────────

/// Строка лога или конец задания.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Line(String),
    /// Шаг задания («Сборка foo 2/3»).
    Stage(String),
    Done(Result<(), String>),
}

/// Как удалять пакеты, нужные другим.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoveMode {
    /// `-Rns`: с ненужными больше зависимостями; пакет, нужный другим, не удаляется.
    #[default]
    Normal,
    /// `-Rcns`: вместе со всеми, кто от него зависит.
    Cascade,
    /// `-Rdd`: без проверки зависимостей (зависящие останутся и могут не запуститься).
    Force,
}

/// Что сделать.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Install(Vec<String>),
    InstallAur(Vec<String>),
    Remove { names: Vec<String>, mode: RemoveMode },
    /// Обновить систему (и, если `aur`, пакеты AUR); `ignore` — пропустить эти пакеты,
    /// `install` — доустановить в той же транзакции (`pacman -Syu a b`).
    Upgrade { aur: bool, ignore: Vec<String>, install: Vec<String> },
    /// Обновить базы репозиториев.
    Refresh,
    /// Несколько операций по очереди (очередь synpkg); первая ошибка останавливает.
    Batch(Vec<Op>),
}

/// Что выйдет из удаления `names` (проверка без root: `pacman -Rp`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoveCheck {
    /// Обычное удаление: пакеты (вместе с ненужными больше зависимостями).
    pub removed: Vec<String>,
    /// Мешающие зависимости: (удаляемый, кому он нужен).
    pub blockers: Vec<(String, String)>,
    /// Каскадное удаление: всё, что уйдёт вместе с зависящими.
    pub cascade: Vec<String>,
}

/// Проверить удаление: кому нужны пакеты и что уйдёт каскадом.
pub fn remove_check(names: &[String]) -> RemoveCheck {
    let run = |flags: &str| -> (bool, String, String) {
        let mut c = Command::new("pacman");
        c.arg(flags).args(["--print-format", "%n"]).args(names).env("LC_ALL", "C").stdin(Stdio::null());
        match c.output() {
            Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned()),
            Err(_) => (false, String::new(), String::new()),
        }
    };
    let list = |t: &str| t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains(' ')).map(String::from).collect::<Vec<_>>();
    let (ok, out, err) = run("-Rsp");
    let mut check = RemoveCheck::default();
    if ok {
        check.removed = list(&out);
        return check;
    }
    check.blockers = parse_breaks(&format!("{out}\n{err}"));
    let (ok, out, _) = run("-Rcsp");
    if ok {
        check.cascade = list(&out);
    }
    check
}

/// Строки pacman «:: removing aha breaks dependency 'aha' required by kinfocenter».
pub fn parse_breaks(text: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for l in text.lines() {
        let Some(rest) = l.trim().trim_start_matches(":: ").strip_prefix("removing ") else { continue };
        let Some((pkg, tail)) = rest.split_once(" breaks dependency ") else { continue };
        let Some((_, by)) = tail.rsplit_once(" required by ") else { continue };
        let e = (pkg.trim().to_string(), by.trim().to_string());
        if !v.contains(&e) {
            v.push(e);
        }
    }
    v
}

pub struct Job {
    pub rx: mpsc::Receiver<JobEvent>,
    pub cancel: JobCancel,
}

/// Текст ошибки отменённого задания.
pub const CANCELLED: &str = n_!("отменено");

/// Путь помощника pacman с отменой (`crates/synpkg/data/pacman-helper`); его же
/// называет правило polkit `org.synshell.synpkg.pacman`.
pub const PACMAN_HELPER: &str = "/usr/lib/synpkg/pacman-helper";

/// Отмена задания: текущему процессу — SIGINT (pacman прерывает транзакцию в
/// безопасной точке и снимает блокировку базы, makepkg и curl просто выходят),
/// следующие шаги не начинаются. pacman от root через pkexec пользователю
/// сигналом не достать — ему «cancel» передаёт [`PACMAN_HELPER`].
#[derive(Clone, Default)]
pub struct JobCancel(Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    current: Mutex<Option<Running>>,
}

struct Running {
    pid: u32,
    /// stdin помощника pacman: команда отмены.
    helper: Option<std::process::ChildStdin>,
}

impl PartialEq for JobCancel {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl JobCancel {
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let mut cur = self.0.current.lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = cur.as_mut() else { return };
        if let Some(w) = r.helper.as_mut() {
            use std::io::Write;
            let _ = writeln!(w, "cancel");
            let _ = w.flush();
            return;
        }
        let pid = r.pid as i32;
        // Своя группа процессов (process_group(0) при запуске) — сигнал и детям (makepkg → gcc…).
        // SAFETY: kill без побочных эффектов в этом процессе.
        unsafe { libc::kill(-pid, libc::SIGINT) };
        // Не вышел за 5 с (или сигнал игнорирует) — SIGTERM группе.
        let st = self.0.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let still = st.current.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|r| r.pid as i32 == pid);
            if still {
                // SAFETY: как выше.
                unsafe { libc::kill(-pid, libc::SIGTERM) };
            }
        });
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }
}

fn is_root() -> bool {
    // SAFETY: geteuid без побочных эффектов.
    unsafe { libc::geteuid() == 0 }
}

/// pacman с правами root: напрямую или через pkexec (пароль спросит агент
/// polkit приложения — [`crate::polkit_agent`], если приложение задало окно).
/// Через pkexec — помощником [`PACMAN_HELPER`], если он есть (отмена задания),
/// иначе самим pacman.
fn pacman_cmd(args: &[&str]) -> Command {
    let mut c = if is_root() {
        Command::new("pacman")
    } else {
        if let Err(e) = crate::polkit_agent::ensure() {
            tracing::warn!("{e}");
        }
        let mut c = Command::new("pkexec");
        c.arg(if std::path::Path::new(PACMAN_HELPER).exists() { PACMAN_HELPER } else { "pacman" });
        c
    };
    c.args(args).env("LC_ALL", "C");
    c
}

/// Запустить команду, передавая вывод построчно; `Ok` — код 0.
/// Отменённое задание новых команд не запускает.
fn run_streaming(mut cmd: Command, tx: &mpsc::Sender<JobEvent>, cancel: &JobCancel) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let _ = tx.send(JobEvent::Line(format!("$ {:?}", cmd)));
    let helper = cmd.get_args().any(|a| a == PACMAN_HELPER);
    let via_pkexec = cmd.get_program() == "pkexec";
    cmd.stdin(if helper { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.process_group(0);
    let mut child = {
        // Запуск и запись под замком: отмена между ними не потеряется.
        let mut cur = cancel.0.current.lock().unwrap_or_else(|e| e.into_inner());
        if cancel.is_cancelled() {
            return Err(CANCELLED.into());
        }
        let mut child = cmd.spawn().map_err(|e| t!("не запустить: {e}", e = e))?;
        *cur = Some(Running { pid: child.id(), helper: child.stdin.take() });
        child
    };
    let err = child.stderr.take();
    let tx2 = tx.clone();
    let t = std::thread::spawn(move || {
        if let Some(e) = err {
            for l in BufReader::new(e).lines().map_while(Result::ok) {
                let _ = tx2.send(JobEvent::Line(l));
            }
        }
    });
    if let Some(o) = child.stdout.take() {
        for l in BufReader::new(o).lines().map_while(Result::ok) {
            let _ = tx.send(JobEvent::Line(l));
        }
    }
    let _ = t.join();
    let st = child.wait().map_err(|e| e.to_string());
    *cancel.0.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let st = st?;
    // Отмена не успела (pacman без помощника от root сигналу недоступен) — задание всё же выполнено.
    if cancel.is_cancelled() && !st.success() {
        return Err(CANCELLED.into());
    }
    if st.success() {
        Ok(())
    } else if via_pkexec && matches!(st.code(), Some(126 | 127)) {
        // pkexec: 126 — окно пароля закрыто, 127 — не разрешено (отмена в окне агента или неверный пароль).
        Err(t!("права администратора не получены: окно пароля закрыто или пароль неверен").into())
    } else {
        Err(t!("завершилось с кодом {v}", v = st.code().unwrap_or(-1)))
    }
}

/// Каталог сборки AUR.
pub fn aur_cache() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".cache")
    });
    base.join("synpkg/aur")
}

/// Пакеты AUR с зависимостями из AUR — в порядке сборки (зависимости
/// раньше); и зависимости из репозиториев, которые нужно поставить до сборки.
fn aur_plan(names: &[String], tx: &mpsc::Sender<JobEvent>) -> Result<(Vec<String>, Vec<String>), String> {
    let installed = installed_map();
    let mut order: Vec<String> = Vec::new();
    let mut repo_deps: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    fn strip(d: &str) -> String {
        d.split(['<', '>', '=']).next().unwrap_or(d).to_string()
    }
    fn visit(
        name: &str,
        installed: &HashMap<String, String>,
        order: &mut Vec<String>,
        repo_deps: &mut BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        tx: &mpsc::Sender<JobEvent>,
        depth: u32,
    ) -> Result<(), String> {
        if !seen.insert(name.to_string()) || depth > 12 {
            return Ok(());
        }
        let url = format!("https://aur.archlinux.org/rpc/v5/info?arg[]={}", urlencode(name));
        let j = curl_json(&url).ok_or_else(|| t!("AUR недоступен (сеть?)").to_string())?;
        let Some(v) = j.get("results").and_then(|r| r.as_array()).and_then(|a| a.first()).cloned() else {
            return Err(t!("{name}: нет ни в репозиториях, ни в AUR", name = name));
        };
        let deps: Vec<String> = ["Depends", "MakeDepends", "CheckDepends"]
            .iter()
            .flat_map(|k| v.get(*k).and_then(|x| x.as_array()).cloned().unwrap_or_default())
            .filter_map(|x| x.as_str().map(strip))
            .collect();
        for d in deps {
            if installed.contains_key(&d) || repo_deps.contains(&d) {
                continue;
            }
            // Есть в репозиториях (или его «предоставляет» пакет репозитория)?
            let in_repo = pacman(&["-Sp", "--print-format", "%n", &d]).is_some_and(|s| !s.trim().is_empty());
            if in_repo {
                repo_deps.insert(d);
            } else {
                let _ = tx.send(JobEvent::Line(t!("зависимость из AUR: {d}", d = d)));
                visit(&d, installed, order, repo_deps, seen, tx, depth + 1)?;
            }
        }
        order.push(name.to_string());
        Ok(())
    }
    for n in names {
        visit(n, &installed, &mut order, &mut repo_deps, &mut seen, tx, 0)?;
    }
    Ok((order, repo_deps.into_iter().collect()))
}

/// Собрать и установить пакеты AUR.
fn build_aur(names: &[String], build_user: &str, tx: &mpsc::Sender<JobEvent>, cancel: &JobCancel) -> Result<(), String> {
    let _ = tx.send(JobEvent::Stage(t!("Разбор зависимостей AUR").into()));
    let (order, repo_deps) = aur_plan(names, tx)?;
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if !repo_deps.is_empty() {
        let _ = tx.send(JobEvent::Stage(t!("Зависимости из репозиториев: {n}", n = repo_deps.len())));
        let mut args = vec!["-S", "--needed", "--noconfirm", "--asdeps"];
        args.extend(repo_deps.iter().map(|s| s.as_str()));
        run_streaming(pacman_cmd(&args), tx, cancel)?;
    }
    let root = is_root();
    let user = if build_user.trim().is_empty() {
        if root {
            return Err(t!("makepkg не работает от root — задайте [packages] build_user (обычный пользователь, например `useradd -m builder`)").into());
        }
        String::new()
    } else {
        build_user.trim().to_string()
    };
    let cache = aur_cache();
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let total = order.len();
    for (i, name) in order.iter().enumerate() {
        let _ = tx.send(JobEvent::Stage(t!("Сборка {name} ({v}/{total})", name = name, v = i + 1, total = total)));
        let dir = cache.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        let tarball = cache.join(format!("{name}.tar.gz"));
        let url = format!("https://aur.archlinux.org/cgit/aur.git/snapshot/{name}.tar.gz");
        let mut dl = Command::new("curl");
        dl.args(["-sfL", "--max-time", "120", "-o"]).arg(&tarball).arg(&url);
        run_streaming(dl, tx, cancel)?;
        let mut untar = Command::new("tar");
        untar.arg("-xzf").arg(&tarball).arg("-C").arg(&cache);
        run_streaming(untar, tx, cancel)?;
        let _ = std::fs::remove_file(&tarball);
        let mut mk = if root {
            // Каталог сборки — сборщику.
            let _ = Command::new("chown").args(["-R", &format!("{user}:"), &dir.to_string_lossy()]).status();
            let mut c = Command::new("runuser");
            c.args(["-u", &user, "--", "makepkg", "--noconfirm", "-f"]);
            c
        } else {
            let mut c = Command::new("makepkg");
            c.args(["--noconfirm", "-f"]);
            c
        };
        mk.current_dir(&dir).env("PKGDEST", &dir);
        run_streaming(mk, tx, cancel)?;
        let built: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".pkg.tar"))
            .filter(|p| !p.to_string_lossy().ends_with(".sig"))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if built.is_empty() {
            return Err(t!("{name}: makepkg не создал пакет", name = name));
        }
        let _ = tx.send(JobEvent::Stage(t!("Установка {name}", name = name)));
        let mut args: Vec<&str> = vec!["-U", "--noconfirm", "--needed"];
        // Зависимости AUR — как зависимости, запрошенные — как явные.
        if !names.contains(name) {
            args.push("--asdeps");
        }
        args.extend(built.iter().map(|s| s.as_str()));
        run_streaming(pacman_cmd(&args), tx, cancel)?;
    }
    Ok(())
}

fn run_op(op: &Op, build_user: &str, tx: &mpsc::Sender<JobEvent>, c: &JobCancel) -> Result<(), String> {
    match op {
        Op::Install(p) => {
            let _ = tx.send(JobEvent::Stage(t!("Установка: {v}", v = p.join(", "))));
            let mut a = vec!["-S", "--noconfirm", "--needed"];
            a.extend(p.iter().map(|s| s.as_str()));
            run_streaming(pacman_cmd(&a), tx, c)
        }
        Op::Remove { names, mode } => {
            let _ = tx.send(JobEvent::Stage(t!("Удаление: {v}", v = names.join(", "))));
            let mut a = vec![
                match mode {
                    RemoveMode::Normal => "-Rns",
                    RemoveMode::Cascade => "-Rcns",
                    RemoveMode::Force => "-Rdd",
                },
                "--noconfirm",
            ];
            a.extend(names.iter().map(|s| s.as_str()));
            run_streaming(pacman_cmd(&a), tx, c)
        }
        Op::InstallAur(p) => build_aur(p, build_user, tx, c),
        Op::Refresh => {
            let _ = tx.send(JobEvent::Stage(t!("Обновление баз").into()));
            run_streaming(pacman_cmd(&["-Sy"]), tx, c)
        }
        Op::Upgrade { aur, ignore, install } => {
            let _ = tx.send(JobEvent::Stage(t!("Обновление системы").into()));
            let list = ignore.join(",");
            let mut a = vec!["-Syu", "--noconfirm", "--needed"];
            if !ignore.is_empty() {
                a.extend(["--ignore", list.as_str()]);
            }
            a.extend(install.iter().map(|s| s.as_str()));
            let mut r = run_streaming(pacman_cmd(&a), tx, c);
            if r.is_ok() && *aur {
                let ups: Vec<String> = updates(true).into_iter().filter(|u| u.source == Source::Aur && !ignore.contains(&u.name)).map(|u| u.name).collect();
                if !ups.is_empty() {
                    r = build_aur(&ups, build_user, tx, c);
                }
            }
            r
        }
        Op::Batch(ops) => {
            for (i, o) in ops.iter().enumerate() {
                let _ = tx.send(JobEvent::Line(t!("— шаг {v} из {n} —", v = i + 1, n = ops.len())));
                run_op(o, build_user, tx, c)?;
            }
            Ok(())
        }
    }
}

/// Запустить операцию в фоне.
pub fn start(op: Op, build_user: String) -> Job {
    let (tx, rx) = mpsc::channel();
    let cancel = JobCancel::default();
    let c = cancel.clone();
    std::thread::Builder::new()
        .name("synpkg-job".into())
        .spawn(move || {
            let r = run_op(&op, &build_user, &tx, &c);
            let _ = tx.send(JobEvent::Done(r));
        })
        .ok();
    Job { rx, cancel }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_appstream_components() {
        let xml = r#"<?xml version="1.0"?>
<components>
<component type="desktop-application">
  <id>a.desktop</id>
  <name>Acme</name>
  <name xml:lang="ru">Акме</name>
  <summary>Editor &amp; more</summary>
  <developer id="x"><name>Someone</name></developer>
  <pkgname>plan9port</pkgname>
  <icon type="cached" width="64" height="64">p.jxl</icon>
  <icon type="stock">acme</icon>
  <categories>
    <category>Development</category>
    <category>TextEditor</category>
  </categories>
  <description>
    <p>Acme is <em>an</em>
      editor.</p>
    <ul><li>Fast</li></ul>
  </description>
  <description xml:lang="de"><p>Editor</p></description>
  <screenshots>
    <screenshot><image type="source">https://x/2.png</image></screenshot>
    <screenshot type="default"><caption>Main</caption><image type="thumbnail">https://x/t.png</image><image type="source">https://x/1.png</image></screenshot>
  </screenshots>
</component>
<component type="font">
  <name>Font</name>
  <pkgname>ttf-x</pkgname>
</component>
</components>"#;
        let v = parse_appstream(xml);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].pkgname, "plan9port");
        assert_eq!(v[0].name, "Акме");
        assert_eq!(v[0].summary, "Editor & more");
        assert_eq!(v[0].icon.as_deref(), Some("acme"));
        assert_eq!(v[0].categories, vec!["Development", "TextEditor"]);
        assert_eq!(v[0].id, "a.desktop");
        assert_eq!(v[0].description, vec!["Acme is an editor.", "• Fast"]);
        assert_eq!(v[0].screenshots, vec!["https://x/1.png", "https://x/2.png"]);
        assert_eq!(v[0].cached_icons, vec![(64, "p.jxl".to_string())]);
    }

    #[test]
    fn cancel_interrupts_running_command() {
        let (tx, rx) = mpsc::channel();
        let cancel = JobCancel::default();
        let c = cancel.clone();
        let t = std::thread::spawn(move || {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo start; sleep 30; echo never"]);
            run_streaming(cmd, &tx, &c)
        });
        // Дождаться запуска.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !matches!(rx.recv_timeout(std::time::Duration::from_secs(1)), Ok(JobEvent::Line(l)) if l == "start") {
            assert!(std::time::Instant::now() < deadline, "команда не запустилась");
        }
        let t0 = std::time::Instant::now();
        cancel.cancel();
        assert_eq!(t.join().unwrap(), Err(CANCELLED.to_string()));
        assert!(t0.elapsed() < std::time::Duration::from_secs(4), "sleep прерван SIGINT группе");
        // Следующие шаги не запускаются.
        let (tx2, _rx2) = mpsc::channel();
        assert_eq!(run_streaming(Command::new("true"), &tx2, &cancel), Err(CANCELLED.to_string()));
    }

    #[test]
    fn parse_search() {
        let mut inst = HashMap::new();
        inst.insert("firefox".to_string(), "156.0-1".to_string());
        let v = parse_ss("extra/firefox 156.0-1 [installed]\n    Fast, Private & Safe Web Browser\nextra/firefox-i18n-ru 156.0-1\n    Russian language pack\n", &inst);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].source, Source::Repo("extra".into()));
        assert_eq!(v[0].installed.as_deref(), Some("156.0-1"));
        assert_eq!(v[1].description, "Russian language pack");
    }

    #[test]
    fn breaks() {
        let v = parse_breaks("error: failed to prepare transaction (could not satisfy dependencies)\n:: removing aha breaks dependency 'aha' required by kinfocenter\n:: removing aha breaks dependency 'aha' required by plasma-disks\n");
        assert_eq!(v, vec![("aha".to_string(), "kinfocenter".to_string()), ("aha".to_string(), "plasma-disks".to_string())]);
    }

    #[test]
    fn fields() {
        let f = parse_fields("Name            : foo\nDepends On      : a  b>=2\nOptional Deps   : x: X\n                  y: Y\n");
        assert_eq!(f[0], ("Name".into(), "foo".into()));
        assert_eq!(list_field(&f[1].1), ["a", "b>=2"]);
        assert_eq!(f[2].1, "x: X\ny: Y");
        assert_eq!(urlencode("a b+c"), "a%20b%2Bc");
    }

    #[test]
    fn metas_and_groups() {
        assert!(is_meta("base", "0.00 KiB", "filesystem  glibc"));
        assert!(is_meta("multilib-devel", "0.00 B", "gcc-multilib"));
        assert!(is_meta("kde-development-environment-meta", "27.26 KiB", "kdevelop"));
        assert!(!is_meta("haskell-src-meta", "576.09 KiB", "ghc-libs"));
        assert!(!is_meta("empty", "0.00 B", "None"));
        assert!(!is_meta("lib", "12.00 MiB", "glibc"));
        let g = parse_groups("gnome gdm\ngnome nautilus\nxfce4 thunar\ngnome gdm\n");
        assert_eq!(g, vec![Group { name: "gnome".into(), members: vec!["gdm".into(), "nautilus".into()] }, Group { name: "xfce4".into(), members: vec!["thunar".into()] }]);
    }
}
