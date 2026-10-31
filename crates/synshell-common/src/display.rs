//! Тип экрана устройства и режимы экрана E Ink.
//!
//! Драйвер E Ink (ONYX BOOX: `onyx-tcon-fpga`, DRM-устройство `onyx-epd`) даёт в sysfs
//! устройства каталог управления:
//! - `panel` — тип панели: `kaleido` (цветной фильтр поверх ч/б E Ink) или `carta` (монохромная);
//! - `color` — `1` цветной вывод (CFA Kaleido накладывает FPGA), `0` — монохром;
//! - `update_mode` — режимы как у BOOX: чтение `hd [balanced] fast ultrafast x` (текущий в скобках),
//!   запись — имя (`quality` → hd и `a2` → ultrafast — синонимы);
//! - расширенные (числа): `contrast`, `saturation` (%), `a2_frames`, `a2_white`, `a2_black`,
//!   `full_every`, `full_pct` — см. [`Tuning`];
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

/// Режим обновления E Ink — имя из `update_mode` драйвера. Список режимов даёт драйвер
/// ([`Eink::update_modes`]); известные (как у BOOX: [`UpdateMode::KNOWN`]) — с названием
/// и пояснением, неизвестные показываются как есть. Старые имена — синонимы:
/// `quality` → `hd`, `a2` → `ultrafast` (драйвер их тоже принимает).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UpdateMode(String);

impl UpdateMode {
    /// Режимы драйвера onyx-tcon-fpga по порядку: от качества к скорости.
    pub const KNOWN: [&'static str; 5] = ["hd", "balanced", "fast", "ultrafast", "x"];

    /// Имя режима (без учёта регистра, с синонимами); `None` — пустое или не похожее на имя.
    pub fn from_name(s: &str) -> Option<UpdateMode> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return None;
        }
        let s = match s.as_str() {
            "quality" => "hd".to_string(),
            "a2" => "ultrafast".to_string(),
            _ => s,
        };
        Some(UpdateMode(s))
    }

    pub fn hd() -> UpdateMode {
        UpdateMode("hd".into())
    }

    pub fn balanced() -> UpdateMode {
        UpdateMode("balanced".into())
    }

    /// Известные режимы ([`UpdateMode::KNOWN`]) — если драйвер списка не дал.
    pub fn known() -> Vec<UpdateMode> {
        UpdateMode::KNOWN.iter().map(|s| UpdateMode((*s).into())).collect()
    }

    /// Имя в sysfs и в конфиге.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_known(&self) -> bool {
        UpdateMode::KNOWN.contains(&self.0.as_str())
    }

    /// Название для интерфейса (неизвестный режим — имя драйвера).
    pub fn title(&self) -> String {
        match self.0.as_str() {
            "hd" => "HD".into(),
            "balanced" => t!("Сбалансированный"),
            "fast" => t!("Быстрый"),
            "ultrafast" => t!("Сверхбыстрый"),
            "x" => "X".into(),
            other => other.to_string(),
        }
    }

    /// Пояснение для интерфейса (неизвестный режим — пусто).
    pub fn hint(&self) -> String {
        match self.0.as_str() {
            "hd" => t!("16 оттенков, частая полная очистка — текст и картинки"),
            "balanced" => t!("Оттенки, полная очистка реже — на каждый день"),
            "fast" => t!("Ч/б быстрее, серые слабее — листание и набор"),
            "ultrafast" => t!("Авторежим панели — видео и прокрутка"),
            "x" => t!("A2, самый быстрый — с артефактами"),
            _ => String::new(),
        }
    }

    /// Следующий по кругу в списке `modes` (плитка шторки); нет в списке — первый.
    pub fn next_in(&self, modes: &[UpdateMode]) -> UpdateMode {
        match modes.iter().position(|m| m == self) {
            Some(i) => modes[(i + 1) % modes.len()].clone(),
            None => modes.first().cloned().unwrap_or_else(|| self.clone()),
        }
    }
}

impl TryFrom<String> for UpdateMode {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        UpdateMode::from_name(&s).ok_or_else(|| format!("недопустимое имя режима E Ink: {s:?}"))
    }
}

