//! Тема экрана входа — копия вида оболочки последнего пользователя, как тема SDDM = тема блокировки в KDE.
//!
//! Экран входа (`synlogin greeter`, synwm от root) работает до входа и конфиг пользователя не читает;
//! оболочка при каждой смене темы, акцента, шрифта или обоев пишет сюда готовое: переменные палитры и
//! стили темы (MSS), картинку обоев и её режим. Экран входа добавляет их поверх своих стилей — цвета,
//! скругления, шрифт и обои те же, что на экране блокировки.
//!
//! Файл — [`system_file`], `/var/lib/synshell/theme.toml` (`root:video 0664`, как
//! [`crate::rotation_state`]): каталог и файл создаёт `synlogin daemon` ([`ensure_system_file`]).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Подмена файла (проверка): `SYNSHELL_LOGIN_THEME_FILE=/tmp/x`, пустое — без файла.
pub const SYSTEM_FILE_ENV: &str = "SYNSHELL_LOGIN_THEME_FILE";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginTheme {
    /// Переменные палитры, шрифт, стили темы и `.wallpaper-color` — добавляются после стилей экрана входа.
    pub mss: String,
    /// Картинка обоев (файл, уже пригодный для показа); пусто — цвет или градиент из `mss`.
    pub wallpaper: String,
    /// Режим обоев: `fill`, `fit`, `stretch`, `center`.
    pub mode: String,
}

/// Файл темы экрана входа (`None` — отключён подменой).
pub fn system_file() -> Option<PathBuf> {
    match std::env::var_os(SYSTEM_FILE_ENV) {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(PathBuf::from(v)),
        None => Some(PathBuf::from("/var/lib/synshell/theme.toml")),
    }
}

/// Создать каталог и файл с правами для группы `video` (вызывает `synlogin daemon` от root).
pub fn ensure_system_file() -> std::io::Result<()> {
    match system_file() {
        Some(f) => crate::rotation_state::ensure_shared_file(&f),
        None => Ok(()),
    }
}

/// Сохранённая тема (нет файла, пустой или кривой — `None`).
pub fn load() -> Option<LoginTheme> {
    let text = std::fs::read_to_string(system_file()?).ok()?;
    let t: LoginTheme = toml::from_str(&text).ok()?;
    (!t.mss.trim().is_empty()).then_some(t)
}

/// Записать тему, если она изменилась. Файла нет или он недоступен (оболочка не в группе `video`,
/// экран входа не ставился) — молча ничего: это лишь вид экрана входа.
pub fn save(t: &LoginTheme) -> std::io::Result<bool> {
    let Some(path) = system_file() else { return Ok(false) };
    if !path.exists() {
        return Ok(false);
    }
    let text = toml::to_string(t).map_err(std::io::Error::other)?;
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    // На месте, без переименования: владелец и права (root:video 0664) остаются.
    std::fs::write(&path, text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_skip_unchanged() {
        let dir = std::env::temp_dir().join(format!("synshell-login-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("theme.toml");
        std::env::set_var(SYSTEM_FILE_ENV, &file);
        let t = LoginTheme { mss: ":root { --accent: #88c0d0; }\n".into(), wallpaper: "/x.jpg".into(), mode: "fill".into() };
        // Файла нет — не создаётся (его создаёт synlogin daemon).
        assert!(!save(&t).unwrap());
        std::fs::write(&file, "").unwrap();
        assert!(load().is_none());
        assert!(save(&t).unwrap());
        assert!(!save(&t).unwrap());
        assert_eq!(load(), Some(t));
        std::env::remove_var(SYSTEM_FILE_ENV);
        let _ = std::fs::remove_dir_all(dir);
    }
}
