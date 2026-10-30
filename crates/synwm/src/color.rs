//! `wp_color_management_v1` (staging): клиенты сообщают, в каком цвете их буфер — scRGB (линейный
//! extended sRGB, FP16), HDR10 (PQ, BT.2020), HLG, — а композитор переводит его в цвет вывода: в
//! HDR-режиме — в общий кадр без потери светов, на SDR-выводе — с тонмаппингом. Обычные sRGB-окна
//! (Mesa вешает описание `srgb` на любое окно Vulkan) рисуются как всегда, без лишнего шейдера.
//!
//! Vulkan-клиенты (Mesa WSI) берут цветовые пространства из объявленных здесь возможностей:
//! `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT` — при `srgb` + `ext_linear` + `extended_target_volume`,
//! HDR10 — при `bt2020` + `st2084_pq`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use smithay::backend::renderer::element::Id;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::color_management::v1::server::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
    wp_color_management_surface_v1::{self, WpColorManagementSurfaceV1},
    wp_color_manager_v1::{self, Feature, Primaries, RenderIntent, TransferFunction, WpColorManagerV1},
    wp_image_description_creator_icc_v1::{self, WpImageDescriptionCreatorIccV1},
    wp_image_description_creator_params_v1::{self, WpImageDescriptionCreatorParamsV1},
    wp_image_description_info_v1::WpImageDescriptionInfoV1,
    wp_image_description_v1::{self, WpImageDescriptionV1},
};
use smithay::reexports::wayland_server::{
    backend::ClientId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
    Resource, WEnum, Weak,
};
use smithay::wayland::compositor::with_states;

use crate::state::State;

// ─── описание цвета ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tf {
    Srgb,
    Gamma22,
    Bt1886,
    ExtLinear,
    Pq,
    Hlg,
}

impl Tf {
    fn from_wl(v: u32) -> Option<Self> {
        Some(match TransferFunction::try_from(v).ok()? {
            TransferFunction::Srgb => Tf::Srgb,
            TransferFunction::Gamma22 => Tf::Gamma22,
            TransferFunction::Bt1886 => Tf::Bt1886,
            TransferFunction::ExtLinear => Tf::ExtLinear,
            TransferFunction::St2084Pq => Tf::Pq,
            TransferFunction::Hlg => Tf::Hlg,
            _ => return None,
        })
    }

    fn to_wl(self) -> TransferFunction {
        match self {
            Tf::Srgb => TransferFunction::Srgb,
            Tf::Gamma22 => TransferFunction::Gamma22,
            Tf::Bt1886 => TransferFunction::Bt1886,
            Tf::ExtLinear => TransferFunction::ExtLinear,
            Tf::Pq => TransferFunction::St2084Pq,
            Tf::Hlg => TransferFunction::Hlg,
        }
    }

    /// Код для шейдера (`ColorParams::tf`).
    pub fn code(self) -> f32 {
        match self {
            Tf::ExtLinear => 1.0,
            Tf::Pq => 2.0,
            Tf::Hlg => 3.0,
            Tf::Srgb => 4.0,
            Tf::Gamma22 | Tf::Bt1886 => 5.0,
        }
    }

    /// Яркости по умолчанию (протокол, `set_luminances`): (min, max, reference), нит.
    fn default_lum(self) -> (f32, f32, f32) {
        match self {
            Tf::Pq => (0.005, 10_000.0, 203.0),
            Tf::Hlg => (0.005, 1000.0, 203.0),
            _ => (0.2, 80.0, 80.0),
        }
    }
}

/// Цвет буфера поверхности (или вывода).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Desc {
    pub tf: Tf,
    /// Первичные BT.2020; иначе sRGB/BT.709.
    pub bt2020: bool,
    pub min_lum: f32,
    /// Для относительных кривых (sRGB, линейная) — яркость значения 1.0.
    pub max_lum: f32,
    /// Опорный белый контента: он ложится на опорный белый вывода.
    pub ref_lum: f32,
    /// Самое яркое в контенте (MaxCLL, иначе мастеринг), нит; 0 — неизвестно.
    pub max_cll: f32,
    pub max_fall: f32,
}

impl Desc {
    pub fn srgb() -> Self {
        let (min_lum, max_lum, ref_lum) = Tf::Srgb.default_lum();
        Desc { tf: Tf::Srgb, bt2020: false, min_lum, max_lum, ref_lum, max_cll: 0.0, max_fall: 0.0 }
    }

