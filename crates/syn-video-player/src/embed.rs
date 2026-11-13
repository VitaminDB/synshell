//! Плеер «Видео» внутри другой программы (tv_rezka): тот же экран плеера — кадр, панель управления
//! syngui, шапка с чипом «как идёт воспроизведение», звуковые дорожки, субтитры с оформлением,
//! настройки (декодирование, изображение, перемотка, субтитры, интерфейс) — со своими вещами
//! программы поверх: кнопки и чипы в шапке, слои над плеером (диалоги, панели), свои разделы
//! настроек, соседние ролики (серии), что делать в конце ролика и как открыть его заново.
//!
//! ```ignore
//! struct MyHost;
//! impl embed::Host for MyHost { /* название, назад, переоткрыть… */ }
//!
//! let st = embed::new(MyHost);          // главный поток, после запуска syngui
//! embed::open(st, "ключ", Some(start), move |hw| VideoPlayer::open_with_hwaccel(&url, hw).map_err(|e| e.to_string()));
//! embed::view(st)                        // экран плеера
//! ```
//!
//! Настройки плеера и субтитров — в `video-player.toml` и `video-player-subtitles.toml` каталога
//! настроек synshell (`SYNSHELL_CONFIG_DIR` задаёт его, например, на Android) — общие с программой
//! «Видео». Места остановки ведёт программа-хозяин ([`Host::progress`]).

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::input::{Key, Modifiers};
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widgets::FullscreenCtl;

use crate::ui::St;

pub type W = Box<dyn Widget>;

/// Как открыть ролик: ускорение по настройкам плеера («аппаратно» / «программно») → плеер. Зовётся
/// в фоновом потоке; сетевые зеркала, заголовки и таймауты — забота программы.
pub type Opener = Box<dyn FnOnce(HwAccel) -> std::result::Result<VideoPlayer, String> + Send + 'static>;

/// Программа, в которую встроен плеер. Методы, строящие виджеты, зовутся внутри `Reactive` —
/// сигналы, прочитанные в них, перестраивают свою часть экрана.
pub trait Host: Send + Sync + 'static {
    /// Название в шапке.
    fn title(&self) -> String;
    /// Строка под названием («Сезон 1 · Серия 3 · Озвучка · 1080p»); пустая — нет строки.
    fn caption(&self) -> String {
        String::new()
    }
    /// Свои кнопки и чипы в шапке — перед кнопками субтитров, дорожек и настроек.
    fn header(&self, _st: St) -> Vec<W> {
        Vec::new()
    }
    /// Слои поверх плеера (диалоги, панели программы).
    fn overlay(&self, _st: St) -> Vec<W> {
        Vec::new()
    }
    /// Свои разделы панели настроек — первыми.
    fn settings(&self, _st: St) -> Vec<W> {
        Vec::new()
    }
    /// «Назад» в шапке.
    fn back(&self);
    /// Есть ли соседний ролик (`delta` −1 — предыдущий, +1 — следующий): стрелки по бокам кадра,
    /// Page Up/Down.
    fn has_neighbor(&self, _delta: isize) -> bool {
        false
    }
    fn neighbor(&self, _delta: isize) {}
    /// Ролик досмотрен до конца (оборвавшийся посреди поток плеер переоткрывает сам).
    fn ended(&self) {}
    /// Открыть тот же ролик заново с секунды `pos`: поток завис или оборвался, сменили
    /// декодирование. Программа снова зовёт [`open`] (свежие адреса, те же зеркала).
    fn reopen(&self, pos: f64);
    /// Ускорение под «аппаратно» (`true`) / «программно» из настроек плеера.
    fn hwaccel(&self, hardware: bool) -> HwAccel {
        if hardware {
            HwAccel::Auto
        } else {
            HwAccel::None
        }
    }
    /// Громкость у программы (она её хранит) и её предел (1.5 — до 150 %).
    fn volume(&self) -> Option<(RwSignal<f32>, f32)> {
        None
    }
    /// Свои клавиши поверх клавиш плеера.
    fn keys(&self) -> Vec<Key> {
        Vec::new()
    }
    fn on_key(&self, _key: Key, _mods: Modifiers) -> bool {
        false
    }
    /// Во весь экран (кнопка в панели плеера, F); `None` — кнопки нет (окно и так во весь экран).
    fn fullscreen(&self) -> Option<FullscreenCtl> {
        None
    }
    /// Где сейчас (секунды и длительность): раз в 5 с из фонового потока и при закрытии ролика.
    fn progress(&self, _pos: f64, _duration: f64, _closing: bool) {}
    /// Ролик открылся (главный поток): плеер ещё на паузе, позиция стартовая.
    fn opened(&self, _p: &mut VideoPlayer) {}
}

/// Состояние плеера под программу-хозяина. Главный поток, после запуска syngui; хозяин живёт до
/// конца программы.
pub fn new(host: impl Host) -> St {
    let host: &'static dyn Host = Box::leak(Box::new(host));
    crate::ui::new_state(None, false, use_signal(syngui::window::WindowState::default()), Some(host))
}

/// Экран плеера.
pub fn view(st: St) -> W {
    crate::player::view(st)
}

/// Открыть ролик (`key` — имя для журнала и признак «тот же ролик») с секунды `start`.
pub fn open(st: St, key: impl Into<String>, start: Option<f64>, opener: impl FnOnce(HwAccel) -> std::result::Result<VideoPlayer, String> + Send + 'static) {
    crate::ui::open_with(st, key.into().into(), start, true, Box::new(opener));
}

/// Закрыть ролик (декодер останавливается в фоне).
pub fn close(st: St) {
    crate::ui::close(st);
}

/// Открытый плеер.
pub fn player(st: St) -> Option<Arc<Mutex<VideoPlayer>>> {
    st.playing.get_untracked().map(|p| p.player)
}

/// Открыт ли ролик или открывается.
pub fn active(st: St) -> bool {
    st.playing.get_untracked().is_some() || st.opening.get_untracked().is_some()
}

/// Ошибка открытия (текст) — показывается на экране плеера.
pub fn set_error(st: St, e: Option<String>) {
    st.opening.set(None);
    st.error.set(e);
}

/// «Назад» с пульта или системной кнопкой: закрыть открытую панель плеера (настройки, субтитры,
/// дорожки). `false` — панелей нет, «назад» за программой.
pub fn back(st: St) -> bool {
    crate::ui::close_panels(st)
}

/// Открыта ли панель плеера (настройки, субтитры, дорожки).
pub fn panel_open(st: St) -> bool {
    st.settings_open.get_untracked() || st.subs_open.get_untracked() || st.tracks_open.get_untracked() || st.filters_open.get_untracked()
}

/// Открыть настройки плеера.
pub fn open_settings(st: St) {
    st.settings_open.set(true);
}

/// Подключить переводы плеера (язык — `lang`, иначе как в системе / synshell).
pub fn init_i18n(lang: Option<&str>) {
    if let Some(l) = lang {
        std::env::set_var("SYNSHELL_LANG", l);
    }
    synshell_common::i18n::init(&crate::catalogs());
}
