//! Игровой режим (gameshell) и смена режима оболочки на лету.
//!
//! В игровом режиме корневые окна приложений — во весь экран без рамок и
//! заголовков (как на игровой консоли), диалоги — плавают по центру поверх
//! родителя. Окно без фокуса не остаётся «ничьим»: клавиатура уходит
//! главному экрану оболочки (слой `syngame-home`), чтобы стрелки и Enter
//! работали в лаунчере сразу после закрытия игры.
//!
//! Режим оболочки (рабочий стол, телефон, игровой) меняется без перезапуска
//! сеанса: действие `form-factor …` или правка `[platform] form_factor` —
//! политика конфига пересчитывается, раскладки столов и окна подстраиваются,
//! оболочка перезапускается своей командой.

use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::xwayland::xwm::WmWindowType;
use synshell_common::action::FormFactorTarget;
use synshell_common::config::FormFactor;
use synshell_common::ipc::Event;

use super::WindowId;
use crate::state::State;

/// Пространство имён слоя главного экрана gameshell.
pub const GAME_HOME_NAMESPACE: &str = "syngame-home";

impl State {
    /// Игровой режим с полноэкранной политикой окон.
    pub fn game_fullscreen_policy(&self) -> bool {
        self.core.form_factor == FormFactor::Game && self.core.config.gameshell.force_fullscreen
    }

    /// Корневое окно приложения (не диалог, не утилита, не всплывающее).
    fn is_app_root(&self, id: WindowId) -> bool {
        let Some(m) = self.core.wm.get(id) else { return false };
        if m.override_redirect {
            return false;
        }
        if let Some(t) = m.window.toplevel() {
            return t.parent().is_none();
        }
        if let Some(x) = m.window.x11_surface() {
            if x.is_transient_for().is_some() || x.is_popup() {
                return false;
            }
            // Окно фиксированного размера (min = max) — заставка или диалог.
            if let (Some(min), Some(max)) = (x.min_size(), x.max_size()) {
                if min == max && min.w > 0 {
                    return false;
                }
            }
            return matches!(x.window_type(), None | Some(WmWindowType::Normal));
        }
        false
    }

    fn fullscreen_excluded(&self, id: WindowId) -> bool {
        let Some(m) = self.core.wm.get(id) else { return true };
        let app = m.app_id();
        self.core.config.gameshell.fullscreen_exclude.iter().any(|p| synshell_common::config::glob_match(p.trim(), &app))
    }

    /// Политика игрового режима для нового окна (до первого configure или
    /// при появлении X11-окна): корень — во весь экран, диалог — по центру.
    pub fn apply_game_policy(&mut self, id: WindowId) {
        if !self.game_fullscreen_policy() {
            return;
        }
        let root = self.is_app_root(id) && !self.fullscreen_excluded(id);
        let Some(m) = self.core.wm.get_mut(id) else { return };
        if m.game_policy {
            return;
        }
        m.game_policy = true;
        m.ssd = false;
        if root {
            if !m.fullscreen {
                m.fullscreen = true;
                m.game_forced = true;
            }
        } else {
            m.floating = true;
            m.rule_center = true;
        }
    }

    /// Клиент просит выйти из полноэкранного режима: в игровом режиме корню
    /// это не положено (останется без рамок посреди экрана).
    pub fn game_keeps_fullscreen(&self, id: WindowId) -> bool {
        self.game_fullscreen_policy() && self.core.wm.get(id).is_some_and(|m| m.game_forced || m.fullscreen) && self.is_app_root(id) && !self.fullscreen_excluded(id)
    }

