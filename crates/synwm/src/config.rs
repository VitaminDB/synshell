//! Перечитывание конфига на лету.

use smithay::input::keyboard::XkbConfig;
use synshell_common::{ipc::Event, watch::FileWatcher, Config};

use crate::{bindings::Bindings, deco::DecoTheme, state::State};

/// Конфиг и файлы активной темы.
pub fn watched_files(config: &Config) -> Vec<std::path::PathBuf> {
    let mut v = vec![synshell_common::paths::config_file()];
    v.extend(config.appearance.theme_files());
    v
}

impl State {
    /// Раз в секунду: не изменился ли файл конфигурации или темы.
    pub fn poll_config(&mut self) {
        if self.core.config_watcher.poll() {
            self.reload_config();
            self.core.config_watcher = FileWatcher::new(watched_files(&self.core.config));
        }
    }

    pub fn reload_config(&mut self) {
        let (mut config, error) = Config::load();
        // Политика форм-фактора (телефон: monocle, без рамок, своя оболочка) —
        // и после перечитывания, иначе правка config.toml возвращала бы
        // десктопные умолчания.
        config.apply_form_factor(self.core.form_factor);
        if let Some(e) = &error {
            tracing::warn!(error = e, "конфиг не разобран — остаётся прежний");
            self.core.config_error = Some(e.clone());
            self.core.ipc.broadcast(&Event::ConfigReloaded { error: Some(e.clone()) });
            return;
        }
        tracing::info!("конфиг перечитан");
        let old = std::mem::replace(&mut self.core.config, config);
        let mut errors = Vec::new();

        let (bindings, be) = Bindings::from_config(&self.core.config);
        self.core.bindings = bindings;
        errors.extend(be);
        let (rules, re) = crate::wm::rules::compile(&self.core.config.rules);
        self.core.rules = rules;
        errors.extend(re);

        // Рамки.
        self.core.deco_generation += 1;
        self.core.deco_theme = DecoTheme::from_config(&self.core.config.decorations, &self.core.config.appearance, self.core.deco_generation);
        if old.appearance.font != self.core.config.appearance.font {
            self.core.title_font = crate::deco::TitleFont::load(&self.core.config.appearance.font);
        }
        self.core
            .cursor
            .reload(&self.core.config.appearance.cursor_theme, self.core.config.appearance.cursor_size);

        // Клавиатура.
        let kb = self.core.config.input.keyboard.clone();
        if kb != old.input.keyboard {
            let keyboard = self.core.keyboard.clone();
            let res = keyboard.set_xkb_config(
                self,
                XkbConfig {
                    rules: "",
                    model: &kb.model,
                    layout: &kb.layouts,
                    variant: &kb.variants,
                    options: if kb.options.is_empty() { None } else { Some(kb.options.clone()) },
                },
            );
            if let Err(e) = res {
                errors.push(format!("раскладка клавиатуры: {e:?}"));
            }
            keyboard.change_repeat_info(kb.repeat_rate, kb.repeat_delay);
            self.core.last_kb_layout = None;
            self.broadcast_keyboard_layout();
        }
        // Мышь и тачпад.
        let input = self.core.config.input.clone();
        for d in &mut self.core.input_devices {
            crate::libinput_config::apply(d, &input);
        }
        if !matches!(self.backend, crate::backend::Backend::Winit(_)) {
            // Указательные устройства не хранятся — настройки применятся к
            // новым; для уже подключённых перечисляем через libinput.
            self.reapply_pointer_config();
        }

        // Столы, выводы, раскладки.
        self.core.wm.apply_config(&self.core.config);
        self.backend.apply_output_config(&mut self.core);
        self.outputs_changed();
        for id in self.core.wm.windows.iter().map(|w| w.id).collect::<Vec<_>>() {
            if let Some(m) = self.core.wm.get_mut(id) {
                m.shadow = Default::default();
            }
        }
        if old.general.shell != self.core.config.general.shell {
            self.core.shell.stop();
            self.core.shell = Default::default();
            self.start_shell();
        }

        self.core.config_error = if errors.is_empty() { None } else { Some(errors.join("; ")) };
        if let Some(e) = &self.core.config_error {
            tracing::warn!(error = e, "ошибки в конфиге");
        }
        let error = self.core.config_error.clone();
        self.core.ipc.broadcast(&Event::ConfigReloaded { error });
        self.broadcast_workspaces();
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    fn reapply_pointer_config(&mut self) {
        // Указательные устройства запоминаются вместе с клавиатурами.
    }
}
