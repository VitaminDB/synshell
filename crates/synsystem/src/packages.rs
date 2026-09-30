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
            Source::Local => "локальный".into(),
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
        let url = format!("https://aur.archlinux.org/rpc/v5/search/{}?by=name-desc", urlencode(q));
        if let Some(j) = curl_json(&url) {
            let repo_names: BTreeSet<String> = out.iter().map(|p| p.name.clone()).collect();
            for v in j.get("results").and_then(|r| r.as_array()).into_iter().flatten() {
                if let Some(p) = aur_pkg(v, &installed) {
                    if !repo_names.contains(&p.name) {
                        out.push(p);
                    }
                }
            }
        }
    }
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
    out
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Все установленные с описаниями (`pacman -Qi`, один вызов).
pub fn installed() -> Vec<Pkg> {
    let foreign = foreign();
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
            source: if foreign.contains_key(&name) { Source::Local } else { Source::Repo(String::new()) },
            installed: Some(version.clone()),
            description: get("Description"),
            votes: None,
            popularity: None,
            out_of_date: false,
            version,
            name,
        });
    }
    out
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
            let mut fields = vec![("Версия".into(), p.version.clone())];
            if let Some(m) = s("Maintainer") {
                fields.push(("Сопровождающий".into(), m));
            }
            if let Some(n) = v.get("NumVotes").and_then(|x| x.as_u64()) {
                fields.push(("Голоса".into(), n.to_string()));
            }
            if let Some(l) = v.get("License").and_then(|x| x.as_array()) {
                fields.push(("Лицензия".into(), l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
            }
            Details { fields, depends: arr("Depends"), make_depends: arr("MakeDepends"), optional: arr("OptDepends"), url: s("URL"), files: Vec::new() }
        }
        _ => {
            let text = pacman(&["-Si", &p.name]).filter(|t| !t.trim().is_empty()).or_else(|| pacman(&["-Qi", &p.name])).unwrap_or_default();
            let f = parse_fields(&text);
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            let names = [
                ("Version", "Версия"),
                ("Repository", "Репозиторий"),
                ("Licenses", "Лицензия"),
                ("Download Size", "Загрузка"),
                ("Installed Size", "Размер"),
                ("Packager", "Сборщик"),
                ("Build Date", "Собран"),
                ("Install Date", "Установлен"),
                ("Required By", "Нужен для"),
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
}

/// Каталог AppStream: `/usr/share/swcatalog/xml/*.xml.gz` (и старый путь
/// `/usr/share/app-info/xmls`). Только пакеты, которые есть в синхронизированных
/// базах pacman этой архитектуры, — версия и репозиторий из `pacman -Sl`.
/// Пусто, если данных AppStream нет.
pub fn catalog() -> Vec<CatalogApp> {
    let installed = installed_map();
    let mut sync: HashMap<String, (String, String)> = HashMap::new();
    for l in pacman(&["-Sl"]).unwrap_or_default().lines() {
        let mut p = l.split_whitespace();
        if let (Some(repo), Some(name), Some(ver)) = (p.next(), p.next(), p.next()) {
            sync.entry(name.to_string()).or_insert((repo.to_string(), ver.to_string()));
        }
    }
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
        for c in parse_appstream(&text) {
            let Some((repo, ver)) = sync.get(&c.pkgname) else { continue };
            if let Some(&i) = index.get(&c.pkgname) {
                // Несколько программ в одном пакете — один пункт, категории вместе.
                for cat in c.categories {
                    if !out[i].categories.contains(&cat) {
                        out[i].categories.push(cat);
                    }
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
                    name: c.pkgname,
                },
                title: c.name,
                categories: c.categories,
                icon: c.icon,
            });
        }
    }
    out.sort_by_cached_key(|a| a.title.to_lowercase());
    out
}

/// Компонент AppStream (только нужные поля).
#[derive(Debug, Default, PartialEq)]
pub struct AppStreamComponent {
    pub pkgname: String,
    pub name: String,
    pub summary: String,
    pub categories: Vec<String>,
    pub icon: Option<String>,
}

