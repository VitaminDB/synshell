//! syn-video-player — видеоплеер synshell: ffmpeg (демуксер, звук, программные кодеки) и виджет плеера
//! syngui. Видео декодируется аппаратно, где есть чем: на телефонах — V4L2-декодер SoC (`HwAccel::V4l2`,
//! Qualcomm `msm_vidc`), на компьютерах — VAAPI/NVDEC; иначе — программно.
//!
//!   syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [ФАЙЛ|URI]
//!   syn-video-player --bench [--hw …] ФАЙЛ   — декодировать полным ходом без окна, напечатать fps
//!
//! Без файла — список видео из «Видео» (XDG_VIDEOS_DIR).

mod bench;
mod library;
mod player;
mod settings;
mod ui;

use syngui::prelude::*;
use syngui::video::HwAccel;
use synshell_common::Config;

pub struct Args {
    pub file: Option<String>,
    /// `--hw`; без него — по настройкам программы.
    pub hw: Option<HwAccel>,
    pub bench: bool,
    /// `--bench --yuv`: кадры в YUV (перевод цвета на GPU), как при показе.
    pub yuv: bool,
}

fn parse_hw(s: &str) -> Option<HwAccel> {
    Some(match s {
        "auto" => HwAccel::Auto,
        "none" | "sw" => HwAccel::None,
        "v4l2" => HwAccel::V4l2,
        "vaapi" => HwAccel::Vaapi,
        "nvdec" => HwAccel::Nvdec,
        "vulkan" => HwAccel::Vulkan,
        _ => return None,
    })
}

/// `file:///…` из ярлыка (`%U`) — в путь; сетевые адреса ffmpeg открывает сам.
fn input_of(arg: &str) -> String {
    match arg.strip_prefix("file://") {
        Some(p) => percent_decode(p),
        None => arg.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_args() -> Args {
    let mut a = Args { file: None, hw: None, bench: false, yuv: false };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--bench" => a.bench = true,
            "--yuv" => a.yuv = true,
            "--hw" => {
                let v = it.next().unwrap_or_default();
                a.hw = Some(parse_hw(&v).unwrap_or_else(|| {
                    eprintln!("неизвестное ускорение «{v}» (auto|none|v4l2|vaapi|nvdec|vulkan)");
                    std::process::exit(2)
                }));
            }
            "-h" | "--help" => {
                println!("syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [--bench [--yuv]] [ФАЙЛ|URI]");
                std::process::exit(0)
            }
            _ if a.file.is_none() => a.file = Some(input_of(&s)),
            _ => {}
        }
    }
    a
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "syn_video_player=info,syngui::video=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    let args = parse_args();
    if args.bench {
        let Some(file) = args.file.as_deref() else {
            eprintln!("--bench: нужен файл");
            std::process::exit(2)
        };
        std::process::exit(bench::run(file, args.hw.unwrap_or(HwAccel::Auto), args.yuv));
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title("Видео")
        .app_id("syn-video-player")
        .size(960, 640)
        .min_size(320, 400)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = ui::St {
                hw: args.hw,
                settings: use_signal(settings::Settings::load()),
                info: use_signal(None),
                settings_open: use_signal(false),
                cards: use_signal(Vec::new()),
                scanning: use_signal(true),
                playing: use_signal(None),
                opening: use_signal(None),
                error: use_signal(None),
                fullscreen: use_signal(false),
                direct: args.file.is_some(),
            };
            match &args.file {
                Some(f) => ui::open(st, f.into()),
                None => ui::rescan(st),
            }
            ui::root(st)
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-video-player.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}
