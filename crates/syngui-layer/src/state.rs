//! Цикл событий: calloop + Wayland (sctk), команды из очереди, кадры.

use crate::gpu::Gpu;
use crate::keys::{map_cursor, map_key};
use crate::surface::{Backend, Surface};
use crate::{Command, KeyInfo, OutputInfo, RunOptions, SurfaceId, SurfaceSpec};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_session_lock, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            self,
            ping::{make_ping, Ping},
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, RegistrationToken,
        },
        calloop_wayland_source::WaylandSource,
        protocols::wp::{
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
                wp_fractional_scale_v1::{self, WpFractionalScaleV1},
            },
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    session_lock::{SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface, SessionLockSurfaceConfigure},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{Shm, ShmHandler},
};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;
use syngui::core::Point;
use syngui::input::MouseButton;
use syngui::mss::StyleEngine;
use wayland_client::{
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};

pub struct State {
    conn: Option<Connection>,
    qh: Option<QueueHandle<State>>,
    registry: Option<RegistryState>,
    seat_state: Option<SeatState>,
    output_state: Option<OutputState>,
    compositor: Option<CompositorState>,
    layer_shell: Option<LayerShell>,
    shm: Option<Shm>,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    lock_state: Option<SessionLockState>,
    session_lock: Option<SessionLock>,
    lock_factory: Option<crate::LockFactory>,
    gpu: Gpu,
    engine: StyleEngine,
    surfaces: BTreeMap<SurfaceId, Surface>,
    pointer: Option<ThemedPointer>,
    pointer_focus: Option<SurfaceId>,
    last_cursor: Option<syngui::input::CursorIcon>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    kb_focus: Option<SurfaceId>,
    modifiers: syngui::input::Modifiers,
    handle: LoopHandle<'static, State>,
    timers: HashMap<u64, RegistrationToken>,
    headless: Option<(u32, u32)>,
    dump_dir: Option<PathBuf>,
    options: RunOptions,
    exit: bool,
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn parse_stylesheet(mss: &str) -> StyleEngine {
    match syngui::mss::parse_stylesheet_str(mss) {
        Ok(ss) => StyleEngine::new(ss),
        Err(e) => {
            log::error!("syngui-layer: ошибка разбора MSS: {e:?}");
            StyleEngine::new(syngui::mss::parse_stylesheet_str("").unwrap_or_default())
        }
    }
}

pub fn run(options: RunOptions, stylesheet: &str, init: Box<dyn FnOnce()>) -> anyhow::Result<()> {
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();

    let headless = std::env::var("SYNGUI_LAYER_HEADLESS").ok().and_then(|s| parse_size(&s));
    let dump_dir = std::env::var_os("SYNGUI_LAYER_DUMP").map(PathBuf::from);
    if let Some(d) = &dump_dir {
        std::fs::create_dir_all(d)?;
    }

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    // Пробуждение цикла из других потоков (кросс-поточные сигналы,
    // run_on_main_thread) и из очереди команд.
    let (ping, ping_source) = make_ping()?;
    handle
        .insert_source(ping_source, |_, _, _| {})
        .map_err(|e| anyhow::anyhow!("ping: {e}"))?;
    {
        let p = ping.clone();
        syngui::async_runtime::set_main_thread_waker(move || p.ping());
        syngui::signal::set_notifier(std::sync::Arc::new(Notifier(ping.clone())));
        syngui::async_runtime::set_async_notifier(std::sync::Arc::new(Notifier(ping.clone())));
        let p = ping.clone();
        crate::set_wake(Box::new(move || p.ping()));
    }

    let mut state = State {
        conn: None,
        qh: None,
        registry: None,
        seat_state: None,
        output_state: None,
        compositor: None,
        layer_shell: None,
        shm: None,
        fractional: None,
        viewporter: None,
        lock_state: None,
        session_lock: None,
        lock_factory: None,
        gpu: Gpu::new(),
        engine: parse_stylesheet(stylesheet),
        surfaces: BTreeMap::new(),
        pointer: None,
        pointer_focus: None,
        last_cursor: None,
        keyboard: None,
        kb_focus: None,
        modifiers: syngui::input::Modifiers::empty(),
        handle: handle.clone(),
        timers: HashMap::new(),
        headless,
        dump_dir,
        options,
        exit: false,
    };

    if let Some((w, h)) = headless {
        log::info!("syngui-layer: headless {w}x{h}, кадры — в SYNGUI_LAYER_DUMP");
        crate::outputs().set(vec![OutputInfo {
            name: "HEADLESS-1".into(),
            description: "Headless".into(),
            size: (w as i32, h as i32),
            scale: 1,
            ..Default::default()
        }]);
    } else {
        let conn = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
        let qh = event_queue.handle();
        state.bind_globals(&globals, &qh)?;
        state.conn = Some(conn.clone());
        state.qh = Some(qh);
        WaylandSource::new(conn, event_queue)
            .insert(handle.clone())
            .map_err(|e| anyhow::anyhow!("wayland source: {e}"))?;
        // Дождаться выводов (xdg-output имена) до init.
        for _ in 0..3 {
            event_loop.dispatch(Duration::from_millis(50), &mut state)?;
        }
        state.publish_outputs();
    }

    // Выход по времени — для проверок в CI/headless.
    if let Some(secs) = std::env::var("SYNGUI_LAYER_EXIT_AFTER").ok().and_then(|s| s.parse::<f64>().ok()) {
        handle
            .insert_source(Timer::from_duration(Duration::from_secs_f64(secs)), |_, _, st| {
                st.exit = true;
                TimeoutAction::Drop
            })
            .ok();
    }

    init();

    while !state.exit {
        state.tick();
        if state.exit {
            break;
        }
        let timeout = if state.headless.is_some() && state.surfaces.values().any(|s| s.needs_frame) {
            Some(Duration::from_millis(16))
        } else if state.any_ready_to_draw() {
            Some(Duration::ZERO)
        } else {
            None
        };
        event_loop.dispatch(timeout, &mut state)?;
        // Ошибка протокола убивает соединение: дальше крутиться бессмысленно.
        if let Some(err) = state.conn.as_ref().and_then(|c| c.protocol_error()) {
            anyhow::bail!("ошибка протокола Wayland: {} (объект {}@{})", err.message, err.object_interface, err.object_id);
        }
    }
    // Сначала wgpu-поверхности, потом всё остальное.
    for s in state.surfaces.values_mut() {
        s.teardown();
    }
    state.surfaces.clear();
    Ok(())
}

struct Notifier(Ping);

impl syngui::signal::RedrawNotifier for Notifier {
    fn request_redraw(&self) {
        self.0.ping();
    }
}

impl State {
    fn bind_globals(&mut self, globals: &GlobalList, qh: &QueueHandle<State>) -> anyhow::Result<()> {
        self.registry = Some(RegistryState::new(globals));
        self.seat_state = Some(SeatState::new(globals, qh));
        self.output_state = Some(OutputState::new(globals, qh));
        self.compositor = Some(CompositorState::bind(globals, qh)?);
        self.layer_shell =
            Some(LayerShell::bind(globals, qh).map_err(|_| anyhow::anyhow!("композитор не поддерживает wlr-layer-shell"))?);
        self.shm = Shm::bind(globals, qh).ok();
        self.fractional = globals.bind::<WpFractionalScaleManagerV1, _, _>(qh, 1..=1, ()).ok();
        self.viewporter = globals.bind::<WpViewporter, _, _>(qh, 1..=1, ()).ok();
        self.lock_state = Some(SessionLockState::new(globals, qh));
        Ok(())
    }

