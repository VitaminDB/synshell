//! База ИК-кодов: Flipper-IRDB (github.com/Lucaslhm/Flipper-IRDB, «Тип/Марка/модель.ir») и универсальные
//! библиотеки прошивки Flipper Zero (tv, ac, audio, projector — коды питания и громкости по популярности).
//! Скачивается архивом с GitHub в `~/.local/share/synshell/irdb`; каталог раскладывается в указатель
//! при старте, файлы пультов читаются по требованию.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::icons;
use crate::ir::{self, Signal};
use crate::keys;

const ARCHIVE: &str = "https://codeload.github.com/Lucaslhm/Flipper-IRDB/zip/refs/heads/main";
const UNIVERSAL: &str = "https://raw.githubusercontent.com/flipperdevices/flipperzero-firmware/dev/applications/main/infrared/resources/infrared/assets";
const UNIVERSAL_DIR: &str = "_universal";
const META: &str = "irdb.json";
/// Архив Flipper-IRDB: пульты, переведённые из других баз, разложены только по маркам.
pub const ARCHIVE_DIR: &str = "_Converted_";

/// Каталог базы.
pub fn root() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/share"));
    base.join("synshell/irdb")
}

/// Сведения о скачанной базе.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// Когда скачана, секунды от эпохи.
    pub updated: i64,
    pub remotes: usize,
    pub source: String,
}

/// Пульт из базы: путь от корня базы и модель (имя файла).
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Brand {
    pub name: String,
    pub models: Vec<Model>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    /// Каталог в базе («TVs», «ACs»…).
    pub dir: String,
    pub brands: Vec<Brand>,
}

impl Category {
    pub fn count(&self) -> usize {
        self.brands.iter().map(|b| b.models.len()).sum()
    }
    pub fn brand(&self, name: &str) -> Option<&Brand> {
        self.brands.iter().find(|b| b.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Db {
    pub meta: Meta,
    pub categories: Vec<Category>,
}

impl Db {
    pub fn category(&self, dir: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.dir == dir)
    }
    pub fn count(&self) -> usize {
        self.categories.iter().map(|c| c.count()).sum()
    }
}

/// Тип техники: название, значок, место в списке (меньше — выше; 100 — «прочее»).
pub fn category_info(dir: &str) -> (String, &'static str, u32) {
    use syngui::t;
    match dir {
        "TVs" => (t!("Телевизоры"), icons::TV, 0),
        "ACs" => (t!("Кондиционеры"), icons::AC, 1),
        "Projectors" => (t!("Проекторы"), icons::PROJECTOR, 2),
        "Audio_and_Video_Receivers" => (t!("Ресиверы и усилители"), icons::SPEAKER_GROUP, 3),
        "SoundBars" => (t!("Саундбары"), icons::SPEAKER, 4),
        "Speakers" => (t!("Колонки"), icons::SPEAKER, 5),
        "Streaming_Devices" => (t!("ТВ-приставки"), icons::CAST, 6),
        "Cable_Boxes" | "DVB-T" | "TV_Tuner" => (t!("Цифровое ТВ и тюнеры"), icons::LIVE_TV, 7),
        "Fans" => (t!("Вентиляторы"), icons::FAN, 8),
        "LED_Lighting" => (t!("Освещение"), icons::LIGHT, 9),
        "Air_Purifiers" => (t!("Очистители воздуха"), icons::AIR, 10),
        "Heaters" | "Fireplaces" => (t!("Обогреватели и камины"), icons::HEAT, 11),
        "Humidifiers" => (t!("Увлажнители"), icons::WATER, 12),
        "Vacuum_Cleaners" | "Window_cleaners" => (t!("Пылесосы"), icons::CLEAN, 13),
        "Monitors" => (t!("Мониторы"), icons::MONITOR, 14),
        "DVD_Players" | "Blu-Ray" | "VCR" | "Laserdisc" | "CD_Players" | "MiniDisc" => (t!("Плееры"), icons::ALBUM, 15),
        "Consoles" => (t!("Приставки"), icons::GAMEPAD, 16),
        "Cameras" | "CCTV" => (t!("Камеры"), icons::CAMERA, 17),
        "Car_Multimedia" | "Head_Units" => (t!("Автомагнитолы"), icons::MUSIC, 18),
        "Universal_TV_Remotes" => (t!("Универсальные ТВ-пульты"), icons::REMOTE, 19),
        ARCHIVE_DIR => (t!("Архив по маркам"), icons::STORAGE, 200),
        _ => (dir.replace('_', " "), icons::DEVICES, 100),
    }
}

/// Каталоги базы, которые показываются одним типом техники: «DVB-T» и «TV_Tuner» — вместе с
/// «Cable_Boxes» и т. п.
fn group_of(dir: &str) -> &str {
    match dir {
        "DVB-T" | "TV_Tuner" => "Cable_Boxes",
        "Fireplaces" => "Heaters",
        "Window_cleaners" => "Vacuum_Cleaners",
        "Blu-Ray" | "VCR" | "Laserdisc" | "CD_Players" | "MiniDisc" => "DVD_Players",
        "CCTV" => "Cameras",
        "Head_Units" => "Car_Multimedia",
        _ => dir,
    }
}

/// Универсальная библиотека Flipper для типа техники.
fn universal_for(dir: &str) -> Option<&'static str> {
    match dir {
        "TVs" | "Monitors" | "Universal_TV_Remotes" => Some("tv"),
        "ACs" => Some("ac"),
        "Audio_and_Video_Receivers" | "SoundBars" | "Speakers" => Some("audio"),
        "Projectors" => Some("projector"),
        _ => None,
    }
}

/// Указатель скачанной базы; `None` — базы нет.
pub fn load() -> Option<Db> {
    let root = root();
    let meta: Meta = std::fs::read_to_string(root.join(META)).ok().and_then(|s| serde_json::from_str(&s).ok())?;
    let mut categories = Vec::new();
    for cat in sorted_dirs(&root) {
        let dir = file_name(&cat);
        if dir == ARCHIVE_DIR {
            if let Some(c) = load_archive(&root, &cat) {
                categories.push(c);
            }
            continue;
        }
        if dir.starts_with('_') || dir.starts_with('.') {
            continue;
        }
        let mut brands: Vec<Brand> = Vec::new();
        let mut loose = Vec::new();
        for e in sorted_entries(&cat) {
            if e.is_dir() {
                let mut models = Vec::new();
                collect(&root, &e, &mut models);
                if !models.is_empty() {
                    brands.push(Brand { name: file_name(&e).replace('_', " "), models });
                }
            } else if is_ir(&e) {
                loose.push(model(&root, &e));
            }
        }
        if !loose.is_empty() {
            brands.push(Brand { name: syngui::t!("Разные"), models: loose });
        }
        let group = group_of(&dir).to_string();
        match categories.iter_mut().find(|c: &&mut Category| c.dir == group) {
            Some(c) => {
                for b in brands {
                    match c.brands.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&b.name)) {
                        Some(x) => x.models.extend(b.models),
                        None => c.brands.push(b),
                    }
                }
            }
            None if !brands.is_empty() => categories.push(Category { dir: group, brands }),
            None => {}
        }
    }
    for c in &mut categories {
        c.brands.sort_by_key(|b| b.name.to_lowercase());
    }
    categories.sort_by_key(|c| {
        let (name, _, order) = category_info(&c.dir);
        (order, name)
    });
    Some(Db { meta, categories })
}

