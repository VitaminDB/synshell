use std::path::PathBuf;

use serde::Deserialize;

pub const CONFIG: &str = "/etc/synaptix/synaptixd.toml";
pub const STATE: &str = "/var/lib/synaptix/state.toml";
pub const NPU_CACHE_RO: &str = "/usr/lib/synaptix/npu-cache";

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub min_available_mb: u64,
    pub idle_unload_min: u32,
    /// Сколько минут держать в RAM KV-кэш чатов выгруженной модели (0 — нет).
    pub kv_keep_min: u32,
    pub max_loaded: usize,
    pub model_dirs: Vec<PathBuf>,
    pub state: PathBuf,
    pub socket: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_available_mb: 1024,
            idle_unload_min: 10,
            kv_keep_min: 60,
            max_loaded: 0,
            model_dirs: Vec::new(),
            state: PathBuf::from(STATE),
            socket: synaptixd::socket_path(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = std::env::var_os("SYNAPTIXD_CONFIG").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(CONFIG));
        let mut cfg: Config = match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        if let Some(p) = std::env::var_os("SYNAPTIX_MODEL_PATH") {
            let mut dirs: Vec<PathBuf> = std::env::split_paths(&p).collect();
            dirs.extend(cfg.model_dirs);
            cfg.model_dirs = dirs;
        }
        if let Some(p) = std::env::var_os("SYNAPTIXD_STATE") {
            cfg.state = PathBuf::from(p);
        }
        if std::env::var_os("SYNAPTIXD_SOCKET").is_some() {
            cfg.socket = synaptixd::socket_path();
        }
        cfg
    }
}
