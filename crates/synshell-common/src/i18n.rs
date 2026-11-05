//! Язык программ synshell. Код пишется по-русски, строки интерфейса —
//! `t!("…")`/`tn!`/`n_!` (syngui, ключ — сама строка), переводы — каталоги
//! `i18n/<язык>.lang` каждого крейта (`syngui/scripts/i18n-extract.py`).
//!
//! Язык: `SYNSHELL_LANG`, иначе `[general] language`, иначе язык системы
//! (`LANG`). Строки библиотек без интерфейса (synshell-common, synsystem,
//! synmodem) помечены [`n_!`](crate::n_) и переводятся общим каталогом
//! этого крейта там, где показываются (`syngui::i18n::t(s)`).

/// Общий каталог: строки synshell-common, synsystem, synmodem.
const SHARED: &[&str] = &synshell_tr::catalogs!("../i18n");

/// Язык исходных строк.
pub const SOURCE: &str = "ru";

/// Запрошенный язык интерфейса (до разрешения по каталогам).
pub fn requested(cfg: &crate::Config) -> String {
    if let Ok(l) = std::env::var("SYNSHELL_LANG") {
        if !l.trim().is_empty() {
            return l;
        }
    }
    let own = app_language();
    if !own.is_empty() {
        return own;
    }
    if !cfg.general.language.trim().is_empty() {
        return cfg.general.language.trim().to_string();
    }
    syngui::i18n::system_language().to_string()
}

/// Подключить переводы программы: общий каталог и свои (`include_str!`
/// каталогов крейта), язык — из конфига или системы.
pub fn init(catalogs: &[&'static str]) {
    syngui::i18n::set_source_language(SOURCE);
    syngui::i18n::register_catalogs(SHARED);
    syngui::i18n::register_catalogs(catalogs);
    // Строки библиотек (synsystem, synmodem, synshell-common) — этими же каталогами.
    synshell_tr::set_translator(syngui::i18n::t, |n, forms| syngui::i18n::tn_args(n, forms, &[]));
    let (cfg, _) = crate::Config::load();
    apply(&cfg);
}

/// Язык из конфига (перечитанного): подписанные части интерфейса
/// перестраиваются сами.
pub fn apply(cfg: &crate::Config) {
    let lang = requested(cfg);
    syngui::i18n::set_language(lang.as_str());
    // числа, описания типов файлов и т. п. в библиотеках — на том же языке
    synshell_tr::set_language(&lang);
    // Даты (strftime: дни недели, месяцы) — на том же языке, если язык выбран явно.
    if std::env::var("SYNSHELL_LANG").is_ok_and(|l| !l.trim().is_empty()) || !cfg.general.language.trim().is_empty() {
        set_time_locale(&lang);
    } else {
        // SAFETY: setlocale из главного потока; пустая строка — локаль окружения (LANG, LC_TIME).
        unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };
    }
}

/// Локаль дат для языка: `en` → `en_US.UTF-8` и т. п. (нужна в системе —
/// `locale.gen`); нет такой — остаётся прежняя.
fn set_time_locale(lang: &str) {
    const REGIONS: &[(&str, &str)] = &[
        ("en", "US"), ("ru", "RU"), ("uk", "UA"), ("de", "DE"), ("fr", "FR"), ("es", "ES"), ("it", "IT"),
        ("pt", "BR"), ("pl", "PL"), ("kk", "KZ"), ("tr", "TR"), ("zh", "CN"), ("ja", "JP"), ("ko", "KR"),
    ];
    let l = syngui::i18n::Lang::new(lang);
    let region = l.region().map(String::from).or_else(|| REGIONS.iter().find(|r| r.0 == l.base()).map(|r| r.1.to_string()));
    let mut names = Vec::new();
    if let Some(r) = &region {
        names.push(format!("{}_{r}.UTF-8", l.base()));
        names.push(format!("{}_{r}.utf8", l.base()));
    }
    names.push(format!("{}.UTF-8", l.base()));
    // Английский есть всегда — у локали C.
    if l.base() == "en" {
        names.push("C.UTF-8".into());
    }
    for name in names {
        let Ok(c) = std::ffi::CString::new(name) else { continue };
        // SAFETY: setlocale с корректной C-строкой; вызывается из главного потока при старте и перечитывании конфига.
        if !unsafe { libc::setlocale(libc::LC_TIME, c.as_ptr()) }.is_null() {
            return;
        }
    }
}