    /// Рисуется как обычное окно (без перевода): sRGB-подобная кривая, BT.709, белый = опорный.
    pub fn is_plain(&self) -> bool {
        matches!(self.tf, Tf::Srgb | Tf::Gamma22 | Tf::Bt1886) && !self.bt2020 && (self.max_lum - self.ref_lum).abs() < 0.5
    }

    /// Пик контента относительно опорного белого.
    pub fn peak_rel(&self) -> f32 {
        let peak = if self.max_cll > 0.0 {
            self.max_cll
        } else {
            match self.tf {
                Tf::Pq => 1000.0,
                Tf::Hlg => 1000.0,
                // scRGB без метаданных: на SDR-выводе — обрезка на белом, без сжатия (клиент, знающий
                // свой пик, тонмаппит сам, и его картинка не должна меняться)
                Tf::ExtLinear => self.ref_lum,
                _ => self.max_lum,
            }
        };
        (peak / self.ref_lum.max(1.0)).max(1.0)
    }

    fn identity(&self) -> u32 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.tf.hash(&mut h);
        self.bt2020.hash(&mut h);
        for v in [self.min_lum, self.max_lum, self.ref_lum, self.max_cll, self.max_fall] {
            v.to_bits().hash(&mut h);
        }
        (h.finish() as u32).max(1)
    }
}

/// Описание цвета вывода (для `get_output` и предпочтения поверхности).
pub fn output_desc(output: &Output) -> Desc {
    let st = output
        .user_data()
        .get::<crate::backend::tty::HdrStatus>()
        .and_then(|s| s.0.lock().ok().map(|g| *g))
        .unwrap_or_default();
    if st.active {
        Desc {
            tf: Tf::Pq,
            bt2020: true,
            min_lum: 0.005,
            max_lum: if st.peak_nits > 0.0 { st.peak_nits } else { 1000.0 },
            ref_lum: st.sdr_nits,
            max_cll: 0.0,
            max_fall: 0.0,
        }
    } else {
        Desc::srgb()
    }
}

// ─── реестр поверхностей ────────────────────────────────────────────────────

/// Цвет поверхности: объект `wp_color_management_surface_v1` есть, ожидающее и текущее описание
/// (двойная буферизация: применяется на коммите).
#[derive(Default)]
struct SurfaceColor {
    has_object: bool,
    pending: Option<Option<Desc>>,
    current: Option<Desc>,
}

#[derive(Default)]
struct SurfaceColorData(Mutex<SurfaceColor>);