/// Архив Flipper-IRDB («_Converted_/CSV|Pronto|IR_Plus/Буква/Марка/…»): пульты из других баз,
/// разложенные только по маркам. Марки из разных форматов сливаются.
fn load_archive(root: &Path, dir: &Path) -> Option<Category> {
    let mut brands: HashMap<String, Vec<Model>> = HashMap::new();
    for format in sorted_dirs(dir) {
        let fname = file_name(&format).replace('_', " ");
        for letter in sorted_dirs(&format) {
            for brand in sorted_dirs(&letter) {
                let mut models = Vec::new();
                collect(root, &brand, &mut models);
                for m in &mut models {
                    m.name = format!("{} · {fname}", m.name);
                }
                brands.entry(file_name(&brand).replace('_', " ")).or_default().extend(models);
            }
        }
    }
    let mut brands: Vec<Brand> = brands.into_iter().filter(|(_, m)| !m.is_empty()).map(|(name, models)| Brand { name, models }).collect();
    brands.sort_by_key(|b| b.name.to_lowercase());
    (!brands.is_empty()).then(|| Category { dir: ARCHIVE_DIR.into(), brands })
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<Model>) {
    for e in sorted_entries(dir) {
        if e.is_dir() {
            collect(root, &e, out);
        } else if is_ir(&e) {
            out.push(model(root, &e));
        }
    }
}

fn model(root: &Path, p: &Path) -> Model {
    let name = p.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
    Model { path: p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned(), name }
}

fn is_ir(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ir"))
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    sorted_entries(dir).into_iter().filter(|p| p.is_dir()).collect()
}

/// Кнопки пульта из базы (путь — от корня базы или абсолютный).
pub fn read(path: &str) -> Vec<Signal> {
    let p = Path::new(path);
    let p = if p.is_absolute() { p.to_path_buf() } else { root().join(p) };
    std::fs::read_to_string(p).map(|s| ir::parse(&s)).unwrap_or_default()
}

