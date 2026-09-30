//! Общее состояние оболочки — сигналы, которые читают апплеты.

use std::sync::Arc;
use synshell_common::ipc::{KeyboardLayouts, OutputInfo, WindowInfo, WorkspaceInfo};
use synshell_common::Config;
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
    /// Подключение к сети Wi-Fi `ssid`: пароль (с показом), статус, ошибки.
    WifiConnect { ssid: String },
    Battery,
    Power,
    Notifications,
    WindowMenu(u64),
    /// Меню значка лотка (ключ элемента).
    TrayMenu(String),
    /// Спрятанные значки лотка.
    TrayOverflow,
    /// Меню верхнего уровня `id` глобального меню (апплет `appmenu` на
    /// выводе `output`).
    GlobalMenu { output: String, id: i32 },
    /// Раздел или папка на панели/доке: апплет `index` панели `panel`
    /// (номер `[[panel]]`). `hover` — открыто наведением (закрывается,
    /// когда указатель уходит).
    Stack { panel: usize, index: usize, hover: bool },
    /// Меню значка панели/дока (апплет `index`).
    ItemMenu { panel: usize, index: usize },
    /// Меню работающего приложения на доке (из апплета `taskbar`).
    AppMenu { panel: usize, app: String },
    /// Меню панели (правый клик по пустому месту).
    PanelMenu(usize),
    /// Добавить значок, раздел, папку или апплет на панель.
    AddItem(usize),
    /// Изменить значок/раздел/папку.
    EditItem { panel: usize, index: usize },
    /// Меню рабочего стола (правый клик по обоям, удержание на телефоне):
    /// добавить панель или док, режим окон, обои, параметры.
    DesktopMenu,
    /// Меню значка приложения на домашнем экране (id .desktop).
    HomeAppMenu(String),
    /// Устройства (synlink): соединённые, спаренные, рядом.
    Link,
    /// Запрос спаривания с устройством (id).
    LinkPair(String),
}

impl PopupKind {
    /// Окна режима редактирования: перечитывание конфига их не закрывает
    /// (каждое добавление значка пишет файл).
    /// Контекстное меню у точки нажатия (на телефоне — у пальца, а не
    /// нижним листом).
    pub fn is_context_menu(&self) -> bool {
        matches!(
            self,
            PopupKind::DesktopMenu
                | PopupKind::HomeAppMenu(_)
                | PopupKind::ItemMenu { .. }
                | PopupKind::AppMenu { .. }
                | PopupKind::PanelMenu(_)
                | PopupKind::WindowMenu(_)
                | PopupKind::TrayMenu(_)
        )
    }

    pub fn survives_reload(&self) -> bool {
        matches!(self, PopupKind::AddItem(_) | PopupKind::Launcher)
    }
}

/// Где открыть всплывающее окно: вывод и прямоугольник-якорь (кнопка
/// апплета) в логических координатах вывода, край панели.
#[derive(Debug, Clone, PartialEq)]
pub struct PopupAnchor {
    pub output: Option<String>,
    /// [x, y, w, h]; `None` — по центру вывода.
    pub rect: Option<[f32; 4]>,
    pub edge: synshell_common::config::Edge,
    /// Панель прижата к краю без зазора: карточка примыкает к ней и
    /// перетекает в неё (вогнутые углы), а не висит рядом.
    pub attached: bool,
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
    /// Телефон или рабочий стол: раскладка всплывающих окон, какие панели
    /// показывать (`[[panel]] form_factor`).
    pub form_factor: synshell_common::config::FormFactor,
    /// Композитор synshell на связи.
    pub connected: RwSignal<bool>,
    pub windows: RwSignal<Vec<WindowInfo>>,
    pub workspaces: RwSignal<Vec<WorkspaceInfo>>,
    /// Режим окон телефона и страница (от композитора).
    pub mobile: RwSignal<Option<synshell_common::ipc::MobileInfo>>,
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
    /// Лента Alt+Tab: выбранное окно и порядок окон (от композитора).
    pub switcher: RwSignal<Option<(u64, Vec<u64>)>>,
    /// Панель (номер `[[panel]]`) в режиме редактирования.
    pub editing: RwSignal<Option<usize>>,
    /// Запускаемые приложения (id .desktop → момент запуска, мс): значок
    /// «прыгает», пока не появится окно.
    pub launching: RwSignal<Vec<(String, u64)>>,
    /// Счётчики всплесков частиц по значкам (ключ — id приложения или
    /// «панель:апплет»).
    pub bursts: RwSignal<std::collections::HashMap<String, u32>>,
    /// Связь с устройствами (демон synlink); `None` — демона нет.
    pub link: RwSignal<Option<synshell_common::link::Status>>,
}

impl ShellCtx {
    pub fn new(config: Config, form_factor: synshell_common::config::FormFactor) -> Self {
        Self {
            form_factor,
            mobile: use_signal(None),
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
            switcher: use_signal(None),
            editing: use_signal(None),
            launching: use_signal(Vec::new()),
            bursts: use_signal(std::collections::HashMap::new()),
            link: use_signal(None),
        }
    }

    /// Текущий режим окон: от композитора, иначе из конфига.
    pub fn mobile_mode(&self) -> synshell_common::action::MobileMode {
        self.mobile.get().map(|m| m.mode).unwrap_or_else(|| self.cfg().mobile.mode)
    }

    pub fn is_phone(&self) -> bool {
        self.form_factor == synshell_common::config::FormFactor::Phone
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

    /// Всплеск частиц у значка `key`.
    pub fn burst(&self, key: &str) {
        let mut m = self.bursts.get_untracked();
        *m.entry(key.to_string()).or_insert(0) += 1;
        self.bursts.set(m);
    }
}
