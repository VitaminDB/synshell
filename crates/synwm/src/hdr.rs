//! HDR-вывод: коннектор в режиме PQ (SMPTE ST 2084) / BT.2020 с 10-битным кадром, а всё, что
//! рисует композитор (обычные SDR-окна в sRGB), переводится в PQ одним проходом: кадр сначала
//! собирается как обычно во внеэкранную текстуру (тот же damage, то же смешивание, что в SDR), затем
//! шейдер: sRGB → линейный свет → BT.709 → BT.2020 → яркость белого `sdr_nits` → PQ. Кадр — FP16 в
//! extended sRGB: HDR-окна (`color::ManagedElement`) кладут в него света выше белого и цвета вне
//! BT.709, обычные окна смешиваются с ними как всегда.
//!
//! Монитору сообщаются метаданные HDR10 (`HDR_OUTPUT_METADATA`), `Colorspace=BT2020_RGB` и
//! `max bpc` 10. Возможности монитора — из EDID (блок «HDR Static Metadata», EOTF ST 2084).

use smithay::backend::renderer::{
    damage::OutputDamageTracker,
    element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
    gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform},
    utils::CommitCounter,
    Renderer, Texture,
};
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Size, Transform};

/// Что монитор сообщил о HDR в EDID.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdidHdr {
    /// EOTF SMPTE ST 2084 (PQ) поддержан.
    pub pq: bool,
    /// Желаемая максимальная яркость контента, нит (0 — не сказано).
    pub max_nits: f32,
    /// Желаемая средняя по кадру, нит.
    pub max_fall: f32,
    /// Желаемая минимальная, нит.
    pub min_nits: f32,
}

/// Найти в EDID блок «HDR Static Metadata» (CTA-861 расширенный тег 6) — и в CTA-расширении, и
/// внутри блоков CTA в DisplayID (так у OLED-панелей ноутбуков).
pub fn parse_edid_hdr(edid: &[u8]) -> EdidHdr {
    let mut best = EdidHdr::default();
    // Блок данных CTA: заголовок (тег 7 = «расширенный» в старших 3 битах, длина в младших 5),
    // затем расширенный тег 6, EOTF, дескрипторы, яркости.
    for i in 0..edid.len().saturating_sub(3) {
        let hdr = edid[i];
        let len = (hdr & 0x1f) as usize;
        if hdr >> 5 != 7 || !(3..=6).contains(&len) || edid[i + 1] != 6 || i + 1 + len > edid.len() {
            continue;
        }
        let eotf = edid[i + 2];
        // старшие биты EOTF зарезервированы — так отсекаем случайные совпадения
        if eotf & 0xc0 != 0 || eotf == 0 {
            continue;
        }
        let lum = |k: usize| -> Option<u8> { (len >= k + 1).then(|| edid[i + 1 + k]) };
        let max = lum(3).filter(|&v| v > 0).map(|v| 50.0 * 2f32.powf(v as f32 / 32.0)).unwrap_or(0.0);
        let fall = lum(4).filter(|&v| v > 0).map(|v| 50.0 * 2f32.powf(v as f32 / 32.0)).unwrap_or(0.0);
        let min = lum(5).map(|v| max * (v as f32 / 255.0).powi(2) / 100.0).unwrap_or(0.0);
        let found = EdidHdr { pq: eotf & 0x04 != 0, max_nits: max, max_fall: fall, min_nits: min };
        if found.pq && (!best.pq || found.max_nits > best.max_nits) {
            best = found;
        }
    }
    best
}