#[derive(Default)]
struct Registry {
    /// Поверхности с не-sRGB цветом — по id элемента рендера (он строится из `wl_surface`).
    managed: HashMap<Id, Desc>,
    outputs: Vec<(Weak<WpColorManagementOutputV1>, Output)>,
    feedbacks: Vec<(Weak<WpColorManagementSurfaceFeedbackV1>, Weak<WlSurface>)>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Цвет поверхности с этим id элемента рендера, если он не обычный sRGB.
pub fn managed(id: &Id) -> Option<Desc> {
    registry().lock().ok()?.managed.get(id).copied()
}

/// Есть ли сейчас хоть одна цветоуправляемая поверхность (быстрый путь рендера).
pub fn any_managed() -> bool {
    registry().lock().map(|r| !r.managed.is_empty()).unwrap_or(false)
}

/// Коммит поверхности: ожидающее описание становится текущим.
pub fn on_commit(surface: &WlSurface) {
    let changed = with_states(surface, |s| {
        let Some(d) = s.data_map.get::<SurfaceColorData>() else { return None };
        let mut g = d.0.lock().ok()?;
        let p = g.pending.take()?;
        g.current = p;
        Some(p)
    });
    let Some(desc) = changed else { return };
    let id = Id::from_wayland_resource(surface);
    if let Ok(mut r) = registry().lock() {
        match desc.filter(|d| !d.is_plain()) {
            Some(d) => {
                if r.managed.insert(id, d) != Some(d) {
                    tracing::info!(surface = ?surface.id(), ?d, "цвет поверхности");
                }
            }
            None => {
                r.managed.remove(&id);
            }
        }
    }
}

fn set_pending(surface: &WlSurface, desc: Option<Desc>) {
    if !surface.is_alive() {
        return;
    }
    with_states(surface, |s| {
        s.data_map.insert_if_missing_threadsafe(SurfaceColorData::default);
        if let Ok(mut g) = s.data_map.get::<SurfaceColorData>().unwrap().0.lock() {
            g.pending = Some(desc);
        }
    });
}

/// HDR вывода включился или выключился: описания выводов и предпочтения поверхностей поменялись.
pub fn outputs_changed() {
    let Ok(mut r) = registry().lock() else { return };
    r.outputs.retain(|(o, _)| o.upgrade().is_ok());
    for (o, _) in &r.outputs {
        if let Ok(o) = o.upgrade() {
            o.image_description_changed();
        }
    }
    r.feedbacks.retain(|(f, s)| f.upgrade().is_ok() && s.upgrade().is_ok());
    for (f, s) in &r.feedbacks {
        if let (Ok(f), Ok(s)) = (f.upgrade(), s.upgrade()) {
            f.preferred_changed(preferred_for(&s).identity());
        }
    }
}

/// Предпочтительный цвет поверхности: HDR-вывода, если он есть (один монитор у ноутбука — его),
/// иначе sRGB.
fn preferred_for(_surface: &WlSurface) -> Desc {
    ALL_OUTPUTS.with(|o| o.borrow().iter().map(output_desc).find(|d| d.tf == Tf::Pq)).unwrap_or_else(Desc::srgb)
}

thread_local! {
    /// Выводы композитора (обновляет `set_outputs`): для предпочтения поверхностей.
    static ALL_OUTPUTS: std::cell::RefCell<Vec<Output>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Список выводов изменился (подключение, HDR).
pub fn set_outputs(outputs: Vec<Output>) {
    ALL_OUTPUTS.with(|o| *o.borrow_mut() = outputs);
}

// ─── протокол ───────────────────────────────────────────────────────────────

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, WpColorManagerV1, ()>(1, ());
}

/// Описание: готовое (`Some`) или отклонённое.
pub struct ImageDesc {
    desc: Option<Desc>,
}

#[derive(Default)]
pub struct Params {
    tf: Option<Tf>,
    primaries: Option<bool>,
    lum: Option<(f32, f32, f32)>,
    mastering_max: Option<f32>,
    max_cll: Option<f32>,
    max_fall: Option<f32>,
}

impl GlobalDispatch<WpColorManagerV1, ()> for State {
    fn bind(_state: &mut State, _dh: &DisplayHandle, _client: &Client, resource: New<WpColorManagerV1>, _data: &(), di: &mut DataInit<'_, State>) {
        let m = di.init(resource, ());
        m.supported_intent(RenderIntent::Perceptual);
        for f in [Feature::Parametric, Feature::SetLuminances, Feature::SetMasteringDisplayPrimaries, Feature::ExtendedTargetVolume] {
            m.supported_feature(f);
        }
        for tf in [Tf::Srgb, Tf::Gamma22, Tf::Bt1886, Tf::ExtLinear, Tf::Pq, Tf::Hlg] {
            m.supported_tf_named(tf.to_wl());
        }
        m.supported_primaries_named(Primaries::Srgb);
        m.supported_primaries_named(Primaries::Bt2020);
        m.done();
    }
}

fn send_ready(desc_obj: &WpImageDescriptionV1, desc: &Desc) {
    desc_obj.ready(desc.identity());
}

impl Dispatch<WpColorManagerV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpColorManagerV1,
        request: wp_color_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        use wp_color_manager_v1::Request;
        match request {
            Request::GetOutput { id, output } => {
                let obj = di.init(id, ());
                if let Some(o) = Output::from_resource(&output) {
                    if let Ok(mut r) = registry().lock() {
                        r.outputs.push((obj.downgrade(), o));
                    }
                }
            }
            Request::GetSurface { id, surface } => {
                let exists = with_states(&surface, |s| {
                    s.data_map.insert_if_missing_threadsafe(SurfaceColorData::default);
                    let mut g = s.data_map.get::<SurfaceColorData>().unwrap().0.lock().unwrap();
                    std::mem::replace(&mut g.has_object, true)
                });
                if exists {
                    resource.post_error(wp_color_manager_v1::Error::SurfaceExists, "у поверхности уже есть объект управления цветом");
                    return;
                }
                di.init(id, surface.downgrade());
            }
            Request::GetSurfaceFeedback { id, surface } => {
                let obj = di.init(id, surface.downgrade());
                obj.preferred_changed(preferred_for(&surface).identity());
                if let Ok(mut r) = registry().lock() {
                    r.feedbacks.push((obj.downgrade(), surface.downgrade()));
                }
            }
            Request::CreateIccCreator { obj } => {
                di.init(obj, ());
                resource.post_error(wp_color_manager_v1::Error::UnsupportedFeature, "ICC-профили не поддерживаются");
            }
            Request::CreateParametricCreator { obj } => {
                di.init(obj, Mutex::new(Params::default()));
            }
            Request::CreateWindowsScrgb { image_description } => {
                let d = di.init(image_description, Arc::new(ImageDesc { desc: None }));
                let _ = d;
                resource.post_error(wp_color_manager_v1::Error::UnsupportedFeature, "windows_scrgb не объявлен");
            }
            Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<WpColorManagementOutputV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpColorManagementOutputV1,
        request: wp_color_management_output_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        if let wp_color_management_output_v1::Request::GetImageDescription { image_description } = request {
            let output = registry()
                .lock()
                .ok()
                .and_then(|r| r.outputs.iter().find(|(o, _)| o.upgrade().ok().as_ref() == Some(resource)).map(|(_, o)| o.clone()));
            let desc = output.as_ref().map(output_desc).unwrap_or_else(Desc::srgb);
            let obj = di.init(image_description, Arc::new(ImageDesc { desc: Some(desc) }));
            send_ready(&obj, &desc);
        }
    }

