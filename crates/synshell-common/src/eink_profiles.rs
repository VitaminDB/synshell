//! Профили экрана E Ink: именованные наборы режима волны (`scheme`) и необязательных
//! настроек драйвера (`full_every`, `full_pct`, A2, контраст, насыщенность).
//!
//! Встроенный только один профиль — «По умолчанию» ([`DEFAULT_ID`]: `auto 5`, без автоочистки);
//! он не меняется и не удаляется. Остальные создаёт и называет пользователь в «Тесте экрана
//! E Ink» (syn-epdtest); выбирают — там же, в шторке и в «Параметрах».
//!
//! Хранилище — общий файл [`system_file`] (`/var/lib/synshell/eink-profiles.toml`, `root:video
//! 0664`, создаёт `synlogin daemon`): профили видят экран входа и все пользователи. Общий
//! недоступен (компьютер, проверка) — [`user_file`] в `$XDG_DATA_HOME/synshell`. Формат:
//!
//! ```toml
//! [[profile]]
//! name = "Чтение"
//! scheme = "ui 0x15"
//! full_every = 6
//! full_pct = 50
//! ```
//!
//! Активный профиль — `[eink] profile = "<имя>"` у пользователя (`"default"` — встроенный) и
//! в `/var/lib/synshell/eink` ([`crate::eink_state`]) для экрана входа. Применение —
//! [`crate::display::Eink::apply_profile`]: сначала `scheme`, затем только заданные поля;
//! `update_mode` не пишется (его запись в драйвере сбрасывает `scheme`, `full_*`, `a2_frames`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::display::{Scheme, Tuning};
use crate::t;

/// Имя встроенного профиля в конфиге и в файле состояния; в интерфейсе — «По умолчанию».
pub const DEFAULT_ID: &str = "default";
/// Подмена общего файла (проверка): `SYNSHELL_EINK_PROFILES_FILE=/tmp/x`, пустое — без общего.
pub const FILE_ENV: &str = "SYNSHELL_EINK_PROFILES_FILE";
/// Предел длины имени профиля, символов.
pub const NAME_MAX: usize = 48;

/// Профиль экрана E Ink. Не заданное (`None`) при применении не пишется — остаётся как было.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Режим волны: `ui <hex>` (волну выбирает драйвер командой) или `auto <n>` (авторежим FPGA).
    pub scheme: Scheme,
    /// Полная очистка после N обновлений, 0 — никогда.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_every: Option<u32>,
    /// Доля экрана, %, с которой обновление сразу полное; 101 — никогда.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_pct: Option<u32>,
    /// Кадров волны A2 (0..6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2_frames: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2_white: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2_black: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<u8>,
    /// Насыщенность цвета, % (только цветная панель).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<u16>,
}

impl Profile {
    /// Встроенный «По умолчанию»: авторежим FPGA 5, без автоочистки (лучший по оценке на Note Air 4C).
    pub fn builtin() -> Profile {
        Profile {
            name: DEFAULT_ID.into(),
            scheme: Scheme::Auto(5),
            full_every: Some(0),
            full_pct: Some(101),
            a2_frames: None,
            a2_white: None,
            a2_black: None,
            contrast: None,
            saturation: None,
        }
    }

    pub fn is_default(&self) -> bool {
        self.name == DEFAULT_ID
    }

    /// Название для интерфейса.
    pub fn title(&self) -> String {
        if self.is_default() {
            t!("По умолчанию")
        } else {
            self.name.clone()
        }
    }

    /// Значение настройки.
    pub fn get(&self, t: Tuning) -> Option<u32> {
        match t {
            Tuning::Contrast => self.contrast.map(u32::from),
            Tuning::Saturation => self.saturation.map(u32::from),
            Tuning::A2Frames => self.a2_frames.map(u32::from),
            Tuning::A2White => self.a2_white.map(u32::from),
            Tuning::A2Black => self.a2_black.map(u32::from),
            Tuning::FullEvery => self.full_every,
            Tuning::FullPct => self.full_pct,
        }
    }

