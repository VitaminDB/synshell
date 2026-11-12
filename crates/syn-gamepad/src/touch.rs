//! Режим касания игры: что делает палец мимо элементов контроллера — левая, правая, средняя кнопка
//! мыши, удержание (тап — левая, палец лежит на месте — правая, пока лежит) или только наведение курсора. Кнопка контроллера с назначением `touch:<режимы>` перебирает
//! режимы по кругу (`touch:left,hold,right,hover`); режим — слово в `$XDG_RUNTIME_DIR/syn-touch-mode`, его
//! читает syn-wine при каждом касании (`winex11-touch-dinput.patch`, `wayland-touch.patch`). Без файла —
//! левая кнопка. Контроллер спрятан, сменилось приложение или раскладка — снова левая.

use std::path::PathBuf;

use syngui::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Left,
    Right,
    Middle,
    /// Тап — левая, палец лежит на месте дольше порога — правая (держится, пока лежит), сдвиг — левая (перетаскивание).
    Hold,
    Hover,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim() {
            "left" | "lmb" => Some(Mode::Left),
            "right" | "rmb" => Some(Mode::Right),
            "middle" | "mmb" => Some(Mode::Middle),
            "hold" | "long" => Some(Mode::Hold),
            "hover" | "move" => Some(Mode::Hover),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Left => "left",
            Mode::Right => "right",
            Mode::Middle => "middle",
            Mode::Hold => "hold",
            Mode::Hover => "hover",
        }
    }
}

/// Режимы кнопки из назначения `touch:left,hold,right,hover` (пусто или ошибка — эти четыре).
pub fn cycle(bind: &str) -> Vec<Mode> {
    let list: Vec<Mode> = bind.split_once(':').map(|(_, a)| a).unwrap_or("").split(',').filter_map(Mode::parse).collect();
    if list.is_empty() { vec![Mode::Left, Mode::Hold, Mode::Right, Mode::Hover] } else { list }
}

/// Следующий режим кнопки после текущего (текущего нет в её списке — первый).
pub fn next(bind: &str, cur: Mode) -> Mode {
    let list = cycle(bind);
    match list.iter().position(|m| *m == cur) {
        Some(i) => list[(i + 1) % list.len()],
        None => list[0],
    }
}

fn file() -> PathBuf {
    synshell_common::paths::runtime_dir().join("syn-touch-mode")
}

thread_local! {
    static MODE: std::cell::Cell<Option<RwSignal<Mode>>> = const { std::cell::Cell::new(None) };
}

/// Текущий режим (сигнал: значок кнопки следит за ним).
pub fn signal() -> RwSignal<Mode> {
    MODE.with(|m| match m.get() {
        Some(s) => s,
        None => {
            let s = use_signal(Mode::Left);
            m.set(Some(s));
            s
        }
    })
}

/// Сменить режим: файл для Wine и значок.
pub fn set(mode: Mode) {
    signal().set(mode);
    // Левая — файла нет (как без контроллера).
    let r = match mode {
        Mode::Left => match std::fs::remove_file(file()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        _ => std::fs::write(file(), mode.as_str()),
    };
    if let Err(e) = r {
        log::warn!("{}: {e}", file().display());
    }
    log::info!("режим касания: {}", mode.as_str());
}

/// Значок кнопки: мышь, активная кнопка — зелёная; удержание — левая зелёная, правая бледно-зелёная и часы;
/// «только курсор» — стрелка курсора над мышью.
pub fn svg(mode: Mode) -> String {
    const ON: &str = "#3ddc84";
    const OFF: &str = "rgba(255,255,255,0.18)";
    const LATER: &str = "rgba(61,220,132,0.45)";
    const LINE: &str = "#ffffff";
    let (l, r, m) = match mode {
        Mode::Left => (ON, OFF, OFF),
        Mode::Right => (OFF, ON, OFF),
        Mode::Middle => (OFF, OFF, ON),
        Mode::Hold => (ON, LATER, OFF),
        Mode::Hover => (OFF, OFF, OFF),
    };
    let arrow = if mode == Mode::Hover {
        format!(r##"<path d="M58 6 L58 38 L66 30 L72 43 L77 41 L71 28 L82 28 Z" fill="{ON}" stroke="#000" stroke-opacity="0.45" stroke-width="2" stroke-linejoin="round"/>"##)
    } else if mode == Mode::Hold {
        // часы: правая — после удержания
        format!(r##"<circle cx="72" cy="72" r="17" fill="#1e2228" stroke="{ON}" stroke-width="4"/><path d="M72 62 L72 72 L80 77" fill="none" stroke="{ON}" stroke-width="4" stroke-linecap="round"/>"##)
    } else {
        String::new()
    };
    // Корпус 20..76 × 14..90 (как Material «mouse»), разделение кнопок по y = 46.
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">
<path d="M48 14 C33 14 22 25 22 40 L22 46 L48 46 Z" fill="{l}"/>
<path d="M48 14 C63 14 74 25 74 40 L74 46 L48 46 Z" fill="{r}"/>
<rect x="44" y="22" width="8" height="18" rx="4" fill="{m}" stroke="{LINE}" stroke-width="2.5"/>
<path d="M48 14 C33 14 22 25 22 40 L22 64 C22 79 33 90 48 90 C63 90 74 79 74 64 L74 40 C74 25 63 14 48 14 Z" fill="none" stroke="{LINE}" stroke-width="4"/>
<path d="M22 46 L74 46 M48 14 L48 22 M48 40 L48 46" stroke="{LINE}" stroke-width="3"/>
{arrow}
</svg>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles() {
        assert_eq!(cycle("touch:left,right"), vec![Mode::Left, Mode::Right]);
        assert_eq!(cycle("touch:"), vec![Mode::Left, Mode::Hold, Mode::Right, Mode::Hover]);
        assert_eq!(cycle("touch:bad"), vec![Mode::Left, Mode::Hold, Mode::Right, Mode::Hover]);
        assert_eq!(next("touch:left,hold,right,hover", Mode::Left), Mode::Hold);
        assert_eq!(next("touch:left,right,hover", Mode::Left), Mode::Right);
        assert_eq!(next("touch:left,right,hover", Mode::Hover), Mode::Left);
        assert_eq!(next("touch:right,middle", Mode::Left), Mode::Right);
    }

    #[test]
    fn svg_parses() {
        for m in [Mode::Left, Mode::Right, Mode::Middle, Mode::Hold, Mode::Hover] {
            let s = svg(m);
            assert!(s.starts_with("<svg") && s.ends_with("</svg>"));
            // Посмотреть значки: SYN_TOUCH_SVG_DIR=/tmp/x cargo test -p syn-gamepad svg_parses
            if let Ok(dir) = std::env::var("SYN_TOUCH_SVG_DIR") {
                std::fs::write(format!("{dir}/touch-{}.svg", m.as_str()), s).unwrap();
            }
        }
    }
}