impl From<UpdateMode> for String {
    fn from(m: UpdateMode) -> String {
        m.0
    }
}

impl std::fmt::Display for UpdateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Чтение `update_mode`: все режимы по порядку (синонимы сведены, без повторов) и текущий —
/// в скобках (`hd [balanced] fast ultrafast x`). Без скобок: одно имя — оно и текущее;
/// несколько — текущий неизвестен (драйвер так пишет, если `full_every`/`full_pct`
/// переопределены и не совпадают ни с одним режимом).
pub fn parse_update_modes(s: &str) -> (Vec<UpdateMode>, Option<UpdateMode>) {
    let mut all: Vec<UpdateMode> = Vec::new();
    let mut current = None;
    let mut n = 0;
    for tok in s.split_whitespace() {
        n += 1;
        let bracketed = tok.starts_with('[') && tok.ends_with(']') && tok.len() > 2;
        let name = if bracketed { &tok[1..tok.len() - 1] } else { tok };
        let Some(m) = UpdateMode::from_name(name) else { continue };
        if bracketed {
            current = Some(m.clone());
        }
        if !all.contains(&m) {
            all.push(m);
        }
    }
    if current.is_none() && n == 1 {
        current = all.first().cloned();
    }
    (all, current)
}

/// Текущий режим из чтения `update_mode` (см. [`parse_update_modes`]).
pub fn parse_update_mode(s: &str) -> Option<UpdateMode> {
    parse_update_modes(s).1
}

/// Расширенная настройка экрана E Ink — числовой атрибут драйвера (`[eink]` с тем же именем).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tuning {
    /// Контраст (регистр TCON 0x05), 0 — по умолчанию.
    Contrast,
    /// Насыщенность цветного кадра, % (100 — как есть, по умолчанию 150; только Kaleido).
    Saturation,
    /// Кадров волны A2 (регистр 0x04, 0..6); режим `x` ставит 2.
    A2Frames,
    /// Порог белого A2 (0x0C, по умолчанию 255).
    A2White,
    /// Порог чёрного A2 (0x0D, по умолчанию 0).
    A2Black,
    /// Полная очистка после N быстрых обновлений, 0 — нет (задаёт режим).
    FullEvery,
    /// Доля сменившегося экрана, %, с которой обновление сразу полное (задаёт режим; >100 — никогда).
    FullPct,
}

impl Tuning {
    pub const ALL: [Tuning; 7] =
        [Tuning::Contrast, Tuning::Saturation, Tuning::A2Frames, Tuning::A2White, Tuning::A2Black, Tuning::FullEvery, Tuning::FullPct];