/// Ход скачивания.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Загружено байт из (если известно) всего.
    Download(u64, Option<u64>),
    /// Распаковано файлов.
    Unpack(usize),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(concat!("syn-remote/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Скачать (обновить) базу целиком: архив Flipper-IRDB и универсальные библиотеки. Старая база
/// заменяется только после удачной распаковки. Возвращает число пультов.
pub fn download(progress: impl Fn(Progress)) -> anyhow::Result<usize> {
    let root = root();
    let parent = root.parent().unwrap_or(Path::new("/tmp")).to_path_buf();
    std::fs::create_dir_all(&parent)?;
    let zip_path = parent.join("irdb.zip.part");
    {
        let resp = agent().get(ARCHIVE).call()?;
        let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        let mut body = resp.into_body();
        let mut reader = body.with_config().limit(512 * 1024 * 1024).reader();
        let mut file = std::fs::File::create(&zip_path)?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = 0u64;
        let mut last = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            got += n as u64;
            if got - last >= 256 * 1024 {
                last = got;
                progress(Progress::Download(got, total));
            }
        }
        progress(Progress::Download(got, total));
    }
    let stage = parent.join("irdb.new");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path)?)?;
    let mut count = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(path) = entry.enclosed_name() else { continue };
        // «Flipper-IRDB-main/TVs/LG/LG.ir» → «TVs/LG/LG.ir»
        let rel: PathBuf = path.components().skip(1).collect();
        if entry.is_dir() || !is_ir(&rel) || rel.components().count() < 2 {
            continue;
        }
        let out = stage.join(&rel);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
        count += 1;
        if count % 200 == 0 {
            progress(Progress::Unpack(count));
        }
    }
    progress(Progress::Unpack(count));
    let _ = std::fs::remove_file(&zip_path);
    if count == 0 {
        anyhow::bail!(syngui::t!("в архиве нет файлов пультов"));
    }
    let uni = stage.join(UNIVERSAL_DIR);
    std::fs::create_dir_all(&uni)?;
    for lib in ["tv", "ac", "audio", "projector"] {
        match agent().get(format!("{UNIVERSAL}/{lib}.ir")).call().and_then(|r| r.into_body().read_to_string()) {
            Ok(text) => std::fs::write(uni.join(format!("{lib}.ir")), text)?,
            Err(e) => tracing::warn!("универсальная библиотека {lib}: {e}"),
        }
    }
    let meta = Meta { updated: now(), remotes: count, source: "Flipper-IRDB".into() };
    std::fs::write(stage.join(META), serde_json::to_string_pretty(&meta)?)?;
    let old = parent.join("irdb.old");
    let _ = std::fs::remove_dir_all(&old);
    if root.exists() {
        std::fs::rename(&root, &old)?;
    }
    std::fs::rename(&stage, &root)?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(count)
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Вариант при подборе: код проверочной кнопки и пульты базы, в которых он есть.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Кнопки, которыми проверяют вариант (питание, звук, громкость) — из лучшего пульта группы.
    pub signals: Vec<Signal>,
    /// Пульты с таким кодом питания, лучший (больше всего кнопок) — первым. Пусто — вариант
    /// только из универсальной библиотеки.
    pub models: Vec<Model>,
    /// Марка (для подбора без марки).
    pub brand: String,
}

impl Candidate {
    /// Сигнал для проверки по ключу («power», «mute», «vol_up»); нет такой кнопки — питание.
    pub fn test(&self, key: &str) -> Option<&Signal> {
        keys::find(&self.signals, key).or_else(|| keys::power(&self.signals))
    }
}

