//! Оконный менеджер: модель окон и рабочих столов.
//!
//! Источник истины — [`Wm`]: у каждого окна свой стол, положение, флаги.
//! `Space` smithay содержит только видимые сейчас окна в порядке стопки —
//! его синхронизирует [`crate::state::State::sync_space`]. Операции над
//! окнами — в `ops.rs` (им нужен `&mut State` ради фокуса клавиатуры).

pub mod layout;
pub mod ops;
pub mod rules;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use smithay::{
    desktop::Window,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        compositor::with_states, foreign_toplevel_list::ForeignToplevelHandle, seat::WaylandFocus,
        shell::xdg::XdgToplevelSurfaceData,
    },
};
use synshell_common::{action::LayoutKind, ipc::WindowInfo, Config};

use crate::{
    anim::Animation,
    deco::{Button, Shadow, TitleBar},
};
use layout::SnapZone;

pub type WindowId = u64;

/// Изменение размера мышью: какие края тянем и откуда начали.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeData {
    pub edges: ResizeEdge,
    pub initial_loc: Point<i32, Logical>,
    pub initial_size: Size<i32, Logical>,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP = 1;
        const BOTTOM = 2;
        const LEFT = 4;
        const RIGHT = 8;
    }
}

impl ResizeEdge {
    pub fn cursor(self) -> smithay::input::pointer::CursorIcon {
        use smithay::input::pointer::CursorIcon as C;
        let t = self.contains(ResizeEdge::TOP);
        let b = self.contains(ResizeEdge::BOTTOM);
        let l = self.contains(ResizeEdge::LEFT);
        let r = self.contains(ResizeEdge::RIGHT);
        match (t, b, l, r) {
            (true, _, true, _) => C::NwResize,
            (true, _, _, true) => C::NeResize,
            (_, true, true, _) => C::SwResize,
            (_, true, _, true) => C::SeResize,
            (true, _, _, _) => C::NResize,
            (_, true, _, _) => C::SResize,
            (_, _, true, _) => C::WResize,
            (_, _, _, true) => C::EResize,
            _ => C::Default,
        }
    }
}

impl From<smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge> for ResizeEdge {
    fn from(e: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge) -> Self {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as E;
        match e {
            E::Top => ResizeEdge::TOP,
            E::Bottom => ResizeEdge::BOTTOM,
            E::Left => ResizeEdge::LEFT,
            E::Right => ResizeEdge::RIGHT,
            E::TopLeft => ResizeEdge::TOP | ResizeEdge::LEFT,
            E::TopRight => ResizeEdge::TOP | ResizeEdge::RIGHT,
            E::BottomLeft => ResizeEdge::BOTTOM | ResizeEdge::LEFT,
            E::BottomRight => ResizeEdge::BOTTOM | ResizeEdge::RIGHT,
            _ => ResizeEdge::empty(),
        }
    }
}