    /// Имя атрибута sysfs и ключа `[eink]`.
    pub fn attr(self) -> &'static str {
        match self {
            Tuning::Contrast => "contrast",
            Tuning::Saturation => "saturation",
            Tuning::A2Frames => "a2_frames",
            Tuning::A2White => "a2_white",
            Tuning::A2Black => "a2_black",
            Tuning::FullEvery => "full_every",
            Tuning::FullPct => "full_pct",
        }
    }

    /// Допустимые значения (включительно).
    pub fn range(self) -> (u32, u32) {
        match self {
            Tuning::Contrast | Tuning::A2White | Tuning::A2Black => (0, 255),
            Tuning::Saturation => (0, 400),
            Tuning::A2Frames => (0, 6),
            Tuning::FullEvery => (0, 100),
            Tuning::FullPct => (0, 101),
        }
    }

    /// Значение драйвера по умолчанию; `None` — его задаёт режим обновления
    /// (`a2_frames`, `full_every`, `full_pct`: восстанавливаются повторной записью режима).
    pub fn default_value(self) -> Option<u32> {
        match self {
            Tuning::Contrast => Some(0),
            Tuning::Saturation => Some(150),
            Tuning::A2White => Some(255),
            Tuning::A2Black => Some(0),
            Tuning::A2Frames | Tuning::FullEvery | Tuning::FullPct => None,
        }
    }

    /// Только для цветной панели.
    pub fn color_only(self) -> bool {
        self == Tuning::Saturation
    }

    /// Название для интерфейса.
    pub fn title(self) -> String {
        match self {
            Tuning::Contrast => t!("Контраст"),
            Tuning::Saturation => t!("Насыщенность цвета"),
            Tuning::A2Frames => t!("Кадров A2"),
            Tuning::A2White => t!("Порог белого A2"),
            Tuning::A2Black => t!("Порог чёрного A2"),
            Tuning::FullEvery => t!("Полная очистка через N обновлений"),
            Tuning::FullPct => t!("Полная очистка от доли экрана"),
        }
    }

    /// Пояснение для интерфейса.
    pub fn hint(self) -> String {
        match self {
            Tuning::Contrast => t!("Регистр контраста контроллера; 0 — по умолчанию"),
            Tuning::Saturation => t!("Усиление цвета под фильтром Kaleido, %; 100 — как есть"),
            Tuning::A2Frames => t!("Длина быстрой волны A2: больше — чище, но медленнее"),
            Tuning::A2White => t!("Яркость, начиная с которой точка в A2 белая"),
            Tuning::A2Black => t!("Яркость, до которой точка в A2 чёрная"),
            Tuning::FullEvery => t!("После стольких быстрых обновлений — полная очистка; 0 — никогда"),
            Tuning::FullPct => t!("Если сменилось столько % экрана — сразу полное обновление; 101 — никогда"),
        }
    }

    /// Значение для показа (`0` у очистки — «никогда», проценты).
    pub fn format(self, v: u32) -> String {
        match self {
            Tuning::FullEvery if v == 0 => t!("никогда"),
            Tuning::FullPct if v > 100 => t!("никогда"),
            Tuning::Saturation | Tuning::FullPct => format!("{v}%"),
            _ => v.to_string(),
        }
    }

    /// Значение из `[eink]`.
    pub fn get(self, cfg: &crate::config::EinkSettings) -> Option<u32> {
        match self {
            Tuning::Contrast => cfg.contrast.map(u32::from),
            Tuning::Saturation => cfg.saturation.map(u32::from),
            Tuning::A2Frames => cfg.a2_frames.map(u32::from),
            Tuning::A2White => cfg.a2_white.map(u32::from),
            Tuning::A2Black => cfg.a2_black.map(u32::from),
            Tuning::FullEvery => cfg.full_every,
            Tuning::FullPct => cfg.full_pct,
        }
    }

    /// Записать в `[eink]` (значение приводится к [`Tuning::range`]).
    pub fn put(self, cfg: &mut crate::config::EinkSettings, v: Option<u32>) {
        let v = v.map(|v| self.clamp(v));
        match self {
            Tuning::Contrast => cfg.contrast = v.map(|v| v as u8),
            Tuning::Saturation => cfg.saturation = v.map(|v| v as u16),
            Tuning::A2Frames => cfg.a2_frames = v.map(|v| v as u8),
            Tuning::A2White => cfg.a2_white = v.map(|v| v as u8),
            Tuning::A2Black => cfg.a2_black = v.map(|v| v as u8),
            Tuning::FullEvery => cfg.full_every = v,
            Tuning::FullPct => cfg.full_pct = v,
        }
    }

    pub fn clamp(self, v: u32) -> u32 {
        let (lo, hi) = self.range();
        v.clamp(lo, hi)
    }
}

/// Сырой режим волны драйвера onyx-tcon-fpga (атрибут `scheme`): `ui <hex>` — режим волны
/// UI_MODE_SET (0x10, 0x15, 0x12, 0x06, 0x07, 0x03…), `auto <n>` — авторежим FPGA
/// (AUTO_MODE_CTRL n: 4 — «сверхбыстрый», 3 — качественнее и медленнее), `gc <n>` — полная
/// волна. Чтение драйвера — `ui 0x15` / `auto 0x04`, запись — `ui 0x15` / `auto 4` (`%i`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Gc(u8),
    Ui(u8),
    Auto(u8),
}

