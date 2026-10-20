//! Библиотека Steam без клиента: установленные игры из `appmanifest_*.acf`
//! во всех библиотеках (`libraryfolders.vdf`), обложки и афиши — из кэша
//! клиента (`appcache/librarycache`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use synshell_common::config::Gameshell;

#[derive(Debug, Clone)]
pub struct SteamGame {
    pub appid: u32,
    pub name: String,
    /// Последний запуск (Unix-время) по данным Steam.
    pub last_played: u64,
    pub cover: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub header: Option<PathBuf>,
}

/// Служебное Steam, а не игры.
fn is_tool(appid: u32, name: &str) -> bool {
    const TOOLS: &[u32] = &[228980, 1070560, 1391110, 1628350, 4183110, 1493710, 961940, 1054830, 1113280, 1245040, 1420170, 1580130, 1887720, 2180100, 2348590, 2805730, 3658110];
    if TOOLS.contains(&appid) {
        return true;
    }
    let n = name.to_lowercase();
    n.starts_with("proton") || n.starts_with("steam linux runtime") || n.starts_with("steamworks common") || n.contains("steamvr") && n.contains("runtime")
}

/// Значения `"ключ" "значение"` из файла VDF/ACF (плоско, без вложенности).
fn vdf_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        // `\t"key"\t\t"value"` → ["\t", key, "\t\t", value, ""]
        if parts.len() >= 5 {
            out.push((parts[1].to_string(), parts[3].replace("\\\\", "\\")));
        }
    }
    out
}

/// Каталоги Steam: из настроек или стандартные.
pub fn roots(cfg: &Gameshell) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut v: Vec<PathBuf> = if cfg.steam_dirs.is_empty() {
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
        let mut d = Vec::new();
        if let Some(r) = std::env::var_os("STEAM_ARM64_ROOT") {
            d.push(PathBuf::from(r));
        }
        d.push(data.join("Steam"));
        d.push(home.join(".steam/steam"));
        d.push(home.join(".steam/root"));
        d.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        d
    } else {
        cfg.steam_dirs.iter().map(|d| synshell_common::paths::expand_tilde(d.trim())).collect()
    };
    let mut seen = HashSet::new();
    v.retain(|p| p.join("steamapps").is_dir() && seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    v
}

/// Картинки игры в кэше клиента: новые клиенты кладут их в
/// `librarycache/<appid>/` (часть — в подкаталогах-хэшах), старые — рядом
/// с именем `<appid>_<вид>`.
fn art(root: &Path, appid: u32) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let cache = root.join("appcache/librarycache");
    let mut files: Vec<PathBuf> = Vec::new();
    let dir = cache.join(appid.to_string());
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Ok(sub) = std::fs::read_dir(&p) {
                    files.extend(sub.flatten().map(|e| e.path()));
                }
            } else {
                files.push(p);
            }
        }
    }
    let find = |names: &[&str]| -> Option<PathBuf> {
        for n in names {
            if let Some(p) = files.iter().find(|p| p.file_name().and_then(|f| f.to_str()) == Some(*n)) {
                return Some(p.clone());
            }
            let flat = cache.join(format!("{appid}_{n}"));
            if flat.is_file() {
                return Some(flat);
            }
        }
        None
    };
    (
        find(&["library_600x900.jpg", "library_600x900_2x.jpg", "library_capsule.jpg"]),
        find(&["library_hero.jpg", "library_hero_2x.jpg"]),
        find(&["logo.png", "logo_2x.png"]),
        find(&["header.jpg", "library_header.jpg", "library_header_2x.jpg"]),
    )
}

/// Установленные игры всех библиотек.
pub fn games(cfg: &Gameshell) -> Vec<SteamGame> {
    let mut out: Vec<SteamGame> = Vec::new();
    let mut seen = HashSet::new();
    for root in roots(cfg) {
        let mut libs = vec![root.clone()];
        if let Ok(t) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            for (k, v) in vdf_pairs(&t) {
                if k == "path" {
                    libs.push(PathBuf::from(v));
                }
            }
        }
        let mut lib_seen = HashSet::new();
        for lib in libs {
            if !lib_seen.insert(std::fs::canonicalize(&lib).unwrap_or_else(|_| lib.clone())) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                let pairs = vdf_pairs(&text);
                let get = |k: &str| pairs.iter().find(|(kk, _)| kk.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
                let Some(appid) = get("appid").and_then(|v| v.parse::<u32>().ok()) else { continue };
                let title = get("name").unwrap_or_else(|| format!("Steam {appid}"));
                // Бит 4 — установлено полностью; качающиеся пока не показываем.
                let flags: u32 = get("StateFlags").and_then(|v| v.parse().ok()).unwrap_or(4);
                if flags & 4 == 0 {
                    continue;
                }
                if cfg.steam_hide_tools && is_tool(appid, &title) {
                    continue;
                }
                if !seen.insert(appid) {
                    continue;
                }
                let (cover, hero, logo, header) = art(&root, appid);
                out.push(SteamGame { appid, name: title, last_played: get("LastPlayed").and_then(|v| v.parse().ok()).unwrap_or(0), cover, hero, logo, header });
            }
        }
    }
    out
}

/// AppId из команды ярлыка (`steam steam://rungameid/570`).
pub fn appid_in_command(cmd: &str) -> Option<u32> {
    let i = cmd.find("steam://rungameid/")? + "steam://rungameid/".len();
    cmd[i..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"3105440\"\n\t\"name\"\t\t\"Heroes\"\n\t\"StateFlags\"\t\t\"4\"\n}\n";
        let p = vdf_pairs(acf);
        assert!(p.contains(&("appid".into(), "3105440".into())));
        assert!(p.contains(&("name".into(), "Heroes".into())));
    }

    #[test]
    fn tools_and_commands() {
        assert!(is_tool(1, "Proton Experimental"));
        assert!(is_tool(228980, "x"));
        assert!(!is_tool(570, "Dota 2"));
        assert_eq!(appid_in_command("steam steam://rungameid/570"), Some(570));
        assert_eq!(appid_in_command("env X=1 steam steam://rungameid/3105440 -silent"), Some(3105440));
        assert_eq!(appid_in_command("firefox"), None);
    }
}