    fn destroyed(_state: &mut State, _client: ClientId, resource: &WpColorManagementOutputV1, _data: &()) {
        if let Ok(mut r) = registry().lock() {
            r.outputs.retain(|(o, _)| o.upgrade().ok().as_ref() != Some(resource));
        }
    }
}

impl Dispatch<WpColorManagementSurfaceV1, Weak<WlSurface>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpColorManagementSurfaceV1,
        request: wp_color_management_surface_v1::Request,
        data: &Weak<WlSurface>,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, State>,
    ) {
        use wp_color_management_surface_v1::Request;
        let Ok(surface) = data.upgrade() else {
            if !matches!(request, Request::Destroy) {
                resource.post_error(wp_color_management_surface_v1::Error::Inert, "поверхность уничтожена");
            }
            return;
        };
        match request {
            Request::SetImageDescription { image_description, render_intent } => {
                if !matches!(render_intent, WEnum::Value(RenderIntent::Perceptual)) {
                    resource.post_error(wp_color_management_surface_v1::Error::RenderIntent, "поддержан только perceptual");
                    return;
                }
                let Some(desc) = image_description.data::<Arc<ImageDesc>>().and_then(|d| d.desc) else {
                    resource.post_error(wp_color_management_surface_v1::Error::ImageDescription, "описание не готово");
                    return;
                };
                set_pending(&surface, Some(desc));
            }
            Request::UnsetImageDescription => set_pending(&surface, None),
            Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(_state: &mut State, _client: ClientId, _resource: &WpColorManagementSurfaceV1, data: &Weak<WlSurface>) {
        // Объект ушёл — поверхность снова обычная (со следующего коммита), и можно создать новый.
        if let Ok(surface) = data.upgrade() {
            with_states(&surface, |s| {
                if let Some(d) = s.data_map.get::<SurfaceColorData>() {
                    if let Ok(mut g) = d.0.lock() {
                        g.has_object = false;
                        g.pending = Some(None);
                    }
                }
            });
        }
    }
}

impl Dispatch<WpColorManagementSurfaceFeedbackV1, Weak<WlSurface>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpColorManagementSurfaceFeedbackV1,
        request: wp_color_management_surface_feedback_v1::Request,
        data: &Weak<WlSurface>,
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        use wp_color_management_surface_feedback_v1::Request;
        match request {
            Request::GetPreferred { image_description } | Request::GetPreferredParametric { image_description } => {
                let Ok(surface) = data.upgrade() else {
                    di.init(image_description, Arc::new(ImageDesc { desc: None }));
                    resource.post_error(wp_color_management_surface_feedback_v1::Error::Inert, "поверхность уничтожена");
                    return;
                };
                let desc = preferred_for(&surface);
                let obj = di.init(image_description, Arc::new(ImageDesc { desc: Some(desc) }));
                send_ready(&obj, &desc);
            }
            _ => {}
        }
    }
}

