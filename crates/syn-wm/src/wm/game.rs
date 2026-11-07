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
use synshell_tr::t;

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
        if ff == FormFactor::Game && self.core.form_factor != FormFactor::Game && !self.game_shell_installed() {
            notify_game_missing();
            return;
        }
        // Режим выбран пользователем — прежние падения оболочек не в счёт.
        self.core.failed_form_factors.clear();
        self.set_form_factor(ff, true);
    }

    /// Установлена ли оболочка игрового режима (пакет «Игровой режим» —
    /// необязательный). Ищется и рядом с композитором — так её запускает `start_shell`.
    pub fn game_shell_installed(&self) -> bool {
        game_shell_installed(&self.core.config.gameshell)
    }

    /// Оболочка режима упала слишком часто: вернуться в прежний режим
    /// (из игрового — в телефон или рабочий стол), а не оставлять экран без
    /// оболочки. Упала оболочка основного режима устройства — ничего не делаем
    /// (возвращаться некуда); режим, где оболочка уже падала, второй раз не
    /// выбирается — по кругу не ходим.
    pub fn shell_crash_fallback(&mut self) {
        // Своя «оболочка» на весь сеанс (экран входа) — от режима не зависит.
        if std::env::var("SYNSHELL_SHELL").is_ok_and(|s| !s.trim().is_empty()) {
            return;
        }
        let cur = self.core.form_factor;
        if !self.core.failed_form_factors.contains(&cur) {
            self.core.failed_form_factors.push(cur);
        }
        let Some(to) = fallback_form_factor(cur, self.core.prev_form_factor, self.core.device_form_factor, &self.core.failed_form_factors, self.game_shell_installed()) else {
            return;
        };
        tracing::warn!(from = cur.as_str(), to = to.as_str(), "оболочка режима падает — возврат в прежний режим");
        crate::notify::post(
            t!("Оболочка падает слишком часто"),
            t!("Режим «{from}» не работает — возврат в режим «{to}».", from = mode_label(cur), to = mode_label(to)),
            "dialog-warning",
        );
        self.set_form_factor(to, true);
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
        // Поворот игрового режима не запоминался — вернуть тот, что был до него
        // (выбранный вручную или последний вне игрового режима).
        if prev == FormFactor::Game {
            self.restore_rotation_after_game();
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

    /// Выход из игрового режима: поворот экрана — как до него.
    fn restore_rotation_after_game(&mut self) {
        let Some(o) = crate::backend::rotation_target(&self.core) else { return };
        let saved = self.core.saved_rotation.get(&o.name()).copied();
        let r = crate::backend::startup_rotation(&self.core.config.rotation, saved);
        self.rotate(r);
    }

    /// Правка `[platform] form_factor` в конфиге — переключить на лету.
    pub fn form_factor_setting_changed(&mut self) {
        let setting = self.core.config.form_factor_setting();
        if setting == self.core.form_factor_setting {
            return;
        }
        self.core.form_factor_setting = setting;
        let ff = setting.unwrap_or(self.core.device_form_factor);
        if ff == FormFactor::Game && self.core.form_factor != FormFactor::Game && !self.game_shell_installed() {
            notify_game_missing();
            return;
        }
        self.core.failed_form_factors.clear();
        self.set_form_factor(ff, false);
    }
}

/// Установлена ли оболочка игрового режима: в `PATH` или рядом с композитором.
pub fn game_shell_installed(g: &synshell_common::config::Gameshell) -> bool {
    let exe_dir: Vec<_> = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).into_iter().collect();
    g.installed_in(&exe_dir)
}

/// Уведомление «Игровой режим не установлен» (режим не меняется).
pub fn notify_game_missing() {
    tracing::warn!("игровой режим не установлен (нет оболочки syngame-shell) — режим не меняю");
    crate::notify::post(t!("Игровой режим не установлен"), t!("Поставьте „Игровой режим“ в Программах."), "input-gaming");
}

fn mode_label(ff: FormFactor) -> String {
    match ff {
        FormFactor::Desktop => t!("рабочий стол"),
        FormFactor::Phone => t!("телефон"),
        FormFactor::Game => t!("игровой"),
    }
}

/// Куда вернуться, когда оболочка режима `cur` упала слишком часто: в
/// прежний режим, иначе в режим устройства; `None` — упал сам режим
/// устройства или вернуться некуда (там оболочка тоже падала).
fn fallback_form_factor(cur: FormFactor, prev: FormFactor, device: FormFactor, failed: &[FormFactor], game_installed: bool) -> Option<FormFactor> {
    if cur == device {
        return None;
    }
    [prev, device].into_iter().find(|&f| f != cur && !failed.contains(&f) && (f != FormFactor::Game || game_installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use FormFactor::*;

    #[test]
    fn fallback() {
        // Игровой на телефоне упал — в телефон.
        assert_eq!(fallback_form_factor(Game, Phone, Phone, &[Game], true), Some(Phone));
        // Из рабочего стола в игровой на телефоне — назад в рабочий стол.
        assert_eq!(fallback_form_factor(Game, Desktop, Phone, &[Game], true), Some(Desktop));
        // Прежний режим тоже падал — в режим устройства.
        assert_eq!(fallback_form_factor(Game, Desktop, Phone, &[Desktop, Game], true), Some(Phone));
        // Упал основной режим устройства — как раньше, ничего.
        assert_eq!(fallback_form_factor(Phone, Game, Phone, &[Phone], true), None);
        // Рабочий стол на телефоне упал, телефон уже падал — без круга.
        assert_eq!(fallback_form_factor(Desktop, Phone, Phone, &[Phone, Desktop], true), None);
        // В игровой без установленной оболочки не возвращаемся.
        assert_eq!(fallback_form_factor(Desktop, Game, Phone, &[Desktop], false), Some(Phone));
    }
}
