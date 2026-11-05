//! Персистентность: JSON-снимок в каталоге данных приложения.

pub mod schema;

use std::path::PathBuf;

pub use schema::{AppStateData, CURRENT_VERSION};

const FILE_NAME: &str = "quitsmoke.json";

/// Каталог для данных приложения на текущей платформе.
///
/// На Android — переданный `internal_data_path()`; на desktop — XDG/HOME.
pub fn platform_data_dir(app_data_dir: Option<PathBuf>) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        app_data_dir.unwrap_or_else(|| PathBuf::from("/data/data/com.quitsmoke.app/files"))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app_data_dir; // на desktop не используется
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("quitsmoke")
    }
}

/// Хранилище: знает путь к файлу и умеет читать/писать снимок.
#[derive(Clone, Debug)]
pub struct Persistence {
    file: PathBuf,
}

impl Persistence {
    /// Создать (и обеспечить существование каталога данных).
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            file: dir.join(FILE_NAME),
        }
    }

    /// Загрузить снимок; при отсутствии/повреждении/старой версии — `Default` (+миграция).
    pub fn load(&self) -> AppStateData {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => match serde_json::from_str::<AppStateData>(&text) {
                Ok(data) => {
                    if data.version != CURRENT_VERSION {
                        data.migrate()
                    } else {
                        data
                    }
                }
                Err(_) => AppStateData::default(),
            },
            Err(_) => AppStateData::default(),
        }
    }

    /// Атомарно сохранить снимок (temp → rename).
    pub fn save(&self, data: &AppStateData) {
        let Ok(json) = serde_json::to_string_pretty(data) else {
            return;
        };
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, json.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    /// Путь к файлу (для логов/диагностики).
    pub fn file_path(&self) -> &PathBuf {
        &self.file
    }
}
