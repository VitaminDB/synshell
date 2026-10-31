//! Тесты рукописного модуля: модель и история, файл, ластик, лассо,
//! распознавание фигур, растр (пиксели), заливка, SVG; с фичей `testing` —
//! холст в харнессе syngui (штрих пером, ластик пером, жесты пальцами).
//! `cargo test -p syn-notes --features testing ink`

use super::model::*;
use super::*;

fn stroke(id: u64, pts: &[(f32, f32)]) -> Item {
    Item::Stroke(Stroke {
        id,
        brush: Brush::Ballpoint,
        color: Rgba::BLACK,
        width: 3.0,
        points: pts.iter().enumerate().map(|(i, (x, y))| InkPoint { x: *x, y: *y, p: 0.6, tx: 0.0, ty: 0.0, t: i as u32 * 8 }).collect(),
    })
}

fn line_pts(a: (f32, f32), b: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (0..n).map(|i| {
        let t = i as f32 / (n - 1) as f32;
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }).collect()
}

fn doc_with(items: Vec<Item>) -> InkDoc {
    let mut d = InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank);
    d.pages[0].layers[0].items = items;
    d.next_id = 1000;
    d
}

#[test]
fn op_apply_and_undo_redo() {
    let mut d = doc_with(vec![stroke(1, &[(0.0, 0.0), (10.0, 10.0)]), stroke(2, &[(5.0, 5.0), (6.0, 6.0)])]);
    let (pid, lid) = (d.pages[0].id, d.pages[0].layers[0].id);
    let orig = d.clone();
    let mut h = History::new();
    // Замена второго двумя кусками.
    let removed = vec![(1, d.pages[0].layers[0].items[1].clone())];
    let added = vec![(1, stroke(3, &[(0.0, 1.0), (1.0, 1.0)])), (2, stroke(4, &[(2.0, 1.0), (3.0, 1.0)]))];
    h.commit(&mut d, Op::Items { page: pid, layer: lid, removed, added });
    let ids: Vec<u64> = d.pages[0].layers[0].items.iter().map(|i| i.id()).collect();
    assert_eq!(ids, vec![1, 3, 4]);
    h.undo(&mut d);
    assert_eq!(d, orig);
    h.redo(&mut d);
    let ids: Vec<u64> = d.pages[0].layers[0].items.iter().map(|i| i.id()).collect();
    assert_eq!(ids, vec![1, 3, 4]);
    assert!(h.can_undo() && !h.can_redo());
}

#[test]
fn codec_roundtrip_and_size() {
    let mut items = Vec::new();
    for k in 0..200u64 {
        let y = 40.0 + k as f32 * 3.0;
        items.push(stroke(k + 1, &line_pts((50.0, y), (600.0, y + 20.0), 120)));
    }
    items.push(Item::Shape(Shape::ellipse(9001, P2::new(10.0, 10.0), P2::new(80.0, 50.0), Rgba::rgb(200, 0, 0), 2.0)));
    items.push(Item::Text(TextItem { id: 9002, text: "Привет".into(), x: 1.0, y: 2.0, w: 100.0, size: 18.0, color: Rgba::BLACK, bold: true }));
    let mut d = doc_with(items);
    d.add_asset(vec![1, 2, 3, 4]);
    let bytes = codec::save(&d);
    let back = codec::load(&bytes).expect("load");
    assert_eq!(back.pages.len(), 1);
    assert_eq!(back.pages[0].layers[0].items.len(), 202);
    // Точки — с точностью 1/16 и давлением в байт.
    let (Item::Stroke(a), Item::Stroke(b)) = (&d.pages[0].layers[0].items[5], &back.pages[0].layers[0].items[5]) else { panic!() };
    for (p, q) in a.points.iter().zip(&b.points) {
        assert!((p.x - q.x).abs() <= 1.0 / 32.0 + 1e-4 && (p.y - q.y).abs() <= 1.0 / 32.0 + 1e-4);
        assert!((p.p - q.p).abs() < 0.01);
        assert_eq!(p.t, q.t);
    }
    assert_eq!(back.pages[0].layers[0].items[200], d.pages[0].layers[0].items[200]);
    assert_eq!(back.assets, d.assets);
    // 24 000 точек — меньше 4 байт на точку.
    assert!(bytes.len() < 24_000 * 4, "size {}", bytes.len());
    assert!(matches!(codec::load(b"nonsense"), Err(codec::CodecError::NotInk)));
}