impl Scheme {
    /// Разбор `ui 0x15`, `ui 21`, `auto 0x04`, `auto 4` (регистр и пробелы — любые).
    pub fn parse(s: &str) -> Option<Scheme> {
        let mut it = s.split_whitespace();
        let kind = it.next()?.to_ascii_lowercase();
        let v = it.next()?;
        if it.next().is_some() {
            return None;
        }
        let n = match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            Some(h) => u8::from_str_radix(h, 16).ok()?,
            None => v.parse::<u8>().ok()?,
        };
        match kind.as_str() {
            "gc" => Some(Scheme::Gc(n)),
            "ui" => Some(Scheme::Ui(n)),
            "auto" => Some(Scheme::Auto(n)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scheme::Gc(v) => write!(f, "gc 0x{v:02x}"),
            Scheme::Ui(v) => write!(f, "ui 0x{v:02x}"),
            Scheme::Auto(n) => write!(f, "auto {n}"),
        }
    }
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

    /// Текущий режим; `None` — нет файла или текущий не совпадает ни с одним режимом
    /// (переопределены `full_every`/`full_pct`).
    pub fn update_mode(&self) -> Option<UpdateMode> {
        parse_update_mode(&self.read("update_mode")?)
    }

    /// Режимы, которые знает драйвер (по порядку); пустое чтение — [`UpdateMode::known`].
    pub fn update_modes(&self) -> Vec<UpdateMode> {
        let all = self.read("update_mode").map(|s| parse_update_modes(&s).0).unwrap_or_default();
        if all.is_empty() { UpdateMode::known() } else { all }
    }

    /// Запись режима. Драйвер при этом сбрасывает `a2_frames`, `full_every`, `full_pct`
    /// к значениям режима — переопределения из конфига возвращает [`Eink::apply`].
    pub fn set_update_mode(&self, m: &UpdateMode) -> std::io::Result<()> {
        self.write("update_mode", m.as_str())
    }

    /// Есть ли у драйвера расширенная настройка.
    pub fn has(&self, t: Tuning) -> bool {
        self.dir.join(t.attr()).is_file()
    }

    /// Значение расширенной настройки; `None` — нет файла или драйвер не готов (`-1`).
    pub fn tuning(&self, t: Tuning) -> Option<u32> {
        self.read(t.attr())?.parse::<i64>().ok().and_then(|v| u32::try_from(v).ok())
    }

    /// Записать расширенную настройку (приводится к [`Tuning::range`]); нет файла — ничего.
    pub fn set_tuning(&self, t: Tuning, v: u32) -> std::io::Result<()> {
        if !self.has(t) {
            return Ok(());
        }
        self.write(t.attr(), &t.clamp(v).to_string())
    }

    /// «Сбросить по умолчанию»: настройки с умолчанием драйвера — к нему, остальные (задаёт
    /// режим) — повторной записью режима `mode` (нет — текущий или `balanced`).
    pub fn reset_tunings(&self, mode: Option<&UpdateMode>) -> std::io::Result<()> {
        for t in Tuning::ALL {
            if let Some(d) = t.default_value() {
                if self.has(t) && self.tuning(t) != Some(d) {
                    self.set_tuning(t, d)?;
                }
            }
        }
        let m = mode.cloned().or_else(|| self.update_mode()).unwrap_or_else(UpdateMode::balanced);
        self.set_update_mode(&m)
    }

    /// Есть ли у драйвера режим пера (файл `pen`).
    pub fn has_pen(&self) -> bool {
        self.dir.join("pen").is_file()
    }