/// `struct hdr_output_metadata` ядра (drm_mode.h, 32 байта): HDR10, первичные BT.2020, белый D65.
pub fn metadata_blob(peak_nits: f32, min_nits: f32, max_cll: f32, max_fall: f32) -> [u8; 32] {
    let mut b = [0u8; 32];
    // metadata_type = HDMI_STATIC_METADATA_TYPE1 (0)
    b[0..4].copy_from_slice(&0u32.to_ne_bytes());
    let f = &mut b[4..];
    f[0] = 2; // EOTF: SMPTE ST 2084
    f[1] = 0; // Static Metadata Type 1
    let put = |f: &mut [u8], off: usize, v: u16| f[off..off + 2].copy_from_slice(&v.to_ne_bytes());
    // координаты цветности в единицах 0.00002
    let xy = |v: f32| (v / 0.00002).round() as u16;
    for (k, (x, y)) in [(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)].into_iter().enumerate() {
        put(f, 2 + k * 4, xy(x));
        put(f, 4 + k * 4, xy(y));
    }
    put(f, 14, xy(0.3127));
    put(f, 16, xy(0.3290));
    put(f, 18, peak_nits.round().clamp(1.0, 65535.0) as u16);
    // минимальная — в единицах 0.0001 нит
    put(f, 20, (min_nits / 0.0001).round().clamp(0.0, 65535.0) as u16);
    put(f, 22, max_cll.round().clamp(0.0, 65535.0) as u16);
    put(f, 24, max_fall.round().clamp(0.0, 65535.0) as u16);
    b
}

/// Перевод кадра композитора (sRGB) в PQ BT.2020. Текстура — premultiplied, кадр непрозрачный.
pub const PQ_SHADER: &str = r#"#version 100
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
// яркость белого SDR / 10 000 нит
uniform float white;
varying vec2 v_coords;

// extended sRGB: выше 1.0 — света HDR-окон, меньше 0 — их цвет вне BT.709 (знак сохраняется)
vec3 srgb_to_linear(vec3 c) {
    vec3 a = abs(c);
    vec3 lo = a / 12.92;
    vec3 hi = pow((a + 0.055) / 1.055, vec3(2.4));
    return sign(c) * mix(lo, hi, step(vec3(0.04045), a));
}

