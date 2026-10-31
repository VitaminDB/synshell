//! Чистая геометрия инструментов (без интерфейса): ластик целыми штрихами
//! и точечный (режет штрих), выбор лассо, попадание, пипетка, заливка.

use super::model::{BBox, InkDoc, InkPage, InkPoint, Item, Layer, Rgba, Shape, ShapeKind, Stroke, P2};

/// Расстояние от точки до отрезка.
pub fn dist_seg(p: P2, a: P2, b: P2) -> f32 {
    let ab = b.sub(a);
    let l2 = ab.x * ab.x + ab.y * ab.y;
    if l2 < 1e-9 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / l2).clamp(0.0, 1.0);
    p.dist(a.lerp(b, t))
}

/// Наименьшее расстояние между отрезками (пересечение — 0).
pub fn seg_seg(a: P2, b: P2, c: P2, d: P2) -> f32 {
    if segments_cross(a, b, c, d) {
        return 0.0;
    }
    dist_seg(a, c, d).min(dist_seg(b, c, d)).min(dist_seg(c, a, b)).min(dist_seg(d, a, b))
}

fn cross(o: P2, a: P2, b: P2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

pub fn segments_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Точка внутри многоугольника (чёт-нечет).
pub fn inside(poly: &[P2], p: P2) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut c = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
        j = i;
    }
    c
}

/// Ломаная элемента для попадания: штрих — точки, фигура — контур
/// (замкнутая — с замыкающей точкой), картинка/текст — рамка.
fn item_path(it: &Item) -> (Vec<P2>, f32) {
    match it {
        Item::Stroke(s) => (s.points.iter().map(|p| p.pos()).collect(), s.width / 2.0),
        Item::Shape(s) => {
            let mut o = s.outline();
            if s.kind.closed() {
                if let Some(f) = o.first().copied() {
                    o.push(f);
                }
            }
            (o, s.width / 2.0)
        }
        Item::Image(i) => {
            let c = i.corners();
            (vec![c[0], c[1], c[2], c[3], c[0]], 0.0)
        }
        Item::Text(t) => {
            let b = t.bbox();
            (vec![P2::new(b.x0, b.y0), P2::new(b.x1, b.y0), P2::new(b.x1, b.y1), P2::new(b.x0, b.y1), P2::new(b.x0, b.y0)], 0.0)
        }
    }
}

/// Задевает ли отрезок ластика `a→b` радиуса `r` элемент.
pub fn hits(it: &Item, a: P2, b: P2, r: f32) -> bool {
    let bb = BBox::new(a.x, a.y, b.x, b.y).inflate(r);
    if !it.bbox().intersects(&bb) {
        return false;
    }
    // Картинки и текст стираются касанием рамки или изнутри.
    let filled = match it {
        Item::Image(i) => Some(i.corners().to_vec()),
        Item::Text(t) => {
            let bx = t.bbox();
            Some(vec![P2::new(bx.x0, bx.y0), P2::new(bx.x1, bx.y0), P2::new(bx.x1, bx.y1), P2::new(bx.x0, bx.y1)])
        }
        Item::Shape(s) if s.fill.is_some() && s.kind.closed() => Some(s.outline()),
        _ => None,
    };
    if let Some(poly) = filled {
        if inside(&poly, a) || inside(&poly, b) {
            return true;
        }
    }
    let (path, hw) = item_path(it);
    if path.len() == 1 {
        return dist_seg(path[0], a, b) <= r + hw;
    }
    path.windows(2).any(|w| seg_seg(w[0], w[1], a, b) <= r + hw)
}

/// Ластик целыми элементами: id задетых в незаблокированном слое.
pub fn erase_whole(layer: &Layer, a: P2, b: P2, r: f32) -> Vec<u64> {
    if layer.locked || !layer.visible {
        return Vec::new();
    }
    layer.items.iter().filter(|i| hits(i, a, b, r)).map(|i| i.id()).collect()
}

