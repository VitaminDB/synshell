//! Язык интерфейса программ synshell: `[general] language` в config.toml.
//! Пусто — как в системе (LANG). Выбор сразу применяется в этом окне
//! (корень подписан на смену языка), оболочка перечитывает конфиг сама.

use syngui::prelude::*;

use crate::op;
use crate::store;
use crate::ui::*;

/// Языки для выбора: каталоги переводов и язык исходных строк (русский);
/// подписи — названия на самом языке.
fn choices() -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        syngui::i18n::languages().into_iter().map(|l| (l.tag.tag().to_string(), l.name)).collect();
    if !list.iter().any(|(tag, _)| syngui::i18n::Lang::new(tag).base() == synshell_common::i18n::SOURCE) {
        list.push((synshell_common::i18n::SOURCE.to_string(), "Русский".to_string()));
    }
    list.sort_by(|a, b| a.1.cmp(&b.1));
    list
}

fn apply(tag: &str) {
    if tag.is_empty() {
        unset(&op!["general", "language"]);
    } else {
        set(&op!["general", "language"], tag.to_string());
    }
    // Это окно — сразу; подписанные на язык части интерфейса перестраиваются.
    synshell_common::i18n::apply(&store::config());
}

pub fn language() -> W {
    let c = store::config();
    let current = c.general.language.trim().to_string();
    let system = syngui::i18n::system_language();
    let system_name = choices()
        .into_iter()
        .find(|(tag, _)| syngui::i18n::Lang::new(tag).base() == system.base())
        .map(|(_, name)| name)
        .unwrap_or_else(|| system.to_string());
    let mut dd = Dropdown::new()
        .width(260.0)
        .item(DropdownItem::new("", t!("Как в системе ({name})", name = system_name)));
    for (tag, name) in choices() {
        dd = dd.item(DropdownItem::new(tag, name));
    }
    let dd = dd.selected(current).on_change(|v: &str| apply(v));
    let mut body = vec![group(
        "",
        vec![row(t!("Язык интерфейса"), t!("Программы synshell: оболочка, параметры, проводник и другие"), dd)],
    )];
    if std::env::var("SYNSHELL_LANG").is_ok_and(|l| !l.trim().is_empty()) {
        body.push(note(t!(
            "Сейчас язык задан переменной окружения SYNSHELL_LANG — она главнее этой настройки."
        )));
    }
    body.push(note(t!(
        "Остальные программы (GTK, Qt, консоль) берут язык системы — переменную LANG; её задают локали в /etc/locale.conf."
    )));
    page(t!("Язык"), t!("Язык интерфейса программ synshell."), body)
}
