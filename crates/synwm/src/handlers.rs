//! Обработчики протоколов Wayland.

use std::os::fd::OwnedFd;
use std::sync::Arc;

use smithay::{
    backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state},
    delegate_compositor, delegate_cursor_shape, delegate_data_control, delegate_data_device,
    delegate_dmabuf, delegate_foreign_toplevel_list, delegate_fractional_scale, delegate_idle_inhibit,
    delegate_idle_notify, delegate_input_method_manager, delegate_kde_decoration,
    delegate_keyboard_shortcuts_inhibit, delegate_layer_shell, delegate_output, delegate_pointer_constraints,
    delegate_pointer_gestures, delegate_presentation, delegate_primary_selection, delegate_relative_pointer,
    delegate_seat, delegate_security_context, delegate_session_lock, delegate_shm,
    delegate_single_pixel_buffer, delegate_tablet_manager, delegate_text_input_manager, delegate_viewporter,
    delegate_virtual_keyboard_manager, delegate_xdg_activation, delegate_xdg_decoration,
    delegate_xdg_foreign, delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, LayerSurface, PopupKind,
        PopupManager, Window, WindowSurfaceType,
    },
    input::{
        keyboard::LedState,
        pointer::{CursorImageStatus, CursorImageSurfaceData, Focus, PointerHandle},
        Seat, SeatHandler, SeatState,
    },
    output::Output,
    reexports::{
        calloop::Interest,
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode, shell::server::xdg_toplevel,
        },
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
            Mode as KdeMode, OrgKdeKwinServerDecoration,
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
            Client, Resource, WEnum,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface, with_states, BufferAssignment,
            CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
        },
        dmabuf::{get_dmabuf, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        input_method::{InputMethodHandler, PopupSurface as ImPopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
        },
        output::OutputHandler,
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler},
        seat::WaylandFocus,
        security_context::{SecurityContext, SecurityContextHandler, SecurityContextListenerSource},
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            primary_selection::{set_primary_focus, PrimarySelectionHandler, PrimarySelectionState},
            wlr_data_control::{DataControlHandler, DataControlState},
            SelectionHandler, SelectionSource, SelectionTarget,
        },
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
        shell::{
            kde::decoration::{KdeDecorationHandler, KdeDecorationState},
            wlr_layer::{Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState},
            xdg::{
                decoration::XdgDecorationHandler, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
                XdgShellState,
            },
        },
        shm::{ShmHandler, ShmState},
        tablet_manager::TabletSeatHandler,
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::{
    focus::FocusTarget,
    state::{ClientState, LockState, State},
    wm::ResizeEdge,
};

// ─── compositor ─────────────────────────────────────────────────────────────

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.core.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        crate::state::client_compositor_state(client)
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        // Не показывать dmabuf, пока GPU клиента не дорисовал его.
        add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            let maybe_dmabuf = with_states(surface, |data| {
                data.cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|a| match a {
                        BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                        _ => None,
                    })
            });
            if let Some(dmabuf) = maybe_dmabuf {
                if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                    if let Some(client) = surface.client() {
                        let res = state.core.loop_handle.insert_source(source, move |_, _, state| {
                            let dh = state.core.display_handle.clone();
                            state.client_compositor_state(&client).blocker_cleared(state, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                        }
                    }
                }
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend.early_import(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(id) = self.core.wm.id_by_surface(&root) {
                let window = self.core.wm.get(id).unwrap().window.clone();
                window.on_commit();
                if &root == surface {
                    self.toplevel_commit(id, surface);
                }
            }
        }

        self.core.popups.commit(surface);
        if let Some(popup) = self.core.popups.find_popup(surface) {
            if let PopupKind::Xdg(ref xdg) = popup {
                if !xdg.is_initial_configure_sent() {
                    let _ = xdg.send_configure();
                }
            }
        }

        self.layer_commit(surface);
        self.cursor_commit(surface);

        if let LockState::Locking(_) = self.core.lock {
            self.maybe_finish_lock();
        }

        self.core.queue_redraw_for_surface(surface);
    }
}