    /// Слой главного экрана gameshell (для фокуса клавиатуры без окон).
    pub fn game_home_surface(&self) -> Option<WlSurface> {
        if self.core.form_factor != FormFactor::Game {
            return None;
        }
        for o in self.core.space.outputs() {
            let map = layer_map_for_output(o);
            let found = map.layers().find(|l| l.namespace() == GAME_HOME_NAMESPACE).map(|l| l.wl_surface().clone());
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// Куда ведёт цель действия `form-factor`.
    fn resolve_form_factor(&self, target: FormFactorTarget) -> FormFactor {
        let exit = || -> FormFactor {
            match self.core.config.gameshell.exit_to.trim() {
                "previous" if self.core.prev_form_factor != FormFactor::Game => self.core.prev_form_factor,
                other => FormFactor::parse(other).filter(|f| *f != FormFactor::Game).unwrap_or(self.core.device_form_factor),
            }
        };
        match target {
            FormFactorTarget::Desktop => FormFactor::Desktop,
            FormFactorTarget::Phone => FormFactor::Phone,
            FormFactorTarget::Game => FormFactor::Game,
            FormFactorTarget::Device => self.core.device_form_factor,
            FormFactorTarget::ToggleGame if self.core.form_factor == FormFactor::Game => exit(),
            FormFactorTarget::ToggleGame => FormFactor::Game,
        }
    }

    /// Действие `form-factor`.
    pub fn form_factor_action(&mut self, target: FormFactorTarget) {
        let ff = self.resolve_form_factor(target);
        self.set_form_factor(ff, true);
    }

    /// Сменить режим оболочки на лету. `remember` — записать в config.toml,
    /// если `[gameshell] remember_mode`.
    pub fn set_form_factor(&mut self, ff: FormFactor, remember: bool) {
        let prev = self.core.form_factor;
        if prev == ff {
            return;
        }
        tracing::info!(from = prev.as_str(), to = ff.as_str(), "смена режима оболочки");
        self.core.prev_form_factor = prev;
        self.core.form_factor = ff;
        std::env::set_var("SYNSHELL_FORM_FACTOR", ff.as_str());
        if remember && self.core.config.gameshell.remember_mode {
            let v = toml_edit::Value::from(ff.as_str());
            if let Err(e) = synshell_common::config_edit::set_value(&["platform", "form_factor"], v) {
                tracing::warn!(?e, "не удалось запомнить режим оболочки");
            }
        }
        // Политика конфига нового режима (раскладка, рамки, оболочка);
        // сменилась команда оболочки — её перезапустит уже reload_config.
        let old_shell = self.core.config.general.shell.clone();
        self.reload_config();
        let shell_restarted = old_shell != self.core.config.general.shell;
        self.core.form_factor_setting = self.core.config.form_factor_setting();

        // Окна, развёрнутые игровым режимом, — обратно.
        if prev == FormFactor::Game {
            let forced: Vec<WindowId> = self.core.wm.windows.iter().filter(|m| m.game_forced).map(|m| m.id).collect();
            for m in self.core.wm.windows.iter_mut() {
                m.game_policy = false;
                m.game_forced = false;
            }
            // Развёрнуты по-обычному (в рабочую область, под панелями): запомненная
            // «плавающая» геометрия у них — во весь экран.
            for id in forced {
                if self.core.wm.get(id).is_some_and(|m| m.fullscreen) {
                    self.apply_fullscreen(id, false, None);
                    self.apply_maximized(id, true);
                }
            }
        }
        // Раскладка столов — умолчание нового режима (телефон выставит свою).
        let layout = self.core.config.windows.default_layout;
        for i in 0..self.core.wm.count() {
            self.core.wm.workspace_mut(i).layout = layout;
        }
        if prev == FormFactor::Phone {
            self.core.wm.mobile.enabled = false;
            self.core.wm.mobile.page = None;
            self.core.wm.mobile.transition = None;
            self.core.wm.mobile.drag = None;
            self.core.wm.mobile.camera = smithay::utils::Point::from((0.0, 0.0));
            self.core.wm.mobile.view = super::mobile::DeskView::default();
        }
        self.init_mobile();
        self.apply_mode_decorations();
        if ff == FormFactor::Game {
            let ids: Vec<WindowId> = self.core.wm.windows.iter().filter(|m| m.mapped && !m.override_redirect).map(|m| m.id).collect();
            for id in ids {
                self.apply_game_policy(id);
                let fs = self.core.wm.get(id).is_some_and(|m| m.game_forced);
                if fs {
                    self.apply_fullscreen(id, true, None);
                }
            }
        }
        self.relayout();
        self.core.ipc.broadcast(&Event::ShellCommand { command: format!("form-factor {}", ff.as_str()) });
        if !shell_restarted {
            self.restart_shell();
        }
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    /// Правка `[platform] form_factor` в конфиге — переключить на лету.
    pub fn form_factor_setting_changed(&mut self) {
        let setting = self.core.config.form_factor_setting();
        if setting == self.core.form_factor_setting {
            return;
        }
        self.core.form_factor_setting = setting;
        let ff = setting.unwrap_or(self.core.device_form_factor);
        self.set_form_factor(ff, false);
    }
}