/// Разбор XML AppStream: компоненты `desktop-application`/`console-application`
/// с `<pkgname>`. Название и описание — `xml:lang="ru"`, иначе без языка.
/// Свой маленький разборщик тегов: формат плоский, вложенные `<name>`
/// (`<developer>`) отсекаются по глубине.
pub fn parse_appstream(xml: &str) -> Vec<AppStreamComponent> {
    #[derive(Default)]
    struct Cur {
        c: AppStreamComponent,
        app: bool,
        name_ru: Option<String>,
        summary_ru: Option<String>,
    }
    let mut out = Vec::new();
    let mut cur: Option<Cur> = None;
    // Глубина внутри компонента (1 — прямые дети) и открытый интересный тег.
    let mut depth = 0usize;
    let mut open: Option<(String, Option<String>, Option<String>)> = None; // (тег, xml:lang, type)
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
                    if c.app && !comp.pkgname.is_empty() && !comp.name.is_empty() {
                        out.push(comp);
                    }
                }
                continue;
            }
            if let (Some(c), Some((t, lang, ty))) = (cur.as_mut(), open.take()) {
                let v = xml_unescape(text.trim());
                text.clear();
                match (t.as_str(), lang.as_deref()) {
                    ("pkgname", None) => c.c.pkgname = v,
                    ("name", None) => c.c.name = v,
                    ("name", Some("ru")) => c.name_ru = Some(v),
                    ("summary", None) => c.c.summary = v,
                    ("summary", Some("ru")) => c.summary_ru = Some(v),
                    ("category", _) if !v.is_empty() && !c.c.categories.contains(&v) => c.c.categories.push(v),
                    ("icon", _) if ty.as_deref() == Some("stock") && c.c.icon.is_none() => c.c.icon = Some(v),
                    _ => {}
                }
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
        if cur.is_none() || self_closing {
            continue;
        }
        depth += 1;
        // Прямые дети компонента и <category> внутри <categories>.
        let wanted = match name {
            "pkgname" | "name" | "summary" | "icon" => depth == 1,
            "category" => depth == 2,
            _ => false,
        };
        if wanted {
            open = Some((name.to_string(), xml_attr(attrs, "xml:lang"), xml_attr(attrs, "type")));
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
    pub aur: bool,
}

/// Обновления: `checkupdates` (своя копия базы, без root) или `pacman -Qu`
/// по имеющейся базе; AUR — версии RPC против установленных.
pub fn updates(aur: bool) -> Vec<Update> {
    let text = if which("checkupdates") {
        Command::new("checkupdates").env("LC_ALL", "C").stderr(Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        pacman(&["-Qu"])
    }
    .unwrap_or_default();
    let mut out: Vec<Update> = text
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 4 && p[2] == "->").then(|| Update { name: p[0].into(), old: p[1].into(), new: p[3].into(), aur: false })
        })
        .collect();
    if aur {
        let foreign = foreign();
        if !foreign.is_empty() {
            let args: String = foreign.keys().map(|n| format!("arg[]={}", urlencode(n))).collect::<Vec<_>>().join("&");
            if let Some(j) = curl_json(&format!("https://aur.archlinux.org/rpc/v5/info?{args}")) {
                for v in j.get("results").and_then(|r| r.as_array()).into_iter().flatten() {
                    let (Some(n), Some(new)) = (v.get("Name").and_then(|x| x.as_str()), v.get("Version").and_then(|x| x.as_str())) else { continue };
                    if let Some(old) = foreign.get(n) {
                        if vercmp(new, old) == std::cmp::Ordering::Greater {
                            out.push(Update { name: n.into(), old: old.clone(), new: new.into(), aur: true });
                        }
                    }
                }
            }
        }
    }
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

/// Что сделать.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Install(Vec<String>),
    InstallAur(Vec<String>),
    Remove(Vec<String>),
    /// Обновить систему (и, если `aur`, пакеты AUR).
    Upgrade { aur: bool },
    /// Обновить базы репозиториев.
    Refresh,
}

pub struct Job {
    pub rx: mpsc::Receiver<JobEvent>,
    pub cancel: JobCancel,
}

