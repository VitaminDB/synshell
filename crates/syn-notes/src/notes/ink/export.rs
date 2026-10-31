//! Экспорт страницы в SVG (векторный: штрих — один путь-объединение
//! отпечатков, фигуры — линии, картинки — встроенный PNG/JPEG, текст —
//! `<text>`). PNG — [`super::raster::export_png`].

use std::fmt::Write;

use super::brush;
use super::model::{Brush, InkDoc, Item, Rgba, ShapeKind, Template};
use super::raster::page_extent;

fn col(c: Rgba) -> String {
    format!("rgb({},{},{})", c.0[0], c.0[1], c.0[2])
}

fn opa(c: Rgba) -> String {
    if c.0[3] == 255 {
        String::new()
    } else {
        format!(" fill-opacity=\"{:.3}\"", c.alpha())
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else {
        "image/png"
    }
}

/// SVG страницы `page`.
pub fn svg(doc: &InkDoc, page: usize) -> String {
    let Some(pg) = doc.pages.get(page) else { return String::new() };
    let ext = page_extent(pg);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{:.1} {:.1} {:.1} {:.1}\" width=\"{:.0}\" height=\"{:.0}\">",
        ext.x0,
        ext.y0,
        ext.w(),
        ext.h(),
        ext.w(),
        ext.h()
    );
    let paper = pg.paper.unwrap_or(Rgba::WHITE);
    let _ = writeln!(s, "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{}\"/>", ext.x0, ext.y0, ext.w(), ext.h(), col(paper));
    if let Template::Image { asset } = &pg.template {
        if let Some(a) = doc.asset(asset) {
            let _ = writeln!(s, "<image x=\"0\" y=\"0\" width=\"{:.1}\" height=\"{:.1}\" preserveAspectRatio=\"none\" href=\"data:{};base64,{}\"/>", ext.w(), ext.h(), mime(&a.bytes), b64(&a.bytes));
        }
    }
    for l in pg.layers.iter().filter(|l| l.visible) {
        let _ = writeln!(s, "<g opacity=\"{:.3}\">", l.opacity);
        // Выделители под остальным, как на экране.
        for pass in 0..2 {
            for it in &l.items {
                let hl = matches!(it, Item::Stroke(st) if st.brush == Brush::Highlighter);
                if (pass == 0) != hl {
                    continue;
                }
                item_svg(&mut s, doc, it);
            }
        }
        s.push_str("</g>\n");
    }
    s.push_str("</svg>\n");
    s
}

fn poly_d(pts: &[super::model::P2], close: bool) -> String {
    let mut d = String::new();
    for (i, p) in pts.iter().enumerate() {
        let _ = write!(d, "{}{:.2} {:.2} ", if i == 0 { "M" } else { "L" }, p.x, p.y);
    }
    if close {
        d.push('Z');
    }
    d
}

fn item_svg(s: &mut String, doc: &InkDoc, it: &Item) {
    match it {
        Item::Stroke(st) => {
            let dabs = brush::dabs(st);
            if dabs.is_empty() {
                return;
            }
            // Один путь-объединение (nonzero), цвет — первого отпечатка;
            // у карандаша — средний тон.
            let mut d = String::new();
            let n = dabs.len();
            for i in 0..n {
                d.push_str(&poly_d(&brush::circle(dabs[i].p, dabs[i].r, brush::circle_segments(dabs[i].r, 2.0)), true));
                if i + 1 < n {
                    if let Some(q) = brush::bridge(&dabs[i], &dabs[i + 1]) {
                        d.push_str(&poly_d(&q, true));
                    }
                }
            }
            let c = if st.brush == Brush::Pencil { dabs[n / 2].color } else { dabs[0].color };
            let _ = writeln!(s, "<path d=\"{d}\" fill=\"{}\"{} fill-rule=\"nonzero\"/>", col(c), opa(c));
        }
        Item::Shape(sh) => {
            let o = sh.outline();
            let closed = sh.kind.closed();
            let fill = match sh.fill.filter(|_| closed) {
                Some(f) => format!("fill=\"{}\"{}", col(f), opa(f)),
                None => "fill=\"none\"".into(),
            };
            let _ = writeln!(
                s,
                "<path d=\"{}\" {fill} stroke=\"{}\" stroke-width=\"{:.2}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                poly_d(&o, closed),
                col(sh.color),
                sh.width
            );
            if sh.kind == ShapeKind::Arrow && o.len() >= 2 {
                let h = super::raster::arrow_head(o[o.len() - 2], o[o.len() - 1], sh.width);
                let _ = writeln!(s, "<path d=\"{}\" fill=\"{}\"/>", poly_d(&h, true), col(sh.color));
            }
        }
        Item::Image(im) => {
            let Some(a) = doc.asset(&im.asset) else { return };
            let c = (im.x + im.w / 2.0, im.y + im.h / 2.0);
            let _ = writeln!(
                s,
                "<image x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" transform=\"rotate({:.3} {:.2} {:.2})\" preserveAspectRatio=\"none\" href=\"data:{};base64,{}\"/>",
                im.x,
                im.y,
                im.w,
                im.h,
                im.angle.to_degrees(),
                c.0,
                c.1,
                mime(&a.bytes),
                b64(&a.bytes)
            );
        }
        Item::Text(t) => {
            let mut y = t.y + t.size;
            for line in t.text.split('\n') {
                let _ = writeln!(
                    s,
                    "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{:.1}\" fill=\"{}\"{}>{}</text>",
                    t.x,
                    y,
                    t.size,
                    col(t.color),
                    if t.bold { " font-weight=\"bold\"" } else { "" },
                    esc(line)
                );
                y += t.size * 1.35;
            }
        }
    }
}
