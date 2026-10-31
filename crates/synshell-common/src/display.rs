//! Тип экрана устройства и режимы экрана E Ink.
//!
//! Драйвер E Ink (ONYX BOOX: `onyx-tcon-fpga`, DRM-устройство `onyx-epd`) даёт в sysfs
//! устройства каталог управления:
//! - `panel` — тип панели: `kaleido` (цветной фильтр поверх ч/б E Ink) или `carta` (монохромная);
//! - `color` — `1` цветной вывод (CFA Kaleido накладывает FPGA), `0` — монохром;
//! - `update_mode` — `quality` / `balanced` / `fast` / `a2`, чтение — `[quality] balanced fast a2`;
//! - `refresh` — запись `1`: полная очистка экрана («призраки»).
//!
//! Права — группа `video` (udev), пользователю сеанса root не нужен. На обычном компьютере
//! этих файлов нет: всё молча даёт [`DisplayKind::Lcd`] и `None`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::t;

/// Имена DRM-драйверов экранов E Ink (`drmGetVersion`): каждое обновление — видимая волна на панели.
pub const EINK_DRM_DRIVERS: &[&str] = &["onyx-epd"];

/// Платформенные драйверы E Ink в sysfs (`/sys/bus/platform/drivers/<имя>`), у устройств
/// которых есть каталог управления (`panel`, `color`, `update_mode`, `refresh`).
pub const EINK_PLATFORM_DRIVERS: &[&str] = &["onyx-tcon-fpga"];

/// Подставной корень sysfs (проверка интерфейса E Ink на компьютере): `SYNSHELL_SYSFS=/tmp/sys`.
pub const SYSFS_ENV: &str = "SYNSHELL_SYSFS";

fn sys_root() -> PathBuf {
    std::env::var_os(SYSFS_ENV).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/sys"))
}

/// Тип экрана.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayKind {
    /// Обычный экран (LCD/OLED).
    Lcd,
    /// Монохромный E Ink (Carta) или E Ink без каталога управления.
    EinkMono,
    /// Цветной E Ink: цветной фильтр поверх ч/б панели (`panel` — например `kaleido`).
    EinkColor { panel: String },
}

impl DisplayKind {
    pub fn is_eink(&self) -> bool {
        !matches!(self, DisplayKind::Lcd)
    }

    pub fn is_color_eink(&self) -> bool {
        matches!(self, DisplayKind::EinkColor { .. })
    }
}

/// Режим обновления E Ink (волна драйвера).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// 16 оттенков, частая полная очистка.
    Quality,
    /// Реже полная очистка.
    Balanced,
    /// Быстрая ч/б волна (серые слабее).
    Fast,
    /// Самая быстрая, без автоочистки (видео, прокрутка).
    A2,
}

impl UpdateMode {
    pub const ALL: [UpdateMode; 4] = [UpdateMode::Quality, UpdateMode::Balanced, UpdateMode::Fast, UpdateMode::A2];

    /// Имя в sysfs и в конфиге.
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateMode::Quality => "quality",
            UpdateMode::Balanced => "balanced",
            UpdateMode::Fast => "fast",
            UpdateMode::A2 => "a2",
        }
    }

    pub fn from_name(s: &str) -> Option<UpdateMode> {
        let s = s.trim();
        UpdateMode::ALL.into_iter().find(|m| m.as_str().eq_ignore_ascii_case(s))
    }

    /// Название для интерфейса.
    pub fn title(self) -> String {
        match self {
            UpdateMode::Quality => t!("Качество"),
            UpdateMode::Balanced => t!("Сбалансированный"),
            UpdateMode::Fast => t!("Быстрый"),
            UpdateMode::A2 => "A2".into(),
        }
    }

    /// Пояснение для интерфейса.
    pub fn hint(self) -> String {
        match self {
            UpdateMode::Quality => t!("16 оттенков, частая полная очистка"),
            UpdateMode::Balanced => t!("Оттенки, полная очистка реже"),
            UpdateMode::Fast => t!("Быстрая ч/б волна, серые слабее"),
            UpdateMode::A2 => t!("Самая быстрая, без автоочистки — видео и прокрутка"),
        }
    }

    /// Следующий по кругу (плитка шторки).
    pub fn next(self) -> UpdateMode {
        let i = UpdateMode::ALL.iter().position(|m| *m == self).unwrap_or(0);
        UpdateMode::ALL[(i + 1) % UpdateMode::ALL.len()]
    }
}

/// Текущий режим из чтения `update_mode` (`[quality] balanced fast a2`); без скобок —
/// единственное имя.
pub fn parse_update_mode(s: &str) -> Option<UpdateMode> {
    let s = s.trim();
    if let Some(start) = s.find('[') {
        let rest = &s[start + 1..];
        let end = rest.find(']')?;
        return UpdateMode::from_name(&rest[..end]);
    }
    UpdateMode::from_name(s)
}