impl Dispatch<WpImageDescriptionCreatorIccV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &WpImageDescriptionCreatorIccV1,
        request: wp_image_description_creator_icc_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        if let wp_image_description_creator_icc_v1::Request::Create { image_description } = request {
            let d = di.init(image_description, Arc::new(ImageDesc { desc: None }));
            d.failed(wp_image_description_v1::Cause::Unsupported, "ICC не поддерживается".into());
        }
    }
}

impl Dispatch<WpImageDescriptionCreatorParamsV1, Mutex<Params>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpImageDescriptionCreatorParamsV1,
        request: wp_image_description_creator_params_v1::Request,
        data: &Mutex<Params>,
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        use wp_image_description_creator_params_v1::{Error, Request};
        let mut p = data.lock().unwrap();
        let already = |r: &WpImageDescriptionCreatorParamsV1, what: &str| r.post_error(Error::AlreadySet, format!("{what} уже задано"));
        match request {
            Request::SetTfNamed { tf } => {
                if p.tf.is_some() {
                    return already(resource, "tf");
                }
                let v = match tf {
                    WEnum::Value(v) => v as u32,
                    WEnum::Unknown(v) => v,
                };
                match Tf::from_wl(v) {
                    Some(t) => p.tf = Some(t),
                    None => resource.post_error(Error::InvalidTf, "передаточная функция не поддержана"),
                }
            }
            Request::SetTfPower { .. } => resource.post_error(Error::UnsupportedFeature, "set_tf_power не объявлен"),
            Request::SetPrimariesNamed { primaries } => {
                if p.primaries.is_some() {
                    return already(resource, "primaries");
                }
                match primaries {
                    WEnum::Value(Primaries::Srgb) => p.primaries = Some(false),
                    WEnum::Value(Primaries::Bt2020) => p.primaries = Some(true),
                    _ => resource.post_error(Error::InvalidPrimariesNamed, "первичные не поддержаны"),
                }
            }
            Request::SetPrimaries { .. } => resource.post_error(Error::UnsupportedFeature, "set_primaries не объявлен"),
            Request::SetLuminances { min_lum, max_lum, reference_lum } => {
                if p.lum.is_some() {
                    return already(resource, "luminances");
                }
                let min = min_lum as f32 / 10_000.0;
                if max_lum as f32 <= min || reference_lum as f32 <= min {
                    return resource.post_error(Error::InvalidLuminance, "яркости не по порядку");
                }
                p.lum = Some((min, max_lum as f32, reference_lum as f32));
            }
            Request::SetMasteringDisplayPrimaries { .. } => {}
            Request::SetMasteringLuminance { min_lum, max_lum } => {
                if max_lum as f32 <= min_lum as f32 / 10_000.0 {
                    return resource.post_error(Error::InvalidLuminance, "мастеринг: яркости не по порядку");
                }
                p.mastering_max = Some(max_lum as f32);
            }
            Request::SetMaxCll { max_cll } => p.max_cll = Some(max_cll as f32),
            Request::SetMaxFall { max_fall } => p.max_fall = Some(max_fall as f32),
            Request::Create { image_description } => {
                let (Some(tf), Some(bt2020)) = (p.tf, p.primaries) else {
                    di.init(image_description, Arc::new(ImageDesc { desc: None }));
                    return resource.post_error(Error::IncompleteSet, "нужны tf и primaries");
                };
                let (min_lum, max_lum, ref_lum) = p.lum.unwrap_or_else(|| tf.default_lum());
                let desc = Desc {
                    tf,
                    bt2020,
                    min_lum,
                    max_lum,
                    ref_lum,
                    max_cll: p.max_cll.filter(|v| *v > 0.0).or(p.mastering_max).unwrap_or(0.0),
                    max_fall: p.max_fall.unwrap_or(0.0),
                };
                let obj = di.init(image_description, Arc::new(ImageDesc { desc: Some(desc) }));
                send_ready(&obj, &desc);
            }
            _ => {}
        }
    }
}

