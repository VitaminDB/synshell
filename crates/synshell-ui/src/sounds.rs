//! Звуки событий оболочки по звуковой теме (`[sound]`, в игровом режиме —
//! `[gameshell] sound_theme`): уведомления, громкость, снимок, блокировка,
//! вход, устройства, щелчки по кнопкам и звуки игрового режима. Файлы
//! декодируются один раз (в фоне) и играются микшером `syngui::audio::sfx`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use synshell_common::config::FormFactor;
use syngui::audio::sfx::{self, Sound};

use crate::ctx::ShellCtx;

/// Событие и его группа (настройки включают группы).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ev {
    Notification,
    Volume,
    ScreenCapture,
    Lock,
    Unlock,
    Login,
    DeviceAdded,
    DeviceRemoved,
    PowerPlug,
    PowerUnplug,
    BatteryLow,
    Click,
    Error,
    // ── игровой режим ──
    Navigate,
    Select,
    Back,
    Tab,
    MenuOpen,
    MenuClose,
    Launch,
    Favorite,
    Bump,
    Startup,
}

enum Group {
    Notifications,
    System,
    Devices,
    Clicks,
    Game,
}

impl Ev {
    /// Имя события по спецификации freedesktop (или своё для игрового режима).
    pub fn name(self) -> &'static str {
        match self {
            Ev::Notification => "message-new-instant",
            Ev::Volume => "audio-volume-change",
            Ev::ScreenCapture => "screen-capture",
            Ev::Lock => "desktop-screen-lock",
            Ev::Unlock => "desktop-screen-unlock",
            Ev::Login => "desktop-login",
            Ev::DeviceAdded => "device-added",
            Ev::DeviceRemoved => "device-removed",
            Ev::PowerPlug => "power-plug",
            Ev::PowerUnplug => "power-unplug",
            Ev::BatteryLow => "battery-low",
            Ev::Click => "button-pressed",
            Ev::Error => "dialog-error",
            Ev::Navigate => "gameshell-navigate",
            Ev::Select => "gameshell-select",
            Ev::Back => "gameshell-back",
            Ev::Tab => "gameshell-tab",
            Ev::MenuOpen => "gameshell-menu-open",
            Ev::MenuClose => "gameshell-menu-close",
            Ev::Launch => "gameshell-launch",
            Ev::Favorite => "gameshell-favorite",
            Ev::Bump => "gameshell-bump",
            Ev::Startup => "gameshell-startup",
        }
    }

    fn group(self) -> Group {
        match self {
            Ev::Notification => Group::Notifications,
            Ev::Volume | Ev::ScreenCapture | Ev::Lock | Ev::Unlock | Ev::Login | Ev::Error => Group::System,
            Ev::DeviceAdded | Ev::DeviceRemoved | Ev::PowerPlug | Ev::PowerUnplug | Ev::BatteryLow => Group::Devices,
            Ev::Click => Group::Clicks,
            _ => Group::Game,
        }
    }

    /// Относительная громкость: щелчки и навигация тише.
    fn gain(self) -> f32 {
        match self {
            Ev::Click => 0.45,
            Ev::Navigate | Ev::Bump => 0.55,
            Ev::Volume => 0.7,
            _ => 1.0,
        }
    }
}

type Cache = Mutex<HashMap<(String, Ev), Option<Sound>>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Тема для события: игровой режим — своя (если задана).
fn theme_for(cfg: &synshell_common::Config, ff: FormFactor, ev: Ev) -> String {
    let game = matches!(ev.group(), Group::Game) || ff == FormFactor::Game;
    let gt = cfg.gameshell.sound_theme.trim();
    if game && !gt.is_empty() {
        return gt.to_string();
    }
    let t = cfg.sound.theme.trim();
    if t.is_empty() { synshell_common::sound_theme::DEFAULT_THEME.to_string() } else { t.to_string() }
}

/// Разрешено ли событие настройками; громкость 0..1.
fn allowed(ctx: &ShellCtx, ev: Ev) -> Option<f32> {
    let cfg = ctx.cfg();
    let s = &cfg.sound;
    let game_ff = ctx.form_factor == FormFactor::Game;
    match ev.group() {
        Group::Game => {
            if !cfg.gameshell.sounds {
                return None;
            }
            return Some(cfg.gameshell.sound_volume.min(150) as f32 / 100.0 * ev.gain());
        }
        // Щелчки в игровом режиме — по его выключателю.
        Group::Clicks if game_ff => {
            if !cfg.gameshell.sounds {
                return None;
            }
            return Some(cfg.gameshell.sound_volume.min(150) as f32 / 100.0 * ev.gain());
        }
        _ => {}
    }
    if !s.events {
        return None;
    }
    let on = match ev.group() {
        Group::Notifications => s.notifications && !ctx.dnd.get_untracked(),
        Group::System => s.system,
        Group::Devices => s.devices,
        Group::Clicks => s.clicks,
        Group::Game => true,
    };
    on.then(|| s.volume.min(150) as f32 / 100.0 * ev.gain())
}