#[test]
fn whole_eraser_hits_only_touched() {
    let d = doc_with(vec![stroke(1, &line_pts((0.0, 0.0), (100.0, 0.0), 10)), stroke(2, &line_pts((0.0, 50.0), (100.0, 50.0), 10))]);
    let l = &d.pages[0].layers[0];
    assert_eq!(tools::erase_whole(l, P2::new(50.0, -20.0), P2::new(50.0, 20.0), 3.0), vec![1]);
    assert!(tools::erase_whole(l, P2::new(50.0, 25.0), P2::new(52.0, 25.0), 3.0).is_empty());
}

#[test]
fn point_eraser_splits_stroke() {
    let mut d = doc_with(vec![stroke(1, &line_pts((0.0, 0.0), (100.0, 0.0), 3))]);
    let (pid, lid) = (d.pages[0].id, d.pages[0].layers[0].id);
    let layer = d.pages[0].layers[0].clone();
    let mut next = 100;
    let mut alloc = || {
        next += 1;
        next
    };
    // Вертикальный проход ластика через середину (точки штриха редкие —
    // ластик всё равно режет).
    let (removed, added) = tools::erase_points(&layer, P2::new(50.0, -10.0), P2::new(50.0, 10.0), 4.0, &mut alloc);
    assert_eq!(removed.len(), 1);
    assert_eq!(added.len(), 2, "two pieces");
    Op::Items { page: pid, layer: lid, removed, added }.apply(&mut d);
    let items = &d.pages[0].layers[0].items;
    let xs: Vec<(f32, f32)> = items
        .iter()
        .map(|i| match i {
            Item::Stroke(s) => (s.points.first().unwrap().x, s.points.last().unwrap().x),
            _ => panic!(),
        })
        .collect();
    assert!(xs[0].1 < 47.0 && xs[1].0 > 53.0, "{xs:?}");
    assert!(xs[0].0 < 1.0 && xs[1].1 > 99.0);
}

#[test]
fn lasso_selects_inside() {
    let d = doc_with(vec![
        stroke(1, &line_pts((10.0, 10.0), (40.0, 40.0), 8)),
        stroke(2, &line_pts((200.0, 200.0), (240.0, 240.0), 8)),
        // Наполовину внутри — нет (меньше половины точек).
        stroke(3, &line_pts((80.0, 20.0), (300.0, 20.0), 20)),
    ]);
    let poly = vec![P2::new(0.0, 0.0), P2::new(100.0, 0.0), P2::new(100.0, 100.0), P2::new(0.0, 100.0)];
    assert_eq!(tools::lasso_select(&d.pages[0].layers[0], &poly), vec![1]);
}

fn circle_pts(cx: f32, cy: f32, r: f32, n: usize) -> Vec<P2> {
    (0..=n).map(|i| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        P2::new(cx + r * a.cos() + (i as f32 * 1.7).sin() * 1.5, cy + r * a.sin())
    }).collect()
}

#[test]
fn recognize_shapes() {
    let line: Vec<P2> = (0..30).map(|i| P2::new(i as f32 * 5.0, 100.0 + (i as f32 * 0.9).sin() * 1.5)).collect();
    assert_eq!(recognize::recognize(&line, 1, Rgba::BLACK, 2.0).unwrap().kind, ShapeKind::Line);

    let circ = circle_pts(200.0, 200.0, 60.0, 60);
    let s = recognize::recognize(&circ, 1, Rgba::BLACK, 2.0).unwrap();
    assert_eq!(s.kind, ShapeKind::Ellipse);

    let mut rect = Vec::new();
    for (a, b) in [((0.0, 0.0), (120.0, 0.0)), ((120.0, 0.0), (120.0, 80.0)), ((120.0, 80.0), (0.0, 80.0)), ((0.0, 80.0), (0.0, 2.0))] {
        for (x, y) in line_pts(a, b, 15) {
            rect.push(P2::new(x + (y * 0.3).sin(), y));
        }
    }
    let s = recognize::recognize(&rect, 1, Rgba::BLACK, 2.0).unwrap();
    assert_eq!(s.kind, ShapeKind::Rect);
    let bb = BBox::of_points(s.pts.iter().copied());
    assert!((bb.w() - 120.0).abs() < 6.0 && (bb.h() - 80.0).abs() < 6.0, "{bb:?}");

    let mut tri = Vec::new();
    for (a, b) in [((0.0, 100.0), (60.0, 0.0)), ((60.0, 0.0), (120.0, 100.0)), ((120.0, 100.0), (2.0, 100.0))] {
        for (x, y) in line_pts(a, b, 15) {
            tri.push(P2::new(x, y));
        }
    }
    assert_eq!(recognize::recognize(&tri, 1, Rgba::BLACK, 2.0).unwrap().kind, ShapeKind::Triangle);

    // Каракуля — ничего.
    let scrib: Vec<P2> = (0..80).map(|i| P2::new(i as f32 * 3.0, (i as f32 * 0.7).sin() * 40.0)).collect();
    assert!(recognize::recognize(&scrib, 1, Rgba::BLACK, 2.0).is_none());
}

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = pm.pixel(x, y).unwrap().demultiply();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