    fn output_list(&self) -> Vec<(wl_output::WlOutput, OutputInfo)> {
        let Some(os) = &self.output_state else { return Vec::new() };
        os.outputs()
            .filter_map(|o| {
                let info = os.info(&o)?;
                let size = info
                    .logical_size
                    .or_else(|| {
                        info.modes.iter().find(|m| m.current).map(|m| {
                            let s = info.scale_factor.max(1);
                            (m.dimensions.0 / s, m.dimensions.1 / s)
                        })
                    })
                    .unwrap_or((0, 0));
                Some((
                    o.clone(),
                    OutputInfo {
                        name: info.name.clone().unwrap_or_else(|| format!("output-{}", info.id)),
                        description: info.description.clone().unwrap_or_default(),
                        make: info.make.clone(),
                        model: info.model.clone(),
                        position: info.logical_position.unwrap_or(info.location),
                        size,
                        scale: info.scale_factor,
                    },
                ))
            })
            .collect()
    }

    fn publish_outputs(&mut self) {
        let list: Vec<OutputInfo> = self.output_list().into_iter().map(|(_, i)| i).collect();
        crate::outputs().set(list);
    }

    fn output_size_for(&self, name: Option<&str>) -> (u32, u32) {
        let outs = crate::outputs().get_untracked();
        let o = name.and_then(|n| outs.iter().find(|o| o.name == n)).or_else(|| outs.first());
        o.map(|o| (o.size.0.max(0) as u32, o.size.1.max(0) as u32)).unwrap_or((1920, 1080))
    }

