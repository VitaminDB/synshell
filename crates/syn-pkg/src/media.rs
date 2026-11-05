//! Картинки «Программ»: значки каталога AppStream (в нём они JPEG XL — один
//! раз перекодируются в PNG в кэш) и снимки экрана: сначала из AppStream
//! (ссылки в каталоге), иначе с Flathub — по id AppStream или поиском по
//! имени пакета (подходит только точное совпадение названия). Скачивание —
//! `curl` (уважает `http_proxy`, как и AUR), файлы — в `~/.cache/synpkg`.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn cache_dir(sub: &str) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".cache")
    });
    base.join("synpkg").join(sub)
}

/// PNG в кэше для значка каталога `jxl` (`…/archlinux-arch-extra/64x64/gimp_gimp.jxl`).
fn png_for(jxl: &Path) -> Option<PathBuf> {
    let size = jxl.parent()?.file_name()?.to_string_lossy().into_owned();
    let repo = jxl.parent()?.parent()?.file_name()?.to_string_lossy().into_owned();
    let stem = jxl.file_stem()?.to_string_lossy().into_owned();
    Some(cache_dir("icons").join(format!("{repo}-{size}")).join(format!("{stem}.png")))
}

/// Готовый PNG значка каталога, если уже перекодирован.
pub fn icon_png(jxl: &Path) -> Option<PathBuf> {
    png_for(jxl).filter(|p| p.is_file())
}

/// Перекодировать один значок сейчас (крупный значок в подробностях).
pub fn icon_png_now(jxl: &Path) -> Option<PathBuf> {
    let out = png_for(jxl)?;
    if out.is_file() {
        return Some(out);
    }
    std::fs::create_dir_all(out.parent()?).ok()?;
    let ok = Command::new("magick").arg(jxl).arg(&out).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
        || Command::new("djxl").arg(jxl).arg(&out).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    ok.then_some(out)
}

/// Перекодировать все значки 64×64 каталога (один `magick mogrify` на
/// каталог — секунды; повторно не делается, пока каталог не обновится).
/// `true` — появились новые значки.
pub fn convert_catalog_icons() -> bool {
    let Ok(rd) = std::fs::read_dir("/usr/share/swcatalog/icons") else { return false };
    let mut changed = false;
    for e in rd.flatten() {
        let src = e.path().join("64x64");
        let Ok(meta) = std::fs::metadata(&src) else { continue };
        let repo = e.file_name().to_string_lossy().into_owned();
        let out = cache_dir("icons").join(format!("{repo}-64x64"));
        let stamp = out.join(".done");
        let fresh = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok().zip(meta.modified().ok()).is_some_and(|(s, m)| s >= m);
        if fresh || std::fs::create_dir_all(&out).is_err() {
            continue;
        }
        let files: Vec<PathBuf> = std::fs::read_dir(&src).into_iter().flatten().flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|x| x == "jxl")).collect();
        if files.is_empty() {
            continue;
        }
        let ok = if synsystem_which("magick") {
            // Списком из файла — без предела длины командной строки.
            let list = out.join(".list");
            let body: String = files.iter().map(|p| format!("{}\n", p.display())).collect();
            std::fs::write(&list, body).is_ok()
                && Command::new("magick").arg("mogrify").arg("-path").arg(&out).args(["-format", "png"]).arg(format!("@{}", list.display())).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
        } else if synsystem_which("djxl") {
            for f in &files {
                if let Some(o) = png_for(f) {
                    let _ = Command::new("djxl").arg(f).arg(o).stdout(Stdio::null()).stderr(Stdio::null()).status();
                }
            }
            true
        } else {
            false
        };
        if ok {
            let _ = std::fs::write(&stamp, "");
            changed = true;
        }
    }
    changed
}

fn synsystem_which(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// Снимки и описание программы.
#[derive(Clone, PartialEq, Default)]
pub struct Media {
    /// Пакет, к которому относится.
    pub name: String,
    /// Скачанные снимки (по мере загрузки).
    pub shots: Vec<PathBuf>,
    /// Ещё грузится.
    pub loading: bool,
    /// Откуда: «AppStream» или «Flathub».
    pub source: &'static str,
    /// Описание с Flathub, если в каталоге его нет.
    pub description: Vec<String>,
    /// Значок с Flathub, если своего нет.
    pub icon: Option<PathBuf>,
}

/// Что известно о программе до загрузки.
pub struct Known {
    pub name: String,
    /// Название программы (для сверки с найденным на Flathub).
    pub title: String,
    pub appstream_id: String,
    pub screenshots: Vec<String>,
    pub has_description: bool,
}

const MAX_SHOTS: usize = 6;

/// Найти и скачать снимки; `progress` вызывается после каждого шага.
pub fn load(k: Known, mut progress: impl FnMut(Media)) {
    let mut m = Media { name: k.name.clone(), loading: true, ..Default::default() };
    let mut urls = k.screenshots.clone();
    if !urls.is_empty() {
        m.source = "AppStream";
    } else if let Some(f) = flathub(&k) {
        m.source = "Flathub";
        urls = f.screenshots;
        if !k.has_description {
            m.description = f.description;
        }
        m.icon = f.icon.and_then(|u| download(&u));
    }
    progress(m.clone());
    for u in urls.iter().take(MAX_SHOTS) {
        if let Some(p) = download(u) {
            m.shots.push(p);
            progress(m.clone());
        }
    }
    m.loading = false;
    progress(m);
}

struct Flathub {
    screenshots: Vec<String>,
    description: Vec<String>,
    icon: Option<String>,
}

fn curl_json(args: &[&str]) -> Option<serde_json::Value> {
    let out = Command::new("curl").args(["-sfL", "--max-time", "15"]).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| serde_json::from_slice(&out.stdout).ok()).flatten()
}