/// Текст ошибки отменённого задания.
pub const CANCELLED: &str = "отменено";

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
        let mut child = cmd.spawn().map_err(|e| format!("не запустить: {e}"))?;
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
        Err("права администратора не получены: окно пароля закрыто или пароль неверен".into())
    } else {
        Err(format!("завершилось с кодом {}", st.code().unwrap_or(-1)))
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
        let j = curl_json(&url).ok_or_else(|| "AUR недоступен (сеть?)".to_string())?;
        let Some(v) = j.get("results").and_then(|r| r.as_array()).and_then(|a| a.first()).cloned() else {
            return Err(format!("{name}: нет ни в репозиториях, ни в AUR"));
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
                let _ = tx.send(JobEvent::Line(format!("зависимость из AUR: {d}")));
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
    let _ = tx.send(JobEvent::Stage("Разбор зависимостей AUR".into()));
    let (order, repo_deps) = aur_plan(names, tx)?;
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if !repo_deps.is_empty() {
        let _ = tx.send(JobEvent::Stage(format!("Зависимости из репозиториев: {}", repo_deps.len())));
        let mut args = vec!["-S", "--needed", "--noconfirm", "--asdeps"];
        args.extend(repo_deps.iter().map(|s| s.as_str()));
        run_streaming(pacman_cmd(&args), tx, cancel)?;
    }
    let root = is_root();
    let user = if build_user.trim().is_empty() {
        if root {
            return Err("makepkg не работает от root — задайте [packages] build_user (обычный пользователь, например `useradd -m builder`)".into());
        }
        String::new()
    } else {
        build_user.trim().to_string()
    };
    let cache = aur_cache();
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let total = order.len();
    for (i, name) in order.iter().enumerate() {
        let _ = tx.send(JobEvent::Stage(format!("Сборка {name} ({}/{total})", i + 1)));
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
            return Err(format!("{name}: makepkg не создал пакет"));
        }
        let _ = tx.send(JobEvent::Stage(format!("Установка {name}")));
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

/// Запустить операцию в фоне.
pub fn start(op: Op, build_user: String) -> Job {
    let (tx, rx) = mpsc::channel();
    let cancel = JobCancel::default();
    let c = cancel.clone();
    std::thread::Builder::new()
        .name("synpkg-job".into())
        .spawn(move || {
            let r = match &op {
                Op::Install(p) => {
                    let _ = tx.send(JobEvent::Stage(format!("Установка: {}", p.join(", "))));
                    let mut a = vec!["-S", "--noconfirm", "--needed"];
                    a.extend(p.iter().map(|s| s.as_str()));
                    run_streaming(pacman_cmd(&a), &tx, &c)
                }
                Op::Remove(p) => {
                    let _ = tx.send(JobEvent::Stage(format!("Удаление: {}", p.join(", "))));
                    let mut a = vec!["-Rns", "--noconfirm"];
                    a.extend(p.iter().map(|s| s.as_str()));
                    run_streaming(pacman_cmd(&a), &tx, &c)
                }
                Op::InstallAur(p) => build_aur(p, &build_user, &tx, &c),
                Op::Refresh => {
                    let _ = tx.send(JobEvent::Stage("Обновление баз".into()));
                    run_streaming(pacman_cmd(&["-Sy"]), &tx, &c)
                }
                Op::Upgrade { aur } => {
                    let _ = tx.send(JobEvent::Stage("Обновление системы".into()));
                    let mut r = run_streaming(pacman_cmd(&["-Syu", "--noconfirm"]), &tx, &c);
                    if r.is_ok() && *aur {
                        let ups: Vec<String> = updates(true).into_iter().filter(|u| u.aur).map(|u| u.name).collect();
                        if !ups.is_empty() {
                            r = build_aur(&ups, &build_user, &tx, &c);
                        }
                    }
                    r
                }
            };
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
    fn fields() {
        let f = parse_fields("Name            : foo\nDepends On      : a  b>=2\nOptional Deps   : x: X\n                  y: Y\n");
        assert_eq!(f[0], ("Name".into(), "foo".into()));
        assert_eq!(list_field(&f[1].1), ["a", "b>=2"]);
        assert_eq!(f[2].1, "x: X\ny: Y");
        assert_eq!(urlencode("a b+c"), "a%20b%2Bc");
    }
}