    fn any_ready_to_draw(&self) -> bool {
        self.surfaces
            .values()
            .any(|s| s.configured && !s.frame_pending && (s.needs_frame || s.view.has_dirty()))
    }

    /// Всё, что происходит между событиями: колбэки других потоков,
    /// команды, эффекты, кадры.
    fn tick(&mut self) {
        for _ in 0..4 {
            syngui::async_runtime::drain_main_thread_callbacks();
            let cmds = crate::take_commands();
            if cmds.is_empty() {
                break;
            }
            for c in cmds {
                self.apply(c);
            }
        }
        syngui::signal::drain_and_run_effects();
        // Эффекты могли насоздавать команд.
        for c in crate::take_commands() {
            self.apply(c);
        }
        self.realize_pending();
        self.draw_all();
        if let Some(conn) = &self.conn {
            let _ = conn.flush();
        }
    }

    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Create { id, spec, factory, hooks } => {
                let s = Surface::new(id, spec, hooks, factory);
                self.surfaces.insert(id, s);
                self.realize(id);
            }
            Command::Reconfigure { id, spec } => {
                let Some(s) = self.surfaces.get_mut(&id) else { return };
                let recreate = s.spec.output != spec.output
                    || s.spec.namespace != spec.namespace
                    || s.spec.layer != spec.layer;
                s.requested = spec.size;
                s.spec = spec;
                if recreate {
                    s.teardown();
                    self.realize(id);
                } else if let Some(layer) = s.layer() {
                    s.apply_spec();
                    layer.commit();
                } else if self.headless.is_some() {
                    s.teardown();
                    self.realize(id);
                }
            }
            Command::Close { id } => {
                if let Some(mut s) = self.surfaces.remove(&id) {
                    s.teardown();
                    if self.pointer_focus == Some(id) {
                        self.pointer_focus = None;
                    }
                    if self.kb_focus == Some(id) {
                        self.kb_focus = None;
                    }
                }
            }
            Command::Stylesheet(mss) => {
                self.engine = parse_stylesheet(&mss);
                for s in self.surfaces.values_mut() {
                    s.view.restyle_all(&self.engine);
                    s.needs_frame = true;
                }
            }
            Command::Timer { id, after, mut f } => {
                let tok = self.handle.insert_source(Timer::from_duration(after), move |_, _, st: &mut State| {
                    match f() {
                        Some(d) => TimeoutAction::ToDuration(d),
                        None => {
                            st.timers.remove(&id);
                            TimeoutAction::Drop
                        }
                    }
                });
                if let Ok(tok) = tok {
                    self.timers.insert(id, tok);
                }
            }
            Command::CancelTimer(id) => {
                if let Some(tok) = self.timers.remove(&id) {
                    self.handle.remove(tok);
                }
            }
            Command::Redraw(id) => {
                for (sid, s) in self.surfaces.iter_mut() {
                    if id.is_none() || id == Some(*sid) {
                        s.needs_frame = true;
                        s.view.invalidate();
                    }
                }
            }
            Command::Lock(factory) => self.lock(factory),
            Command::Unlock => self.unlock(),
            Command::Quit => self.exit = true,
        }
    }

    fn realize_pending(&mut self) {
        let pending: Vec<SurfaceId> = self
            .surfaces
            .iter()
            .filter(|(_, s)| matches!(s.backend, Backend::Pending))
            .map(|(id, _)| *id)
            .collect();
        for id in pending {
            self.realize(id);
        }
    }

    /// Создать layer-surface (или offscreen) для поверхности, если вывод есть.
    fn realize(&mut self, id: SurfaceId) {
        if let Some((w, h)) = self.headless {
            let Some(s) = self.surfaces.get_mut(&id) else { return };
            let spec = &s.spec;
            let a = spec.anchor;
            use smithay_client_toolkit::shell::wlr_layer::Anchor;
            let lw = if spec.size.0 == 0 && a.contains(Anchor::LEFT | Anchor::RIGHT) {
                w as i32 - spec.margin[1] - spec.margin[3]
            } else if spec.size.0 == 0 {
                w as i32
            } else {
                spec.size.0 as i32
            };
            let lh = if spec.size.1 == 0 && a.contains(Anchor::TOP | Anchor::BOTTOM) {
                h as i32 - spec.margin[0] - spec.margin[2]
            } else if spec.size.1 == 0 {
                h as i32
            } else {
                spec.size.1 as i32
            };
            s.backend = Backend::Headless { offscreen: None };
            s.logical = (lw.max(1) as u32, lh.max(1) as u32);
            s.configured = true;
            s.needs_frame = true;
            let (w, h) = s.logical;
            if let Some(cb) = s.hooks.on_resize.as_mut() {
                cb(w, h);
            }
            return;
        }
        let (Some(qh), Some(compositor), Some(layer_shell)) = (&self.qh, &self.compositor, &self.layer_shell) else {
            return;
        };
        let Some(s) = self.surfaces.get(&id) else { return };
        let output = match &s.spec.output {
            Some(name) => match self.output_list().into_iter().find(|(_, i)| &i.name == name) {
                Some((o, _)) => Some(o),
                None => return, // ждём вывод
            },
            None => None,
        };
        let wl = compositor.create_surface(qh);
        let layer = layer_shell.create_layer_surface(qh, wl, s.spec.layer, Some(s.spec.namespace.clone()), output.as_ref());
        let fractional = self.fractional.as_ref().map(|m| m.get_fractional_scale(layer.wl_surface(), qh, id));
        let viewport = match (&fractional, &self.viewporter) {
            (Some(_), Some(vp)) => Some(vp.get_viewport(layer.wl_surface(), qh, ())),
            _ => None,
        };
        let fractional = if viewport.is_some() { fractional } else {
            if let Some(f) = fractional {
                f.destroy();
            }
            None
        };
        let s = self.surfaces.get_mut(&id).unwrap();
        s.backend = Backend::Wayland { wsurf: None, viewport, fractional, role: crate::surface::Role::Layer(layer) };
        s.apply_spec();
        s.layer().unwrap().commit();
    }

    fn draw_all(&mut self) {
        let ids: Vec<SurfaceId> = self.surfaces.keys().copied().collect();
        for id in ids {
            let output_size = {
                let s = &self.surfaces[&id];
                self.output_size_for(s.spec.output.as_deref())
            };
            let font = self.options.font_family.clone();
            let dump = self.dump_dir.clone();
            let s = self.surfaces.get_mut(&id).unwrap();
            if !s.configured || s.frame_pending {
                continue;
            }
            if !(s.needs_frame || s.view.has_dirty() || s.factory.is_some()) {
                continue;
            }
            let qh = self.qh.clone();
            if let Err(e) = s.render(&mut self.gpu, &self.engine, qh.as_ref(), font, dump.as_deref(), output_size) {
                log::error!("syngui-layer: кадр {}: {e}", s.spec.namespace);
                s.needs_frame = false;
            }
        }
        self.update_cursor();
    }

    fn surface_id_of(&self, wl: &wl_surface::WlSurface) -> Option<SurfaceId> {
        self.surfaces.iter().find(|(_, s)| s.wl_surface().is_some_and(|l| l == wl)).map(|(id, _)| *id)
    }

    fn update_cursor(&mut self) {
        let Some(id) = self.pointer_focus else { return };
        let Some(s) = self.surfaces.get(&id) else { return };
        let icon = s.view.cursor_icon();
        if self.last_cursor == Some(icon) {
            return;
        }
        self.last_cursor = Some(icon);
        if let (Some(p), Some(conn)) = (&self.pointer, &self.conn) {
            let _ = p.set_cursor(conn, map_cursor(icon));
        }
    }

    fn closed_by_compositor(&mut self, id: SurfaceId) {
        if let Some(mut s) = self.surfaces.remove(&id) {
            s.teardown();
            if let Some(cb) = s.hooks.on_closed.take() {
                cb();
            }
        }
    }

    fn deliver_key(&mut self, event: &KeyEvent, pressed: bool) {
        let Some(id) = self.kb_focus else { return };
        let Some(s) = self.surfaces.get_mut(&id) else { return };
        let key = map_key(event.raw_code, event.keysym);
        let info = KeyInfo {
            key,
            keysym: keysym_name(event.keysym),
            text: event.utf8.clone(),
            pressed,
            modifiers: self.modifiers,
        };
        if let Some(h) = s.hooks.on_key.as_mut() {
            if h(&info) {
                s.needs_frame = true;
                return;
            }
        }
        s.view.set_modifiers(self.modifiers);
        s.view.key(key, pressed, if pressed { event.utf8.as_deref() } else { None });
        s.needs_frame = true;
    }
}