    /// Задать (приводится к [`Tuning::range`]) или снять (`None`) настройку.
    pub fn put(&mut self, t: Tuning, v: Option<u32>) {
        let v = v.map(|v| t.clamp(v));
        match t {
            Tuning::Contrast => self.contrast = v.map(|v| v as u8),
            Tuning::Saturation => self.saturation = v.map(|v| v as u16),
            Tuning::A2Frames => self.a2_frames = v.map(|v| v as u8),
            Tuning::A2White => self.a2_white = v.map(|v| v as u8),
            Tuning::A2Black => self.a2_black = v.map(|v| v as u8),
            Tuning::FullEvery => self.full_every = v,
            Tuning::FullPct => self.full_pct = v,
        }
    }

    /// Кратко для списка: `auto 5 · очистка никогда`.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.scheme.to_string()];
        match (self.full_every, self.full_pct) {
            (Some(0), Some(p)) if p > 100 => parts.push(t!("без автоочистки")),
            (fe, fp) => {
                if let Some(n) = fe {
                    parts.push(if n == 0 { t!("очистка по счёту: никогда") } else { t!("очистка через {n}", n = n) });
                }
                if let Some(p) = fp {
                    parts.push(if p > 100 { t!("по доле: никогда") } else { t!("полное от {p}%", p = p) });
                }
            }
        }
        let rest = Tuning::ALL.iter().filter(|t| !matches!(t, Tuning::FullEvery | Tuning::FullPct) && self.get(**t).is_some()).count();
        if rest > 0 {
            parts.push(t!("ещё {n}", n = rest));
        }
        parts.join(" · ")
    }
}

/// Файл профилей.
#[derive(Debug, Default, Serialize, Deserialize)]
struct FileData {
    #[serde(default, rename = "profile")]
    profiles: Vec<Profile>,
}

/// Ошибка имени профиля (текст — для интерфейса).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameError(pub String);

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Профили: встроенный и пользовательские (в порядке файла).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profiles {
    /// Пользовательские профили (без встроенного).
    pub user: Vec<Profile>,
}

impl Profiles {
    /// Разбор файла: неверные записи (без имени, с неразборчивым `scheme`, повтор имени,
    /// имя встроенного) пропускаются, остальные остаются.
    pub fn parse(text: &str) -> Profiles {
        let mut out = Profiles::default();
        let Ok(v) = text.parse::<toml::Table>() else {
            tracing::warn!("профили E Ink: файл не разобран");
            return out;
        };
        let Some(list) = v.get("profile").and_then(|p| p.as_array()) else { return out };
        for item in list {
            match item.clone().try_into::<Profile>() {
                Ok(mut p) => {
                    p.name = p.name.trim().to_string();
                    if out.check_name(&p.name, None).is_ok() {
                        let p = sanitize(p);
                        out.user.push(p);
                    } else {
                        tracing::warn!(name = %p.name, "профиль E Ink пропущен: имя пустое, повторяется или занято");
                    }
                }
                Err(e) => tracing::warn!(%e, "профиль E Ink пропущен"),
            }
        }
        out
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(&FileData { profiles: self.user.clone() }).unwrap_or_default()
    }