impl State {
    fn toplevel_commit(&mut self, id: crate::wm::WindowId, surface: &WlSurface) {
        let Some(m) = self.core.wm.get(id) else { return };
        let Some(toplevel) = m.window.toplevel().cloned() else {
            return;
        };
        if !toplevel.is_initial_configure_sent() {
            self.initial_configure(surface);
            return;
        }
        let has_buffer = with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
        let mapped = m.mapped;
        if has_buffer && !mapped {
            self.map_window(id);
        } else if !has_buffer && mapped {
            self.unmap_window(id, false);
        } else if mapped {
            self.fixup_resize_location(surface);
        }
    }

    fn layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self
            .core
            .space
            .outputs()
            .find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
        else {
            return;
        };
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .map(|d| d.lock().unwrap().initial_configure_sent)
                .unwrap_or(true)
        });
        let changed = {
            let mut map = layer_map_for_output(&output);
            let before = map.non_exclusive_zone();
            map.arrange();
            if !initial_configure_sent {
                if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                    layer.layer_surface().send_configure();
                }
            }
            before != map.non_exclusive_zone()
        };
        // Панель поменяла резервируемую зону — переложить развёрнутые и плитку.
        if changed {
            self.relayout();
        }
        // Слой с эксклюзивной клавиатурой получил фокус.
        self.update_layer_keyboard_focus();
        self.core.queue_redraw(&output);
    }

    fn cursor_commit(&mut self, surface: &WlSurface) {
        if matches!(&self.core.cursor_status, CursorImageStatus::Surface(s) if s == surface) {
            with_states(surface, |states| {
                if let Some(mut attrs) = states.data_map.get::<CursorImageSurfaceData>().map(|a| a.lock().unwrap()) {
                    let delta = states.cached_state.get::<SurfaceAttributes>().current().buffer_delta.take();
                    if let Some(d) = delta {
                        attrs.hotspot -= d;
                    }
                }
            });
        }
    }

    /// Фокус клавиатуры для слоёв: эксклюзивный слой (меню запуска)
    /// забирает клавиатуру, пока открыт.
    pub fn update_layer_keyboard_focus(&mut self) {
        use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};
        if self.core.is_locked() {
            return;
        }
        let mut exclusive: Option<WlSurface> = None;
        for o in self.core.space.outputs() {
            let map = layer_map_for_output(o);
            for layer in map.layers() {
                let data = with_states(layer.wl_surface(), |s| *s.cached_state.get::<LayerSurfaceCachedState>().current());
                if data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                    && matches!(data.layer, Layer::Top | Layer::Overlay)
                {
                    exclusive = Some(layer.wl_surface().clone());
                }
            }
        }
        let current = self.core.keyboard.current_focus().map(|f| f.0);
        match exclusive {
            Some(s) if current.as_ref() != Some(&s) => self.focus_surface(Some(s)),
            None => {
                // Эксклюзивный слой закрылся — вернуть фокус окну.
                let current_is_layer = current.as_ref().is_some_and(|c| self.is_layer_surface(c));
                let current_dead = current.as_ref().is_some_and(|c| !c.is_alive());
                if current_dead || (current_is_layer && !self.layer_wants_keyboard(current.as_ref().unwrap())) {
                    let f = self.core.wm.focused;
                    self.focus_window(f);
                }
            }
            _ => {}
        }
    }

    pub fn is_layer_surface(&self, s: &WlSurface) -> bool {
        self.core
            .space
            .outputs()
            .any(|o| layer_map_for_output(o).layer_for_surface(s, WindowSurfaceType::TOPLEVEL).is_some())
    }

    fn layer_wants_keyboard(&self, s: &WlSurface) -> bool {
        use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};
        if !s.is_alive() {
            return false;
        }
        let data = with_states(s, |st| *st.cached_state.get::<LayerSurfaceCachedState>().current());
        data.keyboard_interactivity != KeyboardInteractivity::None
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.core.shm_state
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.core.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: smithay::backend::allocator::dmabuf::Dmabuf, notifier: ImportNotifier) {
        if self.backend.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_dmabuf!(State);

// ─── xdg-shell ──────────────────────────────────────────────────────────────

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.core.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        self.add_window(window);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&PopupKind::Xdg(surface.clone()));
        if let Err(e) = self.core.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(?e, "не удалось отследить всплывающее окно");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&PopupKind::Xdg(surface.clone()));
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) else { return };
        let pointer = seat.get_pointer().unwrap();
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start) = pointer.grab_start_data() else { return };
        if !start.focus.as_ref().is_some_and(|(f, _)| f.0.id().same_client_as(&surface.wl_surface().id())) {
            return;
        }
        self.start_move(id, start, serial);
    }

    fn resize_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial, edges: xdg_toplevel::ResizeEdge) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) else { return };
        let pointer = seat.get_pointer().unwrap();
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start) = pointer.grab_start_data() else { return };
        if !start.focus.as_ref().is_some_and(|(f, _)| f.0.id().same_client_as(&surface.wl_surface().id())) {
            return;
        }
        self.start_resize(id, ResizeEdge::from(edges), start, serial);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let root_target = FocusTarget(root);
        let Ok(mut grab) = self.core.popups.grab_popup(root_target, kind, &seat, serial) else {
            return;
        };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, smithay::desktop::PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, smithay::desktop::PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            if self.core.wm.get(id).is_some_and(|m| m.mapped) {
                self.apply_maximized(id, true);
            } else if let Some(m) = self.core.wm.get_mut(id) {
                m.maximized = true;
            }
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            if self.core.wm.get(id).is_some_and(|m| m.maximized) {
                self.apply_maximized(id, false);
            }
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        let output = output.as_ref().and_then(Output::from_resource);
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            if self.core.wm.get(id).is_some_and(|m| m.mapped) {
                self.apply_fullscreen(id, true, output);
            } else if let Some(m) = self.core.wm.get_mut(id) {
                m.fullscreen = true;
                if let Some(o) = output {
                    m.output = Some(o.name());
                }
            }
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            if self.core.wm.get(id).is_some_and(|m| m.fullscreen) {
                self.apply_fullscreen(id, false, None);
            }
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            self.minimize(id);
        }
    }

    fn show_window_menu(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial, location: Point<i32, Logical>) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            let loc = self.core.wm.get(id).map(|m| m.loc).unwrap_or_default() + location;
            self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand {
                command: format!("window-menu {id} {} {}", loc.x, loc.y),
            });
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            self.unmap_window(id, true);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            self.window_title_changed(id);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.core.wm.id_by_surface(surface.wl_surface()) {
            self.window_title_changed(id);
        }
    }
}