fn keysym_name(k: Keysym) -> String {
    k.name().map(|n| n.trim_start_matches("XK_").to_string()).unwrap_or_else(|| format!("0x{:x}", k.raw()))
}

// ─── Wayland-обработчики ─────────────────────────────────────────────────────

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(id) = self.surface_id_of(surface) {
            let s = self.surfaces.get_mut(&id).unwrap();
            if !s.has_fractional {
                s.scale120 = (factor.max(1) * 120) as u32;
                s.needs_frame = true;
            }
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(id) = self.surface_id_of(surface) {
            self.surfaces.get_mut(&id).unwrap().frame_pending = false;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        self.output_state.as_mut().unwrap()
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.publish_outputs();
        // Новый монитор при заблокированном сеансе — тоже под замок.
        if self.session_lock.as_ref().is_some_and(|l| l.is_locked()) {
            self.add_lock_surface(&output);
        }
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_outputs();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let name = self.output_state.as_ref().and_then(|os| os.info(&output)).and_then(|i| i.name);
        if let Some(name) = name {
            let gone: Vec<SurfaceId> = self
                .surfaces
                .iter()
                .filter(|(_, s)| s.spec.output.as_deref() == Some(name.as_str()))
                .map(|(id, _)| *id)
                .collect();
            for id in gone {
                self.closed_by_compositor(id);
            }
        }
        // Список обновится после удаления вывода из OutputState.
        let p = self.handle.clone();
        p.insert_idle(|st| st.publish_outputs());
    }
}