    /// Все по порядку: встроенный первым.
    pub fn all(&self) -> Vec<Profile> {
        std::iter::once(Profile::builtin()).chain(self.user.iter().cloned()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Profile> {
        if name == DEFAULT_ID {
            return Some(Profile::builtin());
        }
        self.user.iter().find(|p| p.name == name).cloned()
    }

    /// Профиль по имени из конфига; нет имени или такого профиля — встроенный.
    pub fn resolve(&self, name: Option<&str>) -> Profile {
        name.and_then(|n| self.get(n)).unwrap_or_else(Profile::builtin)
    }

    /// Следующий по кругу после `name` (плитка шторки).
    pub fn next_after(&self, name: &str) -> Profile {
        let all = self.all();
        match all.iter().position(|p| p.name == name) {
            Some(i) => all[(i + 1) % all.len()].clone(),
            None => all[0].clone(),
        }
    }

    /// Проверка имени нового или переименованного (`except` — его прежнее имя): не пустое,
    /// без управляющих символов, не длиннее [`NAME_MAX`], не занято (без учёта регистра)
    /// ни другим профилем, ни встроенным. Возвращает имя без пробелов по краям.
    pub fn check_name(&self, name: &str, except: Option<&str>) -> Result<String, NameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError(t!("Имя не может быть пустым")));
        }
        if name.chars().count() > NAME_MAX {
            return Err(NameError(t!("Имя длиннее {n} символов", n = NAME_MAX)));
        }
        if name.chars().any(char::is_control) {
            return Err(NameError(t!("В имени не должно быть управляющих символов")));
        }
        let low = name.to_lowercase();
        let reserved = [DEFAULT_ID.to_string(), "по умолчанию".to_string(), t!("По умолчанию").to_lowercase()];
        if reserved.contains(&low) {
            return Err(NameError(t!("Это имя встроенного профиля")));
        }
        if self.user.iter().any(|p| Some(p.name.as_str()) != except && p.name.to_lowercase() == low) {
            return Err(NameError(t!("Профиль «{name}» уже есть", name = name)));
        }
        Ok(name.to_string())
    }

    /// Свободное имя на основе `base`: `base`, `base 2`, `base 3`…
    pub fn unique_name(&self, base: &str) -> String {
        let base = base.trim();
        let base: String = base.chars().take(NAME_MAX - 4).collect();
        if self.check_name(&base, None).is_ok() {
            return base;
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| self.check_name(n, None).is_ok()).unwrap_or_default()
    }

    /// Добавить копию `from` под именем `name` в конец; возвращает новый профиль.
    pub fn add(&mut self, name: &str, from: &Profile) -> Result<Profile, NameError> {
        let name = self.check_name(name, None)?;
        let p = Profile { name, ..from.clone() };
        self.user.push(p.clone());
        Ok(p)
    }

    /// Переименовать пользовательский профиль; возвращает новое имя.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<String, NameError> {
        if old == DEFAULT_ID {
            return Err(NameError(t!("Встроенный профиль не переименовывается")));
        }
        let new = self.check_name(new, Some(old))?;
        match self.user.iter_mut().find(|p| p.name == old) {
            Some(p) => {
                p.name = new.clone();
                Ok(new)
            }
            None => Err(NameError(t!("Профиля «{name}» нет", name = old))),
        }
    }

    /// Удалить пользовательский профиль (встроенный не удаляется).
    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.user.len();
        self.user.retain(|p| p.name != name);
        self.user.len() != n
    }

    /// Заменить значения профиля с тем же именем (встроенный не меняется).
    pub fn update(&mut self, p: &Profile) -> bool {
        match self.user.iter_mut().find(|x| x.name == p.name) {
            Some(x) if *x != *p => {
                *x = sanitize(p.clone());
                true
            }
            _ => false,
        }
    }
}

/// Значения — в пределах драйвера.
fn sanitize(mut p: Profile) -> Profile {
    for t in Tuning::ALL {
        let v = p.get(t);
        p.put(t, v);
    }
    p
}

// ── Файл ─────────────────────────────────────────────────────────────────────────────────

pub fn system_file() -> Option<PathBuf> {
    match std::env::var_os(FILE_ENV) {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(PathBuf::from(v)),
        None => Some(PathBuf::from("/var/lib/synshell/eink-profiles.toml")),
    }
}

pub fn user_file() -> PathBuf {
    crate::paths::data_home().join("synshell").join("eink-profiles.toml")
}

/// Общий файл с правами для сеансов (от root, `synlogin daemon`).
pub fn ensure_system_file() -> std::io::Result<()> {
    match system_file() {
        Some(f) => crate::rotation_state::ensure_shared_file(&f),
        None => Ok(()),
    }
}

/// Профили из более свежего из файлов (общего и пользователя); нет — только встроенный.
pub fn load() -> Profiles {
    load_from(system_file().as_deref(), &user_file())
}

