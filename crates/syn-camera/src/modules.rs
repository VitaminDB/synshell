//! Модули камер телефона по пробе платформы (`/run/camera/status.json`, пишет syncamd): какие задние объективы
//! есть и исправны. Неисправный модуль (датчик не ответил по CCI) в HAL не попадает — в выборе камеры он
//! виден, но недоступен.

use syngui::t;
/// Задний объектив по роли в имени модуля (`…_wide` — основная, `_ultra` — широкоугольная, `_macro`).
#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub role: Role,
    pub name: String,
    /// Исправен (`status: ok`).
    pub ok: bool,
    /// Почему недоступен (по-русски).
    pub fault: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Main,
    Wide,
    Macro,
}

pub fn back_modules() -> Vec<Module> {
    let Ok(s) = std::fs::read_to_string("/run/camera/status.json") else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return Vec::new() };
    let mut out = Vec::new();
    for m in v["modules"].as_array().into_iter().flatten() {
        let name = m["name"].as_str().unwrap_or_default().to_string();
        let role = if name.ends_with("_macro") {
            Role::Macro
        } else if name.ends_with("_ultra") {
            Role::Wide
        } else if name.ends_with("_wide") {
            Role::Main
        } else {
            continue;
        };
        let status = m["status"].as_str().unwrap_or_default();
        let fault = match status {
            "ok" => String::new(),
            "no_response" => t!("Неисправна: датчик не отвечает").into(),
            "mismatch" => t!("Неисправна: отвечает другой датчик").into(),
            other => t!("Недоступна ({other})", other = other),
        };
        out.push(Module { role, name, ok: status == "ok", fault });
    }
    out
}