/// Управляемое окно.
pub struct Managed {
    pub id: WindowId,
    pub window: Window,
    /// Индекс стола (с 0).
    pub workspace: u32,
    /// Имя вывода, к которому окно относится (для плитки).
    pub output: Option<String>,
    /// Положение содержимого (геометрии xdg) в глобальных координатах.
    pub loc: Point<i32, Logical>,
    /// Геометрия плавающего окна, к которой возвращаемся после
    /// развёртывания/прилипания/плитки.
    pub float_geo: Option<Rectangle<i32, Logical>>,
    /// Плавает поверх плиточной раскладки.
    pub floating: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub minimized: bool,
    pub sticky: bool,
    pub above: bool,
    pub urgent: bool,
    pub snap: Option<SnapZone>,
    /// Серверные рамки.
    pub ssd: bool,
    pub opacity: f32,
    pub skip_taskbar: bool,
    pub no_focus: bool,
    /// Получен первый буфер — окно показано.
    pub mapped: bool,
    /// Правила применены (на первичном configure).
    pub rules_applied: bool,
    pub rule_size: Option<Size<i32, Logical>>,
    pub rule_position: Option<Point<i32, Logical>>,
    pub rule_center: bool,
    pub resizing: Option<ResizeData>,
    /// Изменение размера закончено, ждём последнего коммита клиента.
    pub resize_finishing: bool,
    // ── отрисовка ──
    pub title_bar: TitleBar,
    pub shadow: Shadow,
    pub hover: Option<Button>,
    pub pressed: Option<Button>,
    pub open_anim: Option<Animation>,
    /// Анимация сворачивания/разворачивания: (анимация 0→1, к прямоугольнику).
    pub minimize_anim: Option<(Animation, Rectangle<i32, Logical>)>,
    /// Плавное перемещение при смене раскладки: откуда.
    pub move_anim: Option<(Animation, Point<i32, Logical>)>,
    /// Куда сворачиваться (значок на панели задач).
    pub minimize_rect: Option<Rectangle<i32, Logical>>,
    /// Последнее, что разослано по IPC — чтобы слать только изменения.
    pub last_info: Option<WindowInfo>,
    pub foreign_handle: Option<ForeignToplevelHandle>,
    /// Раскладка клавиатуры окна (per_window_layout).
    pub kb_layout: Option<u32>,
    /// X11 override-redirect (меню, подсказки): без управления.
    pub override_redirect: bool,
    pub created: Instant,
}

impl Managed {
    pub fn new(id: WindowId, window: Window, workspace: u32) -> Self {
        Self {
            id,
            window,
            workspace,
            output: None,
            loc: Point::from((0, 0)),
            float_geo: None,
            floating: false,
            maximized: false,
            fullscreen: false,
            minimized: false,
            sticky: false,
            above: false,
            urgent: false,
            snap: None,
            ssd: true,
            opacity: 1.0,
            skip_taskbar: false,
            no_focus: false,
            mapped: false,
            rules_applied: false,
            rule_size: None,
            rule_position: None,
            rule_center: false,
            resizing: None,
            resize_finishing: false,
            title_bar: TitleBar::default(),
            shadow: Shadow::default(),
            hover: None,
            pressed: None,
            open_anim: None,
            minimize_anim: None,
            move_anim: None,
            minimize_rect: None,
            last_info: None,
            foreign_handle: None,
            kb_layout: None,
            override_redirect: false,
            created: Instant::now(),
        }
    }

    pub fn surface(&self) -> Option<WlSurface> {
        self.window.wl_surface().map(|s| s.into_owned())
    }

    pub fn title(&self) -> String {
        self.window
            .toplevel()
            .and_then(|t| {
                with_states(t.wl_surface(), |s| {
                    s.data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().unwrap().title.clone())
                })
            })
            .or_else(|| self.window.x11_surface().map(|x| x.title()))
            .unwrap_or_default()
    }

    pub fn app_id(&self) -> String {
        self.window
            .toplevel()
            .and_then(|t| {
                with_states(t.wl_surface(), |s| {
                    s.data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().unwrap().app_id.clone())
                })
            })
            .or_else(|| self.window.x11_surface().map(|x| x.class()))
            .unwrap_or_default()
    }

    /// Размер содержимого (геометрия окна).
    pub fn size(&self) -> Size<i32, Logical> {
        self.window.geometry().size
    }

    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.loc, self.size())
    }

    /// Окно видно на столе `ws` (без учёта свёрнутости).
    pub fn on_workspace(&self, ws: u32) -> bool {
        self.sticky || self.workspace == ws
    }

    /// Окно участвует в плиточной раскладке.
    pub fn is_tiled(&self, layout: LayoutKind) -> bool {
        layout != LayoutKind::Floating
            && !self.floating
            && !self.fullscreen
            && !self.minimized
            && self.mapped
    }

    /// Рисовать ли заголовок (серверные рамки и не во весь экран; у
    /// развёрнутого — если не включены развёрнутые окна без заголовка).
    pub fn has_titlebar(&self) -> bool {
        self.ssd && !self.fullscreen && !(self.maximized && BORDERLESS_MAXIMIZED.load(Ordering::Relaxed))
    }
}