impl State {
    fn lock(&mut self, factory: crate::LockFactory) {
        if self.session_lock.is_some() || self.surfaces.values().any(|s| s.spec.namespace == LOCK_NS) {
            return;
        }
        self.lock_factory = Some(factory.clone());
        if let Some((w, h)) = self.headless {
            // Проверка без композитора: поверхности блокировки — в PNG.
            let info = crate::outputs().get_untracked().first().cloned().unwrap_or_default();
            let id = SurfaceId(crate::next_id_pub());
            let f = factory.clone();
            let mut s = Surface::new(id, lock_spec(&info), Default::default(), Box::new(move || f(&info)));
            s.backend = Backend::Headless { offscreen: None };
            s.logical = (w, h);
            s.configured = true;
            self.surfaces.insert(id, s);
            crate::session_locked().set(true);
            return;
        }
        let (Some(ls), Some(qh)) = (&self.lock_state, &self.qh) else { return };
        match ls.lock(qh) {
            Ok(l) => {
                log::info!("syngui-layer: запрошена блокировка сеанса");
                self.session_lock = Some(l);
                // Поверхности — сразу, не дожидаясь `locked`: композитор
                // вправе подтвердить блокировку только когда они готовы.
                let outs: Vec<wl_output::WlOutput> = self.output_list().into_iter().map(|(o, _)| o).collect();
                for o in outs {
                    self.add_lock_surface(&o);
                }
            }
            Err(e) => log::error!("syngui-layer: ext-session-lock недоступен: {e}"),
        }
    }

