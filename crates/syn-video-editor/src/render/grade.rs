//! Первичная цветокоррекция (F2): колёса «Тени / Средние / Света»
//! (lift / gamma / gain: оттенок точкой в круге и яркость) и кривые —
//! общая (яркость) и по каналам R, G, B. Каналы не смешиваются, поэтому
//! всё сводится к трём таблицам 256 значений: одинаково в просмотре и
//! экспорте, дёшево на кадр. Применяется до LUT (LUT — «look» поверх).
//!
//! Параметры эффекта `grade`: `lift_x`, `lift_y`, `lift`, `gamma_x`,
//! `gamma_y`, `gamma`, `gain_x`, `gain_y`, `gain` (точка колеса — в
//! единичном круге, яркость −1…1), `curve`, `curve_r`, `curve_g`, `curve_b`
//! — точки `[[x, y], …]` 0–1 (концы 0,0 и 1,1 подразумеваются).

use rayon::prelude::*;
use serde_json::{Map, Value};

/// Колесо: точка оттенка (x, y в единичном круге) и яркость.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel {
    pub x: f32,
    pub y: f32,
    pub l: f32,
}

impl Wheel {
    /// Сдвиг по каналам R, G, B: оттенок точки (красный — 0°, зелёный — 120°, синий — 240°), сумма — 0.
    pub fn tint(&self) -> [f32; 3] {
        let r = self.x.hypot(self.y).min(1.0);
        let h = self.y.atan2(self.x);
        let tau = std::f32::consts::TAU;
        [r * h.cos(), r * (h - tau / 3.0).cos(), r * (h - 2.0 * tau / 3.0).cos()]
    }

    fn is_zero(&self) -> bool {
        self.x.abs() < 1e-4 && self.y.abs() < 1e-4 && self.l.abs() < 1e-4
    }
}

/// Цвет точки колеса (для отрисовки): тот же оттенок, что даёт `tint`.
pub fn wheel_color(x: f32, y: f32) -> [f32; 3] {
    let t = Wheel { x, y, l: 0.0 }.tint();
    [(0.5 + 0.5 * t[0]).clamp(0.0, 1.0), (0.5 + 0.5 * t[1]).clamp(0.0, 1.0), (0.5 + 0.5 * t[2]).clamp(0.0, 1.0)]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grade {
    pub lift: Wheel,
    pub gamma: Wheel,
    pub gain: Wheel,
    /// Кривые: 0 — общая (яркость), 1–3 — R, G, B.
    pub curves: [Vec<[f32; 2]>; 4],
}

pub const CURVE_KEYS: [&str; 4] = ["curve", "curve_r", "curve_g", "curve_b"];
pub const WHEEL_KEYS: [&str; 3] = ["lift", "gamma", "gain"];

fn num(m: &Map<String, Value>, k: &str) -> f32 {
    m.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// Точки кривой из параметра.
pub fn points(v: Option<&Value>) -> Vec<[f32; 2]> {
    let mut p: Vec<[f32; 2]> = v
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|q| {
                    let q = q.as_array()?;
                    Some([q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32])
                })
                .map(|[x, y]| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
                .collect()
        })
        .unwrap_or_default();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
    p
}

