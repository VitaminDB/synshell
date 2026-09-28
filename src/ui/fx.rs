//! Состояние визуальных эффектов (не влияет на модель игры).

use crate::anim::{FloatText, Particles};
use syngui::prelude::*;

/// Кубик только что поставлен — «вдавливается» с вспышкой.
#[derive(Clone, Copy, Debug)]
pub struct Pop {
    pub r: usize,
    pub c: usize,
    pub born: f32,
}

/// Кубик из очищенной линии — сначала вспыхивает, затем схлопывается.
#[derive(Clone, Copy, Debug)]
pub struct Vanish {
    pub r: usize,
    pub c: usize,
    pub color: u8,
    pub born: f32,
    pub delay: f32,
    pub spawned: bool,
}

/// Белая полоса по очищенной линии.
#[derive(Clone, Copy, Debug)]
pub struct LineFlash {
    pub is_row: bool,
    pub idx: usize,
    pub born: f32,
}

/// Крупная надпись над полем («GREAT!», «COMBO ×3», «PERFECT!»).
#[derive(Clone, Debug)]
pub struct Banner {
    pub text: String,
    pub sub: Option<String>,
    pub born: f32,
    pub color: Color,
}

/// Фигура возвращается в слот после неудачного броска.
#[derive(Clone, Copy, Debug)]
pub struct Return {
    pub slot: usize,
    pub from: Point,
    pub from_cell: f32,
    pub born: f32,
}

/// Расходящееся кольцо от места очистки.
#[derive(Clone, Copy, Debug)]
pub struct Shockwave {
    pub center: Point,
    pub born: f32,
    pub color: Color,
    pub max_r: f32,
}

/// Вспышка на весь экран.
#[derive(Clone, Copy, Debug)]
pub struct Flash {
    pub born: f32,
    pub color: Color,
    pub strength: f32,
}

/// Волна по кубикам от очищенной линии: кубики по очереди «подпрыгивают».
#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    /// Центр в координатах клеток.
    pub origin: (f32, f32),
    pub born: f32,
}

/// След перетаскиваемой фигуры.
#[derive(Clone, Copy, Debug)]
pub struct TrailPoint {
    pub origin: Point,
    pub cell: f32,
    pub born: f32,
}

/// Залп фейерверка (запланирован на момент `at`).
#[derive(Clone, Copy, Debug)]
pub struct Firework {
    pub at: f32,
    pub pos: Point,
    pub color: Color,
}

/// Контур «приземления» вокруг только что поставленного кубика.
#[derive(Clone, Copy, Debug)]
pub struct Land {
    pub r: usize,
    pub c: usize,
    pub born: f32,
}

/// Световой луч, пробегающий по очищенной линии.
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub is_row: bool,
    pub idx: usize,
    pub born: f32,
    /// Откуда бежит луч: позиция якоря фигуры вдоль линии (в клетках).
    pub from: f32,
    pub color: Color,
}

pub const LAND_DUR: f32 = 0.4;
pub const BEAM_DUR: f32 = 0.5;
pub const RIPPLE_DUR: f32 = 0.7;
pub const TRAIL_DUR: f32 = 0.22;
pub const SHOCK_DUR: f32 = 0.75;
pub const FLASH_DUR: f32 = 0.4;
/// Период блик-волны по кубикам поля.
pub const SHEEN_PERIOD: f32 = 7.0;
pub const POP_DUR: f32 = 0.30;
pub const VANISH_DUR: f32 = 0.34;
pub const VANISH_STAGGER: f32 = 0.035;
pub const LINE_FLASH_DUR: f32 = 0.40;
pub const BANNER_DUR: f32 = 1.25;
pub const RETURN_DUR: f32 = 0.24;
pub const TRAY_IN_DUR: f32 = 0.38;
pub const OVER_DELAY: f32 = 0.55;
pub const CARD_DUR: f32 = 0.35;