/// `windows.borderless_maximized`: развёрнутые окна без заголовка.
static BORDERLESS_MAXIMIZED: AtomicBool = AtomicBool::new(false);

pub struct Workspace {
    pub layout: LayoutKind,
    pub master_ratio: f32,
    pub master_count: u32,
    /// Окно, которое было в фокусе на этом столе.
    pub last_focus: Option<WindowId>,
}

/// Переключение столов: для анимации.
pub struct WorkspaceSwitch {
    pub from: u32,
    pub to: u32,
    /// Направление: +1 — новый справа, −1 — слева.
    pub dir: i32,
    pub anim: Animation,
}

/// Alt+Tab: окна по порядку последнего фокуса и текущий выбор.
pub struct Switcher {
    pub order: Vec<WindowId>,
    pub index: usize,
}

/// Режим обзора окон.
pub struct Overview {
    pub anim: Animation,
    pub closing: bool,
    /// Окна и их прямоугольники-миниатюры (глобальные координаты).
    pub slots: Vec<(WindowId, Rectangle<i32, Logical>)>,
    pub hovered: Option<WindowId>,
    pub selected: usize,
}

pub struct Wm {
    pub windows: Vec<Managed>,
    /// Порядок стопки снизу вверх (все окна всех столов).
    pub stack: Vec<WindowId>,
    /// История фокуса: последний — самый свежий.
    pub focus_history: Vec<WindowId>,
    pub focused: Option<WindowId>,
    pub workspaces: Vec<Workspace>,
    pub active: u32,
    pub previous: u32,
    pub switch: Option<WorkspaceSwitch>,
    pub switcher: Option<Switcher>,
    pub overview: Option<Overview>,
    /// Подсветка зоны прилипания при перетаскивании.
    pub snap_preview: Option<(Rectangle<i32, Logical>, Animation)>,
    /// Окно, которое тащат мышью (едет при смене стола).
    pub dragging: Option<WindowId>,
    next_id: WindowId,
}

impl Wm {
    pub fn new(config: &Config) -> Self {
        let mut wm = Self {
            windows: Vec::new(),
            stack: Vec::new(),
            focus_history: Vec::new(),
            focused: None,
            workspaces: Vec::new(),
            active: 0,
            previous: 0,
            switch: None,
            switcher: None,
            overview: None,
            snap_preview: None,
            dragging: None,
            next_id: 1,
        };
        wm.apply_config(config);
        wm
    }

    /// Число и раскладки столов из конфига (существующие окна со столов
    /// сверх нового количества переезжают на последний).
    pub fn apply_config(&mut self, config: &Config) {
        BORDERLESS_MAXIMIZED.store(config.windows.borderless_maximized, Ordering::Relaxed);
        let count = config.workspaces.count.clamp(1, 32);
        while self.workspaces.len() < count as usize {
            let i = self.workspaces.len() as u32;
            self.workspaces.push(Workspace {
                layout: config.workspace_layout(i),
                master_ratio: config.windows.master_ratio,
                master_count: config.windows.master_count.max(1),
                last_focus: None,
            });
        }
        self.workspaces.truncate(count as usize);
        for w in &mut self.windows {
            if w.workspace >= count {
                w.workspace = count - 1;
            }
        }
        if self.active >= count {
            self.active = count - 1;
        }
        if self.previous >= count {
            self.previous = 0;
        }
    }

    pub fn alloc_id(&mut self) -> WindowId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn get(&self, id: WindowId) -> Option<&Managed> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Managed> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn by_window(&self, window: &Window) -> Option<&Managed> {
        self.windows.iter().find(|w| &w.window == window)
    }

    pub fn by_surface(&self, surface: &WlSurface) -> Option<&Managed> {
        self.windows
            .iter()
            .find(|w| w.window.wl_surface().as_deref() == Some(surface))
    }

