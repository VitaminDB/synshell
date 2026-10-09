//! Язык программ synshell. Код пишется по-русски, строки интерфейса —
//! `t!("…")`/`tn!`/`n_!` (syngui, ключ — сама строка), переводы — каталоги
//! `i18n/<язык>.lang` каждого крейта (`syngui/scripts/i18n-extract.py`).
//!
//! Язык: `SYNSHELL_LANG`, иначе `[general] language`, иначе язык системы
//! (`LANG`). Строки библиотек без интерфейса (synshell-common, synsystem,
//! synmodem) помечены [`n_!`](crate::n_) и переводятся общим каталогом
//! этого крейта там, где показываются (`syngui::i18n::t(s)`).

/// Общий каталог: строки synshell-common, synsystem, synmodem.
const SHARED: &[&str] = &[include_str!("../i18n/en.lang")];

/// Язык исходных строк.
pub const SOURCE: &str = "ru";

/// Запрошенный язык интерфейса (до разрешения по каталогам).
pub fn requested(cfg: &crate::Config) -> String {
    if let Ok(l) = std::env::var("SYNSHELL_LANG") {
        if !l.trim().is_empty() {
            return l;
        }
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