    fn unlock(&mut self) {
        if let Some(l) = self.session_lock.take() {
            l.unlock();
        }
        let ids: Vec<SurfaceId> = self.surfaces.iter().filter(|(_, s)| s.spec.namespace == LOCK_NS).map(|(id, _)| *id).collect();
        for id in ids {
            if let Some(mut s) = self.surfaces.remove(&id) {
                s.teardown();
            }
            if self.kb_focus == Some(id) {
                self.kb_focus = None;
            }
            if self.pointer_focus == Some(id) {
                self.pointer_focus = None;
            }
        }
        self.lock_factory = None;
        crate::session_locked().set(false);
        if let Some(c) = &self.conn {
            let _ = c.flush();
        }
    }

    fn add_lock_surface(&mut self, output: &wl_output::WlOutput) {
        let (Some(lock), Some(qh), Some(comp), Some(factory)) =
            (&self.session_lock, &self.qh, &self.compositor, self.lock_factory.clone())
        else {
            return;
        };
        let info = self.output_list().into_iter().find(|(o, _)| o == output).map(|(_, i)| i).unwrap_or_default();
        let wl = comp.create_surface(qh);
        let ls: SessionLockSurface = lock.create_lock_surface(wl, output, qh);
        let id = SurfaceId(crate::next_id_pub());
        let fractional = self.fractional.as_ref().map(|m| m.get_fractional_scale(ls.wl_surface(), qh, id));
        let viewport = match (&fractional, &self.viewporter) {
            (Some(_), Some(vp)) => Some(vp.get_viewport(ls.wl_surface(), qh, ())),
            _ => None,
        };
        let f = factory.clone();
        let spec = lock_spec(&info);
        let mut s = Surface::new(id, spec, Default::default(), Box::new(move || f(&info)));
        s.backend = Backend::Wayland { wsurf: None, viewport, fractional, role: crate::surface::Role::Lock(ls) };
        self.surfaces.insert(id, s);
    }
}

const LOCK_NS: &str = "syndesktop-lock";

fn lock_spec(info: &OutputInfo) -> SurfaceSpec {
    SurfaceSpec {
        namespace: LOCK_NS.into(),
        output: Some(info.name.clone()),
        clear_color: [0.0, 0.0, 0.0, 1.0],
        ..Default::default()
    }
}

impl SessionLockHandler for State {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _lock: SessionLock) {
        log::info!("syngui-layer: сеанс заблокирован");
        crate::session_locked().set(true);
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _lock: SessionLock) {
        log::warn!("syngui-layer: композитор отказал в блокировке (или она снята)");
        self.session_lock = None;
        self.unlock();
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: SessionLockSurface,
        cfg: SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_id_of(surface.wl_surface()) else { return };
        let s = self.surfaces.get_mut(&id).unwrap();
        let (w, h) = cfg.new_size;
        if s.logical != (w, h) {
            s.view.invalidate();
        }
        s.logical = (w.max(1), h.max(1));
        s.configured = true;
        s.needs_frame = true;
        s.frame_pending = false;
    }
}

