//! Общее состояние оболочки — сигналы, которые читают апплеты.

use std::sync::Arc;
use syndesktop_common::ipc::{KeyboardLayouts, OutputInfo, WindowInfo, WorkspaceInfo};
use syndesktop_common::Config;
use syngui::prelude::*;

use crate::notifications::Notification;
use crate::system::{Battery, Network, Volume};

/// Что открыто во всплывающем окне (одновременно — одно).
#[derive(Debug, Clone, PartialEq)]
pub enum PopupKind {
    Launcher,
    Run,
    Calendar,
    Volume,
    Network,
    Battery,
    Power,
    Notifications,
    WindowMenu(u64),
    WindowSwitcher,
}

/// Где открыть всплывающее окно: вывод и прямоугольник-якорь (кнопка
/// апплета) в логических координатах вывода, край панели.
#[derive(Debug, Clone, PartialEq)]
pub struct PopupAnchor {
    pub output: Option<String>,
    /// [x, y, w, h]; `None` — по центру вывода.
    pub rect: Option<[f32; 4]>,
    pub edge: syndesktop_common::config::Edge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub kind: PopupKind,
    pub anchor: PopupAnchor,
}

/// Экранная подсказка (громкость, яркость).
#[derive(Debug, Clone, PartialEq)]
pub struct Osd {
    pub icon: &'static str,
    /// 0..=100 (может быть больше при усилении громкости).
    pub value: Option<u32>,
    pub label: String,
    pub serial: u64,
}

#[derive(Clone, Copy)]
pub struct ShellCtx {
    pub config: RwSignal<Arc<Config>>,
    /// Композитор syndesktop на связи.
    pub connected: RwSignal<bool>,
    pub windows: RwSignal<Vec<WindowInfo>>,
    pub workspaces: RwSignal<Vec<WorkspaceInfo>>,
    pub comp_outputs: RwSignal<Vec<OutputInfo>>,
    pub keyboard: RwSignal<KeyboardLayouts>,
    pub focused: RwSignal<Option<u64>>,
    pub popup: RwSignal<Option<Popup>>,
    pub osd: RwSignal<Option<Osd>>,
    /// Всплывающие уведомления (активные карточки).
    pub notifications: RwSignal<Vec<Notification>>,
    /// История (центр уведомлений).
    pub history: RwSignal<Vec<Notification>>,
    pub dnd: RwSignal<bool>,
    pub volume: RwSignal<Option<Volume>>,
    pub battery: RwSignal<Option<Battery>>,
    pub network: RwSignal<Network>,
    pub cpu: RwSignal<f32>,
    pub memory: RwSignal<f32>,
    /// Текущее локальное время, секунды Unix (обновляется таймером).
    pub now: RwSignal<i64>,
    /// Счётчик перечитываний конфига (перестроить поверхности).
    pub generation: RwSignal<u64>,
    /// Окна, свёрнутые кнопкой «показать рабочий стол».
    pub shown_desktop: RwSignal<Vec<u64>>,
}

impl ShellCtx {
    pub fn new(config: Config) -> Self {
        Self {
            config: use_signal(Arc::new(config.clone())),
            connected: use_signal(false),
            windows: use_signal(Vec::new()),
            workspaces: use_signal(Vec::new()),
            comp_outputs: use_signal(Vec::new()),
            keyboard: use_signal(KeyboardLayouts::default()),
            focused: use_signal(None),
            popup: use_signal(None),
            osd: use_signal(None),
            notifications: use_signal(Vec::new()),
            history: use_signal(Vec::new()),
            dnd: use_signal(config.notifications.do_not_disturb),
            volume: use_signal(None),
            battery: use_signal(None),
            network: use_signal(Network::default()),
            cpu: use_signal(0.0),
            memory: use_signal(0.0),
            now: use_signal(crate::clock::unix_now()),
            generation: use_signal(0),
            shown_desktop: use_signal(Vec::new()),
        }
    }

    pub fn get() -> Self {
        use_context::<ShellCtx>()
    }

    pub fn cfg(&self) -> Arc<Config> {
        self.config.get_untracked()
    }

    /// Индекс активного стола.
    pub fn active_workspace(&self) -> Option<u32> {
        self.workspaces.get().iter().find(|w| w.active).map(|w| w.index)
    }

    pub fn open_popup(&self, kind: PopupKind, anchor: PopupAnchor) {
        let cur = self.popup.get_untracked();
        if cur.as_ref().is_some_and(|p| p.kind == kind) {
            // Повторный клик по той же кнопке закрывает.
            self.popup.set(None);
        } else {
            self.popup.set(Some(Popup { kind, anchor }));
        }
    }

    pub fn close_popup(&self) {
        self.popup.set(None);
    }
}
