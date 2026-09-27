//! IPC-сервер: JSON-строки по Unix-сокету. Протокол — `syndesktop_common::ipc`.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::reexports::calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction};
use syndesktop_common::ipc::{Event, ModeInfo, OutputInfo, Request, Response, WindowOp};

use crate::state::State;

struct Conn {
    stream: UnixStream,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    subscribed: bool,
    dead: bool,
}

pub struct IpcServer {
    path: PathBuf,
    clients: HashMap<u64, Conn>,
    next: u64,
}

impl IpcServer {
    pub fn bind(handle: &LoopHandle<'static, State>, wayland_display: &str) -> anyhow::Result<Self> {
        let path = syndesktop_common::paths::socket_path_for(wayland_display);
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
        self.core.ipc.clients.insert(id, Conn { stream, rbuf: Vec::new(), wbuf: Vec::new(), subscribed: false, dead: false });
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
            Request::EventStream => Response::Ok,
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
            }
        })
        .collect()
}