    pub fn id_by_surface(&self, surface: &WlSurface) -> Option<WindowId> {
        self.by_surface(surface).map(|w| w.id)
    }

    pub fn by_surface_mut(&mut self, surface: &WlSurface) -> Option<&mut Managed> {
        self.windows
            .iter_mut()
            .find(|w| w.window.wl_surface().as_deref() == Some(surface))
    }

    /// Видимые сейчас окна (активный стол + закреплённые, не свёрнутые)
    /// в порядке стопки снизу вверх.
    pub fn visible_ids(&self) -> Vec<WindowId> {
        self.stack
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|w| w.mapped && !w.minimized && w.on_workspace(self.active))
            })
            .collect()
    }

    /// Окна стола `ws` в порядке стопки (для анимации переключения).
    pub fn ids_on_workspace(&self, ws: u32) -> Vec<WindowId> {
        self.stack
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some_and(|w| w.mapped && !w.minimized && w.workspace == ws && !w.sticky))
            .collect()
    }

    pub fn raise(&mut self, id: WindowId) {
        self.stack.retain(|x| *x != id);
        self.stack.push(id);
        // «Поверх всех» остаются наверху.
        let (mut above, mut normal): (Vec<_>, Vec<_>) =
            self.stack.iter().partition(|x| self.get(**x).is_some_and(|w| w.above));
        normal.append(&mut above);
        self.stack = normal;
    }

    pub fn lower(&mut self, id: WindowId) {
        self.stack.retain(|x| *x != id);
        self.stack.insert(0, id);
    }

    pub fn note_focus(&mut self, id: WindowId) {
        self.focus_history.retain(|x| *x != id);
        self.focus_history.push(id);
    }

    pub fn workspace(&self, ws: u32) -> &Workspace {
        &self.workspaces[(ws as usize).min(self.workspaces.len() - 1)]
    }

    pub fn workspace_mut(&mut self, ws: u32) -> &mut Workspace {
        let i = (ws as usize).min(self.workspaces.len() - 1);
        &mut self.workspaces[i]
    }

    pub fn count(&self) -> u32 {
        self.workspaces.len() as u32
    }

    /// Есть анимации — нужны кадры.
    pub fn animating(&self) -> bool {
        self.switch.is_some()
            || self.overview.is_some()
            || self.snap_preview.is_some()
            || self.windows.iter().any(|w| w.open_anim.is_some() || w.minimize_anim.is_some() || w.move_anim.is_some())
    }

    /// Убрать завершившиеся анимации. Возвращает `true`, если что-то ещё идёт.
    pub fn tick_animations(&mut self) -> bool {
        for w in &mut self.windows {
            if w.open_anim.as_ref().is_some_and(|a| a.is_done()) {
                w.open_anim = None;
            }
            if w.move_anim.as_ref().is_some_and(|(a, _)| a.is_done()) {
                w.move_anim = None;
            }
            if w.minimize_anim.as_ref().is_some_and(|(a, _)| a.is_done()) {
                w.minimize_anim = None;
            }
        }
        if self.switch.as_ref().is_some_and(|s| s.anim.is_done()) {
            self.switch = None;
        }
        if self.overview.as_ref().is_some_and(|o| o.closing && o.anim.is_done()) {
            self.overview = None;
        }
        self.animating()
    }
}

/// Вывод, к которому относится окно, — по центру его геометрии.
pub fn output_for_rect(outputs: &[(Output, Rectangle<i32, Logical>)], r: Rectangle<i32, Logical>) -> Option<Output> {
    let c = Point::<i32, Logical>::from((r.loc.x + r.size.w / 2, r.loc.y + r.size.h / 2));
    outputs
        .iter()
        .find(|(_, g)| g.contains(c))
        .or_else(|| {
            outputs.iter().max_by_key(|(_, g)| {
                g.intersection(r).map(|i| i.size.w as i64 * i.size.h as i64).unwrap_or(0)
            })
        })
        .map(|(o, _)| o.clone())
}
