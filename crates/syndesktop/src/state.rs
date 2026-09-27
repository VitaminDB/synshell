//! Состояние композитора.
//!
//! `State` = бэкенд (DRM или вложенное окно) + `Core` (всё остальное).
//! Разделение нужно для заимствований: бэкенд рисует, получая `&mut Core`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{
        keyboard::{KeyboardHandle, XkbConfig},
        pointer::{CursorImageStatus, PointerHandle},
        Seat, SeatState,
    },
    output::Output,
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Clock, Logical, Monotonic, Point, Rectangle},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufGlobal, DmabufState},
        foreign_toplevel_list::ForeignToplevelListState,
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        input_method::InputMethodManagerState,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        security_context::SecurityContext,
        selection::{
            data_device::DataDeviceState, primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState,
        },
        session_lock::SessionLockManagerState,
        shell::{
            kde::decoration::KdeDecorationState,
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState},
        },
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        tablet_manager::TabletManagerState,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_activation::XdgActivationState,
        xdg_foreign::XdgForeignState,
    },
    xwayland::XWaylandClientData,
};
use syndesktop_common::{config::Config, watch::FileWatcher};

use crate::{
    backend::Backend,
    bindings::Bindings,
    cursor::CursorManager,
    deco::{DecoTheme, TitleFont},
    ipc::IpcServer,
    wm::{rules::CompiledRule, Wm},
};

pub struct State {
    pub backend: Backend,
    pub core: Core,
}

/// Данные клиента Wayland.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    pub security_context: Option<SecurityContext>,
    /// Клиент — наша оболочка (запущена композитором).
    pub privileged: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// Состояние перерисовки вывода.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedrawState {
    /// Ничего не нужно.
    #[default]
    Idle,
    /// Нужна перерисовка, ждём idle-обработчика.
    Queued,
    /// Кадр отправлен, ждём vblank; `true` — после него снова рисовать.
    WaitingForVBlank { redraw_needed: bool },
    /// Кадр без изменений: ждём расчётного vblank по таймеру (для frame callback).
    WaitingForEstimatedVBlank { redraw_needed: bool },
}

pub struct OutputData {
    pub redraw: RedrawState,
    /// Когда отрисован последний кадр (для темпа анимаций и frame callbacks).
    pub last_frame: Instant,
    /// Счётчик кадров — для отладки.
    pub frames: u64,
    /// Мониторы погашены (DPMS).
    pub powered_off: bool,
    /// Последний кадр был с изменениями (для copy_with_damage).
    pub damaged: bool,
}

impl Default for OutputData {
    fn default() -> Self {
        Self { redraw: RedrawState::Idle, last_frame: Instant::now(), frames: 0, powered_off: false, damaged: false }
    }
}

/// Блокировка экрана (ext-session-lock).
#[derive(Default)]
pub enum LockState {
    #[default]
    Unlocked,
    /// Блокировка запрошена, ждём поверхностей.
    Locking(smithay::wayland::session_lock::SessionLocker),
    Locked,
}

pub struct Core {
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub loop_signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub start_time: Instant,
    pub socket_name: String,
    pub nested: bool,

    // ── конфигурация ──
    pub config: Config,
    pub config_error: Option<String>,
    pub config_watcher: FileWatcher,
    pub bindings: Bindings,
    pub rules: Vec<CompiledRule>,
    pub deco_theme: DecoTheme,
    pub title_font: TitleFont,
    pub deco_generation: u64,

    // ── протоколы ──
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub kde_decoration_state: KdeDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub xdg_activation_state: XdgActivationState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub presentation_state: PresentationState,
    pub viewporter_state: ViewporterState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub pointer_gestures_state: PointerGesturesState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub idle_notifier_state: IdleNotifierState<State>,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub session_lock_state: SessionLockManagerState,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub cursor_shape_state: CursorShapeManagerState,
    pub xdg_foreign_state: XdgForeignState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub tablet_manager_state: TabletManagerState,