/// Последнее звучание события — частые повторы (щелчки громкости) не
/// накладываются друг на друга.
fn throttled(ev: Ev) -> bool {
    static LAST: OnceLock<Mutex<HashMap<Ev, Instant>>> = OnceLock::new();
    let min = match ev {
        Ev::Volume => Duration::from_millis(90),
        Ev::Navigate | Ev::Click => Duration::from_millis(35),
        _ => Duration::from_millis(60),
    };
    let mut g = LAST.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    let now = Instant::now();
    if g.get(&ev).is_some_and(|t| now.duration_since(*t) < min) {
        return true;
    }
    g.insert(ev, now);
    false
}

/// Сыграть событие, если оно включено.
pub fn play(ev: Ev) {
    let ctx = ShellCtx::get();
    let Some(volume) = allowed(&ctx, ev) else { return };
    if throttled(ev) {
        return;
    }
    let theme = theme_for(&ctx.cfg(), ctx.form_factor, ev);
    play_theme(&theme, ev, volume);
}

/// Сыграть событие темы (прослушивание в Параметрах, предзагрузка).
pub fn play_theme(theme: &str, ev: Ev, volume: f32) {
    let key = (theme.to_string(), ev);
    if let Some(entry) = cache().lock().unwrap().get(&key) {
        if let Some(s) = entry {
            sfx::play(s, volume);
        }
        return;
    }
    // Первый раз — найти и декодировать в фоне.
    let theme = theme.to_string();
    std::thread::spawn(move || {
        let s = load(&theme, ev);
        if let Some(s) = &s {
            sfx::play(s, volume);
        }
        cache().lock().unwrap().insert(key, s);
    });
}

fn load(theme: &str, ev: Ev) -> Option<Sound> {
    let path: PathBuf = synshell_common::sound_theme::lookup(theme, ev.name())?;
    match sfx::load(&path) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("звук {}: {e}", path.display());
            None
        }
    }
}

/// Сбросить кэш (сменили тему).
pub fn reset() {
    cache().lock().unwrap().clear();
}

/// Заранее декодировать звуки игрового режима (первая навигация без задержки).
pub fn preload(evs: &[Ev]) {
    let ctx = ShellCtx::get();
    let cfg = ctx.cfg();
    let ff = ctx.form_factor;
    let list: Vec<(String, Ev)> = evs.iter().map(|e| (theme_for(&cfg, ff, *e), *e)).collect();
    std::thread::spawn(move || {
        for (theme, ev) in list {
            if cache().lock().unwrap().contains_key(&(theme.clone(), ev)) {
                continue;
            }
            let s = load(&theme, ev);
            cache().lock().unwrap().insert((theme, ev), s);
        }
    });
}

/// Вход в сеанс: звук один раз за сеанс (метка в `$XDG_RUNTIME_DIR`).
pub fn session_start() {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else { return };
    let mark = PathBuf::from(dir).join("synshell-login-sound");
    if mark.exists() {
        return;
    }
    let _ = std::fs::write(&mark, b"");
    play(Ev::Login);
}

/// Наблюдатели: блокировка и разблокировка, зарядка и низкий заряд, вход в
/// сеанс; смена темы — сбросить кэш.
pub fn start(ctx: ShellCtx) {
    use std::cell::Cell;
    use syngui::prelude::create_effect;
    let locked = syngui_layer::session_locked();
    let was_locked = Cell::new(locked.get_untracked());
    create_effect(move || {
        let l = locked.get();
        if l != was_locked.get() {
            was_locked.set(l);
            play(if l { Ev::Lock } else { Ev::Unlock });
        }
    });
    let prev: Cell<Option<(bool, u32)>> = Cell::new(ctx.battery.get_untracked().map(|b| (b.charging, b.percent)));
    create_effect(move || {
        let Some(b) = ctx.battery.get() else { return };
        if let Some((charging, percent)) = prev.get() {
            if b.charging && !charging {
                play(Ev::PowerPlug);
            } else if !b.charging && charging {
                play(Ev::PowerUnplug);
            } else if !b.charging && percent > 10 && b.percent <= 10 {
                play(Ev::BatteryLow);
            }
        }
        prev.set(Some((b.charging, b.percent)));
    });
    let theme = Cell::new((ctx.cfg().sound.theme.clone(), ctx.cfg().gameshell.sound_theme.clone()));
    create_effect(move || {
        let cfg = ctx.config.get();
        let now = (cfg.sound.theme.clone(), cfg.gameshell.sound_theme.clone());
        let old = theme.replace(now.clone());
        if old != now {
            reset();
        }
    });
    syngui_layer::add_timer(Duration::from_millis(1200), || {
        session_start();
        None
    });
}
