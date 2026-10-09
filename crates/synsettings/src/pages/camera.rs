//! Камера: решения портала Camera (xdg-desktop-portal хранит их в своём хранилище разрешений, WirePlumber по ним
//! открывает программам узлы камер PipeWire). Вопрос задаёт оболочка при первом запросе программы; здесь —
//! разрешить, запретить или забыть ответ (спросить снова).

use syngui::prelude::*;
use synsystem::portal_access;

use crate::state;
use crate::ui::*;

fn apply(what: &str, r: std::result::Result<(), String>) {
    if let Err(e) = r {
        state::toast(format!("{what}: {e}"));
    }
    state::bump();
}

pub fn camera() -> W {
    let mut body = Vec::new();
    match portal_access::camera_permissions() {
        Err(e) => body.push(note(&t!("Хранилище разрешений порталов недоступно: {e}", e = e))),
        Ok(list) if list.is_empty() => body.push(note(&t!("Программы ещё не запрашивали камеру."))),
        Ok(list) => {
            let rows = list
                .into_iter()
                .map(|(id, allowed)| {
                    let name = if id.is_empty() {
                        t!("Программы вне песочницы").to_string()
                    } else {
                        synshell_common::xdg::app_by_id(&id).map(|e| e.name).unwrap_or_else(|| id.clone())
                    };
                    let hint = if allowed { t!("Разрешено") } else { t!("Запрещено") };
                    let (a, b) = (id.clone(), id.clone());
                    let actions = Row::new()
                        .gap(6.0)
                        .child(button(if allowed { t!("Запретить") } else { t!("Разрешить") }, move || {
                            apply(&t!("Камера"), portal_access::set_camera_permission(&a, !allowed))
                        }))
                        .child(danger_icon_button(icons::DELETE, move || apply(&t!("Камера"), portal_access::forget_camera_permission(&b))));
                    row(&name, hint, actions)
                })
                .collect();
            body.push(group(&t!("Ответы программам"), rows));
        }
    }
    body.push(note(
        &t!("Программы просят камеру через портал (браузеры, Flatpak); при первом запросе оболочка спрашивает, ответ \
         запоминается здесь. Удалить ответ — спросить снова. Программы вне песочницы портал не различает: для них \
         одно общее решение. Камеры телефона — узлы PipeWire «Основная», «Фронтальная» и «Широкоугольная камера»."),
    ));
    page(t!("Камера"), t!("Каким программам доступна камера"), body)
}