/// Каталог управления экраном E Ink в sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eink {
    dir: PathBuf,
}

impl Eink {
    /// Каталог управления этого устройства (`/sys`, см. [`SYSFS_ENV`]).
    pub fn find() -> Option<Eink> {
        static DIR: OnceLock<Option<Eink>> = OnceLock::new();
        DIR.get_or_init(|| Eink::find_in(&sys_root())).clone()
    }

    /// Поиск в дереве `sys` (подставной каталог в тестах): устройства драйверов
    /// [`EINK_PLATFORM_DRIVERS`], затем родитель любого DRM-устройства — с файлом `panel`.
    pub fn find_in(sys: &Path) -> Option<Eink> {
        let mut candidates = Vec::new();
        for d in EINK_PLATFORM_DRIVERS {
            candidates.extend(dir_entries(&sys.join("bus/platform/drivers").join(d)));
        }
        for card in dir_entries(&sys.join("class/drm")) {
            if is_card(&card) {
                candidates.push(card.join("device"));
            }
        }
        candidates.into_iter().find(|p| p.join("panel").is_file()).map(|dir| Eink { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok().map(|s| s.trim().to_string())
    }

    fn write(&self, name: &str, value: &str) -> std::io::Result<()> {
        std::fs::write(self.dir.join(name), value)
    }

    /// Тип панели (`kaleido`, `carta`).
    pub fn panel(&self) -> Option<String> {
        self.read("panel").filter(|s| !s.is_empty())
    }

    /// Цветная ли панель (фильтр Kaleido).
    pub fn is_color_panel(&self) -> bool {
        self.panel().is_some_and(|p| p.eq_ignore_ascii_case("kaleido"))
    }

    /// Цветной вывод включён; `None` — нет файла или драйвер ещё не готов (`-1`).
    pub fn color(&self) -> Option<bool> {
        match self.read("color")?.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    pub fn set_color(&self, on: bool) -> std::io::Result<()> {
        self.write("color", if on { "1" } else { "0" })
    }

    pub fn update_mode(&self) -> Option<UpdateMode> {
        parse_update_mode(&self.read("update_mode")?)
    }

    pub fn set_update_mode(&self, m: UpdateMode) -> std::io::Result<()> {
        self.write("update_mode", m.as_str())
    }

    /// Полная очистка экрана сейчас.
    pub fn refresh(&self) -> std::io::Result<()> {
        self.write("refresh", "1")
    }

    /// Применить выбор пользователя (`[eink]`): пишется только то, что задано и отличается
    /// от текущего (запись `color` перерисовывает экран). Цвет — только на цветной панели.
    pub fn apply(&self, cfg: &crate::config::EinkSettings) -> std::io::Result<()> {
        if let Some(on) = cfg.color {
            if self.is_color_panel() && self.color() != Some(on) {
                self.set_color(on)?;
            }
        }
        if let Some(m) = cfg.update_mode {
            if self.update_mode() != Some(m) {
                self.set_update_mode(m)?;
            }
        }
        Ok(())
    }
}

fn dir_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// `cardN` (без коннекторов `cardN-DSI-1`).
fn is_card(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("card"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Тип экрана по дереву `sys`: каталог управления E Ink (`panel = kaleido` — цветной),
/// иначе DRM-устройство платформенного драйвера E Ink — монохромный, иначе обычный.
pub fn display_kind_in(sys: &Path) -> DisplayKind {
    if let Some(e) = Eink::find_in(sys) {
        return match e.panel() {
            Some(p) if p.eq_ignore_ascii_case("kaleido") => DisplayKind::EinkColor { panel: p },
            _ => DisplayKind::EinkMono,
        };
    }
    let eink_drm = dir_entries(&sys.join("class/drm")).into_iter().filter(|c| is_card(c)).any(|c| {
        std::fs::read_link(c.join("device/driver"))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
            .is_some_and(|n| EINK_PLATFORM_DRIVERS.contains(&n.as_str()))
    });
    if eink_drm {
        DisplayKind::EinkMono
    } else {
        DisplayKind::Lcd
    }
}

/// Тип экрана этого устройства (один раз за процесс). Кроме sysfs — `SYNSHELL_EINK=1` от synwm
/// (он узнаёт E Ink по имени DRM-драйвера, [`EINK_DRM_DRIVERS`]) — тогда хотя бы монохромный.
pub fn display_kind() -> DisplayKind {
    static KIND: OnceLock<DisplayKind> = OnceLock::new();
    KIND.get_or_init(|| match display_kind_in(&sys_root()) {
        DisplayKind::Lcd if crate::config::eink() => DisplayKind::EinkMono,
        k => k,
    })
    .clone()
}

/// Экран устройства — E Ink.
pub fn is_eink() -> bool {
    display_kind().is_eink()
}

// TODO: режим обновления на приложение (`[[eink_app]] app_id, update_mode`, как `[[x11_app]]`) —
// synwm менял бы `update_mode` по окну в фокусе; пока режим общий.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn update_mode_parse() {
        assert_eq!(parse_update_mode("[quality] balanced fast a2\n"), Some(UpdateMode::Quality));
        assert_eq!(parse_update_mode("quality balanced [fast] a2"), Some(UpdateMode::Fast));
        assert_eq!(parse_update_mode("quality balanced fast [a2]"), Some(UpdateMode::A2));
        assert_eq!(parse_update_mode("balanced"), Some(UpdateMode::Balanced));
        assert_eq!(parse_update_mode("[nope] quality"), None);
        assert_eq!(parse_update_mode(""), None);
        assert_eq!(UpdateMode::A2.next(), UpdateMode::Quality);
        assert_eq!(UpdateMode::Quality.next(), UpdateMode::Balanced);
    }

    #[test]
    fn update_mode_serde() {
        #[derive(Deserialize)]
        struct T {
            m: UpdateMode,
        }
        assert_eq!(toml::from_str::<T>("m = \"a2\"").unwrap().m, UpdateMode::A2);
        assert_eq!(toml::from_str::<T>("m = \"balanced\"").unwrap().m, UpdateMode::Balanced);
    }

    /// Подставное дерево: устройство драйвера onyx-tcon-fpga и карта DRM под ним.
    fn fake_sys(panel: Option<&str>) -> tempfile::TempDir {
        let sys = tempfile::tempdir().unwrap();
        let dev = sys.path().join("devices/platform/tcon-fpga");
        fs::create_dir_all(&dev).unwrap();
        let drv = sys.path().join("bus/platform/drivers/onyx-tcon-fpga");
        fs::create_dir_all(&drv).unwrap();
        std::os::unix::fs::symlink(&dev, drv.join("tcon-fpga")).unwrap();
        std::os::unix::fs::symlink(&drv, dev.join("driver")).unwrap();
        fs::write(drv.join("bind"), "").unwrap();
        fs::create_dir_all(sys.path().join("class/drm")).unwrap();
        let card = dev.join("drm/card0");
        fs::create_dir_all(&card).unwrap();
        std::os::unix::fs::symlink(&dev, card.join("device")).unwrap();
        std::os::unix::fs::symlink(&card, sys.path().join("class/drm/card0")).unwrap();
        if let Some(p) = panel {
            fs::write(dev.join("panel"), format!("{p}\n")).unwrap();
            fs::write(dev.join("color"), "0\n").unwrap();
            fs::write(dev.join("update_mode"), "[quality] balanced fast a2\n").unwrap();
        }
        sys
    }

    #[test]
    fn kind_lcd_without_sysfs() {
        let sys = tempfile::tempdir().unwrap();
        assert_eq!(display_kind_in(sys.path()), DisplayKind::Lcd);
        assert!(Eink::find_in(sys.path()).is_none());
        assert_eq!(display_kind_in(Path::new("/nonexistent-sys")), DisplayKind::Lcd);
    }

    #[test]
    fn kind_kaleido_and_carta() {
        let sys = fake_sys(Some("kaleido"));
        assert_eq!(display_kind_in(sys.path()), DisplayKind::EinkColor { panel: "kaleido".into() });
        let sys = fake_sys(Some("carta"));
        assert_eq!(display_kind_in(sys.path()), DisplayKind::EinkMono);
        // драйвер без каталога управления — всё равно E Ink
        let sys = fake_sys(None);
        assert_eq!(display_kind_in(sys.path()), DisplayKind::EinkMono);
        assert!(Eink::find_in(sys.path()).is_none());
    }

    #[test]
    fn eink_read_write_apply() {
        let sys = fake_sys(Some("kaleido"));
        let e = Eink::find_in(sys.path()).unwrap();
        assert_eq!(e.panel().as_deref(), Some("kaleido"));
        assert_eq!(e.color(), Some(false));
        assert_eq!(e.update_mode(), Some(UpdateMode::Quality));
        let cfg = crate::config::EinkSettings { color: Some(true), update_mode: Some(UpdateMode::A2) };
        e.apply(&cfg).unwrap();
        assert_eq!(fs::read_to_string(e.dir().join("color")).unwrap(), "1");
        assert_eq!(fs::read_to_string(e.dir().join("update_mode")).unwrap(), "a2");
        e.refresh().unwrap();
        assert_eq!(fs::read_to_string(e.dir().join("refresh")).unwrap(), "1");
        // монохромная панель: цвет не трогаем
        let sys = fake_sys(Some("carta"));
        let e = Eink::find_in(sys.path()).unwrap();
        e.apply(&crate::config::EinkSettings { color: Some(true), update_mode: None }).unwrap();
        assert_eq!(e.color(), Some(false));
    }
}
