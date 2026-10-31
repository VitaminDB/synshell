//! Распознавание фигуры по штриху (перо задержали в конце штриха): прямая,
//! эллипс, прямоугольник, треугольник. Как в Onyx: нарисовал от руки,
//! подержал — штрих стал ровной фигурой.

use super::model::{BBox, Rgba, Shape, ShapeKind, P2};

/// Упрощение ломаной (Рамер — Дуглас — Пекер).
pub fn rdp(pts: &[P2], eps: f32) -> Vec<P2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let mut best = (0, 0.0f32);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = super::tools::dist_seg(*p, a, b);
        if d > best.1 {
            best = (i, d);
        }
    }
    if best.1 > eps {
        let mut left = rdp(&pts[..=best.0], eps);
        let right = rdp(&pts[best.0..], eps);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

fn path_len(pts: &[P2]) -> f32 {
    pts.windows(2).map(|w| w[0].dist(w[1])).sum()
}

/// Узнать фигуру; `None` — штрих не похож ни на что.
pub fn recognize(pts: &[P2], id: u64, color: Rgba, width: f32) -> Option<Shape> {
    if pts.len() < 2 {
        return None;
    }
    let len = path_len(pts);
    let bb = BBox::of_points(pts.iter().copied());
    let diag = (bb.w().powi(2) + bb.h().powi(2)).sqrt();
    if len < 12.0 || diag < 10.0 {
        return None;
    }
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    // Прямая: все точки близко к хорде.
    let chord = first.dist(last);
    if chord > 0.8 * len {
        let dev = pts.iter().map(|p| super::tools::dist_seg(*p, first, last)).fold(0.0, f32::max);
        if dev < (0.06 * chord).max(3.0) {
            return Some(Shape::line(id, first, last, color, width, false));
        }
    }
    // Замкнутая фигура: концы рядом.
    if first.dist(last) > 0.25 * diag.max(1.0) {
        return None;
    }
    let mut closed: Vec<P2> = pts.to_vec();
    closed.push(first);
    // Ошибка как эллипса: отклонение нормированного радиуса от 1.
    let c = bb.center();
    let (rx, ry) = (bb.w() / 2.0, bb.h() / 2.0);
    let ell_err = if rx > 1.0 && ry > 1.0 {
        let s: f32 = pts
            .iter()
            .map(|p| {
                let (dx, dy) = ((p.x - c.x) / rx, (p.y - c.y) / ry);
                ((dx * dx + dy * dy).sqrt() - 1.0).abs()
            })
            .sum();
        s / pts.len() as f32
    } else {
        f32::MAX
    };
    // Многоугольник: углы по упрощению контура.
    let mut corners = rdp(&closed, 0.08 * diag);
    if corners.len() > 1 && corners[0].dist(*corners.last().unwrap()) < 0.15 * diag {
        corners.pop();
    }
    let poly_err = if corners.len() >= 3 {
        let mut ring = corners.clone();
        ring.push(corners[0]);
        let s: f32 = pts
            .iter()
            .map(|p| ring.windows(2).map(|w| super::tools::dist_seg(*p, w[0], w[1])).fold(f32::MAX, f32::min))
            .sum();
        s / pts.len() as f32 / diag
    } else {
        f32::MAX
    };
    let polygon_ok = (corners.len() == 3 || corners.len() == 4) && poly_err < 0.035;
    if polygon_ok && (poly_err * 3.0 < ell_err || ell_err > 0.12) {
        if corners.len() == 3 {
            return Some(Shape { id, kind: ShapeKind::Triangle, pts: corners, color, width, fill: None });
        }
        // Почти прямоугольник по осям — ровный прямоугольник.
        let axis = corners.iter().zip(corners.iter().cycle().skip(1)).all(|(a, b)| {
            let d = b.sub(*a);
            let ang = d.y.atan2(d.x).abs() % std::f32::consts::FRAC_PI_2;
            ang < 0.2 || ang > std::f32::consts::FRAC_PI_2 - 0.2
        });
        if axis {
            let b = BBox::of_points(corners.iter().copied());
            return Some(Shape::rect(id, P2::new(b.x0, b.y0), P2::new(b.x1, b.y1), color, width));
        }
        return Some(Shape { id, kind: ShapeKind::Polygon, pts: corners, color, width, fill: None });
    }
    if ell_err < 0.12 {
        // Почти круг — круг.
        let (rx, ry) = if (rx - ry).abs() < 0.12 * rx.max(ry) {
            let r = (rx + ry) / 2.0;
            (r, r)
        } else {
            (rx, ry)
        };
        return Some(Shape::ellipse(id, P2::new(c.x - rx, c.y - ry), P2::new(c.x + rx, c.y + ry), color, width));
    }
    None
}