vec3 pq(vec3 l) {
    vec3 y = pow(clamp(l, 0.0, 1.0), vec3(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3(78.84375));
}

void main() {
    vec3 c = texture2D(tex, v_coords).rgb;
    vec3 l = srgb_to_linear(c);
    // BT.709 → BT.2020 (линейный свет)
    vec3 w = vec3(
        dot(l, vec3(0.6274, 0.3293, 0.0433)),
        dot(l, vec3(0.0691, 0.9195, 0.0114)),
        dot(l, vec3(0.0164, 0.0880, 0.8956))
    );
    gl_FragColor = vec4(pq(max(w, vec3(0.0)) * white), 1.0);
}
"#;

/// HDR-состояние вывода: внеэкранный кадр и его damage, программа перевода в PQ.
pub struct HdrOut {
    pub sdr_nits: f32,
    pub texture: Option<GlesTexture>,
    pub damage: Option<OutputDamageTracker>,
    pub program: Option<GlesTexProgram>,
    id: Id,
    commit: CommitCounter,
}

impl HdrOut {
    pub fn new(sdr_nits: f32) -> Self {
        Self { sdr_nits, texture: None, damage: None, program: None, id: Id::new(), commit: CommitCounter::default() }
    }

    /// Программа перевода в PQ (компилируется один раз).
    pub fn program(&mut self, gles: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
        if let Some(p) = &self.program {
            return Ok(p.clone());
        }
        let p = gles.compile_custom_texture_shader(PQ_SHADER, &[smithay::backend::renderer::gles::UniformName::new(
            "white",
            smithay::backend::renderer::gles::UniformType::_1f,
        )])?;
        self.program = Some(p.clone());
        Ok(p)
    }

    /// Элемент кадра для DRM: внеэкранная текстура целиком, пересчитанная шейдером. `damage` —
    /// что изменилось во внеэкранном кадре (в координатах вывода).
    pub fn element(&mut self, size: Size<i32, Physical>, transform: Transform, changed: bool) -> Option<HdrElement> {
        if changed {
            self.commit.increment();
        }
        Some(HdrElement {
            id: self.id.clone(),
            commit: self.commit,
            size,
            transform,
            texture: self.texture.clone()?,
            program: self.program.clone()?,
            white: self.sdr_nits / 10_000.0,
        })
    }
}

/// Внеэкранный кадр в PQ — единственный элемент кадра HDR-вывода.
pub struct HdrElement {
    id: Id,
    commit: CommitCounter,
    /// Размер вывода в его координатах (с поворотом).
    size: Size<i32, Physical>,
    /// Поворот вывода: текстура уже в координатах буфера, кадр DRM повернёт ещё раз — снимаем.
    transform: Transform,
    texture: GlesTexture,
    program: GlesTexProgram,
    white: f32,
}

impl Element for HdrElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size(self.texture.size().to_f64())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_size(self.size)
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        smithay::backend::renderer::utils::OpaqueRegions::from_slice(&[Rectangle::from_size(self.size)])
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

/// Рендереры, умеющие нарисовать кадр HDR (GLES).
pub trait HdrDraw: Renderer {
    fn draw_hdr(
        frame: &mut Self::Frame<'_, '_>,
        e: &HdrElement,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error>;
}

fn draw_gles(
    frame: &mut GlesFrame<'_, '_>,
    e: &HdrElement,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    opaque: &[Rectangle<i32, Physical>],
) -> Result<(), GlesError> {
    let src = Rectangle::from_size(e.texture.size().to_f64());
    frame.render_texture_from_to(
        &e.texture,
        src,
        dst,
        damage,
        opaque,
        e.transform.invert(),
        1.0,
        Some(&e.program),
        &[Uniform::new("white", e.white)],
    )
}

impl HdrDraw for GlesRenderer {
    fn draw_hdr(
        frame: &mut GlesFrame<'_, '_>,
        e: &HdrElement,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        draw_gles(frame, e, dst, damage, opaque)
    }
}

impl<'a> HdrDraw for crate::backend::tty::TtyRenderer<'a> {
    fn draw_hdr(
        frame: &mut Self::Frame<'_, '_>,
        e: &HdrElement,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error> {
        let gles: &mut GlesFrame<'_, '_> = frame.as_mut();
        draw_gles(gles, e, dst, damage, opaque).map_err(Into::into)
    }
}

impl<R: HdrDraw> RenderElement<R> for HdrElement {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        R::draw_hdr(frame, self, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EDID панели Samsung ATNA60HU01 (этого ноутбука): блок HDR в CTA внутри DisplayID.
    #[test]
    fn finds_hdr_block() {
        // фрагмент: заголовок расширенного блока (тег 7, длина 6), тег 6, EOTF (SDR + PQ),
        // дескриптор 1, яркости 143/106/2
        let mut edid = vec![0u8; 40];
        edid[20..27].copy_from_slice(&[0xe6, 0x06, 0x05, 0x01, 143, 106, 2]);
        let h = parse_edid_hdr(&edid);
        assert!(h.pq);
        assert!((h.max_nits - 1107.0).abs() < 1.0, "{}", h.max_nits);
        assert!((h.max_fall - 496.7).abs() < 1.0, "{}", h.max_fall);
        assert!(h.min_nits < 0.01);
    }

    #[test]
    fn no_hdr_in_sdr_edid() {
        let edid = vec![0u8; 256];
        assert!(!parse_edid_hdr(&edid).pq);
    }

    #[test]
    fn blob_layout() {
        let b = metadata_blob(1107.0, 0.0011, 1107.0, 497.0);
        assert_eq!(b.len(), 32);
        assert_eq!(b[4], 2); // ST 2084
        // красный x 0.708 → 35400
        assert_eq!(u16::from_ne_bytes([b[6], b[7]]), 35400);
        // белый x 0.3127 → 15635
        assert_eq!(u16::from_ne_bytes([b[18], b[19]]), 15635);
        assert_eq!(u16::from_ne_bytes([b[22], b[23]]), 1107);
        assert_eq!(u16::from_ne_bytes([b[26], b[27]]), 1107);
    }
}