    // ── ввод ──
    pub seat: Seat<State>,
    pub keyboard: KeyboardHandle<State>,
    pub pointer: PointerHandle<State>,
    pub cursor_status: CursorImageStatus,
    pub cursor: CursorManager,
    /// Курсор спрятан (печать/простой); показать при движении.
    pub cursor_hidden: bool,
    pub last_pointer_motion: Instant,
    pub dnd_icon: Option<WlSurface>,
    /// Клавиши, нажатие которых ушло в сочетание — их отпускание не
    /// отдаётся клиенту.
    pub suppressed_keys: HashSet<u32>,
    /// Клавиатура из libinput (для светодиодов).
    pub input_devices: Vec<smithay::reexports::input::Device>,

    // ── рабочий стол ──
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub wm: Wm,
    pub output_data: HashMap<Output, OutputData>,
    pub lock: LockState,
    /// Поверхности блокировки по выводу.
    pub lock_surfaces: HashMap<Output, smithay::wayland::session_lock::LockSurface>,
    /// Поверхности, запретившие простой.
    pub idle_inhibitors: HashSet<WlSurface>,
    pub last_activity: Instant,
    pub monitors_off: bool,
    pub ipc: IpcServer,
    pub shell: crate::spawn::ShellProcess,
    /// После выхода из цикла — заменить процесс новым (`restart`).
    pub restart_requested: bool,
    pub xwayland: Option<crate::xwayland::XwaylandState>,
    /// Нужно разослать изменения окон по IPC после текущего цикла.
    pub ipc_dirty: bool,
    /// Захваты экрана, ждущие кадра с изменениями.
    pub pending_copies: Vec<crate::screencopy::PendingCopy>,
    pub frame_ids: crate::render::FrameIds,
    pub gesture: crate::input::GestureState,
    pub last_title_click: Option<crate::input::LastTitleClick>,
    pub last_kb_layout: Option<u32>,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display: Display<State>,
        loop_handle: LoopHandle<'static, State>,
        loop_signal: LoopSignal,
        seat_name: &str,
        nested: bool,
        config: Config,
        config_error: Option<String>,
    ) -> anyhow::Result<Self> {
        let dh = display.handle();
        let clock = Clock::new();

        // Сокет Wayland.
        let source = ListeningSocketSource::new_auto()?;
        let socket_name = source.socket_name().to_string_lossy().into_owned();
        loop_handle.insert_source(source, |stream, _, state: &mut State| {
            if let Err(e) = state
                .core
                .display_handle
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                tracing::warn!(?e, "не удалось добавить клиента");
            }
        })?;
        loop_handle.insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state: &mut State| {
                // SAFETY: Display не уничтожается, пока жив источник.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            },
        )?;

        let compositor_state = CompositorState::new::<State>(&dh);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<State>(&dh);
        let kde_default = match config.windows.decorations {
            syndesktop_common::config::DecorationMode::Server => {
                smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode::Server
            }
            syndesktop_common::config::DecorationMode::Client => {
                smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode::Client
            }
        };
        let kde_decoration_state = KdeDecorationState::new::<State>(&dh, kde_default);
        let layer_shell_state = WlrLayerShellState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<State>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<State>(&dh);
        let data_control_state =
            DataControlState::new::<State, _>(&dh, Some(&primary_selection_state), |_| true);
        let xdg_activation_state = XdgActivationState::new::<State>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<State>(&dh);
        let presentation_state = PresentationState::new::<State>(&dh, clock.id() as u32);
        let viewporter_state = ViewporterState::new::<State>(&dh);
        let keyboard_shortcuts_inhibit_state = KeyboardShortcutsInhibitState::new::<State>(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<State>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<State>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<State>(&dh);
        let idle_notifier_state = IdleNotifierState::new(&dh, loop_handle.clone());
        let idle_inhibit_state = IdleInhibitManagerState::new::<State>(&dh);
        let session_lock_state = SessionLockManagerState::new::<State, _>(&dh, |_| true);
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<State>(&dh);
        let cursor_shape_state = CursorShapeManagerState::new::<State>(&dh);
        let xdg_foreign_state = XdgForeignState::new::<State>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<State>(&dh);
        let tablet_manager_state = TabletManagerState::new::<State>(&dh);
        crate::screencopy::init(&dh);
        TextInputManagerState::new::<State>(&dh);
        InputMethodManagerState::new::<State, _>(&dh, |_| true);
        VirtualKeyboardManagerState::new::<State, _>(&dh, |_| true);
        crate::appmenu::init(&dh);
        smithay::wayland::security_context::SecurityContextState::new::<State, _>(&dh, |client| {
            client
                .get_data::<ClientState>()
                .is_none_or(|c| c.security_context.is_none())
        });

        let mut seat = seat_state.new_wl_seat(&dh, seat_name);
        let kb = &config.input.keyboard;
        let keyboard = seat
            .add_keyboard(
                XkbConfig {
                    rules: "",
                    model: &kb.model,
                    layout: &kb.layouts,
                    variant: &kb.variants,
                    options: if kb.options.is_empty() { None } else { Some(kb.options.clone()) },
                },
                kb.repeat_delay,
                kb.repeat_rate,
            )
            .or_else(|e| {
                tracing::error!(?e, "раскладка из конфига не принята xkb — беру us");
                seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate)
            })
            .map_err(|e| anyhow::anyhow!("клавиатура: {e:?}"))?;
        let pointer = seat.add_pointer();

        let (bindings, binding_errors) = Bindings::from_config(&config);
        let mut config_error = config_error;
        if !binding_errors.is_empty() {
            let msg = binding_errors.join("; ");
            tracing::warn!(msg, "ошибки в сочетаниях клавиш");
            config_error.get_or_insert(msg);
        }
        let (rules, rule_errors) = crate::wm::rules::compile(&config.rules);
        if !rule_errors.is_empty() {
            config_error.get_or_insert(rule_errors.join("; "));
        }
        let deco_theme = DecoTheme::from_config(&config.decorations, &config.appearance, 1);
        let title_font = TitleFont::load(&config.appearance.font);
        let cursor = CursorManager::new(&config.appearance.cursor_theme, config.appearance.cursor_size);

        let wm = Wm::new(&config);
        let ipc = IpcServer::bind(&loop_handle, &socket_name)?;

        Ok(Self {
            display_handle: dh,
            loop_handle,
            loop_signal,
            clock,
            start_time: Instant::now(),
            socket_name,
            nested,
            config_watcher: FileWatcher::new(crate::config::watched_files(&config)),
            config,
            config_error,
            bindings,
            rules,
            deco_theme,
            title_font,
            deco_generation: 1,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            kde_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            xdg_activation_state,
            fractional_scale_state,
            presentation_state,
            viewporter_state,
            keyboard_shortcuts_inhibit_state,
            pointer_gestures_state,
            relative_pointer_state,
            pointer_constraints_state,
            idle_notifier_state,
            idle_inhibit_state,
            session_lock_state,
            foreign_toplevel_list_state,
            cursor_shape_state,
            xdg_foreign_state,
            single_pixel_buffer_state,
            tablet_manager_state,
            seat,
            keyboard,
            pointer,
            cursor_status: CursorImageStatus::default_named(),
            cursor,
            cursor_hidden: false,
            last_pointer_motion: Instant::now(),
            dnd_icon: None,
            suppressed_keys: HashSet::new(),
            input_devices: Vec::new(),
            space: Space::default(),
            popups: PopupManager::default(),
            wm,
            output_data: HashMap::new(),
            lock: LockState::Unlocked,
            lock_surfaces: HashMap::new(),
            idle_inhibitors: HashSet::new(),
            last_activity: Instant::now(),
            monitors_off: false,
            ipc,
            shell: crate::spawn::ShellProcess::default(),
            xwayland: None,
            ipc_dirty: false,
            pending_copies: Vec::new(),
            frame_ids: Default::default(),
            gesture: Default::default(),
            last_title_click: None,
            last_kb_layout: None,
            restart_requested: false,
        })
    }

    // ── выводы ──

    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.space.outputs()
    }

    pub fn output_geometry(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        self.space.output_geometry(output)
    }

    pub fn output_by_name(&self, name: &str) -> Option<Output> {
        self.space.outputs().find(|o| o.name() == name).cloned()
    }

    /// Вывод под указателем (или первый).
    pub fn output_under_pointer(&self) -> Option<Output> {
        let pos = self.pointer.current_location();
        self.space
            .output_under(pos)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned()
    }

    /// Вывод, на котором находится точка; если ни на каком — ближайший.
    pub fn output_at(&self, p: Point<f64, Logical>) -> Option<Output> {
        if let Some(o) = self.space.output_under(p).next() {
            return Some(o.clone());
        }
        self.space
            .outputs()
            .min_by_key(|o| {
                let g = self.space.output_geometry(o).unwrap_or_default();
                let cx = p.x.clamp(g.loc.x as f64, (g.loc.x + g.size.w) as f64);
                let cy = p.y.clamp(g.loc.y as f64, (g.loc.y + g.size.h) as f64);
                ((cx - p.x).powi(2) + (cy - p.y).powi(2)) as i64
            })
            .cloned()
    }

    /// Основной вывод: из конфига (`primary = true`) или первый.
    pub fn primary_output(&self) -> Option<Output> {
        let primary_name = self.config.outputs.iter().find(|o| o.primary).map(|o| o.name.clone());
        primary_name
            .and_then(|n| self.space.outputs().find(|o| crate::backend::output_matches(o, &n)).cloned())
            .or_else(|| self.space.outputs().next().cloned())
    }

    /// Рабочая область вывода: геометрия минус зоны панелей (глобальные координаты).
    pub fn work_area(&self, output: &Output) -> Rectangle<i32, Logical> {
        let geo = self.space.output_geometry(output).unwrap_or_default();
        let map = smithay::desktop::layer_map_for_output(output);
        let zone = map.non_exclusive_zone();
        Rectangle::new(geo.loc + zone.loc, zone.size)
    }

    // ── перерисовка ──

    /// Запросить перерисовку вывода.
    pub fn queue_redraw(&mut self, output: &Output) {
        let data = self.output_data.entry(output.clone()).or_default();
        data.redraw = match data.redraw {
            RedrawState::Idle => RedrawState::Queued,
            RedrawState::Queued => RedrawState::Queued,
            RedrawState::WaitingForVBlank { .. } => RedrawState::WaitingForVBlank { redraw_needed: true },
            RedrawState::WaitingForEstimatedVBlank { .. } => {
                RedrawState::WaitingForEstimatedVBlank { redraw_needed: true }
            }
        };
    }

    pub fn queue_redraw_all(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for o in outputs {
            self.queue_redraw(&o);
        }
    }

    /// Перерисовать выводы, на которых видна поверхность.
    pub fn queue_redraw_for_surface(&mut self, _surface: &WlSurface) {
        // Проще и надёжнее перерисовать все: трекер повреждений сам отсечёт
        // неизменившиеся выводы, а лишний кадр без повреждений ничего не стоит.
        self.queue_redraw_all();
    }

    /// Истёкшее время сеанса — для анимаций курсора и подписей кадров.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Режим декораций для новых окон.
    pub fn default_decoration_mode(&self) -> zxdg_toplevel_decoration_v1::Mode {
        match self.config.windows.decorations {
            syndesktop_common::config::DecorationMode::Server => zxdg_toplevel_decoration_v1::Mode::ServerSide,
            syndesktop_common::config::DecorationMode::Client => zxdg_toplevel_decoration_v1::Mode::ClientSide,
        }
    }

    pub fn is_locked(&self) -> bool {
        !matches!(self.lock, LockState::Unlocked)
    }
}

/// Данные клиента для обработчиков композитора (xwayland или обычный).
pub fn client_compositor_state(client: &smithay::reexports::wayland_server::Client) -> &CompositorClientState {
    if let Some(state) = client.get_data::<XWaylandClientData>() {
        return &state.compositor_state;
    }
    if let Some(state) = client.get_data::<ClientState>() {
        return &state.compositor_state;
    }
    panic!("неизвестный тип данных клиента")
}