impl State {
    /// Уместить всплывающее окно в пределы вывода.
    pub fn unconstrain_popup(&self, popup: &PopupKind) {
        let Ok(root) = find_popup_root_surface(popup) else { return };
        let PopupKind::Xdg(xdg) = popup else { return };
        // Корень — окно.
        let (root_geo, output_geo) = if let Some(m) = self.core.wm.by_surface(&root) {
            let geo = m.geometry();
            let out = m
                .output
                .as_ref()
                .and_then(|n| self.core.output_by_name(n))
                .or_else(|| self.core.output_at(geo.loc.to_f64()))
                .and_then(|o| self.core.space.output_geometry(&o));
            (geo, out)
        } else if let Some((output, layer_geo)) = self.core.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
            let g = map.layer_geometry(layer)?;
            Some((o.clone(), g))
        }) {
            let og = self.core.space.output_geometry(&output).unwrap_or_default();
            (Rectangle::new(og.loc + layer_geo.loc, layer_geo.size), Some(og))
        } else {
            return;
        };
        let Some(output_geo) = output_geo else { return };
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(popup);
        target.loc -= root_geo.loc;
        xdg.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

delegate_xdg_shell!(State);

// ─── декорации ──────────────────────────────────────────────────────────────

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        let ssd = self
            .core
            .wm
            .by_surface(toplevel.wl_surface())
            .map(|m| m.ssd)
            .unwrap_or(true);
        toplevel.with_pending_state(|s| {
            s.decoration_mode = Some(if ssd { DecoMode::ServerSide } else { DecoMode::ClientSide });
        });
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecoMode) {
        // Желание клиента уважаем (GTK с заголовками-панелями просит CSD),
        // кроме случая, когда правило окна явно задало рамки.
        let forced = self.forced_decorations(toplevel.wl_surface());
        let ssd = forced.unwrap_or(mode == DecoMode::ServerSide);
        self.set_window_ssd(toplevel.wl_surface(), ssd);
        toplevel.with_pending_state(|s| {
            s.decoration_mode = Some(if ssd { DecoMode::ServerSide } else { DecoMode::ClientSide });
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        let ssd = self.core.default_decoration_mode() == DecoMode::ServerSide;
        let ssd = self.forced_decorations(toplevel.wl_surface()).unwrap_or(ssd);
        self.set_window_ssd(toplevel.wl_surface(), ssd);
        toplevel.with_pending_state(|s| {
            s.decoration_mode = Some(if ssd { DecoMode::ServerSide } else { DecoMode::ClientSide });
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.core.kde_decoration_state
    }

    fn request_mode(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration, mode: WEnum<KdeMode>) {
        let WEnum::Value(mode) = mode else { return };
        let forced = self.forced_decorations(surface);
        let ssd = forced.unwrap_or(mode == KdeMode::Server);
        decoration.mode(if ssd { KdeMode::Server } else { KdeMode::Client });
        self.set_window_ssd(surface, ssd);
    }
}

impl State {
    fn forced_decorations(&self, surface: &WlSurface) -> Option<bool> {
        let m = self.core.wm.by_surface(surface)?;
        crate::wm::rules::resolve(&self.core.rules, &m.app_id(), &m.title()).decorations
    }

    fn set_window_ssd(&mut self, surface: &WlSurface, ssd: bool) {
        let Some(m) = self.core.wm.by_surface_mut(surface) else { return };
        if m.ssd == ssd {
            return;
        }
        m.ssd = ssd;
        if m.mapped {
            self.relayout();
        }
    }
}

delegate_xdg_decoration!(State);
delegate_kde_decoration!(State);

// ─── layer-shell ────────────────────────────────────────────────────────────

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.core.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.core.output_under_pointer());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        tracing::debug!(namespace, output = output.name(), "новая layer-поверхность");
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!(?e, "layer-поверхность не размещена");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&PopupKind::Xdg(popup));
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.core.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            layer.map(|l| (o.clone(), l))
        });
        if let Some((output, layer)) = found {
            let mut map = layer_map_for_output(&output);
            map.unmap_layer(&layer);
            drop(map);
            self.relayout();
            self.core.queue_redraw(&output);
        }
        // Фокус был на этом слое — вернуть окну.
        let current = self.core.keyboard.current_focus().map(|f| f.0);
        if current.as_ref() == Some(surface.wl_surface()) {
            let f = self.core.wm.focused;
            self.focus_window(f);
        }
    }
}

