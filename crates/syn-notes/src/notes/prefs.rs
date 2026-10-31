//! Настройки приложения заметок (не проекта): вид, чтение, рукопись.
//! Файл — `~/.local/state/syn-notes/prefs.json`; читаются отовсюду через
//! [`get`], интерфейс подписывается на [`rev`].

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use syngui::prelude::*;

/// Как листают касанием.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TapMode {
    /// Левая треть — назад, правая — вперёд, центр — меню.
    LeftRight,
    /// Верхняя треть — назад, нижняя — вперёд, центр — меню.
    TopBottom,
    /// Любой край — вперёд, левый верхний угол — назад (одной рукой).
    Forward,
}

/// Направление свайпа для листания.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwipeDir {
    Horizontal,
    Vertical,
    Both,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Текст строго чёрный (`#000`): на E Ink серый текст выходит бледным.
    pub pure_black: bool,
    pub tap_mode: TapMode,
    /// Поменять «назад» и «вперёд» местами.
    pub tap_invert: bool,
    pub swipe: SwipeDir,
    /// Колесо мыши и прокрутка жестом листают (по странице за жест).
    pub wheel_turns: bool,
    /// Рукопись: рисовать пальцем.
    pub ink_finger: bool,
    /// Рукопись: без сглаживания краёв.
    pub ink_crisp: bool,
    /// Рукопись: удержание пера в конце штриха делает ровную фигуру.
    pub ink_hold_shape: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        let eink = synshell_common::display::is_eink();
        Self {
            pure_black: eink,
            tap_mode: TapMode::LeftRight,
            tap_invert: false,
            swipe: SwipeDir::Horizontal,
            wheel_turns: false,
            ink_finger: false,
            ink_crisp: eink,
            ink_hold_shape: true,
        }
    }
}

fn path() -> std::path::PathBuf {
    crate::config::state_path().with_file_name("prefs.json")
}

fn cell() -> &'static Mutex<Prefs> {
    static P: OnceLock<Mutex<Prefs>> = OnceLock::new();
    P.get_or_init(|| {
        let p = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Mutex::new(p)
    })
}

/// Текущие настройки.
pub fn get() -> Prefs {
    cell().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

thread_local! {
    static REV: std::cell::OnceCell<RwSignal<u64>> = const { std::cell::OnceCell::new() };
}

/// Счётчик изменений — подписка интерфейса (главный поток).
pub fn rev() -> RwSignal<u64> {
    REV.with(|r| *r.get_or_init(|| use_signal(0)))
}

/// Изменить и записать.
pub fn update(f: impl FnOnce(&mut Prefs)) {
    let snapshot = {
        let mut p = cell().lock().unwrap_or_else(|e| e.into_inner());
        f(&mut p);
        p.clone()
    };
    if let Ok(s) = serde_json::to_string_pretty(&snapshot) {
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = crate::fsutil::write_atomic(&path, s.as_bytes());
    }
    rev().update(|v| *v += 1);
}

/// Добавка к таблице стилей по настройкам.
pub fn mss() -> String {
    let p = get();
    let mut s = String::new();
    if p.pure_black {
        s.push_str(":root { --text: #000000; }\n.reader-page { color: #000000; }\n");
    }
    s
}

/// Куда ведёт касание в точке (доли ширины/высоты 0..1): -1 назад, 1 вперёд,
/// 0 — центр (меню).
pub fn tap_turn(fx: f32, fy: f32) -> i32 {
    let p = get();
    let dir = match p.tap_mode {
        TapMode::LeftRight => {
            if fx < 0.3 {
                -1
            } else if fx > 0.7 {
                1
            } else {
                0
            }
        }
        TapMode::TopBottom => {
            if fy < 0.3 {
                -1
            } else if fy > 0.7 {
                1
            } else {
                0
            }
        }
        TapMode::Forward => {
            let center = (0.3..=0.7).contains(&fx) && (0.3..=0.7).contains(&fy);
            if center {
                0
            } else if fx < 0.3 && fy < 0.3 {
                -1
            } else {
                1
            }
        }
    };
    if p.tap_invert {
        -dir
    } else {
        dir
    }
}

/// Свайп (dx, dy) → -1 назад, 1 вперёд, 0 — не листать.
pub fn swipe_turn(dx: f32, dy: f32) -> i32 {
    let p = get();
    let horiz = dx.abs() > 40.0 && dx.abs() > dy.abs();
    let vert = dy.abs() > 60.0 && dy.abs() > dx.abs() * 1.5;
    let dir = match p.swipe {
        SwipeDir::Horizontal | SwipeDir::Both if horiz => {
            if dx < 0.0 {
                1
            } else {
                -1
            }
        }
        SwipeDir::Vertical | SwipeDir::Both if vert => {
            if dy < 0.0 {
                1
            } else {
                -1
            }
        }
        _ => 0,
    };
    if p.tap_invert {
        -dir
    } else {
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let p = Prefs { pure_black: true, ..Prefs::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&s).unwrap(), p);
        // Неполный файл — умолчания для остального.
        let q: Prefs = serde_json::from_str(r#"{"tap_mode":"top_bottom"}"#).unwrap();
        assert_eq!(q.tap_mode, TapMode::TopBottom);
        assert!(q.ink_hold_shape);
    }
}