impl Dispatch<WpImageDescriptionV1, Arc<ImageDesc>> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        resource: &WpImageDescriptionV1,
        request: wp_image_description_v1::Request,
        data: &Arc<ImageDesc>,
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        if let wp_image_description_v1::Request::GetInformation { information } = request {
            let info = di.init(information, ());
            let Some(d) = data.desc else {
                resource.post_error(wp_image_description_v1::Error::NotReady, "описание не готово");
                return;
            };
            send_info(&info, &d);
        }
    }
}

impl Dispatch<WpImageDescriptionInfoV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &WpImageDescriptionInfoV1,
        _request: <WpImageDescriptionInfoV1 as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, State>,
    ) {
    }
}

fn send_info(info: &WpImageDescriptionInfoV1, d: &Desc) {
    // координаты цветности ×1 000 000
    let (r, g, b) = if d.bt2020 { ((708_000, 292_000), (170_000, 797_000), (131_000, 46_000)) } else { ((640_000, 330_000), (300_000, 600_000), (150_000, 60_000)) };
    let w = (312_700, 329_000);
    info.primaries(r.0, r.1, g.0, g.1, b.0, b.1, w.0, w.1);
    info.primaries_named(if d.bt2020 { Primaries::Bt2020 } else { Primaries::Srgb });
    info.tf_named(d.tf.to_wl());
    info.luminances((d.min_lum * 10_000.0) as u32, d.max_lum as u32, d.ref_lum as u32);
    info.target_primaries(r.0, r.1, g.0, g.1, b.0, b.1, w.0, w.1);
    info.target_luminance((d.min_lum * 10_000.0) as u32, d.max_lum as u32);
    if d.max_cll > 0.0 {
        info.target_max_cll(d.max_cll as u32);
    }
    if d.max_fall > 0.0 {
        info.target_max_fall(d.max_fall as u32);
    }
    info.done();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_srgb_is_not_managed() {
        assert!(Desc::srgb().is_plain());
        let lin = Desc { tf: Tf::ExtLinear, ..Desc::srgb() };
        assert!(!lin.is_plain());
        let pq = Desc { tf: Tf::Pq, bt2020: true, min_lum: 0.005, max_lum: 10_000.0, ref_lum: 203.0, max_cll: 0.0, max_fall: 0.0 };
        assert!(!pq.is_plain());
        // scRGB без метаданных — без сжатия
        assert!((lin.peak_rel() - 1.0).abs() < 1e-6);
        // пик по умолчанию — 1000 нит относительно опорного 203
        assert!((pq.peak_rel() - 1000.0 / 203.0).abs() < 1e-3);
        assert_ne!(pq.identity(), Desc::srgb().identity());
    }
}

// ─── рендер ─────────────────────────────────────────────────────────────────

use smithay::backend::renderer::{
    element::{surface::WaylandSurfaceRenderElement, Element, Kind, RenderElement, UnderlyingStorage},
    gles::{GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType},
    utils::{CommitCounter, DamageSet, OpaqueRegions},
    ImportAll, Renderer,
};
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform};

/// Буфер клиента (scRGB, PQ, HLG; BT.709 или BT.2020) → рабочее пространство кадра:
/// - HDR-вывод: extended sRGB (белый вывода = 1.0, света выше, цвет вне BT.709 — со знаком), его
///   потом переводит в PQ проход вывода (`hdr::PQ_SHADER`);
/// - SDR-вывод: тонмаппинг по максимуму RGB (колено 0.8, пик контента → 1.0) и sRGB.
/// Текстура premultiplied: перевод — над цветом без альфы.
pub const COLOR_SHADER: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
// 1 ext_linear, 2 PQ, 3 HLG, 4 sRGB, 5 гамма 2.2
uniform float tf;
uniform float bt2020;
// относительные кривые: яркость 1.0 в долях опорного белого (max_lum / ref_lum)
uniform float rel_scale;
// опорный белый контента, нит (PQ, HLG)
uniform float ref_lum;
// пик контента в долях опорного белого
uniform float peak_rel;
// 1 — HDR-вывод (extended sRGB), 0 — SDR (тонмаппинг)
uniform float out_hdr;
varying vec2 v_coords;

vec3 pq_eotf(vec3 e) {
    vec3 p = pow(clamp(e, 0.0, 1.0), vec3(1.0 / 78.84375));
    return 10000.0 * pow(max(p - 0.8359375, vec3(0.0)) / (18.8515625 - 18.6875 * p), vec3(1.0 / 0.1593017578125));
}

