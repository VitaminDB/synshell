//! Местоположение для программ: выключатель (оболочка — агент GeoClue, `[location]`) и запомненные ответы
//! программам из песочницы.

use toml_edit::{Array, Value};

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

fn write(key: &'static str, v: &[String]) {
    let mut a = Array::new();
    for s in v {
        a.push(s.as_str());
    }
    set(&op!["location", key], Value::Array(a));
    state::bump();
}

pub fn location() -> W {
    let l = store::config().location.clone();
    let mut body = vec![group(
        "Доступ",
        vec![switch_row(
            "Местоположение для программ",
            "Спутники (GNSS модема), Wi-Fi и сеть через GeoClue; выключено — не получает никто",
            op!["location", "enabled"],
            l.enabled,
        )],
    )];
    for (title, key, list) in [("Разрешено", "allowed", &l.allowed), ("Запрещено", "denied", &l.denied)] {
        if list.is_empty() {
            continue;
        }
        let rows = list
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let name = synshell_common::xdg::app_by_id(id).map(|e| e.name).unwrap_or_else(|| id.clone());
                let all = list.clone();
                row(
                    &name,
                    id,
                    danger_icon_button(icons::DELETE, move || {
                        let mut v = all.clone();
                        v.remove(i);
                        write(key, &v);
                    }),
                )
            })
            .collect();
        body.push(group(title, rows));
    }
    body.push(note(
        "Обычные программы получают местоположение, пока доступ включён: GeoClue не может проверить, кто они. \
         Программы из песочницы (Flatpak) оболочка спрашивает при первом запросе; ответы «Разрешить» и «Запретить» \
         запоминаются здесь, «Только сейчас» — до перезапуска оболочки.",
    ));
    page("Местоположение", "Кому доступно местоположение телефона", body)
}
