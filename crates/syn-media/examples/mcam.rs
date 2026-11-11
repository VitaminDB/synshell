//! Проверка камер без службы: список, поток через [`syn_media::capture`], 3A, кадр в PNG.
//!
//! `[MCAM_CAM=id] cargo run --example mcam -- [кадров=90] [выход.png] [ширина высота] [video]`

use std::time::Instant;

use syn_media::capture::{self, CapEvent};
use syn_media::convert;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(90);
    let out = a.get(2).cloned().unwrap_or_else(|| "/tmp/mcam.png".into());
    let w: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1632);
    let h: u32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(1224);
    let video = a.get(5).is_some_and(|s| s == "video");
    let cams = syn_media::cameras();
    for c in &cams {
        let k = c.caps();
        let sizes: Vec<String> = k.sizes().iter().map(|s| format!("{}×{}", s.width, s.height)).collect();
        println!(
            "{} id={} front={} rot={} sizes=[{}] af={} iso={}..{} exp={}..{} нс",
            c.name(),
            c.id(),
            c.front(),
            c.sensor_rotation(),
            sizes.join(" "),
            k.af,
            k.iso_min,
            k.iso_max,
            k.exposure_min,
            k.exposure_max
        );
        if let syn_media::Cam::Mcam(d) = c {
            println!("  media {} драйвер {} сенсор {} линза {:?}", d.media.display(), d.driver, d.sensor_node.display(), d.lens_node);
        }
    }
    // MCAM_CAM=id — какую камеру открыть (по умолчанию — первую media controller)
    let want: Option<u32> = std::env::var("MCAM_CAM").ok().and_then(|s| s.parse().ok());
    let Some(cam) = cams.into_iter().find(|c| matches!(c, syn_media::Cam::Mcam(_)) && want.is_none_or(|w| c.id() == w)) else {
        eprintln!("нет камер media controller");
        std::process::exit(1);
    };
    let flags = if video { syn_media::syncamd::OPEN_VIDEO } else { 0 };
    let cap = match capture::open(&cam, w, h, 30, flags, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("открыть: {} (занята: {})", e.text, e.busy);
            std::process::exit(1);
        }
    };
    let (sw, sh) = cap.size();
    println!("поток {sw}×{sh}");
    let t0 = Instant::now();
    let mut frames = 0;
    let mut last = None;
    let mut prev_ts = 0u64;
    while frames < n {
        match cap.next() {
            Ok(CapEvent::Frame(f)) => {
                frames += 1;
                let ts = f.ts_ns();
                if frames % 15 == 0 {
                    println!("кадр {frames}: {:.1} к/с (интервал {:.1} мс)", frames as f64 / t0.elapsed().as_secs_f64(), (ts - prev_ts) as f64 / 1e6);
                }
                prev_ts = ts;
                last = Some(f);
            }
            Ok(CapEvent::Meta(m)) => {
                if frames % 15 < 6 {
                    println!("  3A: iso {} выдержка {:.2} мс af {} ae {} фокус {:.2}", m.iso, m.exposure_ns as f64 / 1e6, m.af, m.ae_state, m.focus);
                }
            }
            Ok(CapEvent::Stopped) => break,
            Err(e) => {
                eprintln!("поток: {e}");
                break;
            }
        }
    }
    println!("{frames} кадров за {:.2} с", t0.elapsed().as_secs_f64());
    if let Some(f) = last {
        let rot = cam.sensor_rotation();
        let mut img = None;
        f.with_src(&mut |src| {
            let (ow, oh) = convert::out_size(src.width, src.height, rot, 1);
            let mut rgba = vec![0u8; ow * oh * 4];
            convert::to_rgba(src, rot, false, 1, &mut rgba);
            img = image::RgbaImage::from_raw(ow as u32, oh as u32, rgba);
        });
        if let Some(i) = img {
            i.save(&out).unwrap();
            println!("кадр → {out}");
        }
    }
}
