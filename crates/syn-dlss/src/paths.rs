//! Пути — те же, что у `dlssnr-helper`, слоя и помощника: XDG для настроек и данных,
//! `/tmp/dlssnr-$DLSSNR_UID|$UID` для общей памяти и pid (каталог виден и из контейнера Steam).

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("dlssnr")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.ini")
}

/// Флаг «DLSS 5 в играх syn-wine»: наш Wine, видя его, включает слой (VKLayer_DLSS5=1) своим программам
/// (crates/syn-wine/win32u-dlss5-layer.patch).
pub fn syn_wine_flag() -> PathBuf {
    config_dir().join("syn-wine")
}

pub fn profiles_dir() -> PathBuf {
    config_dir().join("profiles")
}

/// DATA_DIR помощника: сюда импортируются DLL NGX.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("dlssnr")
}

pub fn default_binaries() -> PathBuf {
    data_dir().join("binaries")
}

pub fn default_log() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("dlssnr").join("helper.log")
}

/// ShmRuntimeDir(): DLSSNR_UID, иначе настоящий uid.
pub fn runtime_dir() -> PathBuf {
    match std::env::var("DLSSNR_UID") {
        Ok(u) if !u.is_empty() => PathBuf::from(format!("/tmp/dlssnr-{u}")),
        // SAFETY: getuid не может завершиться ошибкой.
        _ => PathBuf::from(format!("/tmp/dlssnr-{}", unsafe { libc::getuid() })),
    }
}

/// Файл общей памяти: DLSSNR_SHM, затем `shm=` из config.ini, затем по умолчанию — порядок слоя и помощника.
pub fn shm_path() -> PathBuf {
    if let Ok(p) = std::env::var("DLSSNR_SHM") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let cfg = crate::config::Ini::load();
    if let Some(p) = cfg.get("shm").filter(|p| !p.is_empty() && !is_legacy_shm(p)) {
        return PathBuf::from(p);
    }
    runtime_dir().join("shm.bin")
}

/// Старые сборки клали общую память в $XDG_RUNTIME_DIR — игра в контейнере Steam её не видит.
pub fn is_legacy_shm(p: &str) -> bool {
    std::env::var("XDG_RUNTIME_DIR").map(|r| !r.is_empty() && p == format!("{r}/dlssnr/shm.bin")).unwrap_or(false)
}

pub fn pid_file() -> PathBuf {
    shm_path().parent().map(|d| d.join("helper.pid")).unwrap_or_else(|| runtime_dir().join("helper.pid"))
}

/// Команда управления помощником: DLSSNR_HELPER_CLI, затем PATH, затем известные места.
pub fn helper_cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DLSSNR_HELPER_CLI").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = which("dlssnr-helper") {
        return Some(p);
    }
    [home().join(".local/bin/dlssnr-helper"), PathBuf::from("/usr/bin/dlssnr-helper")].into_iter().find(|p| p.is_file())
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(cmd)).find(|p| p.is_file()))
}

/// Помощник жив: pid из файла запуска существует и это действительно dlssnr_helper.exe (pid переиспользуются).
pub fn helper_running() -> bool {
    let Ok(s) = std::fs::read_to_string(pid_file()) else { return false };
    let Ok(pid) = s.trim().parse::<i32>() else { return false };
    if pid <= 0 || !pid_alive(pid as u32) {
        return false;
    }
    std::fs::read(format!("/proc/{pid}/cmdline")).map(|c| c.windows(17).any(|w| w == b"dlssnr_helper.exe")).unwrap_or(true)
}

pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: сигнал 0 только проверяет существование процесса.
    pid != 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

/// Proton-подобные средства Steam, в которых может работать помощник (у syn-wine — системный Wine).
pub fn discover_runners() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let roots = [
        home().join(".steam/root/compatibilitytools.d"),
        home().join(".local/share/Steam/compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
    ];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut found: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.join("proton").is_file()).collect();
        found.sort();
        for p in found {
            let exe = p.join("proton");
            let exe = exe.canonicalize().unwrap_or(exe);
            if out.iter().any(|(_, e)| e == &exe) {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push((name, exe));
        }
    }
    out
}
