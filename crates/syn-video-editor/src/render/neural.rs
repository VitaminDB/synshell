//! Нейрофильтры экспорта (synaptix-vfx, CUDA): промежуточные кадры RIFE
//! (`interpolate` — плавность при частоте проекта выше частоты клипа,
//! замедление) и увеличение (`upscale` — источник меньше кадра проекта).
//! Модели — бандлы `.syn` в папках `vfx` библиотек службы. Нет CUDA или
//! моделей — фильтр пропускается (кадры как есть), причина — в журнал.

use std::sync::Arc;

use syngui::video::RgbaFrame;

/// Найденная модель (id, путь).
pub fn model(arch_pred: impl Fn(&str, &str) -> bool, prefer: &str) -> Option<(String, std::path::PathBuf)> {
    let mut v: Vec<(String, std::path::PathBuf, String)> = synaptixd::vfx_models::bundles().into_iter().filter(|(id, _, arch)| arch_pred(id, arch)).collect();
    v.sort_by_key(|(id, _, _)| if !prefer.is_empty() && id.contains(prefer) { 0 } else { 1 });
    v.into_iter().next().map(|(id, p, _)| (id, p))
}

/// Список моделей для подсказки агенту и инспектору.
pub fn available() -> Vec<(String, String)> {
    synaptixd::vfx_models::bundles().into_iter().map(|(id, _, arch)| (id, arch)).collect()
}

#[cfg(target_arch = "x86_64")]
pub struct Interp {
    rife: synaptix_vfx::rife::Rife,
    pair: Option<(f64, f64)>,
    cut: bool,
}

#[cfg(target_arch = "x86_64")]
impl Interp {
    pub fn open() -> Result<Self, String> {
        let (_, path) = model(|_, arch| arch == "rife", "rife-4-25").ok_or("моделей RIFE нет (rife-*.syn в папке vfx библиотеки нейросетей)")?;
        let w = synaptix_vfx::weights::Weights::open(&path).map_err(|e| e.to_string())?;
        let g = synaptix_vfx::gpu::Gpu::new(0).map_err(|e| e.to_string())?;
        let rife = synaptix_vfx::rife::Rife::load(g, &w).map_err(|e| e.to_string())?;
        Ok(Self { rife, pair: None, cut: false })
    }

    /// Кадр в момент `s` (время источника) между `a` и `b`.
    pub fn between(&mut self, a: &RgbaFrame, b: &RgbaFrame, s: f64) -> Result<RgbaFrame, String> {
        if a.width != b.width || a.height != b.height || b.pts <= a.pts {
            return Ok(a.clone());
        }
        if self.pair != Some((a.pts, b.pts)) {
            let (w, h) = (a.width as usize, a.height as usize);
            self.rife.set_flow_scale(if h > 1440 { 0.5 } else { 1.0 });
            self.rife.reset();
            self.rife.push(&a.rgba, w, h).map_err(|e| e.to_string())?;
            self.rife.push(&b.rgba, w, h).map_err(|e| e.to_string())?;
            // смена сцены — без промежуточных (иначе каша из двух планов)
            self.cut = self.rife.scene_diff().map_err(|e| e.to_string())? > 28.0;
            self.pair = Some((a.pts, b.pts));
        }
        let t = ((s - a.pts) / (b.pts - a.pts)).clamp(0.0, 1.0) as f32;
        if self.cut || t <= 0.01 {
            return Ok(if t < 0.5 || !self.cut { a.clone() } else { b.clone() });
        }
        if t >= 0.99 {
            return Ok(b.clone());
        }
        let px = self.rife.interpolate(t).map_err(|e| e.to_string())?;
        Ok(RgbaFrame { width: a.width, height: a.height, pts: s, rgba: Arc::new(px.to_vec()) })
    }
}

#[cfg(target_arch = "x86_64")]
pub struct Upscale {
    up: Box<dyn synaptix_vfx::Upscaler>,
}

#[cfg(target_arch = "x86_64")]
impl Upscale {
    /// `model` — часть id или arch (plksr, span, compact, nanovsr); пусто — любая покадровая.
    pub fn open(model_hint: &str) -> Result<Self, String> {
        let hint = model_hint.to_ascii_lowercase();
        let (_, path) = model(|id, arch| arch != "rife" && arch != "nanovsr" && (hint.is_empty() || id.to_ascii_lowercase().contains(&hint) || arch.contains(&hint)), "").ok_or_else(|| format!("моделей увеличения{} нет в папке vfx", if hint.is_empty() { String::new() } else { format!(" «{hint}»") }))?;
        let up = synaptix_vfx::open_upscaler(&path, 0).map_err(|e| e.to_string())?;
        Ok(Self { up })
    }

    pub fn scale(&self) -> u32 {
        self.up.scale()
    }

    pub fn apply(&mut self, f: &RgbaFrame) -> Result<RgbaFrame, String> {
        let mut out = None;
        self.up.push(&f.rgba, f.width, f.height, &mut |fr| out = Some(RgbaFrame { width: fr.width, height: fr.height, pts: f.pts, rgba: Arc::new(fr.rgba.to_vec()) })).map_err(|e| e.to_string())?;
        out.ok_or_else(|| "увеличение не отдало кадр".into())
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub struct Interp;
#[cfg(not(target_arch = "x86_64"))]
impl Interp {
    pub fn open() -> Result<Self, String> {
        Err("нейрофильтры — только на компьютере с CUDA".into())
    }
    pub fn between(&mut self, a: &RgbaFrame, _b: &RgbaFrame, _s: f64) -> Result<RgbaFrame, String> {
        Ok(a.clone())
    }
}
#[cfg(not(target_arch = "x86_64"))]
pub struct Upscale;
#[cfg(not(target_arch = "x86_64"))]
impl Upscale {
    pub fn open(_m: &str) -> Result<Self, String> {
        Err("нейрофильтры — только на компьютере с CUDA".into())
    }
    pub fn scale(&self) -> u32 {
        1
    }
    pub fn apply(&mut self, f: &RgbaFrame) -> Result<RgbaFrame, String> {
        Ok(f.clone())
    }
}