/// Сгустить точки штриха, чтобы шаг был не больше `step` (иначе быстрый
/// штрих с редкими точками ластик «перепрыгивает»).
fn densify(points: &[InkPoint], step: f32) -> Vec<InkPoint> {
    let mut out = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            let q = points[i - 1];
            let d = q.pos().dist(p.pos());
            let n = (d / step).floor() as usize;
            for k in 1..=n {
                let t = k as f32 / (n + 1) as f32;
                out.push(InkPoint {
                    x: q.x + (p.x - q.x) * t,
                    y: q.y + (p.y - q.y) * t,
                    p: q.p + (p.p - q.p) * t,
                    tx: q.tx + (p.tx - q.tx) * t,
                    ty: q.ty + (p.ty - q.ty) * t,
                    t: q.t + ((p.t as f32 - q.t as f32) * t) as u32,
                });
            }
        }
        out.push(*p);
    }
    out
}

/// Точечный ластик по штриху: куски, что остались (`None` — штрих не
/// задет). Пустой вектор — стёрт целиком.
pub fn cut_points(points: &[InkPoint], width: f32, a: P2, b: P2, r: f32) -> Option<Vec<Vec<InkPoint>>> {
    let reach = r + width * 0.35;
    let bb = BBox::new(a.x, a.y, b.x, b.y).inflate(reach);
    if !BBox::of_points(points.iter().map(|p| p.pos())).inflate(width).intersects(&bb) {
        return None;
    }
    let dense = densify(points, (r * 0.5).max(0.5));
    let hit: Vec<bool> = dense.iter().map(|p| dist_seg(p.pos(), a, b) <= reach).collect();
    if !hit.iter().any(|h| *h) {
        return None;
    }
    let mut pieces = Vec::new();
    let mut cur: Vec<InkPoint> = Vec::new();
    for (p, h) in dense.iter().zip(&hit) {
        if *h {
            if cur.len() >= 2 {
                pieces.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        } else {
            cur.push(*p);
        }
    }
    if cur.len() >= 2 {
        pieces.push(cur);
    }
    Some(pieces)
}

/// Точечный ластик по слою: (что убрать, что добавить на их место).
/// Новые куски получают id из `alloc`.
pub fn erase_points(layer: &Layer, a: P2, b: P2, r: f32, alloc: &mut dyn FnMut() -> u64) -> (Vec<(usize, Item)>, Vec<(usize, Item)>) {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    if layer.locked || !layer.visible {
        return (removed, added);
    }
    let mut shift: isize = 0;
    for (idx, it) in layer.items.iter().enumerate() {
        let pieces: Option<Vec<Item>> = match it {
            Item::Stroke(s) => cut_points(&s.points, s.width, a, b, r).map(|ps| {
                ps.into_iter().map(|pts| Item::Stroke(Stroke { id: alloc(), points: pts, ..s.clone() })).collect()
            }),
            Item::Shape(s) if s.fill.is_none() => {
                if !hits(it, a, b, r) {
                    None
                } else {
                    // Контур фигуры режем как штрих и оставляем ломаными.
                    let mut o = s.outline();
                    if s.kind.closed() {
                        if let Some(f) = o.first().copied() {
                            o.push(f);
                        }
                    }
                    let pts: Vec<InkPoint> = o.iter().map(|p| InkPoint::new(p.x, p.y, 0.5)).collect();
                    cut_points(&pts, s.width, a, b, r).map(|ps| {
                        ps.into_iter()
                            .map(|pp| {
                                Item::Shape(Shape {
                                    id: alloc(),
                                    kind: ShapeKind::Polyline,
                                    pts: pp.iter().map(|p| p.pos()).collect(),
                                    color: s.color,
                                    width: s.width,
                                    fill: None,
                                })
                            })
                            .collect()
                    })
                }
            }
            _ => hits(it, a, b, r).then(Vec::new),
        };
        if let Some(pieces) = pieces {
            let at = (idx as isize + shift) as usize;
            removed.push((idx, it.clone()));
            let n = pieces.len();
            for (k, p) in pieces.into_iter().enumerate() {
                added.push((at + k, p));
            }
            shift += n as isize - 1;
        }
    }
    (removed, added)
}

/// Что выбрано лассо в слое: штрих/фигура — если внутри больше половины
/// точек, картинка/текст — если внутри центр.
pub fn lasso_select(layer: &Layer, poly: &[P2]) -> Vec<u64> {
    if poly.len() < 3 || layer.locked || !layer.visible {
        return Vec::new();
    }
    let pb = BBox::of_points(poly.iter().copied());
    layer
        .items
        .iter()
        .filter(|it| it.bbox().intersects(&pb))
        .filter(|it| {
            let pts: Vec<P2> = match it {
                Item::Stroke(s) => s.points.iter().map(|p| p.pos()).collect(),
                Item::Shape(s) => s.outline(),
                Item::Image(i) => vec![i.bbox().center()],
                Item::Text(t) => vec![t.bbox().center()],
            };
            if pts.is_empty() {
                return false;
            }
            let n_in = pts.iter().filter(|p| inside(poly, **p)).count();
            n_in * 2 >= pts.len().max(1)
        })
        .map(|it| it.id())
        .collect()
}

/// Самый верхний элемент видимых слоёв под точкой.
pub fn item_at(page: &InkPage, p: P2, tol: f32) -> Option<(usize, u64)> {
    for (li, l) in page.layers.iter().enumerate().rev() {
        if !l.visible {
            continue;
        }
        for it in l.items.iter().rev() {
            if hits(it, p, p, tol) {
                return Some((li, it.id()));
            }
        }
    }
    None
}

/// Пипетка: цвет верхнего элемента под точкой.
pub fn pick_color(page: &InkPage, p: P2, tol: f32) -> Option<Rgba> {
    let (li, id) = item_at(page, p, tol)?;
    let l = &page.layers[li];
    let it = &l.items[l.find(id)?];
    match it {
        Item::Shape(s) if s.fill.is_some() && super::tools::inside(&s.outline(), p) => s.fill,
        _ => it.color().map(|c| c.with_alpha(255)),
    }
}

/// Заливка замкнутой области от точки `seed` в границах `region` (единицы
/// страницы): растр штрихов → обход в ширину по пустым пикселям →
/// картинка-заливка (PNG, прозрачная вне области). `None` — щелчок по
/// линии.
pub fn flood_fill(doc: &InkDoc, page: &InkPage, region: BBox, seed: P2, color: Rgba, scale: f32) -> Option<(Vec<u8>, BBox)> {
    let pm = super::raster::render_ink_only(doc, page, region, scale)?;
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let sx = ((seed.x - region.x0) * scale) as i32;
    let sy = ((seed.y - region.y0) * scale) as i32;
    if sx < 0 || sy < 0 || sx >= w || sy >= h {
        return None;
    }
    let data = pm.data();
    let wall = |x: i32, y: i32| data[((y * w + x) * 4 + 3) as usize] > 100;
    if wall(sx, sy) {
        return None;
    }
    let mut mask = vec![false; (w * h) as usize];
    let mut stack = vec![(sx, sy)];
    mask[(sy * w + sx) as usize] = true;
    let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx, sy);
    while let Some((x, y)) = stack.pop() {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let k = (ny * w + nx) as usize;
            if !mask[k] && !wall(nx, ny) {
                mask[k] = true;
                stack.push((nx, ny));
            }
        }
    }
    // Расширить на 1 px — под сглаженный край линии, без светлой щели.
    let (x0, y0) = ((x0 - 1).max(0), (y0 - 1).max(0));
    let (x1, y1) = ((x1 + 1).min(w - 1), (y1 + 1).min(h - 1));
    let (cw, ch) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let mut img = image::RgbaImage::new(cw, ch);
    let filled = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && mask[(y * w + x) as usize];
    for y in y0..=y1 {
        for x in x0..=x1 {
            let on = filled(x, y) || filled(x + 1, y) || filled(x - 1, y) || filled(x, y + 1) || filled(x, y - 1);
            if on {
                img.put_pixel((x - x0) as u32, (y - y0) as u32, image::Rgba(color.0));
            }
        }
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    let bb = BBox::new(
        region.x0 + x0 as f32 / scale,
        region.y0 + y0 as f32 / scale,
        region.x0 + (x1 + 1) as f32 / scale,
        region.y0 + (y1 + 1) as f32 / scale,
    );
    Some((png, bb))
}