vec3 hlg_eotf(vec3 e) {
    vec3 lo = e * e / 3.0;
    vec3 hi = (exp((e - 0.55991073) / 0.17883277) + 0.28466892) / 12.0;
    vec3 scene = mix(lo, hi, step(vec3(0.5), e));
    return 1000.0 * pow(max(scene, vec3(0.0)), vec3(1.2));
}

vec3 srgb_eotf(vec3 c) {
    vec3 a = abs(c);
    vec3 lo = a / 12.92;
    vec3 hi = pow((a + 0.055) / 1.055, vec3(2.4));
    return sign(c) * mix(lo, hi, step(vec3(0.04045), a));
}

vec3 srgb_oetf(vec3 l) {
    vec3 a = abs(l);
    vec3 lo = a * 12.92;
    vec3 hi = 1.055 * pow(a, vec3(1.0 / 2.4)) - 0.055;
    return sign(l) * mix(lo, hi, step(vec3(0.0031308), a));
}

void main() {
    vec4 px = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    px.a = 1.0;
#endif
    vec3 e = px.a > 0.0 ? px.rgb / px.a : vec3(0.0);
    // в свет, в долях опорного белого
    vec3 l;
    if (tf < 1.5) {
        l = e * rel_scale;
    } else if (tf < 2.5) {
        l = pq_eotf(e) / ref_lum;
    } else if (tf < 3.5) {
        l = hlg_eotf(e) / ref_lum;
    } else if (tf < 4.5) {
        l = srgb_eotf(e) * rel_scale;
    } else {
        l = sign(e) * pow(abs(e), vec3(2.2)) * rel_scale;
    }
    if (bt2020 > 0.5) {
        l = vec3(
            dot(l, vec3(1.660491, -0.587641, -0.072850)),
            dot(l, vec3(-0.124550, 1.132900, -0.008349)),
            dot(l, vec3(-0.018151, -0.100579, 1.118730))
        );
    }
    vec3 o;
    if (out_hdr > 0.5) {
        o = srgb_oetf(l);
    } else {
        // тонмаппинг: до колена без изменений, выше — плавно к 1.0 у пика контента
        float m = max(l.r, max(l.g, l.b));
        float k = 0.8;
        if (m > k && peak_rel > 1.001) {
            float t = (m - k) / (peak_rel - k);
            float y = k + (1.0 - k) * (1.0 - exp(-3.0 * t)) / (1.0 - exp(-3.0));
            l *= y / m;
        }
        // вне охвата BT.709 — к яркости
        float mn = min(l.r, min(l.g, l.b));
        if (mn < 0.0) {
            float lum = dot(l, vec3(0.2126, 0.7152, 0.0722));
            l = lum > 0.0 ? vec3(lum) + (l - vec3(lum)) * (lum / (lum - mn)) : vec3(0.0);
        }
        o = srgb_oetf(clamp(l, 0.0, 1.0));
    }
    gl_FragColor = vec4(o * px.a, px.a) * alpha;
}
"#;

/// Рендереры, умеющие рисовать цветоуправляемые поверхности (GLES).
pub trait ColorDraw: Renderer {
    /// Программа перевода цвета (компилируется один раз на контекст GL).
    fn color_program(&mut self) -> Option<GlesTexProgram>;
    /// Подменить программу текстур кадра (до `end_color`).
    fn begin_color(frame: &mut Self::Frame<'_, '_>, program: &GlesTexProgram, uniforms: Vec<Uniform<'static>>);
    fn end_color(frame: &mut Self::Frame<'_, '_>);
}

fn gles_program(gles: &mut GlesRenderer) -> Option<GlesTexProgram> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<usize, GlesTexProgram>> = std::cell::RefCell::new(HashMap::new());
    }
    let key = gles.egl_context().get_context_handle() as usize;
    if let Some(p) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(p);
    }
    let names: Vec<UniformName<'static>> = ["tf", "bt2020", "rel_scale", "ref_lum", "peak_rel", "out_hdr"]
        .into_iter()
        .map(|n| UniformName::new(n, UniformType::_1f))
        .collect();
    match gles.compile_custom_texture_shader(COLOR_SHADER, &names) {
        Ok(p) => {
            CACHE.with(|c| c.borrow_mut().insert(key, p.clone()));
            Some(p)
        }
        Err(e) => {
            tracing::warn!(?e, "шейдер управления цветом");
            None
        }
    }
}