/// Язык этой программы: свой выбор в её настройках поверх общего языка
/// («Параметры → Язык»). Хранится отдельно от config.toml — `app-languages.toml`
/// в каталоге настроек (`имя_программы = "тег"`), чтобы новый ключ не ломал
/// конфиг у старых бинарников. Пусто — «Как в системе» (общий язык).
fn app_id() -> String {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
}

fn app_languages_path() -> std::path::PathBuf {
    crate::paths::config_dir().join("app-languages.toml")
}

fn app_languages() -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(app_languages_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

/// Свой язык программы (тег) или пусто — как в системе.
pub fn app_language() -> String {
    app_languages().remove(&app_id()).unwrap_or_default()
}

/// Выбрать язык программы (пусто — как в системе) и сразу применить.
pub fn set_app_language(tag: &str) {
    let mut all = app_languages();
    let id = app_id();
    if tag.trim().is_empty() {
        all.remove(&id);
    } else {
        all.insert(id, tag.trim().to_string());
    }
    let p = app_languages_path();
    let _ = std::fs::create_dir_all(crate::paths::config_dir());
    if let Err(e) = toml::to_string(&all).map_err(std::io::Error::other).and_then(|s| std::fs::write(&p, s)) {
        tracing::warn!("{}: {e}", p.display());
    }
    let (cfg, _) = crate::Config::load();
    apply(&cfg);
}

/// Выбор для списка: «Как в системе (язык)» и все языки каталогов — самоназваниями.
/// Значение «auto» — как в системе.
pub fn language_choices() -> Vec<(String, String)> {
    let (cfg, _) = crate::Config::load();
    let global = {
        let env = std::env::var("SYNSHELL_LANG").ok().filter(|l| !l.trim().is_empty());
        env.unwrap_or_else(|| if cfg.general.language.trim().is_empty() { syngui::i18n::system_language().to_string() } else { cfg.general.language.trim().to_string() })
    };
    let mut langs: Vec<(String, String)> =
        syngui::i18n::languages().into_iter().map(|l| (l.tag.tag().to_string(), l.name)).collect();
    langs.sort_by(|a, b| a.1.cmp(&b.1));
    let gl = syngui::i18n::Lang::new(&global);
    let gname = langs.iter().find(|(t, _)| syngui::i18n::Lang::new(t).base() == gl.base()).map(|(_, n)| n.clone()).unwrap_or(global);
    let mut out = vec![("auto".to_string(), syngui::i18n::t_args(crate::n_!("Как в системе ({lang})"), &[("lang", &gname)]))];
    out.extend(langs);
    out
}

/// Подписи строки «Язык» в настройках программ.
pub struct LanguageRow {
    /// «Язык».
    pub title: String,
    /// Пояснение.
    pub hint: String,
}

pub fn language_row() -> LanguageRow {
    LanguageRow {
        title: syngui::i18n::t(crate::n_!("Язык")),
        hint: syngui::i18n::t(crate::n_!("«Как в системе» — язык из Параметров")),
    }
}

/// Готовый список языка программы: «Как в системе» по умолчанию и все языки;
/// выбор применяется сразу. Вид — классом программы (`.class(…)`).
pub fn language_dropdown() -> syngui::prelude::Dropdown {
    use syngui::prelude::{Dropdown, DropdownItem};
    let cur = app_language();
    let choices = language_choices();
    let selected = if cur.is_empty() {
        "auto".to_string()
    } else {
        choices.iter().find(|(t, _)| syngui::i18n::Lang::new(t).base() == syngui::i18n::Lang::new(&cur).base()).map(|(t, _)| t.clone()).unwrap_or_else(|| "auto".into())
    };
    let mut d = Dropdown::new();
    for (tag, name) in choices {
        d = d.item(DropdownItem::new(tag, name));
    }
    d.selected(selected).on_change(|v| set_app_language(if v == "auto" { "" } else { v }))
}

/// Открыть «Параметры → Язык» (общий язык всех программ).
pub fn open_settings() {
    if let Err(e) = std::process::Command::new("synsettings").arg("--page").arg("language").spawn() {
        tracing::warn!("synsettings: {e}");
    }
}
