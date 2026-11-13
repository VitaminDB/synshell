//! syn-video-player как библиотека: плеер «Видео» целиком — виджет плеера syngui, шапка с чипом
//! «как идёт воспроизведение», звуковые дорожки, субтитры с оформлением, настройки, лента соседних
//! роликов, монтаж. Программа `syn-video-player` (фича `desktop`) — его библиотека роликов и окно;
//! другие программы встраивают тот же плеер через [`embed`] (tv_rezka): всё, что улучшается здесь,
//! сразу появляется и у них.

pub mod chrome;
pub mod edit;
pub mod editor;
pub mod embed;
pub mod library;
pub mod chain;
pub mod player;
pub mod settings;
pub mod subs;
pub mod tracks;
pub mod ui;

#[cfg(feature = "desktop")]
pub mod bench;
#[cfg(feature = "desktop")]
pub mod dlss;
#[cfg(feature = "desktop")]
pub mod filters;
#[cfg(feature = "desktop")]
pub mod interp;
#[cfg(feature = "desktop")]
pub mod neural;
#[cfg(feature = "desktop")]
pub mod vram;

// Без рабочего стола: тот же интерфейс модулей, фильтров нет.
#[cfg(not(feature = "desktop"))]
#[path = "lite/dlss.rs"]
pub mod dlss;
#[cfg(not(feature = "desktop"))]
#[path = "lite/filters.rs"]
pub mod filters;
#[cfg(not(feature = "desktop"))]
#[path = "lite/interp.rs"]
pub mod interp;
#[cfg(not(feature = "desktop"))]
#[path = "lite/neural.rs"]
pub mod neural;
#[cfg(not(feature = "desktop"))]
#[path = "lite/vram.rs"]
pub mod vram;

/// Стили плеера (шапка, чип, панели настроек, субтитров и дорожек, лента миниатюр). Палитра —
/// переменные `--bg`, `--fg`, `--surface`, `--muted`, `--accent`, `--hover`, `--pressed`, `--border`.
pub const PLAYER_STYLE: &str = include_str!("../styles/player.mss");

/// Каталоги переводов плеера (для `synshell_common::i18n::init`).
pub fn catalogs() -> Vec<&'static str> {
    synshell_common::catalogs!("../i18n").to_vec()
}
