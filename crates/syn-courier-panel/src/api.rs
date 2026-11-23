//! Связь со службой (`/api/*` того же адреса) и Telegram WebApp — через `syngui::web`.

use syngui::web::{self, telegram};

use crate::State;

fn auth() -> String {
    telegram::init_data().unwrap_or_default()
}

fn error_text(code: u16, body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v.get("error")?.as_str().map(str::to_string));
    match (code, msg) {
        (401, _) => "Откройте пульт кнопкой «Пульт» в чате бота".into(),
        (_, Some(m)) => m,
        (c, None) => format!("HTTP {c}"),
    }
}

pub fn get_state(cb: impl FnOnce(Result<State, String>) + 'static) {
    web::spawn_local(async move {
        let a = auth();
        let r = match web::fetch_json("GET", "api/state", &[("X-Telegram-Init-Data", &a)], None).await {
            Ok((200, body)) => serde_json::from_str::<State>(&body).map_err(|e| format!("ответ службы: {e}")),
            Ok((code, body)) => Err(error_text(code, &body)),
            Err(_) => Err("Компьютер не отвечает".into()),
        };
        cb(r);
    });
}

pub fn post(path: &'static str, body: serde_json::Value, cb: impl FnOnce(Result<(), String>) + 'static) {
    web::spawn_local(async move {
        let a = auth();
        let url = format!("api/{path}");
        let r = match web::fetch_json("POST", &url, &[("X-Telegram-Init-Data", &a)], Some(body.to_string())).await {
            Ok((200, _)) => Ok(()),
            Ok((code, body)) => Err(error_text(code, &body)),
            Err(_) => Err("компьютер не отвечает".into()),
        };
        cb(r);
    });
}

pub fn after_ms(ms: u32, f: impl FnOnce() + 'static) {
    web::spawn_local(async move {
        web::sleep_ms(ms).await;
        f();
    });
}

pub fn every_ms(ms: u32, f: impl Fn() + 'static) {
    // Вне браузера sleep блокирует — опрос только в вебе.
    if !web::is_web() {
        return;
    }
    web::spawn_local(async move {
        loop {
            web::sleep_ms(ms).await;
            f();
        }
    });
}

pub fn telegram_ready() {
    telegram::ready();
    telegram::expand();
}

pub fn on_theme_changed(f: impl Fn() + 'static) {
    telegram::on_theme_changed(f);
}

pub fn haptic_tap() {
    telegram::haptic_impact("light");
}

pub fn haptic_select() {
    telegram::haptic_selection();
}

pub fn haptic_ok() {
    telegram::haptic_notification("success");
}

pub fn haptic_err() {
    telegram::haptic_notification("error");
}

pub struct Theme {
    pub bg: String,
    pub card: String,
    pub fg: String,
    pub muted: String,
    pub accent: String,
    pub accent_fg: String,
    pub link: String,
    pub danger: String,
}

/// Цвета темы Telegram (вне Telegram — его тёмная палитра по умолчанию).
pub fn theme() -> Theme {
    let dark = telegram::theme_colors().is_none() || telegram::is_dark();
    let c = telegram::theme_colors().unwrap_or_default().or(&telegram::ThemeColors::fallback(dark));
    let hex = |k: &str, d: &str| c.get(k).map(|v| v.to_hex()).unwrap_or_else(|| d.to_string());
    Theme {
        bg: hex("secondary_bg_color", "#0e1621"),
        card: hex("section_bg_color", &hex("bg_color", "#17212b")),
        fg: hex("text_color", "#f5f5f5"),
        muted: hex("hint_color", "#708499"),
        accent: hex("button_color", "#5288c1"),
        accent_fg: hex("button_text_color", "#ffffff"),
        link: hex("accent_text_color", &hex("link_color", "#6ab3f3")),
        danger: hex("destructive_text_color", "#ec3942"),
    }
}
