//! Контекст приложения: уведомления окна, видимость и положения панелей.
//! Замена `AppCtx` synthos — поля названы так же, код заметок их не меняет.

use syngui::prelude::*;
use syngui::widgets::feedback::NotificationCtx;

use crate::config::AppConfig;

#[derive(Clone, Copy)]
pub struct PanelsCtx {
    /// Левая (содержимое) и правая (свойства) панели.
    pub notes: (RwSignal<bool>, RwSignal<bool>),
}

#[derive(Clone)]
pub struct AppCtx {
    /// Тосты окна: `info`, `success`, `warning`, `error`, `show`.
    pub notifications: NotificationCtx,
    pub panels: PanelsCtx,
    pub notes_left_split_ratio: RwSignal<f32>,
    pub notes_right_split_ratio: RwSignal<f32>,
    /// Показанный маршрут. В syn-notes он один — «notes».
    pub current_route: RwSignal<String>,
}

impl AppCtx {
    pub fn new(cfg: &AppConfig) -> Self {
        Self {
            notifications: NotificationCtx::new(),
            panels: PanelsCtx { notes: (use_signal(cfg.notes_left), use_signal(cfg.notes_right)) },
            notes_left_split_ratio: use_signal(cfg.notes_left_split_ratio),
            notes_right_split_ratio: use_signal(cfg.notes_right_split_ratio),
            current_route: use_signal("notes".to_string()),
        }
    }
}
