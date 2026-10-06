//! IPC-сервер: JSON-строки по Unix-сокету. Протокол — `synshell_common::ipc`.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::reexports::calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction};
use synshell_common::ipc::{Event, ModeInfo, OutputInfo, Request, Response, WindowOp};

use crate::state::State;

struct Conn {
    stream: UnixStream,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    subscribed: bool,
    dead: bool,
    /// Поток кадров (synlink), если соединение в него переведено.
    frame: Option<crate::stream::FrameConn>,
}

pub struct IpcServer {
    path: PathBuf,
    clients: HashMap<u64, Conn>,
    next: u64,
}

impl IpcServer {
    pub fn bind(handle: &LoopHandle<'static, State>, wayland_display: &str) -> anyhow::Result<Self> {
        let path = synshell_common::paths::socket_path_for(wayland_display);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        tracing::info!(path = %path.display(), "IPC");
        handle.insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state: &mut State| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => state.ipc_accept(stream),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!(?e, "IPC accept");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })?;
        Ok(Self { path, clients: HashMap::new(), next: 1 })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Разослать событие подписчикам.
    pub fn broadcast(&mut self, event: &Event) {
        let Ok(mut line) = serde_json::to_string(event) else { return };
        line.push('\n');
        for c in self.clients.values_mut().filter(|c| c.subscribed && !c.dead) {
            c.wbuf.extend_from_slice(line.as_bytes());
            flush(c);
        }
        self.clients.retain(|_, c| !c.dead);
    }

    pub fn subscribers(&self) -> usize {
        self.clients.values().filter(|c| c.subscribed && !c.dead).count()
    }

    /// Дописать всё накопленное (перед выходом) — блокирующе, с таймаутом.
    pub fn flush_all(&mut self) {
        for c in self.clients.values_mut() {
            let _ = c.stream.set_nonblocking(false);
            let _ = c.stream.set_write_timeout(Some(std::time::Duration::from_millis(200)));
            let _ = c.stream.write_all(&c.wbuf);
            c.wbuf.clear();
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn flush(c: &mut Conn) {
    while !c.wbuf.is_empty() {
        match c.stream.write(&c.wbuf) {
            Ok(0) => {
                c.dead = true;
                return;
            }
            Ok(n) => {
                c.wbuf.drain(..n);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => {
                c.dead = true;
                return;
            }
        }
    }
    // Клиент не читает — отключаем, чтобы не копить память.
    if c.wbuf.len() > 4 * 1024 * 1024 {
        c.dead = true;
    }
}

impl State {
    fn ipc_accept(&mut self, stream: UnixStream) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let Ok(read_half) = stream.try_clone() else { return };
        let id = self.core.ipc.next;
        self.core.ipc.next += 1;
        self.core.ipc.clients.insert(id, Conn { stream, rbuf: Vec::new(), wbuf: Vec::new(), subscribed: false, dead: false, frame: None });
        let res = self.core.loop_handle.insert_source(
            Generic::new(read_half, Interest::READ, Mode::Level),
            move |_, stream, state: &mut State| {
                let mut buf = [0u8; 8192];
                // SAFETY: поток не закрывается, пока жив источник.
                let s = unsafe { stream.get_mut() };
                loop {
                    match s.read(&mut buf) {
                        Ok(0) => {
                            state.core.ipc.clients.remove(&id);
                            return Ok(PostAction::Remove);
                        }
                        Ok(n) => {
                            if let Some(c) = state.core.ipc.clients.get_mut(&id) {
                                c.rbuf.extend_from_slice(&buf[..n]);
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => {
                            state.core.ipc.clients.remove(&id);
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                state.ipc_process(id);
                if state.core.ipc.clients.get(&id).is_none_or(|c| c.dead) {
                    state.core.ipc.clients.remove(&id);
                    return Ok(PostAction::Remove);
                }
                Ok(PostAction::Continue)
            },
        );
        if let Err(e) = res {
            tracing::warn!(?e, "IPC: источник клиента");
            self.core.ipc.clients.remove(&id);
        }
    }

    fn ipc_process(&mut self, id: u64) {
        loop {
            let line = {
                let Some(c) = self.core.ipc.clients.get_mut(&id) else { return };
                let Some(pos) = c.rbuf.iter().position(|b| *b == b'\n') else { return };
                let line: Vec<u8> = c.rbuf.drain(..=pos).collect();
                String::from_utf8_lossy(&line).trim().to_string()
            };
            if line.is_empty() {
                continue;
            }
            let (response, subscribe) = match serde_json::from_str::<Request>(&line) {
                Ok(Request::EventStream) => (Response::Ok, true),
                Ok(Request::FrameStream { output, cursor, video }) => (self.frame_stream_start(id, output, cursor, video), false),
                Ok(Request::FrameNext { key }) => match self.frame_stream_next(id, key) {
                    Some(r) => (r, false),
                    // Ответ придёт, когда вывод изменится.
                    None => continue,
                },
                Ok(req) => (self.handle_request(req), false),
                Err(e) => (Response::Error { message: format!("неверный запрос: {e}") }, false),
            };
            let mut out = serde_json::to_string(&response).unwrap_or_default();
            out.push('\n');
            if subscribe {
                let snapshot = Event::Snapshot {
                    windows: self.all_window_infos(),
                    workspaces: self.workspace_infos(),
                    outputs: output_infos(self),
                    keyboard: self.keyboard_layouts(),
                };
                if let Ok(s) = serde_json::to_string(&snapshot) {
                    out.push_str(&s);
                    out.push('\n');
                }
                // Телефон: режим окон и страница — сразу после снимка.
                if self.core.wm.mobile.enabled {
                    if let Ok(s) = serde_json::to_string(&Event::MobileChanged { mobile: self.mobile_info() }) {
                        out.push_str(&s);
                        out.push('\n');
                    }
                }
            }
            if let Some(c) = self.core.ipc.clients.get_mut(&id) {
                c.wbuf.extend_from_slice(out.as_bytes());
                if subscribe {
                    c.subscribed = true;
                }
                flush(c);
            }
        }
    }

    pub fn handle_request(&mut self, req: Request) -> Response {
        match req {
            Request::Version => Response::Version { version: env!("CARGO_PKG_VERSION").into() },
            Request::Windows => Response::Windows { windows: self.all_window_infos() },
            Request::Workspaces => Response::Workspaces { workspaces: self.workspace_infos() },
            Request::Mobile => Response::Mobile { mobile: self.mobile_info() },
            Request::X11Display { keys } => match self.x11_display_for(&keys) {
                Ok((d, resolution)) => Response::X11Display { display: format!(":{d}"), resolution },
                Err(message) => Response::Error { message },
            },
            Request::Outputs => Response::Outputs { outputs: output_infos(self) },
            Request::KeyboardLayouts => Response::KeyboardLayouts { layouts: self.keyboard_layouts() },
            Request::Action { action } => {
                self.do_action(action);
                Response::Ok
            }
            Request::WindowAction { id, op } => {
                if self.core.wm.get(id).is_none() {
                    return Response::Error { message: format!("нет окна {id}") };
                }
                self.window_op(id, op);
                Response::Ok
            }
            Request::SetMinimizeRect { id, output, rect } => {
                let origin = self
                    .core
                    .output_by_name(&output)
                    .and_then(|o| self.core.space.output_geometry(&o))
                    .map(|g| g.loc)
                    .unwrap_or_default();
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.minimize_rect = Some(smithay::utils::Rectangle::new(
                        (origin.x + rect[0], origin.y + rect[1]).into(),
                        (rect[2].max(1), rect[3].max(1)).into(),
                    ));
                }
                Response::Ok
            }
            Request::Capture if self.core.is_locked() => Response::Error { message: "экран заблокирован".into() },
            Request::Capture => match self.capture_all() {
                Ok(capture) => Response::Capture { capture },
                Err(e) => Response::Error { message: format!("захват экрана: {e:#}") },
            },
            Request::Thumbnails { .. } if self.core.is_locked() => Response::Error { message: "экран заблокирован".into() },
            Request::Thumbnails { ids, max } => match self.thumbnails(&ids, max) {
                Ok(thumbs) => Response::Thumbnails { thumbs },
                Err(e) => Response::Error { message: format!("миниатюры окон: {e:#}") },
            },
            Request::Input { output, events } => match self.remote_input(output.as_deref(), events) {
                Ok(()) => Response::Ok,
                Err(message) => Response::Error { message },
            },
            Request::EventStream | Request::FrameStream { .. } | Request::FrameNext { .. } => Response::Ok,
        }
    }

    /// `frame-stream`: завести поток и сразу отдать полный кадр.
    fn frame_stream_start(&mut self, id: u64, output: Option<String>, cursor: bool, video: Option<synshell_common::ipc::VideoRequest>) -> Response {
        let name = output
            .or_else(|| self.core.space.outputs().next().map(|o| o.name()))
            .unwrap_or_default();
        let mut fc = match crate::stream::FrameConn::new(id, name, cursor, video) {
            Ok(f) => f,
            Err(e) => return Response::Error { message: format!("поток кадров: {e:#}") },
        };
        let resp = match self.stream_capture(&mut fc) {
            Ok(frame) => Response::Frame { frame },
            Err(message) => return Response::Error { message },
        };
        if let Some(c) = self.core.ipc.clients.get_mut(&id) {
            c.frame = Some(fc);
        }
        resp
    }

    /// `frame-next`: изменения уже есть — кадр сразу, иначе ждать.
    fn frame_stream_next(&mut self, id: u64, key: bool) -> Option<Response> {
        let mut fc = self.core.ipc.clients.get_mut(&id)?.frame.take();
        let Some(f) = fc.as_mut() else {
            return Some(Response::Error { message: "нет потока кадров (сначала frame-stream)".into() });
        };
        if key {
            f.opts.force_key = true;
        }
        let mut resp = None;
        if f.dirty {
            match self.stream_capture(f) {
                Ok(frame) if !frame.rects.is_empty() || frame.video.is_some() => resp = Some(Response::Frame { frame }),
                Ok(_) => f.waiting = true,
                Err(message) => resp = Some(Response::Error { message }),
            }
        } else {
            f.waiting = true;
        }
        // Экран замер после видеокадров — дослать их места без потерь.
        if f.waiting && !f.opts.lossy.is_empty() && !f.refine_timer {
            f.refine_timer = true;
            let delay = f
                .opts
                .last_video
                .map(|t| crate::stream::REFINE_DELAY.saturating_sub(t.elapsed()))
                .unwrap_or_default()
                .max(std::time::Duration::from_millis(1));
            let _ = self.core.loop_handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(delay),
                move |_, _, st: &mut State| {
                    st.frame_stream_refine(id);
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                },
            );
        }
        // Монитор погашен — кадры никто не рисует: разбудить цикл кадров без
        // экрана (`headless_frame`), клиентам — frame callbacks.
        if f.waiting && self.core.monitors_off {
            if let Some(o) = self.core.output_by_name(&f.output) {
                self.core.queue_redraw(&o);
            }
        }
        if let Some(c) = self.core.ipc.clients.get_mut(&id) {
            c.frame = fc;
        }
        resp
    }

    /// Досылка без потерь: клиент ждёт, а после видеокадров экран не менялся.
    fn frame_stream_refine(&mut self, id: u64) {
        let Some(mut fc) = self.core.ipc.clients.get_mut(&id).and_then(|c| c.frame.take()) else { return };
        fc.refine_timer = false;
        let mut out = None;
        let quiet = fc.opts.last_video.is_none_or(|t| t.elapsed() >= crate::stream::REFINE_DELAY);
        if fc.waiting && !fc.dirty && !fc.opts.lossy.is_empty() {
            if quiet {
                fc.opts.refine = true;
                match self.stream_capture(&mut fc) {
                    Ok(frame) if frame.rects.is_empty() && frame.video.is_none() => fc.waiting = true,
                    Ok(frame) => out = Some(Response::Frame { frame }),
                    Err(message) => out = Some(Response::Error { message }),
                }
            }
        }
        let rearm = out.is_none() && fc.waiting && !fc.opts.lossy.is_empty();
        if let Some(c) = self.core.ipc.clients.get_mut(&id) {
            c.frame = Some(fc);
            if let Some(r) = out {
                let mut line = serde_json::to_string(&r).unwrap_or_default();
                line.push('\n');
                c.wbuf.extend_from_slice(line.as_bytes());
                flush(c);
            }
        }
        if rearm {
            if let Some(f) = self.core.ipc.clients.get_mut(&id).and_then(|c| c.frame.as_mut()) {
                f.refine_timer = true;
            }
            let _ = self.core.loop_handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(crate::stream::REFINE_DELAY),
                move |_, _, st: &mut State| {
                    st.frame_stream_refine(id);
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                },
            );
        }
    }

    /// Есть ли на выводе потоки кадров.
    pub fn has_frame_streams(&self, output: &smithay::output::Output) -> bool {
        let name = output.name();
        self.core.ipc.clients.values().any(|c| !c.dead && c.frame.as_ref().is_some_and(|f| f.output == name))
    }

    /// Вывод перерисован с изменениями: ждущим потокам — кадр, остальным —
    /// отметка о накопленных изменениях.
    pub fn frame_streams_damaged(&mut self, output: &smithay::output::Output) {
        let name = output.name();
        let ids: Vec<u64> = self
            .core
            .ipc
            .clients
            .iter()
            .filter(|(_, c)| !c.dead && c.frame.as_ref().is_some_and(|f| f.output == name))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(mut fc) = self.core.ipc.clients.get_mut(&id).and_then(|c| c.frame.take()) else { continue };
            let mut out = None;
            if fc.waiting {
                match self.stream_capture(&mut fc) {
                    Ok(frame) if frame.rects.is_empty() && frame.video.is_none() => fc.waiting = true,
                    Ok(frame) => out = Some(Response::Frame { frame }),
                    Err(message) => out = Some(Response::Error { message }),
                }
            } else {
                fc.dirty = true;
            }
            if let Some(c) = self.core.ipc.clients.get_mut(&id) {
                c.frame = Some(fc);
                if let Some(r) = out {
                    let mut line = serde_json::to_string(&r).unwrap_or_default();
                    line.push('\n');
                    c.wbuf.extend_from_slice(line.as_bytes());
                    flush(c);
                }
            }
        }
    }

    pub fn window_op(&mut self, id: crate::wm::WindowId, op: WindowOp) {
        match op {
            WindowOp::Activate => self.focus_window(Some(id)),
            WindowOp::ToggleMinimize => {
                let (minimized, focused, on_ws) = {
                    let m = self.core.wm.get(id).unwrap();
                    (m.minimized, self.core.wm.focused == Some(id), m.on_workspace(self.core.wm.active))
                };
                if focused && !minimized && on_ws {
                    self.minimize(id);
                } else {
                    self.focus_window(Some(id));
                }
            }
            WindowOp::Minimize => self.minimize(id),
            WindowOp::Close => self.close_window(id),
            WindowOp::Kill => self.kill_window(id),
            WindowOp::ToggleMaximize => self.toggle_maximize(id),
            WindowOp::ToggleFullscreen => self.toggle_fullscreen(id),
            WindowOp::ToggleFloating => self.toggle_floating(id),
            WindowOp::ToggleSticky => self.toggle_sticky(id),
            WindowOp::ToggleAlwaysOnTop => self.toggle_above(id),
            WindowOp::StartMove => {
                // Кнопка ещё нажата (неявный захват поверхности оболочки) —
                // перехватить указатель и тащить окно.
                let pointer = self.core.pointer.clone();
                if let Some(start) = pointer.grab_start_data() {
                    let start = smithay::input::pointer::GrabStartData {
                        focus: None,
                        button: start.button,
                        location: pointer.current_location(),
                    };
                    self.start_move(id, start, smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
            WindowOp::MoveToWorkspace(ws) => self.move_to_workspace(id, ws, false),
        }
    }
}

/// Слой экранной клавиатуры (synkeyboard, squeekboard и подобные).
pub fn is_osk_namespace(ns: &str) -> bool {
    let ns = ns.to_ascii_lowercase();
    ns == "synkeyboard" || ns == "osk" || ns.contains("keyboard")
}

/// Высота открытой экранной клавиатуры на выводе, 0 — её нет. Поверхность
/// может быть выше клавиш (прозрачный запас synkeyboard под всплывающую
/// букву) — тогда высота клавиатуры — её exclusive zone.
fn keyboard_height(o: &smithay::output::Output) -> i32 {
    use smithay::wayland::shell::wlr_layer::ExclusiveZone;
    let map = smithay::desktop::layer_map_for_output(o);
    map.layers()
        .filter(|l| is_osk_namespace(l.namespace()))
        .filter_map(|l| {
            let g = map.layer_geometry(l)?;
            Some(match l.cached_state().exclusive_zone {
                ExclusiveZone::Exclusive(z) if z > 0 => (z as i32).min(g.size.h),
                _ => g.size.h,
            })
        })
        .max()
        .unwrap_or(0)
}

impl State {
    /// Разослать сведения о выводах, если на каком-то сменилась высота
    /// экранной клавиатуры (открылась, закрылась, другой размер).
    pub fn broadcast_outputs_if_keyboard_changed(&mut self) {
        let outputs = output_infos(self);
        let now: Vec<(String, i32)> = outputs.iter().map(|o| (o.name.clone(), o.keyboard)).collect();
        if now == self.core.osk_heights {
            return;
        }
        self.core.osk_heights = now;
        self.core.ipc.broadcast(&synshell_common::ipc::Event::OutputsChanged { outputs });
    }
}

pub fn output_infos(state: &State) -> Vec<OutputInfo> {
    let focused = state.core.output_under_pointer();
    let primary = state.core.primary_output();
    state
        .core
        .space
        .outputs()
        .map(|o| {
            let g = state.core.space.output_geometry(o).unwrap_or_default();
            let p = o.physical_properties();
            let current = o.current_mode();
            let preferred = o.preferred_mode();
            let modes: Vec<ModeInfo> = o
                .modes()
                .into_iter()
                .map(|m| ModeInfo { width: m.size.w, height: m.size.h, refresh_mhz: m.refresh, preferred: Some(m) == preferred })
                .collect();
            let current_idx = current.and_then(|c| o.modes().iter().position(|m| *m == c));
            OutputInfo {
                name: o.name(),
                description: format!("{} {}", p.make, p.model),
                make: p.make.clone(),
                model: p.model.clone(),
                geometry: [g.loc.x, g.loc.y, g.size.w, g.size.h],
                scale: o.current_scale().fractional_scale(),
                refresh_mhz: current.map(|m| m.refresh as u32).unwrap_or(0),
                physical_mm: [p.size.w, p.size.h],
                modes,
                current_mode: current_idx,
                transform: crate::backend::transform_name(o.current_transform()).into(),
                primary: primary.as_ref() == Some(o),
                focused: focused.as_ref() == Some(o),
                keyboard: keyboard_height(o),
            }
        })
        .collect()
}