impl ColorDraw for GlesRenderer {
    fn color_program(&mut self) -> Option<GlesTexProgram> {
        gles_program(self)
    }
    fn begin_color(frame: &mut GlesFrame<'_, '_>, program: &GlesTexProgram, uniforms: Vec<Uniform<'static>>) {
        frame.override_default_tex_program(program.clone(), uniforms);
    }
    fn end_color(frame: &mut GlesFrame<'_, '_>) {
        frame.clear_tex_program_override();
    }
}

impl<'a> ColorDraw for crate::backend::tty::TtyRenderer<'a> {
    fn color_program(&mut self) -> Option<GlesTexProgram> {
        gles_program(self.as_mut())
    }
    fn begin_color(frame: &mut Self::Frame<'_, '_>, program: &GlesTexProgram, uniforms: Vec<Uniform<'static>>) {
        let g: &mut GlesFrame<'_, '_> = frame.as_mut();
        g.override_default_tex_program(program.clone(), uniforms);
    }
    fn end_color(frame: &mut Self::Frame<'_, '_>) {
        let g: &mut GlesFrame<'_, '_> = frame.as_mut();
        g.clear_tex_program_override();
    }
}

#[cfg(feature = "pixman")]
impl ColorDraw for smithay::backend::renderer::pixman::PixmanRenderer {
    fn color_program(&mut self) -> Option<GlesTexProgram> {
        None
    }
    fn begin_color(_frame: &mut Self::Frame<'_, '_>, _program: &GlesTexProgram, _uniforms: Vec<Uniform<'static>>) {}
    fn end_color(_frame: &mut Self::Frame<'_, '_>) {}
}

/// Поверхность с цветом не sRGB: рисуется через [`COLOR_SHADER`]; на план дисплея напрямую не идёт.
pub struct ManagedElement<R: Renderer> {
    inner: WaylandSurfaceRenderElement<R>,
    desc: Desc,
    hdr_out: bool,
    program: GlesTexProgram,
}

impl<R: Renderer + ImportAll> ManagedElement<R> {
    pub fn new(inner: WaylandSurfaceRenderElement<R>, desc: Desc, hdr_out: bool, program: GlesTexProgram) -> Self {
        Self { inner, desc, hdr_out, program }
    }

    fn uniforms(&self) -> Vec<Uniform<'static>> {
        let d = &self.desc;
        vec![
            Uniform::new("tf", d.tf.code()),
            Uniform::new("bt2020", if d.bt2020 { 1.0f32 } else { 0.0 }),
            Uniform::new("rel_scale", d.max_lum / d.ref_lum.max(0.01)),
            Uniform::new("ref_lum", d.ref_lum.max(1.0)),
            Uniform::new("peak_rel", d.peak_rel()),
            Uniform::new("out_hdr", if self.hdr_out { 1.0f32 } else { 0.0 }),
        ]
    }
}

impl<R: Renderer + ImportAll> Element for ManagedElement<R> {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R> RenderElement<R> for ManagedElement<R>
where
    R: Renderer + ImportAll + ColorDraw,
    R::TextureId: smithay::backend::renderer::Texture + 'static,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        R::begin_color(frame, &self.program, self.uniforms());
        let r = RenderElement::<R>::draw(&self.inner, frame, src, dst, damage, opaque_regions);
        R::end_color(frame);
        r
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Обернуть элемент поверхности, если её цвет не sRGB (иначе как есть).
pub fn wrap<R: Renderer + ImportAll>(
    e: WaylandSurfaceRenderElement<R>,
    hdr_out: bool,
    program: Option<&GlesTexProgram>,
) -> Result<ManagedElement<R>, WaylandSurfaceRenderElement<R>> {
    match (program, managed(e.id())) {
        (Some(p), Some(d)) => Ok(ManagedElement::new(e, d, hdr_out, p.clone())),
        _ => Err(e),
    }
}

/// Включён ли HDR на выводе.
pub fn output_hdr(output: &Output) -> bool {
    output
        .user_data()
        .get::<crate::backend::tty::HdrStatus>()
        .and_then(|s| s.0.lock().ok().map(|g| g.active))
        .unwrap_or(false)
}