delegate_session_lock!(State);

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.surface_id_of(layer.wl_surface()) {
            self.closed_by_compositor(id);
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, cfg: LayerSurfaceConfigure, _: u32) {
        let Some(id) = self.surface_id_of(layer.wl_surface()) else { return };
        let s = self.surfaces.get_mut(&id).unwrap();
        let w = if cfg.new_size.0 == 0 { s.requested.0.max(1) } else { cfg.new_size.0 };
        let h = if cfg.new_size.1 == 0 { s.requested.1.max(1) } else { cfg.new_size.1 };
        let changed = s.logical != (w, h);
        s.logical = (w, h);
        s.configured = true;
        s.needs_frame = true;
        // Configure без нового буфера кадр не ждёт — снять ожидание.
        s.frame_pending = false;
        if changed {
            s.view.invalidate();
            if let Some(cb) = s.hooks.on_resize.as_mut() {
                cb(w, h);
            }
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        self.seat_state.as_mut().unwrap()
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            let handle = self.handle.clone();
            let kb = self.seat_state.as_mut().unwrap().get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                handle,
                Box::new(|st: &mut State, _kb, ev| st.deliver_key(&ev, true)),
            );
            self.keyboard = kb.ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let (Some(shm), Some(comp)) = (&self.shm, &self.compositor) {
                let cursor_surface = comp.create_surface(qh);
                let theme = std::env::var("XCURSOR_THEME").ok();
                let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
                let spec = match &theme {
                    Some(name) => ThemeSpec::Named { name, size },
                    None => ThemeSpec::System,
                };
                let shm = shm.wl_shm().clone();
                self.pointer = self
                    .seat_state
                    .as_mut()
                    .unwrap()
                    .get_pointer_with_theme(qh, &seat, &shm, cursor_surface, spec)
                    .ok();
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.pointer().release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.kb_focus = self.surface_id_of(surface);
        if let Some(s) = self.kb_focus.and_then(|id| self.surfaces.get_mut(&id)) {
            s.view.set_modifiers(self.modifiers);
            s.view.keyboard_enter();
            s.needs_frame = true;
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(id) = self.surface_id_of(surface) {
            if let Some(s) = self.surfaces.get_mut(&id) {
                s.view.keyboard_leave();
                s.needs_frame = true;
            }
            if self.kb_focus == Some(id) {
                self.kb_focus = None;
            }
        }
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.deliver_key(&event, true);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.deliver_key(&event, false);
    }
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, m: Modifiers, _: u32) {
        self.modifiers = syngui::input::Modifiers { shift: m.shift, ctrl: m.ctrl, alt: m.alt, meta: m.logo };
        if let Some(s) = self.kb_focus.and_then(|id| self.surfaces.get_mut(&id)) {
            s.view.set_modifiers(self.modifiers);
        }
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for ev in events {
            let Some(id) = self.surface_id_of(&ev.surface) else { continue };
            let pos = Point::new(ev.position.0 as f32, ev.position.1 as f32);
            let Some(s) = self.surfaces.get_mut(&id) else { continue };
            match ev.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_focus = Some(id);
                    self.last_cursor = None;
                    s.view.pointer_motion(pos);
                    if let Some(h) = s.hooks.on_pointer.as_mut() {
                        h(true);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    s.view.pointer_leave();
                    if let Some(h) = s.hooks.on_pointer.as_mut() {
                        h(false);
                    }
                    if self.pointer_focus == Some(id) {
                        self.pointer_focus = None;
                    }
                }
                PointerEventKind::Motion { .. } => {
                    s.view.pointer_motion(pos);
                }
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(ev.kind, PointerEventKind::Press { .. });
                    let b = match button {
                        0x110 => MouseButton::Left,
                        0x111 => MouseButton::Right,
                        0x112 => MouseButton::Middle,
                        0x113 => MouseButton::Back,
                        0x114 => MouseButton::Forward,
                        other => MouseButton::Other(other as u16),
                    };
                    s.view.pointer_motion(pos);
                    s.view.pointer_button(b, pressed);
                }
                PointerEventKind::Axis { horizontal, vertical, .. } => {
                    let conv = |a: &smithay_client_toolkit::seat::pointer::AxisScroll| -> f32 {
                        if a.discrete != 0 {
                            -(a.discrete as f32) * 40.0
                        } else {
                            -(a.absolute as f32)
                        }
                    };
                    s.view.wheel(conv(&horizontal), conv(&vertical));
                }
            }
            s.needs_frame = true;
        }
        self.update_cursor();
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        self.shm.as_mut().unwrap()
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        self.registry.as_mut().unwrap()
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_layer!(State);
delegate_registry!(State);

impl Dispatch<WpFractionalScaleManagerV1, ()> for State {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleV1, SurfaceId> for State {
    fn event(st: &mut Self, _: &WpFractionalScaleV1, ev: wp_fractional_scale_v1::Event, id: &SurfaceId, _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            if let Some(s) = st.surfaces.get_mut(id) {
                s.has_fractional = true;
                if s.scale120 != scale {
                    s.scale120 = scale.max(1);
                    s.needs_frame = true;
                }
            }
        }
    }
}

impl Dispatch<WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for State {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