delegate_layer_shell!(State);

// ─── seat ───────────────────────────────────────────────────────────────────

impl SeatHandler for State {
    type KeyboardFocus = FocusTarget;
    type PointerFocus = FocusTarget;
    type TouchFocus = FocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.core.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&FocusTarget>) {
        let dh = &self.core.display_handle;
        let client = focused.and_then(|f| dh.get_client(f.0.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.core.cursor_status = image;
        self.core.queue_redraw_all();
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, led_state: LedState) {
        for d in &mut self.core.input_devices {
            if d.has_capability(smithay::reexports::input::DeviceCapability::Keyboard) {
                d.led_update(led_state.into());
            }
        }
    }
}

delegate_seat!(State);

impl TabletSeatHandler for State {}
delegate_tablet_manager!(State);
delegate_cursor_shape!(State);

// ─── буфер обмена и DnD ────────────────────────────────────────────────────

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(xwm) = self.core.xwayland.as_mut().and_then(|x| x.wm.as_mut()) {
            if let Err(e) = xwm.new_selection(ty, source.map(|s| s.mime_types())) {
                tracing::warn!(?e, "буфер обмена не передан в Xwayland");
            }
        }
    }

    fn send_selection(&mut self, ty: SelectionTarget, mime_type: String, fd: OwnedFd, _seat: Seat<Self>, _user_data: &()) {
        let handle = self.core.loop_handle.clone();
        if let Some(xwm) = self.core.xwayland.as_mut().and_then(|x| x.wm.as_mut()) {
            if let Err(e) = xwm.send_selection(ty, mime_type, fd, handle) {
                tracing::warn!(?e, "буфер обмена не получен из Xwayland");
            }
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.core.data_device_state
    }
}

