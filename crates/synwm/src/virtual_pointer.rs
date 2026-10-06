//! Виртуальный указатель по протоколу wlr-virtual-pointer-unstable-v1 (v2):
//! wayvnc (удалённый рабочий стол вместе с screencopy и виртуальной
//! клавиатурой), wlrctl, ydotool-подобные утилиты.
//!
//! События копятся до `frame` и уходят пачкой тем же путём, что удалённый
//! ввод synlink (`State::remote_input`): движение, кнопки, прокрутка, жесты
//! оболочки. Абсолютные координаты — доли вывода, к которому привязан
//! указатель (`create_virtual_pointer_with_output`), иначе первого.

use std::sync::Mutex;

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::virtual_pointer::v1::server::{
            zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
            zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
        },
        wayland_server::{
            protocol::wl_pointer::{Axis, ButtonState},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum,
        },
    },
};
use synshell_common::ipc::InputEvent as Remote;

use crate::state::{ClientState, State};

pub struct PointerData {
    output: Option<String>,
    pending: Vec<Remote>,
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, ZwlrVirtualPointerManagerV1, _>(2, ());
}

impl GlobalDispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        // Песочницы (flatpak через security-context) управлять указателем не могут.
        client.get_data::<ClientState>().is_none_or(|c| c.security_context.is_none())
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _manager: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (id, output) = match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { id, .. } => (id, None),
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput { id, output, .. } => {
                (id, output.as_ref().and_then(Output::from_resource).map(|o| o.name()))
            }
            _ => return,
        };
        data_init.init(id, Mutex::new(PointerData { output, pending: Vec::new() }));
    }
}

impl Dispatch<ZwlrVirtualPointerV1, Mutex<PointerData>> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _pointer: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        data: &Mutex<PointerData>,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        use zwlr_virtual_pointer_v1::Request as R;
        let mut d = data.lock().unwrap();
        let axis = |a: WEnum<Axis>, v: f64, discrete: bool| match a {
            WEnum::Value(Axis::HorizontalScroll) => Some(Remote::Axis { dx: v, dy: 0.0, discrete }),
            WEnum::Value(Axis::VerticalScroll) => Some(Remote::Axis { dx: 0.0, dy: v, discrete }),
            _ => None,
        };
        let ev = match request {
            R::Motion { dx, dy, .. } => Some(Remote::MotionRelative { dx, dy }),
            R::MotionAbsolute { x, y, x_extent, y_extent, .. } if x_extent > 0 && y_extent > 0 => {
                Some(Remote::Motion { x: x as f64 / x_extent as f64, y: y as f64 / y_extent as f64 })
            }
            R::Button { button, state: s, .. } => Some(Remote::Button { button, pressed: s == WEnum::Value(ButtonState::Pressed) }),
            R::Axis { axis: a, value, .. } => axis(a, value, false),
            R::AxisDiscrete { axis: a, value, .. } => axis(a, value, true),
            R::Frame => {
                let events = std::mem::take(&mut d.pending);
                let output = d.output.clone();
                drop(d);
                if !events.is_empty() {
                    if let Err(e) = state.remote_input(output.as_deref(), events) {
                        tracing::debug!("виртуальный указатель: {e}");
                    }
                }
                return;
            }
            _ => None,
        };
        if let Some(e) = ev {
            d.pending.push(e);
            // Клиент без frame — не копить бесконечно.
            if d.pending.len() > 256 {
                d.pending.remove(0);
            }
        }
    }
}