/// Варианты для подбора: разные коды питания пультов марки (или всего типа техники, если марка не
/// выбрана), самые распространённые — первыми. Без марки впереди идут коды универсальной библиотеки
/// Flipper в её порядке (он и есть порядок популярности).
pub fn candidates(db: &Db, cat: &str, brand: Option<&str>) -> Vec<Candidate> {
    let Some(c) = db.category(cat) else { return vec![] };
    struct Group {
        models: Vec<(Model, usize, String)>,
        /// Пультов из самого типа техники (не из архива) — по ним порядок.
        primary: usize,
        best: Vec<Signal>,
        best_len: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut add = |m: &Model, brand: &str, primary: bool| {
        let signals = read(&m.path);
        // в архиве кнопки «питание» может не быть — тогда пульт не годится для проверки
        let p = if primary { keys::power(&signals) } else { keys::power_strict(&signals) };
        let Some(p) = p else { return };
        if ir::encode(&p.code).is_none() {
            return;
        }
        let gi = *index.entry(format!("{:?}", p.code)).or_insert_with(|| {
            groups.push(Group { models: vec![], primary: 0, best: vec![], best_len: 0 });
            groups.len() - 1
        });
        let g = &mut groups[gi];
        // сервисные и гостиничные пульты (заводское меню) — основными только если других нет
        let rank = signals.len() * if is_service(&m.name) { 1 } else { 1000 };
        if rank > g.best_len && (primary || g.primary == 0) {
            g.best_len = rank;
            g.best = signals.clone();
        }
        g.primary += primary as usize;
        g.models.push((m.clone(), rank, brand.to_string()));
    };
    for b in &c.brands {
        if brand.is_some_and(|n| n != b.name) {
            continue;
        }
        for m in &b.models {
            add(m, &b.name, true);
        }
    }
    if let (Some(name), Some(arch)) = (brand, db.category(ARCHIVE_DIR).filter(|_| cat != ARCHIVE_DIR)) {
        let key = name.to_lowercase();
        for b in arch.brands.iter().filter(|b| b.name.to_lowercase() == key) {
            for m in &b.models {
                add(m, &b.name, false);
            }
        }
    }
    let mut out: Vec<Candidate> = Vec::new();
    if brand.is_none() {
        if let Some(lib) = universal_for(cat) {
            let text = std::fs::read_to_string(root().join(UNIVERSAL_DIR).join(format!("{lib}.ir"))).unwrap_or_default();
            for block in universal_blocks(&ir::parse(&text)) {
                let Some(p) = keys::power(&block) else { continue };
                if ir::encode(&p.code).is_none() || out.iter().any(|c| keys::power(&c.signals).map(|s| &s.code) == Some(&p.code)) {
                    continue;
                }
                if let Some(&gi) = index.get(&format!("{:?}", p.code)) {
                    if !groups[gi].models.is_empty() {
                        let models = std::mem::take(&mut groups[gi].models);
                        out.push(group_candidate(&groups[gi].best, models));
                        continue;
                    }
                }
                out.push(Candidate { signals: block, models: vec![], brand: String::new() });
            }
        }
    }
    groups.retain(|g| !g.models.is_empty());
    groups.sort_by(|a, b| (b.primary > 0).cmp(&(a.primary > 0)).then(b.primary.cmp(&a.primary)).then(b.models.len().cmp(&a.models.len())));
    for g in groups {
        out.push(group_candidate(&g.best, g.models));
    }
    out
}

/// Сервисный, заводской или гостиничный пульт.
fn is_service(name: &str) -> bool {
    let n = name.to_lowercase();
    ["service", "factory", "hotel", "svc", "install"].iter().any(|w| n.contains(w))
}

fn group_candidate(best: &[Signal], mut models: Vec<(Model, usize, String)>) -> Candidate {
    models.sort_by(|a, b| b.1.cmp(&a.1));
    // самая частая марка группы
    let mut brands: HashMap<&str, usize> = HashMap::new();
    for (_, _, b) in &models {
        *brands.entry(b.as_str()).or_default() += 1;
    }
    let brand = brands.into_iter().max_by_key(|(_, n)| *n).map(|(b, _)| b.to_string()).unwrap_or_default();
    Candidate { signals: best.to_vec(), models: models.into_iter().map(|(m, _, _)| m).collect(), brand }
}

/// Универсальная библиотека — подряд идущие кнопки разных пультов; новый пульт начинается с «Power».
fn universal_blocks(signals: &[Signal]) -> Vec<Vec<Signal>> {
    let mut out: Vec<Vec<Signal>> = Vec::new();
    for s in signals {
        let is_power = keys::key_of(&s.name) == Some("power");
        match out.last_mut() {
            Some(b) if !is_power && keys::find(b, keys::key_of(&s.name).unwrap_or("-")).is_none() => b.push(s.clone()),
            _ => out.push(vec![s.clone()]),
        }
    }
    out
}

/// Поиск пультов по марке и модели во всей базе: (тип, марка, модель).
pub fn search<'a>(db: &'a Db, query: &str, limit: usize) -> Vec<(&'a Category, &'a Brand, &'a Model)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut out = Vec::new();
    for c in &db.categories {
        for b in &c.brands {
            for m in &b.models {
                let hay = format!("{} {}", b.name, m.name).to_lowercase();
                if words.iter().all(|w| hay.contains(w)) {
                    out.push((c, b, m));
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universal_split() {
        let text = "name: Power\ntype: parsed\nprotocol: NEC\naddress: 04 00 00 00\ncommand: 08 00 00 00\n#\nname: Mute\ntype: parsed\nprotocol: NEC\naddress: 04 00 00 00\ncommand: 09 00 00 00\n#\nname: Power\ntype: parsed\nprotocol: SIRC\naddress: 01 00 00 00\ncommand: 15 00 00 00\n#\nname: Power\ntype: parsed\nprotocol: SIRC\naddress: 10 00 00 00\ncommand: 15 00 00 00\n";
        let b = universal_blocks(&ir::parse(text));
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].len(), 2);
    }
}