impl ClientDndGrabHandler for State {
    fn started(&mut self, _source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>, icon: Option<WlSurface>, _seat: Seat<Self>) {
        self.core.dnd_icon = icon;
    }

    fn dropped(&mut self, _target: Option<WlSurface>, _validated: bool, _seat: Seat<Self>) {
        self.core.dnd_icon = None;
        self.core.queue_redraw_all();
    }
}

impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.core.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.core.data_control_state
    }
}

delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_data_control!(State);

// ─── выводы и масштаб ──────────────────────────────────────────────────────

impl OutputHandler for State {}
delegate_output!(State);

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let mut root = surface.clone();
        while let Some(p) = get_parent(&root) {
            root = p;
        }
        let output = self
            .core
            .wm
            .by_surface(&root)
            .and_then(|m| m.output.as_ref())
            .and_then(|n| self.core.output_by_name(n))
            .or_else(|| self.core.output_under_pointer());
        if let Some(o) = output {
            with_states(&surface, |states| {
                with_fractional_scale(states, |fs| fs.set_preferred_scale(o.current_scale().fractional_scale()));
            });
        }
    }
}

delegate_fractional_scale!(State);
delegate_presentation!(State);
delegate_viewporter!(State);
delegate_single_pixel_buffer!(State);

// ─── активация окон ────────────────────────────────────────────────────────

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.core.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        // Токен законен, если выдан по свежему вводу в окне с фокусом
        // (или запросила наша оболочка — у неё нет серийника клавиатуры).
        if let Some((serial, seat)) = data.serial {
            let keyboard = self.core.keyboard.clone();
            Seat::<State>::from_resource(&seat).as_ref() == Some(&self.core.seat)
                && keyboard.last_enter().map(|e| serial.is_no_older_than(&e)).unwrap_or(false)
        } else {
            data.client_id
                .and_then(|id| self.core.display_handle.backend_handle().get_client_data(id).ok())
                .and_then(|d| d.downcast_ref::<ClientState>().map(|c| c.privileged))
                .unwrap_or(false)
        }
    }

    fn request_activation(&mut self, _token: XdgActivationToken, token_data: XdgActivationTokenData, surface: WlSurface) {
        // Сюда доходят только законные токены (см. token_created).
        let fresh = token_data.timestamp.elapsed().as_secs() < 10;
        let allowed = fresh || self.core.config.windows.focus_stealing;
        self.activate_or_mark_urgent(&surface, allowed);
    }
}

delegate_xdg_activation!(State);

// ─── указатель ──────────────────────────────────────────────────────────────

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().is_some_and(|f| &f.0 == surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
        }
    }

    fn cursor_position_hint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>, location: Point<f64, Logical>) {
        let active = with_pointer_constraint(surface, pointer, |c| c.is_some_and(|c| c.is_active()));
        if active {
            if let Some(m) = self.core.wm.by_surface(surface) {
                let origin = m.loc.to_f64();
                pointer.set_location(origin + location);
            }
        }
    }
}

delegate_pointer_constraints!(State);
delegate_relative_pointer!(State);
delegate_pointer_gestures!(State);

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.core.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Виртуальные машины и удалённые рабочие столы — пусть забирают.
        inhibitor.activate();
    }
}

