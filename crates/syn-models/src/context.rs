//! Контекст приложения: уведомления окна, видимость и положения панелей.
//! Замена `AppCtx` synthos — поля названы так же, перенесённый код их не меняет.

use syngui::prelude::*;
use syngui::widgets::feedback::NotificationCtx;

use crate::config::AppConfig;

#[derive(Clone, Copy)]
pub struct PanelsCtx {
    /// Левая и правая панели раздела.
    pub huggingface: (RwSignal<bool>, RwSignal<bool>),
    pub syn_explorer: (RwSignal<bool>, RwSignal<bool>),
}

/// Раздел программы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    HuggingFace,
    Library,
    Settings,
}

impl Page {
    pub fn from_config(s: &str) -> Self {
        match s {
            "syn_explorer" | "library" => Page::Library,
            "settings" => Page::Settings,
            _ => Page::HuggingFace,
        }
    }

    pub fn as_config(self) -> &'static str {
        match self {
            Page::HuggingFace => "huggingface",
            Page::Library => "syn_explorer",
            Page::Settings => "settings",
        }
    }
}

#[derive(Clone)]
pub struct AppCtx {
    /// Тосты окна: `info`, `success`, `warning`, `error`, `show`.
    pub notifications: NotificationCtx,
    pub panels: PanelsCtx,
    pub hf_left_split_ratio: RwSignal<f32>,
    pub hf_right_split_ratio: RwSignal<f32>,
    pub page: RwSignal<Page>,
}

impl AppCtx {
    pub fn new(cfg: &AppConfig) -> Self {
        Self {
            notifications: NotificationCtx::new(),
            panels: PanelsCtx {
                huggingface: (use_signal(cfg.huggingface_left), use_signal(cfg.huggingface_right)),
                syn_explorer: (use_signal(cfg.syn_explorer_left), use_signal(cfg.syn_explorer_right)),
            },
            hf_left_split_ratio: use_signal(cfg.hf_left_split_ratio),
            hf_right_split_ratio: use_signal(cfg.hf_right_split_ratio),
            page: use_signal(Page::from_config(&cfg.page)),
        }
    }
}