/// Программа на Flathub: по id AppStream, иначе поиском по имени с точным
/// совпадением названия или последней части id.
fn flathub(k: &Known) -> Option<Flathub> {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let mut ids: Vec<String> = Vec::new();
    let id = k.appstream_id.trim_end_matches(".desktop");
    if !id.is_empty() {
        ids.push(id.to_string());
    }
    if ids.is_empty() {
        let body = serde_json::json!({ "query": k.name }).to_string();
        let j = curl_json(&["-X", "POST", "-H", "Content-Type: application/json", "-d", &body, "https://flathub.org/api/v2/search"])?;
        let (n, t) = (norm(&k.name), norm(&k.title));
        for h in j.get("hits").and_then(|h| h.as_array()).into_iter().flatten().take(12) {
            let app_id = h.get("app_id").and_then(|x| x.as_str()).unwrap_or("");
            let name = norm(h.get("name").and_then(|x| x.as_str()).unwrap_or(""));
            let last = norm(app_id.rsplit('.').next().unwrap_or(""));
            if !app_id.is_empty() && (name == n || name == t || last == n) {
                ids.push(app_id.to_string());
                break;
            }
        }
    }
    let id = ids.first()?;
    let j = curl_json(&[&format!("https://flathub.org/api/v2/appstream/{id}")])?;
    let mut screenshots = Vec::new();
    for s in j.get("screenshots").and_then(|x| x.as_array()).into_iter().flatten() {
        // Размер около 1248 px — чётко в просмотре и не тяжело.
        let mut sizes: Vec<(u32, String)> = s
            .get("sizes")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|z| Some((z.get("width")?.as_str()?.parse().ok()?, z.get("src")?.as_str()?.to_string())))
            .collect();
        sizes.sort_by_key(|(w, _)| w.abs_diff(1248));
        if let Some((_, u)) = sizes.into_iter().next() {
            screenshots.push(u);
        }
    }
    let description = j.get("description").and_then(|x| x.as_str()).map(html_paragraphs).unwrap_or_default();
    let icon = j.get("icon").and_then(|x| x.as_str()).map(String::from);
    Some(Flathub { screenshots, description, icon })
}

/// Абзацы из HTML описания Flathub (`<p>`, `<li>`).
pub fn html_paragraphs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut rest = html;
    let mut flush = |cur: &mut String, li: bool| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        let t = t.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
        if !t.is_empty() {
            out.push(if li { format!("• {t}") } else { t });
        }
        cur.clear();
    };
    let mut in_li = false;
    while let Some(lt) = rest.find('<') {
        cur.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = rest[lt + 1..lt + gt].trim().to_lowercase();
        rest = &rest[lt + gt + 1..];
        match tag.split_whitespace().next().unwrap_or("") {
            "p" | "/p" | "ul" | "/ul" | "ol" | "/ol" | "br" | "br/" => flush(&mut cur, in_li),
            "li" => {
                flush(&mut cur, in_li);
                in_li = true;
            }
            "/li" => {
                flush(&mut cur, true);
                in_li = false;
            }
            _ => {}
        }
    }
    cur.push_str(rest);
    flush(&mut cur, in_li);
    out
}

/// Скачать в кэш (повторно — из кэша).
pub fn download(url: &str) -> Option<PathBuf> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let ext = url.rsplit('.').next().filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("img").to_lowercase();
    let dir = cache_dir("shots");
    let path = dir.join(format!("{:016x}.{ext}", h.finish()));
    if path.is_file() {
        return Some(path);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = path.with_extension("part");
    let ok = Command::new("curl").args(["-sfL", "--max-time", "40", "-o"]).arg(&tmp).arg(url).stdin(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html() {
        assert_eq!(html_paragraphs("<p>One &amp; two</p>\n<ul><li>A</li><li>B <em>c</em></li></ul><p>End</p>"), vec!["One & two", "• A", "• B c", "End"]);
    }
}