    /// Режим пера включён; `None` — нет файла или драйвер не готов (`-1`).
    pub fn pen(&self) -> Option<bool> {
        match self.read("pen")?.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    /// Режим пера: 1 — быстрые ч/б обновления без ожидания прошлой волны и без автоочистки,
    /// 0 — выход (затронутое пером драйвер перерисует качественно). Нет файла `pen` — ничего
    /// не делает. Запись может ждать замок драйвера — с главного потока не звать.
    pub fn set_pen(&self, on: bool) -> std::io::Result<()> {
        if !self.has_pen() {
            return Ok(());
        }
        self.write("pen", if on { "1" } else { "0" })
    }

    /// Есть ли у драйвера сырой режим волны (`scheme`).
    pub fn has_scheme(&self) -> bool {
        self.dir.join("scheme").is_file()
    }

    /// Сырой режим волны (`scheme`): `ui 0x15` или `auto 0x04`; `None` — нет файла или не разобрать.
    pub fn scheme(&self) -> Option<Scheme> {
        Scheme::parse(&self.read("scheme")?)
    }

    /// Записать сырой режим (`ui <hex>` / `auto <n>`) — для подбора режимов; обычные режимы —
    /// [`Eink::set_update_mode`]. Запись может ждать замок драйвера — с главного потока не звать.
    pub fn set_scheme(&self, s: Scheme) -> std::io::Result<()> {
        self.write("scheme", &s.to_string())
    }

    /// Полная очистка экрана сейчас.
    pub fn refresh(&self) -> std::io::Result<()> {
        self.write("refresh", "1")
    }

    /// Применить выбор пользователя (`[eink]`): пишется только то, что задано и отличается
    /// от текущего (запись `color` перерисовывает экран). Цвет и насыщенность — только на
    /// цветной панели; расширенные — только если у драйвера есть атрибут. Порядок: цвет,
    /// режим, расширенные (режим сбрасывает часть из них).
    pub fn apply(&self, cfg: &crate::config::EinkSettings) -> std::io::Result<()> {
        if let Some(on) = cfg.color {
            if self.is_color_panel() && self.color() != Some(on) {
                self.set_color(on)?;
            }
        }
        if let Some(m) = &cfg.update_mode {
            if self.update_mode().as_ref() != Some(m) {
                self.set_update_mode(m)?;
            }
        }
        // после режима: запись режима сбрасывает a2_frames/full_every/full_pct
        let color = self.is_color_panel();
        for t in Tuning::ALL {
            let Some(v) = t.get(cfg) else { continue };
            if (t.color_only() && !color) || !self.has(t) {
                continue;
            }
            if self.tuning(t) != Some(t.clamp(v)) {
                self.set_tuning(t, v)?;
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

    fn m(s: &str) -> UpdateMode {
        UpdateMode::from_name(s).unwrap()
    }

    #[test]
    fn update_mode_parse() {
        let (all, cur) = parse_update_modes("hd [balanced] fast ultrafast x\n");
        assert_eq!(all.iter().map(|m| m.as_str()).collect::<Vec<_>>(), UpdateMode::KNOWN);
        assert_eq!(cur, Some(m("balanced")));
        assert_eq!(parse_update_mode("hd balanced fast ultrafast [x]"), Some(m("x")));
        assert_eq!(parse_update_mode("[hd] balanced"), Some(UpdateMode::hd()));
        // текущий неизвестен (переопределены full_every/full_pct) — список есть, текущего нет
        let (all, cur) = parse_update_modes("hd balanced fast ultrafast x");
        assert_eq!(all.len(), 5);
        assert_eq!(cur, None);
        assert_eq!(parse_update_mode("balanced"), Some(m("balanced")));
        assert_eq!(parse_update_mode(""), None);
        // старый драйвер: синонимы сводятся, без повторов
        let (all, cur) = parse_update_modes("[quality] balanced fast a2");
        assert_eq!(all, vec![m("hd"), m("balanced"), m("fast"), m("ultrafast")]);
        assert_eq!(cur, Some(UpdateMode::hd()));
        // неизвестный режим драйвера — как есть
        let (all, cur) = parse_update_modes("hd [night]");
        assert_eq!(all, vec![m("hd"), m("night")]);
        assert_eq!(cur.as_ref().map(|m| m.as_str()), Some("night"));
        assert!(!m("night").is_known());
        assert_eq!(m("night").title(), "night");
        assert!(m("x").is_known());
    }

    #[test]
    fn scheme_parse() {
        assert_eq!(Scheme::parse("ui 0x15\n"), Some(Scheme::Ui(0x15)));
        assert_eq!(Scheme::parse("auto 0x04"), Some(Scheme::Auto(4)));
        assert_eq!(Scheme::parse(" AUTO  7 "), Some(Scheme::Auto(7)));
        assert_eq!(Scheme::parse("gc 0x02"), Some(Scheme::Gc(2)));
        assert_eq!(Scheme::parse("ui"), None);
        assert_eq!(Scheme::parse("ui 0x1ff"), None);
        assert_eq!(Scheme::parse("x 1"), None);
        assert_eq!(Scheme::Ui(0x06).to_string(), "ui 0x06");
        assert_eq!(Scheme::Auto(4).to_string(), "auto 4");
        assert_eq!(Scheme::parse(&Scheme::Ui(0x10).to_string()), Some(Scheme::Ui(0x10)));
    }

    #[test]
    fn update_mode_names_and_cycle() {
        assert_eq!(m("Quality"), UpdateMode::hd());
        assert_eq!(m(" A2 "), m("ultrafast"));
        assert_eq!(UpdateMode::from_name(""), None);
        assert_eq!(UpdateMode::from_name("a b"), None);
        assert_eq!(UpdateMode::from_name("[hd]"), None);
        let all = UpdateMode::known();
        assert_eq!(m("hd").next_in(&all), m("balanced"));
        assert_eq!(m("x").next_in(&all), m("hd"));
        assert_eq!(m("night").next_in(&all), m("hd"));
        assert_eq!(m("hd").title(), "HD");
    }

    #[test]
    fn update_mode_serde() {
        #[derive(Deserialize, Serialize)]
        struct T {
            m: UpdateMode,
        }
        assert_eq!(toml::from_str::<T>("m = \"a2\"").unwrap().m, m("ultrafast"));
        assert_eq!(toml::from_str::<T>("m = \"quality\"").unwrap().m, UpdateMode::hd());
        assert_eq!(toml::from_str::<T>("m = \"x\"").unwrap().m, m("x"));
        assert!(toml::from_str::<T>("m = \"\"").is_err());
        assert_eq!(toml::to_string(&T { m: m("ultrafast") }).unwrap().trim(), "m = \"ultrafast\"");
    }

    #[test]
    fn tuning_ranges_and_config() {
        let mut cfg = crate::config::EinkSettings::default();
        for t in Tuning::ALL {
            assert_eq!(t.get(&cfg), None);
            let (lo, hi) = t.range();
            t.put(&mut cfg, Some(hi + 1000));
            assert_eq!(t.get(&cfg), Some(hi), "{t:?}");
            t.put(&mut cfg, Some(lo));
            assert_eq!(t.get(&cfg), Some(lo));
            t.put(&mut cfg, None);
            assert_eq!(t.get(&cfg), None);
        }
        assert_eq!(Tuning::FullEvery.format(0), t!("никогда"));
        assert_eq!(Tuning::FullPct.format(101), t!("никогда"));
        assert_eq!(Tuning::Saturation.format(150), "150%");
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
            fs::write(dev.join("update_mode"), "hd [balanced] fast ultrafast x\n").unwrap();
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
        assert_eq!(e.update_mode(), Some(UpdateMode::balanced()));
        assert_eq!(e.update_modes(), UpdateMode::known());
        // старое имя в конфиге — синоним
        let cfg: crate::config::EinkSettings = toml::from_str("color = true\nupdate_mode = \"a2\"").unwrap();
        e.apply(&cfg).unwrap();
        assert_eq!(fs::read_to_string(e.dir().join("color")).unwrap(), "1");
        assert_eq!(fs::read_to_string(e.dir().join("update_mode")).unwrap(), "ultrafast");
        e.refresh().unwrap();
        assert_eq!(fs::read_to_string(e.dir().join("refresh")).unwrap(), "1");
        // режим пера: нет файла — ничего не делаем
        assert!(!e.has_pen());
        assert_eq!(e.pen(), None);
        e.set_pen(true).unwrap();
        assert!(!e.dir().join("pen").exists());
        fs::write(e.dir().join("pen"), "0\n").unwrap();
        assert!(e.has_pen());
        assert_eq!(e.pen(), Some(false));
        e.set_pen(true).unwrap();
        assert_eq!(e.pen(), Some(true));
        e.set_pen(false).unwrap();
        assert_eq!(fs::read_to_string(e.dir().join("pen")).unwrap(), "0");
        // монохромная панель: цвет и насыщенность не трогаем
        let sys = fake_sys(Some("carta"));
        let e = Eink::find_in(sys.path()).unwrap();
        fs::write(e.dir().join("saturation"), "150\n").unwrap();
        let cfg = crate::config::EinkSettings { color: Some(true), saturation: Some(300), ..Default::default() };
        e.apply(&cfg).unwrap();
        assert_eq!(e.color(), Some(false));
        assert_eq!(e.tuning(Tuning::Saturation), Some(150));
    }

    #[test]
    fn eink_tunings_apply_and_reset() {
        let sys = fake_sys(Some("kaleido"));
        let e = Eink::find_in(sys.path()).unwrap();
        let dir = e.dir().to_path_buf();
        let rd = |n: &str| fs::read_to_string(dir.join(n)).unwrap().trim().to_string();
        // у драйвера есть всё, кроме full_pct
        for (n, v) in [("contrast", "0"), ("saturation", "150"), ("a2_frames", "0"), ("a2_white", "255"), ("a2_black", "0"), ("full_every", "25")] {
            fs::write(dir.join(n), format!("{v}\n")).unwrap();
        }
        assert!(e.has(Tuning::Contrast) && !e.has(Tuning::FullPct));
        assert_eq!(e.tuning(Tuning::A2White), Some(255));
        assert_eq!(e.tuning(Tuning::FullPct), None);
        // драйвер не готов
        fs::write(dir.join("contrast"), "-1\n").unwrap();
        assert_eq!(e.tuning(Tuning::Contrast), None);
        fs::write(dir.join("contrast"), "0\n").unwrap();

        let cfg: crate::config::EinkSettings = toml::from_str(
            "update_mode = \"x\"\ncontrast = 3\nsaturation = 200\na2_frames = 4\na2_white = 200\na2_black = 40\nfull_every = 7\nfull_pct = 60\n",
        )
        .unwrap();
        e.apply(&cfg).unwrap();
        assert_eq!(rd("update_mode"), "x");
        assert_eq!(rd("contrast"), "3");
        assert_eq!(rd("saturation"), "200");
        assert_eq!(rd("a2_frames"), "4");
        assert_eq!(rd("a2_white"), "200");
        assert_eq!(rd("a2_black"), "40");
        assert_eq!(rd("full_every"), "7");
        assert!(!dir.join("full_pct").exists(), "нет атрибута — не создаём");
        // совпадающее не переписывается: файлы только для чтения — запись дала бы ошибку
        use std::os::unix::fs::PermissionsExt;
        fs::write(dir.join("update_mode"), "hd balanced fast ultrafast [x]\n").unwrap();
        let ro = ["update_mode", "contrast", "saturation", "a2_frames", "a2_white", "a2_black", "full_every"];
        for n in ro {
            fs::set_permissions(dir.join(n), fs::Permissions::from_mode(0o444)).unwrap();
        }
        e.apply(&cfg).unwrap();
        for n in ro {
            fs::set_permissions(dir.join(n), fs::Permissions::from_mode(0o644)).unwrap();
        }
        // запись приводится к диапазону
        e.set_tuning(Tuning::A2Frames, 99).unwrap();
        assert_eq!(rd("a2_frames"), "6");
        // сброс: умолчания драйвера + повторная запись режима
        e.reset_tunings(None).unwrap();
        assert_eq!(rd("contrast"), "0");
        assert_eq!(rd("saturation"), "150");
        assert_eq!(rd("a2_white"), "255");
        assert_eq!(rd("a2_black"), "0");
        assert_eq!(rd("update_mode"), "x");
        e.reset_tunings(Some(&UpdateMode::hd())).unwrap();
        assert_eq!(rd("update_mode"), "hd");
    }
}