#[derive(Default)]
pub struct Fx {
    pub pops: Vec<Pop>,
    pub vanish: Vec<Vanish>,
    pub flashes: Vec<LineFlash>,
    pub particles: Particles,
    pub floats: Vec<FloatText>,
    pub banner: Option<Banner>,
    pub returning: Option<Return>,
    /// Показанный счёт (догоняет настоящий).
    pub score_shown: f32,
    pub score_punch_born: f32,
    pub best_flash_born: Option<f32>,
    pub tray_born: [f32; 3],
    pub tray_hover: [f32; 3],
    /// Момент, когда началось затухание поля после проигрыша.
    pub over_at: Option<f32>,
    /// Плавность появления оверлея (0..1).
    pub overlay_t: f32,
    pub shake_born: f32,
    pub shake_mag: f32,
    pub last_place_at: f32,
    pub shocks: Vec<Shockwave>,
    pub screen_flashes: Vec<Flash>,
    /// Накопитель для равномерного выброса искр в «огненном» режиме.
    pub ember_acc: f32,
    pub ripples: Vec<Ripple>,
    pub trail: Vec<TrailPoint>,
    pub last_trail_at: f32,
    /// Баннеры, ждущие своей очереди (уровень после «КЛАСС!» и т. п.).
    pub banner_queue: Vec<Banner>,
    pub fireworks: Vec<Firework>,
    pub lands: Vec<Land>,
    pub beams: Vec<Beam>,
    /// Момент последнего повышения уровня (пульс кольца уровня).
    pub level_born: f32,
}

impl Fx {
    pub fn new() -> Self {
        Fx { score_punch_born: -10.0, shake_born: -10.0, last_place_at: -10.0, level_born: -10.0, ..Default::default() }
    }

    pub fn prune(&mut self, now: f32) {
        self.pops.retain(|p| now - p.born < POP_DUR);
        self.vanish.retain(|v| now - v.born - v.delay < VANISH_DUR);
        self.flashes.retain(|f| now - f.born < LINE_FLASH_DUR);
        self.floats.retain(|f| f.alive(now));
        if let Some(b) = &self.banner {
            if now - b.born > BANNER_DUR {
                self.banner = None;
            }
        }
        if self.banner.is_none() && !self.banner_queue.is_empty() {
            let mut b = self.banner_queue.remove(0);
            b.born = now;
            self.banner = Some(b);
        }
        self.lands.retain(|l| now - l.born < LAND_DUR);
        self.beams.retain(|b| now - b.born < BEAM_DUR);
        if let Some(r) = &self.returning {
            if now - r.born > RETURN_DUR {
                self.returning = None;
            }
        }
        self.shocks.retain(|s| now - s.born < SHOCK_DUR);
        self.screen_flashes.retain(|f| now - f.born < FLASH_DUR);
        self.ripples.retain(|r| now - r.born < RIPPLE_DUR);
        self.trail.retain(|t| now - t.born < TRAIL_DUR);
    }

    /// Масштаб кубика `(r, c)` от волн: 1.0 — покой.
    pub fn ripple_scale(&self, now: f32, r: usize, c: usize) -> f32 {
        let mut k = 0.0f32;
        for rp in &self.ripples {
            let t = now - rp.born;
            let d = ((r as f32 + 0.5 - rp.origin.0).powi(2) + (c as f32 + 0.5 - rp.origin.1).powi(2)).sqrt();
            let front = t * 11.0;
            let w = (-(d - front).powi(2) / 1.3).exp() * (1.0 - t / RIPPLE_DUR).max(0.0);
            k = k.max(w);
        }
        1.0 + 0.14 * k
    }

    /// Сила блика волны для клетки `(r, c)`: диагональная полоса света
    /// проходит по полю раз в `SHEEN_PERIOD` секунд.
    pub fn sheen(now: f32, r: usize, c: usize) -> f32 {
        let phase = (now % SHEEN_PERIOD) / SHEEN_PERIOD;
        if phase > 0.45 {
            return 0.0;
        }
        let pos = phase / 0.45 * 20.0 - 3.0;
        let d = ((r + c) as f32 - pos).abs();
        (1.0 - d / 1.6).max(0.0).powi(2)
    }

    /// Смещение поля от «встряски» (после большого комбо).
    pub fn shake_offset(&self, now: f32) -> (f32, f32) {
        let t = now - self.shake_born;
        if t < 0.0 || t > 0.35 || self.shake_mag <= 0.0 {
            return (0.0, 0.0);
        }
        let k = (1.0 - t / 0.35).powi(2) * self.shake_mag;
        ((t * 61.0).sin() * k, (t * 47.0).cos() * k * 0.6)
    }
}