impl Grade {
    pub fn from_params(m: &Map<String, Value>) -> Self {
        let w = |k: &str| Wheel { x: num(m, &format!("{k}_x")), y: num(m, &format!("{k}_y")), l: num(m, k) };
        Grade {
            lift: w("lift"),
            gamma: w("gamma"),
            gain: w("gain"),
            curves: CURVE_KEYS.map(|k| points(m.get(k))),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.lift.is_zero() && self.gamma.is_zero() && self.gain.is_zero() && self.curves.iter().all(|c| c.iter().all(|p| (p[0] - p[1]).abs() < 1e-4))
    }

    /// Значение канала `ch` (0–2) для входа 0–1.
    pub fn map(&self, ch: usize, v: f32) -> f32 {
        let (lt, gt, kt) = (self.lift.tint()[ch], self.gamma.tint()[ch], self.gain.tint()[ch]);
        // оттенок на краю круга — заметный, но не «заливка» (как колёса Resolve)
        let lift = self.lift.l * 0.5 + lt * 0.15;
        let gain = 1.0 + self.gain.l + kt * 0.3;
        let gamma = (1.0 + self.gamma.l + gt * 0.3).max(0.1);
        let mut x = v + lift * (1.0 - v);
        x = (x * gain).clamp(0.0, 1.0);
        x = x.powf(1.0 / gamma);
        x = eval_curve(&self.curves[0], x);
        eval_curve(&self.curves[ch + 1], x).clamp(0.0, 1.0)
    }

    /// Таблицы по каналам.
    pub fn tables(&self) -> [[u8; 256]; 3] {
        let mut t = [[0u8; 256]; 3];
        for (ch, tab) in t.iter_mut().enumerate() {
            for (i, o) in tab.iter_mut().enumerate() {
                *o = (self.map(ch, i as f32 / 255.0) * 255.0 + 0.5) as u8;
            }
        }
        t
    }
}

/// Монотонная кубическая кривая (Фрича — Карлсона) через точки и концы 0,0 / 1,1.
pub fn eval_curve(pts: &[[f32; 2]], x: f32) -> f32 {
    if pts.is_empty() {
        return x;
    }
    let mut p: Vec<[f32; 2]> = Vec::with_capacity(pts.len() + 2);
    if pts[0][0] > 1e-4 {
        p.push([0.0, 0.0]);
    }
    p.extend_from_slice(pts);
    if p.last().is_some_and(|l| l[0] < 1.0 - 1e-4) {
        p.push([1.0, 1.0]);
    }
    let n = p.len();
    if n < 2 {
        return x;
    }
    if x <= p[0][0] {
        return p[0][1];
    }
    if x >= p[n - 1][0] {
        return p[n - 1][1];
    }
    let d: Vec<f32> = (0..n - 1).map(|i| (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0]).max(1e-6)).collect();
    let mut m = vec![0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for i in 1..n - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
    }
    for i in 0..n - 1 {
        if d[i].abs() < 1e-6 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
    }
    let i = p.windows(2).position(|w| x >= w[0][0] && x <= w[1][0]).unwrap_or(n - 2);
    let (x0, x1, y0, y1) = (p[i][0], p[i + 1][0], p[i][1], p[i + 1][1]);
    let h = (x1 - x0).max(1e-6);
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * m[i] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * m[i + 1]
}

/// Применить таблицы к RGBA; `from_x` — с какого столбца (сравнение «до/после»).
pub fn apply(t: &[[u8; 256]; 3], rgba: &mut [u8], width: usize, from_x: usize) {
    let row = width.max(1) * 4;
    rgba.par_chunks_mut(row).for_each(|line| {
        for px in line.chunks_exact_mut(4).skip(from_x) {
            px[0] = t[0][px[0] as usize];
            px[1] = t[1][px[1] as usize];
            px[2] = t[2][px[2] as usize];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identity_and_wheels() {
        let g = Grade::default();
        assert!(g.is_identity());
        assert_eq!(g.tables()[0][100], 100);
        // светлее света — выше
        let m = json!({"gain": 0.3}).as_object().unwrap().clone();
        let g = Grade::from_params(&m);
        assert!(g.map(0, 0.5) > 0.6);
        // тени в синий: синий канал в тенях поднимается, красный — нет
        let m = json!({"lift_x": -0.5, "lift_y": -0.866}).as_object().unwrap().clone();
        let g = Grade::from_params(&m);
        assert!(g.map(2, 0.05) > g.map(0, 0.05) + 0.1, "{} {}", g.map(2, 0.05), g.map(0, 0.05));
        // света почти не трогаются
        assert!((g.map(2, 1.0) - g.map(0, 1.0)).abs() < 0.02);
    }

    #[test]
    fn curves_monotone_and_through_points() {
        let s = [[0.25, 0.15], [0.75, 0.85]];
        assert!((eval_curve(&s, 0.25) - 0.15).abs() < 1e-4);
        assert!((eval_curve(&s, 0.75) - 0.85).abs() < 1e-4);
        let mut prev = -1.0;
        for i in 0..=100 {
            let v = eval_curve(&s, i as f32 / 100.0);
            assert!(v >= prev - 1e-6 && (0.0..=1.0).contains(&v));
            prev = v;
        }
        // кривая канала: только красный темнее
        let m = json!({"curve_r": [[0.5, 0.3]]}).as_object().unwrap().clone();
        let g = Grade::from_params(&m);
        assert!(g.map(0, 0.5) < 0.35 && (g.map(1, 0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn apply_from_column() {
        let t = Grade::from_params(json!({"gain": 0.5}).as_object().unwrap()).tables();
        let mut px = vec![100u8; 4 * 4 * 2];
        apply(&t, &mut px, 4, 2);
        assert_eq!(px[0], 100);
        assert!(px[8] > 100);
        assert!(px[16] == 100 && px[24] > 100);
    }
}