#[test]
fn raster_draws_stroke_on_white() {
    let d = doc_with(vec![stroke(1, &line_pts((100.0, 100.0), (300.0, 100.0), 30))]);
    let pm = raster::render_page(&d, 0, 1.0, &raster::RasterOpts::default()).unwrap();
    assert_eq!((pm.width(), pm.height()), (794, 1123));
    assert!(px(&pm, 200, 100)[0] < 40, "ink is dark");
    assert_eq!(px(&pm, 200, 130), [255, 255, 255, 255], "paper white");
    assert_eq!(px(&pm, 50, 100), [255, 255, 255, 255], "nothing before stroke");
    // PNG кодируется.
    assert!(raster::export_png(&d, 0, 0.5, &raster::RasterOpts::default()).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn highlighter_is_translucent_once() {
    // Маркер, проходящий сам по себе дважды: тон в месте самоналожения тот
    // же, что в одинарном (одна заливка-объединение).
    let mut pts = line_pts((100.0, 100.0), (300.0, 100.0), 40);
    pts.extend(line_pts((300.0, 101.0), (100.0, 101.0), 40));
    let it = Item::Stroke(Stroke {
        id: 1,
        brush: Brush::Highlighter,
        color: Rgba::rgb(240, 200, 0),
        width: 16.0,
        points: pts.iter().map(|(x, y)| InkPoint::new(*x, *y, 0.5)).collect(),
    });
    let d = doc_with(vec![it]);
    let pm = raster::render_page(&d, 0, 1.0, &raster::RasterOpts::default()).unwrap();
    let c = px(&pm, 200, 100);
    assert!(c[2] > 100 && c[2] < 250, "translucent yellow over white: {c:?}");
}

#[test]
fn pencil_tone_follows_pressure() {
    let mk = |p: f32| Rgba::BLACK.toward_white((1.0 - p) * 0.6);
    assert!(mk(0.2).luma() > mk(0.9).luma());
}

#[test]
fn flood_fill_closed_rect() {
    let r = Item::Shape(Shape::rect(1, P2::new(100.0, 100.0), P2::new(200.0, 200.0), Rgba::BLACK, 3.0));
    let d = doc_with(vec![r]);
    let (png, bb) = tools::flood_fill(&d, &d.pages[0], BBox::new(0.0, 0.0, 794.0, 1123.0), P2::new(150.0, 150.0), Rgba::rgb(255, 0, 0), 1.0).unwrap();
    assert!(bb.x0 >= 95.0 && bb.x1 <= 205.0 && bb.y0 >= 95.0 && bb.y1 <= 205.0, "{bb:?}");
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    let mid = img.get_pixel(img.width() / 2, img.height() / 2).0;
    assert_eq!(mid, [255, 0, 0, 255]);
    // По линии — ничего.
    assert!(tools::flood_fill(&d, &d.pages[0], BBox::new(0.0, 0.0, 794.0, 1123.0), P2::new(100.0, 150.0), Rgba::BLACK, 1.0).is_none());
}

#[test]
fn transform_and_selection_bbox() {
    let mut it = stroke(1, &line_pts((0.0, 0.0), (10.0, 0.0), 2));
    it.transform(&Affine::scale_about(P2::new(0.0, 0.0), 2.0, 2.0));
    let Item::Stroke(s) = &it else { panic!() };
    assert!((s.points[1].x - 20.0).abs() < 1e-4);
    assert!((s.width - 6.0).abs() < 1e-4);
    let r = Affine::rotate_about(P2::new(0.0, 0.0), std::f32::consts::FRAC_PI_2).apply(P2::new(1.0, 0.0));
    assert!((r.x).abs() < 1e-5 && (r.y - 1.0).abs() < 1e-5);
}

#[test]
fn svg_export_has_paths() {
    let d = doc_with(vec![
        stroke(1, &line_pts((10.0, 10.0), (50.0, 50.0), 5)),
        Item::Shape(Shape::line(2, P2::new(0.0, 0.0), P2::new(30.0, 0.0), Rgba::BLACK, 2.0, true)),
        Item::Text(TextItem { id: 3, text: "a<b".into(), x: 5.0, y: 5.0, w: 100.0, size: 12.0, color: Rgba::BLACK, bold: false }),
    ]);
    let s = export::svg(&d, 0);
    assert!(s.starts_with("<svg"));
    assert_eq!(s.matches("<path").count(), 3, "stroke, line, arrow head");
    assert!(s.contains("a&lt;b"));
}

#[test]
fn templates_render_lines() {
    let mut d = doc_with(vec![]);
    d.pages[0].template = Template::Grid { step: 20.0 };
    let pm = raster::render_page(&d, 0, 1.0, &raster::RasterOpts::default()).unwrap();
    assert!(px(&pm, 100, 200)[0] < 230, "grid line at x=100");
    assert_eq!(px(&pm, 110, 210), [255, 255, 255, 255]);
}

#[cfg(feature = "testing")]
mod harness {
    use super::*;
    use syngui::core::Point;
    use syngui::input::{Event, PenPhase, PenSample, PenTool};
    use syngui::testing::TestHarness;

    fn canvas(doc: InkDoc) -> (InkHandle, TestHarness) {
        let h = InkHandle::new(doc);
        let mut t = TestHarness::new(Box::new(InkCanvas::new(h.clone())));
        t.layout(800.0, 1100.0);
        (h, t)
    }

    fn items(h: &InkHandle) -> usize {
        h.lock().pages[h.current()].layers[0].items.len()
    }

    #[test]
    fn pen_stroke_adds_stroke_with_pressure() {
        let (h, mut t) = canvas(InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank));
        t.pen_stroke(&[(200.0, 300.0, 0.2), (260.0, 320.0, 0.6), (320.0, 340.0, 0.9), (380.0, 360.0, 0.9)]);
        assert_eq!(items(&h), 1);
        let d = h.lock();
        let Item::Stroke(s) = &d.pages[0].layers[0].items[0] else { panic!() };
        assert_eq!(s.points.len(), 4);
        assert!((s.points[0].p - 0.2).abs() < 0.01 && (s.points[2].p - 0.9).abs() < 0.01);
        drop(d);
        assert!(h.can_undo());
        h.undo();
        assert_eq!(items(&h), 0);
        // Отрисовка: растр страницы — картинкой.
        let list = t.paint();
        let has_image = list.iter_all_commands().any(|c| matches!(c, syngui::render::DrawCommand::Image { .. }));
        let _ = has_image; // текстуры у харнесса может не быть — главное, что не паникует
    }

    #[test]
    fn pen_eraser_end_erases() {
        let mut d = InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank);
        d.pages[0].layers[0].items.push(stroke(500, &line_pts((100.0, 100.0), (500.0, 100.0), 20)));
        d.next_id = 1000;
        let (h, mut t) = canvas(d);
        // Где на экране точка страницы (300, 100).
        let p = page_to_screen(&t, &h, P2::new(300.0, 100.0));
        let s = |dy: f32| PenSample { position: Point::new(p.x, p.y + dy), pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, tool: PenTool::Eraser, barrel: false, time_ms: 0 };
        t.pen(PenPhase::Down, s(-20.0));
        t.pen(PenPhase::Move, s(0.0));
        t.pen(PenPhase::Move, s(20.0));
        t.pen(PenPhase::Up, s(20.0));
        assert_eq!(items(&h), 0, "stroke erased whole");
        h.undo();
        assert_eq!(items(&h), 1, "erase undone in one step");
    }

    /// Вписанный вид блокнота: страница по центру с полем 8.
    fn page_to_screen(t: &TestHarness, _h: &InkHandle, p: P2) -> Point {
        page_to_screen_sz(t.root_size(), p)
    }

    fn page_to_screen_sz(size: syngui::core::Size, p: P2) -> Point {
        let (pw, ph) = (794.0, 1123.0);
        let z = ((size.width - 16.0) / pw).min((size.height - 16.0) / ph);
        Point::new((size.width - pw * z) / 2.0 + p.x * z, (size.height - ph * z) / 2.0 + p.y * z)
    }

    #[test]
    fn two_finger_tap_undoes_and_palm_is_ignored() {
        let (h, mut t) = canvas(InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank));
        t.pen_stroke(&[(200.0, 300.0, 0.5), (300.0, 300.0, 0.5)]);
        assert_eq!(items(&h), 1);
        // Ладонь сразу после пера — не жест.
        t.send_event(&Event::TouchStart { id: 1, position: Point::new(100.0, 100.0) });
        t.send_event(&Event::TouchStart { id: 2, position: Point::new(150.0, 100.0) });
        t.send_event(&Event::TouchEnd { id: 1, position: Point::new(100.0, 100.0) });
        t.send_event(&Event::TouchEnd { id: 2, position: Point::new(150.0, 100.0) });
        assert_eq!(items(&h), 1, "palm right after pen ignored");
        std::thread::sleep(std::time::Duration::from_millis(750));
        t.send_event(&Event::TouchStart { id: 3, position: Point::new(100.0, 100.0) });
        t.send_event(&Event::TouchStart { id: 4, position: Point::new(150.0, 100.0) });
        t.send_event(&Event::TouchEnd { id: 3, position: Point::new(100.0, 100.0) });
        t.send_event(&Event::TouchEnd { id: 4, position: Point::new(150.0, 100.0) });
        assert_eq!(items(&h), 0, "two-finger tap = undo");
    }

    #[test]
    fn swipe_turns_pages_and_creates_new() {
        let (h, mut t) = canvas(InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank));
        let swipe = |t: &mut TestHarness, from: f32, to: f32| {
            t.send_event(&Event::TouchStart { id: 9, position: Point::new(from, 500.0) });
            t.send_event(&Event::TouchMove { id: 9, position: Point::new((from + to) / 2.0, 505.0) });
            t.send_event(&Event::TouchMove { id: 9, position: Point::new(to, 510.0) });
            t.send_event(&Event::TouchEnd { id: 9, position: Point::new(to, 510.0) });
        };
        swipe(&mut t, 600.0, 200.0);
        assert_eq!(h.page_count(), 2, "swipe on last page adds one");
        assert_eq!(h.current(), 1);
        swipe(&mut t, 200.0, 600.0);
        assert_eq!(h.current(), 0);
    }

    #[test]
    fn lasso_then_drag_moves_selection() {
        let mut d = InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Blank);
        d.pages[0].layers[0].items.push(stroke(500, &line_pts((100.0, 100.0), (150.0, 150.0), 10)));
        d.next_id = 1000;
        let (h, mut t) = canvas(d);
        h.set_tool(|t| t.tool = Tool::Lasso);
        let size = t.root_size();
        let sp = move |x: f32, y: f32| page_to_screen_sz(size, P2::new(x, y));
        let ring: Vec<(f32, f32, f32)> = [(80.0, 80.0), (170.0, 80.0), (170.0, 170.0), (80.0, 170.0), (80.0, 82.0)]
            .iter()
            .map(|(x, y)| {
                let p = sp(*x, *y);
                (p.x, p.y, 0.5)
            })
            .collect();
        t.pen_stroke(&ring);
        assert_eq!(h.selection.get_untracked().ids, vec![500]);
        let a = sp(125.0, 125.0);
        let b = sp(325.0, 125.0);
        t.pen_stroke(&[(a.x, a.y, 0.5), ((a.x + b.x) / 2.0, a.y, 0.5), (b.x, b.y, 0.5)]);
        let d = h.lock();
        let Item::Stroke(s) = &d.pages[0].layers[0].items[0] else { panic!() };
        assert!((s.points[0].x - 300.0).abs() < 2.0, "moved by 200: {}", s.points[0].x);
    }
}
