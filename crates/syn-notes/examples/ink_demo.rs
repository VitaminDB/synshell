//! Рукописный холст отдельно от проекта — ручная проверка и снимки:
//!
//! ```text
//! cargo run -p syn-notes --example ink_demo --profile fast-release -- [ФАЙЛ.ink] [--drawing]
//! cargo run -p syn-notes --example ink_demo -- --screenshot out.png [--size 900x1200] [--drawing] [--sample] [--pop pen|eraser|shape|color|layers]
//! ```
//! Файл сохраняется при каждой правке (если задан). `--sample` — нарисовать
//! пример (штрихи всеми кистями, фигуры, текст) для снимка.

use std::sync::Arc;

use syn_notes::notes::ink::model::*;
use syn_notes::notes::ink::{self, views, InkHandle};
use syngui::prelude::*;

fn sample(h: &InkHandle) {
    let mut y = 160.0;
    for (k, b) in Brush::ALL.iter().enumerate() {
        let pts: Vec<InkPoint> = (0..80)
            .map(|i| {
                let t = i as f32 / 79.0;
                let x = 120.0 + t * 520.0;
                InkPoint { x, y: y + (t * 12.0).sin() * 18.0, p: 0.2 + 0.8 * (t * 3.1).sin().abs(), tx: 0.0, ty: 0.0, t: i * 8 }
            })
            .collect();
        let color = if *b == Brush::Highlighter { Rgba::rgb(240, 200, 0) } else { ink::PALETTE[[0, 1, 0, 6, 9, 0][k]] };
        h.add_item(Item::Stroke(Stroke { id: 0, brush: *b, color, width: b.widths()[2], points: pts }));
        y += 80.0;
    }
    h.add_item(Item::Shape(Shape::rect(0, P2::new(120.0, 680.0), P2::new(300.0, 800.0), Rgba::BLACK, 2.8)));
    h.add_item(Item::Shape(Shape::ellipse(0, P2::new(340.0, 680.0), P2::new(520.0, 800.0), Rgba::rgb(200, 30, 40), 2.8)));
    h.add_item(Item::Shape(Shape::line(0, P2::new(560.0, 800.0), P2::new(680.0, 680.0), Rgba::BLACK, 2.8, true)));
    h.add_item(Item::Text(TextItem { id: 0, text: "Рукописные заметки — пример".into(), x: 120.0, y: 70.0, w: 560.0, size: 26.0, color: Rgba::BLACK, bold: true }));
}

fn main() {
    let mut file = None;
    let mut shot = None;
    let mut size = (900u32, 1200u32);
    let mut drawing = false;
    let mut with_sample = false;
    let mut pop: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => shot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x').map(|(w, h)| (w.parse().unwrap_or(900), h.parse().unwrap_or(1200)))) {
                    size = (w, h);
                }
            }
            "--drawing" => drawing = true,
            "--sample" => with_sample = true,
            "--pop" => pop = it.next(),
            _ => file = Some(a),
        }
    }
    let _ = pop;
    synshell_common::i18n::init(&syn_notes::i18n::CATALOGS);
    let (cfg, _) = synshell_common::Config::load();
    let mss = format!("{}\n{}", syn_notes::app::theme::mss(&cfg), ink::INK_MSS);
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();
    let doc = file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| ink::codec::load(&b).ok())
        .unwrap_or_else(|| if drawing { InkDoc::drawing() } else { InkDoc::notebook() });
    let h = InkHandle::new(doc);
    if with_sample {
        sample(&h);
    }
    if let Some(f) = file.clone() {
        h.on_change(move |h| {
            let _ = std::fs::write(&f, h.save_bytes());
        });
    }
    let build = {
        let h = h.clone();
        move || -> Box<dyn Widget> {
            if h.kind() == DocKind::Drawing {
                views::drawing_view(h.clone())
            } else {
                views::notebook_view(h.clone())
            }
        }
    };
    if let Some(out) = shot {
        let opts = syngui::shot::ShotOptions { settle: std::time::Duration::from_millis(300), ..Default::default() };
        match syngui::shot::render_png_with(move || build(), &mss, size, 1.0, &out, opts) {
            Ok((w, hh)) => println!("{out}: {w}×{hh}"),
            Err(e) => {
                eprintln!("снимок: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let theme = use_signal(mss);
    let build = Arc::new(build);
    App::new()
        .title("Рукописные заметки")
        .app_id("syn-notes-ink-demo")
        .size(size.0, size.1)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(theme)
        .run(move |_| build());
}
