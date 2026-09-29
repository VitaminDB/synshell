//! Анимации композитора: значение от 0 до 1 во времени с кривой.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// Быстрый старт, мягкая остановка (для появления и перемещения).
    EaseOutCubic,
    /// С небольшим перелётом (появление окон «зумом»).
    EaseOutBack,
    EaseInOutCubic,
    /// Для исчезновения.
    EaseInCubic,
    Linear,
}

impl Curve {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Curve::EaseInCubic => t * t * t,
            Curve::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Curve::EaseOutBack => {
                let c1 = 1.10158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Animation {
    start: Instant,
    duration: Duration,
    from: f64,
    to: f64,
    curve: Curve,
}

impl Animation {
    pub fn new(from: f64, to: f64, duration: Duration, curve: Curve) -> Self {
        Self { start: Instant::now(), duration, from, to, curve }
    }

    pub fn value(&self) -> f64 {
        let t = if self.duration.is_zero() {
            1.0
        } else {
            self.start.elapsed().as_secs_f64() / self.duration.as_secs_f64()
        };
        self.from + (self.to - self.from) * self.curve.apply(t)
    }

    pub fn is_done(&self) -> bool {
        self.start.elapsed() >= self.duration
    }

    pub fn to(&self) -> f64 {
        self.to
    }
}

/// Длительность с учётом `animations.speed`; 0 — если анимации выключены.
pub fn duration(config: &synshell_common::config::Animations, base_ms: u64) -> Duration {
    if !config.enabled || config.speed <= 0.0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(base_ms as f64 / 1000.0 * config.speed as f64)
}
