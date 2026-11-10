//! `ext-background-effect-v1`: клиент сообщает область своей поверхности, под которой композитор
//! размывает то, что за ней (меню, шторка, всплывающие окна оболочки с полупрозрачным фоном).
//! Область — двойная буферизация, применяется при commit поверхности. Возможность `blur`
//! объявляется, только пока размытие включено (`[animations] blur`) и кадр рисует GPU; при
//! перечитывании конфига клиентам приходит новое значение.
//!
//! Само размытие — `crate::blur`.

use std::sync::Mutex;

use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self as mgr, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self as surf, ExtBackgroundEffectSurfaceV1},
};
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};
use smithay::wayland::compositor::{get_region_attributes, with_states, Cacheable, RegionAttributes};

use crate::state::State;

/// Область размытия поверхности (логические координаты поверхности).
#[derive(Default, Clone, Debug)]
pub struct BlurState {
    pub region: Option<RegionAttributes>,
}

impl Cacheable for BlurState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }
    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// У поверхности уже есть объект эффекта (повторный — ошибка протокола).
#[derive(Default)]
struct HasEffect(Mutex<bool>);

/// Привязанные менеджеры — им рассылается смена возможности.
static MANAGERS: Mutex<Vec<Weak<ExtBackgroundEffectManagerV1>>> = Mutex::new(Vec::new());

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
}

/// Размытие доступно: включено в настройках и кадр рисует GPU (на CPU-композиторе — нет).
pub fn enabled(state: &State) -> bool {
    #[cfg(feature = "pixman")]
    let gpu = !matches!(state.backend, crate::backend::Backend::KmsCpu(_));
    #[cfg(not(feature = "pixman"))]
    let gpu = true;
    gpu && state.core.config.animations.effect("blur")
}

fn caps(state: &State) -> mgr::Capability {
    if enabled(state) {
        mgr::Capability::Blur
    } else {
        mgr::Capability::empty()
    }
}

/// Разослать возможность заново (после перечитывания конфига).
pub fn update_capabilities(state: &State) {
    let c = caps(state);
    let mut list = MANAGERS.lock().unwrap();
    list.retain(|w| w.upgrade().is_ok());
    for m in list.iter().filter_map(|w| w.upgrade().ok()) {
        m.capabilities(c);
    }
}

/// Текущая область размытия поверхности (`None` — не задана или пуста).
pub fn blur_region(surface: &WlSurface) -> Option<RegionAttributes> {
    with_states(surface, |s| s.cached_state.get::<BlurState>().current().region.clone()).filter(|r| !r.rects.is_empty())
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn bind(
        state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _data: &(),
        di: &mut DataInit<'_, State>,
    ) {
        let m = di.init(resource, ());
        m.capabilities(caps(state));
        MANAGERS.lock().unwrap().push(m.downgrade());
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &ExtBackgroundEffectManagerV1,
        request: mgr::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        match request {
            mgr::Request::GetBackgroundEffect { id, surface } => {
                let exists = with_states(&surface, |s| {
                    s.data_map.insert_if_missing_threadsafe(HasEffect::default);
                    let mut has = s.data_map.get::<HasEffect>().unwrap().0.lock().unwrap();
                    std::mem::replace(&mut *has, true)
                });
                if exists {
                    resource.post_error(mgr::Error::BackgroundEffectExists, "у поверхности уже есть объект эффекта");
                    return;
                }
                di.init(id, surface.downgrade());
            }
            mgr::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, Weak<WlSurface>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &ExtBackgroundEffectSurfaceV1,
        request: surf::Request,
        data: &Weak<WlSurface>,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, State>,
    ) {
        let surface = data.upgrade().ok();
        match request {
            surf::Request::SetBlurRegion { region } => {
                let Some(surface) = surface else {
                    resource.post_error(surf::Error::SurfaceDestroyed, "поверхность уничтожена");
                    return;
                };
                let attrs = region.as_ref().map(get_region_attributes);
                with_states(&surface, |s| s.cached_state.get::<BlurState>().pending().region = attrs);
            }
            surf::Request::Destroy => {
                // Объект удалён — эффект снимается со следующего commit поверхности.
                if let Some(surface) = surface {
                    with_states(&surface, |s| {
                        s.cached_state.get::<BlurState>().pending().region = None;
                        if let Some(h) = s.data_map.get::<HasEffect>() {
                            *h.0.lock().unwrap() = false;
                        }
                    });
                }
            }
            _ => {}
        }
    }
}
