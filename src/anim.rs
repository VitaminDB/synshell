//! Вспомогательная анимация: easing, частицы, всплывающие подписи.

use syngui::prelude::*;

#[inline]
pub fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    1.0 - (1.0 - t).powi(3)
}

#[inline]
pub fn ease_in_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * t
}

#[inline]
pub fn ease_out_back(t: f32) -> f32 {
    let t = clamp01(t);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

#[inline]
pub fn ease_out_elastic(t: f32) -> f32 {
    let t = clamp01(t);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let c4 = std::f32::consts::TAU / 3.0;
    2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
}

#[inline]
pub fn smoothstep(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

/// «Удар»: быстро вверх, плавно обратно (для pop-эффекта счёта).
#[inline]
pub fn punch(t: f32) -> f32 {
    let t = clamp01(t);
    (1.0 - t) * (t * std::f32::consts::PI).sin().max(0.0).powf(0.6)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Экспоненциальное сглаживание, независимое от кадра.
#[inline]
pub fn damp(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let k = 1.0 - (-rate * dt).exp();
    cur + (target - cur) * k
}

// ── Частицы ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Square,
    Round,
    /// Горизонтальная щепка/осколок.
    BarH,
    /// Вертикальная щепка/осколок.
    BarV,
    /// Ромб (искра/кристалл): квадрат, повёрнутый визуально через два прямоугольника.
    Spark,
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Point,
    pub vel: (f32, f32),
    pub size: f32,
    pub color: Color,
    pub age: f32,
    pub life: f32,
    pub gravity: f32,
    pub drag: f32,
    pub round: bool,
    pub shape: Shape,
}

impl Default for Particle {
    fn default() -> Self {
        Particle {
            pos: Point::new(0.0, 0.0),
            vel: (0.0, 0.0),
            size: 4.0,
            color: Color::WHITE,
            age: 0.0,
            life: 1.0,
            gravity: 300.0,
            drag: 1.0,
            round: false,
            shape: Shape::Square,
        }
    }
}

#[derive(Default)]
pub struct Particles {
    pub items: Vec<Particle>,
}

impl Particles {
    pub const MAX: usize = 600;

    pub fn push(&mut self, p: Particle) {
        if self.items.len() >= Self::MAX {
            self.items.remove(0);
        }
        self.items.push(p);
    }

    pub fn update(&mut self, dt: f32) {
        for p in self.items.iter_mut() {
            p.age += dt;
            p.vel.1 += p.gravity * dt;
            let k = (-p.drag * dt).exp();
            p.vel.0 *= k;
            p.vel.1 *= k;
            p.pos.x += p.vel.0 * dt;
            p.pos.y += p.vel.1 * dt;
        }
        self.items.retain(|p| p.age < p.life);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn draw(&self, list: &mut DisplayList) {
        for p in &self.items {
            let t = clamp01(p.age / p.life);
            let alpha = (1.0 - t).powf(1.4);
            let s = p.size * (1.0 - t * 0.6);
            let col = p.color.with_alpha(p.color.a * alpha);
            match p.shape {
                Shape::BarH => {
                    let rect = Rect::new(Point::new(p.pos.x - s, p.pos.y - s * 0.22), Size::new(s * 2.0, s * 0.44));
                    list.push_rect(rect, col, [s * 0.2; 4]);
                }
                Shape::BarV => {
                    let rect = Rect::new(Point::new(p.pos.x - s * 0.22, p.pos.y - s), Size::new(s * 0.44, s * 2.0));
                    list.push_rect(rect, col, [s * 0.2; 4]);
                }
                Shape::Spark => {
                    let a = Rect::new(Point::new(p.pos.x - s, p.pos.y - s * 0.16), Size::new(s * 2.0, s * 0.32));
                    let b = Rect::new(Point::new(p.pos.x - s * 0.16, p.pos.y - s), Size::new(s * 0.32, s * 2.0));
                    list.push_rect(a, col, [s * 0.16; 4]);
                    list.push_rect(b, col, [s * 0.16; 4]);
                }
                _ => {
                    let rect = Rect::new(Point::new(p.pos.x - s * 0.5, p.pos.y - s * 0.5), Size::new(s, s));
                    let r = if p.round || p.shape == Shape::Round { s * 0.5 } else { s * 0.22 };
                    list.push_rect(rect, col, [r; 4]);
                }
            }
        }
    }
}

// ── Всплывающие подписи ────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FloatText {
    pub text: String,
    pub pos: Point,
    pub born: f32,
    pub life: f32,
    pub rise: f32,
    pub size: f32,
    pub color: Color,
    pub weight: u16,
}

impl FloatText {
    pub fn progress(&self, now: f32) -> f32 {
        clamp01((now - self.born) / self.life)
    }

    pub fn alive(&self, now: f32) -> bool {
        now - self.born < self.life
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_hit_endpoints() {
        for f in [ease_out_cubic, ease_in_cubic, ease_out_back, ease_out_elastic, smoothstep] {
            assert!((f(0.0)).abs() < 1e-5);
            assert!((f(1.0) - 1.0).abs() < 1e-4);
            assert!((f(-3.0)).abs() < 1e-5);
            assert!((f(9.0) - 1.0).abs() < 1e-4);
        }
        assert!(punch(0.0).abs() < 1e-5 && punch(1.0).abs() < 1e-5 && punch(0.4) > 0.3);
    }

    #[test]
    fn damp_converges() {
        let mut v = 0.0;
        for _ in 0..200 {
            v = damp(v, 10.0, 12.0, 1.0 / 60.0);
        }
        assert!((v - 10.0).abs() < 1e-3);
    }

    #[test]
    fn particles_die_and_cap() {
        let mut ps = Particles::default();
        for i in 0..(Particles::MAX + 50) {
            ps.push(Particle { vel: (1.0, 0.0), life: if i % 2 == 0 { 0.1 } else { 1.0 }, gravity: 0.0, drag: 0.0, ..Default::default() });
        }
        assert_eq!(ps.items.len(), Particles::MAX);
        ps.update(0.2);
        assert!(ps.items.len() < Particles::MAX);
        ps.update(2.0);
        assert!(ps.is_empty());
    }
}