delegate_keyboard_shortcuts_inhibit!(State);

// ─── простой и блокировка ──────────────────────────────────────────────────

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.core.idle_notifier_state
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.core.idle_inhibitors.insert(surface);
        self.update_idle_inhibit();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.core.idle_inhibitors.remove(&surface);
        self.update_idle_inhibit();
    }
}

impl State {
    pub fn update_idle_inhibit(&mut self) {
        self.core.idle_inhibitors.retain(|s| s.is_alive());
        let inhibited = !self.core.idle_inhibitors.is_empty();
        self.core.idle_notifier_state.set_is_inhibited(inhibited);
    }
}

delegate_idle_notify!(State);
delegate_idle_inhibit!(State);

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.core.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        tracing::info!("блокировка экрана");
        if self.core.space.outputs().next().is_none() {
            confirmation.lock();
            self.core.lock = LockState::Locked;
        } else {
            self.core.lock = LockState::Locking(confirmation);
        }
        // Снять фокус с окон: клавиатура — только экрану блокировки.
        self.focus_surface(None);
        self.core.queue_redraw_all();
    }

    fn unlock(&mut self) {
        tracing::info!("разблокировка экрана");
        self.core.lock = LockState::Unlocked;
        self.core.lock_surfaces.clear();
        let f = self.core.wm.focused;
        self.focus_window(f);
        self.core.queue_redraw_all();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else { return };
        let size = self.core.space.output_geometry(&output).map(|g| g.size).unwrap_or_default();
        surface.with_pending_state(|s| {
            s.size = Some((size.w as u32, size.h as u32).into());
        });
        surface.send_configure();
        // Клавиатура — поверхности блокировки вывода под указателем.
        let under = self.core.output_under_pointer();
        if under.as_ref() == Some(&output) || self.core.lock_surfaces.is_empty() {
            let s = surface.wl_surface().clone();
            let keyboard = self.core.keyboard.clone();
            keyboard.set_focus(self, Some(FocusTarget(s)), SERIAL_COUNTER.next_serial());
        }
        self.core.lock_surfaces.insert(output, surface);
        self.core.queue_redraw_all();
    }
}

impl State {
    /// Все выводы закрыты поверхностями блокировки с буферами — подтвердить.
    fn maybe_finish_lock(&mut self) {
        let all = self.core.space.outputs().all(|o| {
            self.core.lock_surfaces.get(o).is_some_and(|s| {
                with_renderer_surface_state(s.wl_surface(), |st| st.buffer().is_some()).unwrap_or(false)
            })
        });
        if all {
            if let LockState::Locking(locker) = std::mem::take(&mut self.core.lock) {
                locker.lock();
                self.core.lock = LockState::Locked;
                tracing::info!("экран заблокирован");
            }
        }
    }
}

delegate_session_lock!(State);

// ─── прочее ────────────────────────────────────────────────────────────────

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.core.foreign_toplevel_list_state
    }
}

delegate_foreign_toplevel_list!(State);

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.core.xdg_foreign_state
    }
}

delegate_xdg_foreign!(State);

impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let res = self.core.loop_handle.insert_source(source, move |stream, _, state| {
            let data = ClientState { security_context: Some(context.clone()), ..Default::default() };
            if let Err(e) = state.core.display_handle.insert_client(stream, Arc::new(data)) {
                tracing::warn!(?e, "клиент из security-context не добавлен");
            }
        });
        if let Err(e) = res {
            tracing::warn!(?e, "security-context: источник не зарегистрирован");
        }
    }
}

delegate_security_context!(State);

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: ImPopupSurface) {
        if let Err(e) = self.core.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!(?e, "всплывающее окно метода ввода");
        }
    }

    fn popup_repositioned(&mut self, _surface: ImPopupSurface) {}

    fn dismiss_popup(&mut self, surface: ImPopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.core
            .wm
            .by_surface(parent)
            .map(|m| m.window.geometry())
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(State);
delegate_text_input_manager!(State);
delegate_virtual_keyboard_manager!(State);