pub fn load_from(system: Option<&Path>, user: &Path) -> Profiles {
    let read = |p: &Path| -> Option<(std::time::SystemTime, String)> {
        let text = std::fs::read_to_string(p).ok()?;
        let mtime = std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        Some((mtime, text))
    };
    system.into_iter().chain([user]).filter_map(read).max_by_key(|(t, _)| *t).map(|(_, s)| Profiles::parse(&s)).unwrap_or_default()
}

/// Записать профили: в общий файл, если можно, иначе в файл пользователя.
pub fn save(p: &Profiles) -> std::io::Result<PathBuf> {
    save_to(system_file().as_deref(), &user_file(), p)
}

pub fn save_to(system: Option<&Path>, user: &Path, p: &Profiles) -> std::io::Result<PathBuf> {
    crate::eink_state::write_shared(system, user, &p.to_toml())
}

// ── Активный профиль ─────────────────────────────────────────────────────────────────────

/// Имя активного профиля: `[eink] profile`, иначе последнее запомненное для экрана входа,
/// иначе встроенный.
pub fn active_name(cfg: &crate::config::EinkSettings) -> String {
    cfg.profile.clone().or_else(|| crate::eink_state::load().profile).unwrap_or_else(|| DEFAULT_ID.to_string())
}

/// Выбрать активный профиль в `config.toml` (`[eink] profile`). Устаревшие ключи `[eink]`
/// (`update_mode` и настройки драйвера — до профилей) при этом убираются: они больше не
/// применяются. Драйвер не трогает — это делает вызывающий ([`crate::display::Eink::apply_profile`])
/// и synwm при перечитывании конфига.
pub fn set_active(name: &str) -> anyhow::Result<bool> {
    let name = name.to_string();
    crate::config_edit::edit(move |doc| {
        let root = doc.as_table_mut();
        if !root.contains_key("eink") {
            root.insert("eink", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let Some(t) = root.get_mut("eink").and_then(toml_edit::Item::as_table_mut) else { return false };
        let mut changed = false;
        for k in LEGACY_KEYS {
            changed |= t.remove(k).is_some();
        }
        if t.get("profile").and_then(|v| v.as_str()) != Some(name.as_str()) {
            t.insert("profile", toml_edit::value(name));
            changed = true;
        }
        changed
    })
}

/// Ключи `[eink]` до профилей: разбираются, но не применяются.
pub const LEGACY_KEYS: [&str; 8] = ["update_mode", "contrast", "saturation", "a2_frames", "a2_white", "a2_black", "full_every", "full_pct"];

#[cfg(test)]
mod tests {
    use super::*;

    fn named(n: &str, s: Scheme) -> Profile {
        Profile { name: n.into(), scheme: s, ..Profile::builtin() }
    }

    #[test]
    fn builtin_and_resolve() {
        let p = Profiles::default();
        assert_eq!(p.all().len(), 1);
        let d = p.resolve(None);
        assert!(d.is_default());
        assert_eq!(d.scheme, Scheme::Auto(5));
        assert_eq!((d.full_every, d.full_pct), (Some(0), Some(101)));
        assert_eq!(p.resolve(Some("нет такого")), d);
        assert_eq!(p.get(DEFAULT_ID), Some(d.clone()));
        assert_eq!(d.summary(), format!("auto 5 · {}", t!("без автоочистки")));
    }

    #[test]
    fn names_add_rename_remove() {
        let mut p = Profiles::default();
        let base = Profile::builtin();
        assert!(p.add("", &base).is_err());
        assert!(p.add("default", &base).is_err());
        assert!(p.add("По умолчанию", &base).is_err());
        assert!(p.add("a\nb", &base).is_err());
        assert!(p.add(&"я".repeat(NAME_MAX + 1), &base).is_err());
        let r = p.add("  Чтение ", &base).unwrap();
        assert_eq!(r.name, "Чтение");
        assert_eq!(r.scheme, base.scheme);
        assert!(p.add("чтение", &base).is_err(), "без учёта регистра");
        p.add("Видео", &named("x", Scheme::Ui(0x07))).unwrap();
        assert_eq!(p.get("Видео").unwrap().scheme, Scheme::Ui(0x07));
        assert_eq!(p.unique_name("Чтение"), "Чтение 2");
        assert_eq!(p.unique_name("Новый"), "Новый");
        // по кругу: встроенный → Чтение → Видео → встроенный
        assert_eq!(p.next_after(DEFAULT_ID).name, "Чтение");
        assert_eq!(p.next_after("Видео").name, DEFAULT_ID);
        assert_eq!(p.next_after("нет").name, DEFAULT_ID);
        // переименование: в себя (другой регистр) можно, в занятое — нет
        assert_eq!(p.rename("Чтение", "ЧТЕНИЕ").unwrap(), "ЧТЕНИЕ");
        assert!(p.rename("ЧТЕНИЕ", "видео").is_err());
        assert!(p.rename(DEFAULT_ID, "x").is_err());
        assert!(p.rename("нет", "y").is_err());
        // правка: значения приводятся к пределам; встроенный не меняется
        let mut v = p.get("Видео").unwrap();
        v.put(Tuning::A2Frames, Some(3));
        v.a2_frames = Some(99);
        assert!(p.update(&v));
        assert_eq!(p.get("Видео").unwrap().a2_frames, Some(6));
        assert!(!p.update(&Profile { full_every: Some(5), ..Profile::builtin() }));
        assert!(!p.remove(DEFAULT_ID));
        assert!(p.remove("ЧТЕНИЕ"));
        assert_eq!(p.all().iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), [DEFAULT_ID, "Видео"]);
    }

    #[test]
    fn parse_lenient_and_roundtrip() {
        let text = r#"
[[profile]]
name = "Чтение"
scheme = "ui 0x15"
full_every = 6
full_pct = 50
future_key = 1

[[profile]]
name = "плохой"
scheme = "что-то"

[[profile]]
name = "default"
scheme = "auto 4"

[[profile]]
name = "чтение"
scheme = "auto 3"

[[profile]]
name = "Быстро"
scheme = "auto 0x04"
a2_frames = 40
"#;
        let p = Profiles::parse(text);
        assert_eq!(p.user.len(), 2);
        assert_eq!(p.user[0], Profile { name: "Чтение".into(), scheme: Scheme::Ui(0x15), full_every: Some(6), full_pct: Some(50), ..named("", Scheme::Ui(0)).cleared() });
        assert_eq!(p.user[1].scheme, Scheme::Auto(4));
        assert_eq!(p.user[1].a2_frames, Some(6));
        let back = Profiles::parse(&p.to_toml());
        assert_eq!(back, p);
        assert!(p.to_toml().contains("scheme = \"ui 0x15\""));
        assert!(p.to_toml().contains("scheme = \"auto 4\""));
        assert_eq!(Profiles::parse("не toml ["), Profiles::default());
        assert_eq!(Profiles::parse(""), Profiles::default());
    }

    impl Profile {
        fn cleared(self) -> Profile {
            Profile { full_every: None, full_pct: None, a2_frames: None, a2_white: None, a2_black: None, contrast: None, saturation: None, ..self }
        }
    }

    #[test]
    fn file_save_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sys")).unwrap();
        let sys = dir.path().join("sys/eink-profiles.toml");
        let user = dir.path().join("data/synshell/eink-profiles.toml");
        assert_eq!(load_from(Some(&sys), &user), Profiles::default());
        let mut p = Profiles::default();
        p.add("Чтение", &Profile::builtin()).unwrap();
        assert_eq!(save_to(Some(&sys), &user, &p).unwrap(), sys);
        assert_eq!(load_from(Some(&sys), &user), p);
        // общего каталога нет — файл пользователя
        std::thread::sleep(std::time::Duration::from_millis(20));
        p.add("Видео", &Profile::builtin()).unwrap();
        assert_eq!(save_to(Some(&dir.path().join("nodir/x.toml")), &user, &p).unwrap(), user);
        assert_eq!(load_from(Some(&dir.path().join("nodir/x.toml")), &user), p);
        assert_eq!(load_from(None, &user), p);
    }
}
