//! syn-video-player — видеоплеер synshell: ffmpeg (демуксер, звук, программные кодеки) и виджет плеера
//! syngui. Видео декодируется аппаратно, где есть чем: на телефонах — V4L2-декодер SoC (`HwAccel::V4l2`,
//! Qualcomm `msm_vidc`), на компьютерах — VAAPI/NVDEC; иначе — программно.
//!
//!   syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [ФАЙЛ|URI]
//!   syn-video-player --bench [--hw …] [--dlss] ФАЙЛ   — декодировать полным ходом без окна, напечатать fps
//!                                                    (SVP_BENCH_FRAMES=N — только N кадров)
//!                                                    (`--dlss` — через DLSS 5, время модели)
//!
//! Без файла — список видео из «Видео» (XDG_VIDEOS_DIR).

mod bench;
mod edit;
mod editor;
mod library;
mod chain;
mod dlss;
mod filters;
mod interp;
mod neural;
mod player;
mod settings;
mod subs;
mod tracks;
mod ui;
mod vram;

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
    /// `--bench --dlss`: кадры через DLSS 5 (помощник должен быть запущен).
    pub dlss: bool,
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
    let mut a = Args { file: None, hw: None, bench: false, yuv: false, dlss: false };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--bench" => a.bench = true,
            "--yuv" => a.yuv = true,
            "--dlss" => a.dlss = true,
            "--hw" => {
                let v = it.next().unwrap_or_default();
                a.hw = Some(parse_hw(&v).unwrap_or_else(|| {
                    eprintln!("{}", t!("неизвестное ускорение «{v}» (auto|none|v4l2|vaapi|nvdec|vulkan)", v = v));
                    std::process::exit(2)
                }));
            }
            "-h" | "--help" => {
                println!("{}", t!("syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [--bench [--yuv] [--dlss]] [ФАЙЛ|URI]"));
                std::process::exit(0)
            }
            _ if a.file.is_none() => a.file = Some(input_of(&s)),
            _ => {}
        }
    }
    a
}

/// SIGTERM и SIGINT ловит отдельный поток (sigwait): до выхода вернуть играм настройки DLSS 5 в общей
/// памяти. Маска ставится до любых других потоков — они её наследуют.
fn catch_term() {
    // SAFETY: sigset заполняется sigemptyset/sigaddset, маска главного потока меняется до порождения
    // потоков; sigwait в своём потоке — обычный вызов, не обработчик сигнала.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        let set = set;
        std::thread::Builder::new()
            .name("svp-signals".into())
            .spawn(move || {
                let mut sig = 0;
                libc::sigwait(&set, &mut sig);
                dlss::shutdown();
                std::process::exit(128 + sig);
            })
            .ok();
    }
}

fn main() {
    catch_term();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "syn_video_player=info,syngui::video=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let args = parse_args();
    if args.bench {
        let Some(file) = args.file.as_deref() else {
            eprintln!("{}", t!("--bench: нужен файл"));
            std::process::exit(2)
        };
        std::process::exit(bench::run(file, args.hw.unwrap_or(HwAccel::Auto), args.yuv, args.dlss));
    }
    hdr_watch();
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title(t!("Видео"))
        .app_id("syn-video-player")
        .size(960, 640)
        .min_size(320, 400)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .on_close_request(|| {
            // настройки DLSS 5 в общей памяти — снова игровые
            dlss::shutdown();
            true
        })
        .run(move |_| {
            let subs = subs::SubSettings::load();
            let settings = settings::Settings::load();
            let st = ui::St {
                hw: args.hw,
                settings: use_signal(settings.clone()),
                info: use_signal(None),
                settings_open: use_signal(false),
                tracks_open: use_signal(false),
                audio_pref: use_signal(None),
                subs: use_signal(subs.clone()),
                sub_style: use_signal(subs.style()),
                subs_open: use_signal(false),
                filters_open: use_signal(false),
                filters_rev: use_signal(0),
                split: use_signal(settings.dlss_split),
                view_key: use_signal((settings.fill, settings.seek_step)),
                sub_pref: use_signal(None),
                sub_last: use_signal(None),
                sub_rev: use_signal(0),
                sub_gen: use_signal(0),
                vram: use_signal(vram::Vram::None),
                dlss: use_signal(dlss::Status::Off),
                sr: use_signal(neural::Status::Off),
                rife: use_signal(interp::Status::Off),
                cards: use_signal(Vec::new()),
                siblings: use_signal(Vec::new()),
                scanning: use_signal(true),
                playing: use_signal(None),
                opening: use_signal(None),
                error: use_signal(None),
                fullscreen: use_signal(false),
                direct: args.file.is_some(),
                edit: use_signal(None),
                sheet: use_signal(editor::Sheet::None),
                pos: use_signal(0.0),
            };
            syngui::signal::create_detached_effect(move || editor::follow(st));
            match &args.file {
                Some(f) => ui::open(st, f.into()),
                None => ui::rescan(st),
            }
            ui::root(st)
        });
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-video-player").font_selector("Text, Button").base(include_str!("../styles/syn-video-player.mss")).build()
}

/// HDR: окно плеера — FP16 (если композитор умеет управление цветом), а пока монитор в HDR-режиме,
/// HDR-видео идёт в него светом без тонмаппинга. Режим монитора спрашивается у synwm раз в 2 с.
fn hdr_watch() {
    syngui::gpu::request_hdr_surface(true);
    std::thread::Builder::new()
        .name("svp-hdr".into())
        .spawn(|| {
            use synshell_common::ipc::{Client, Request, Response};
            let mut last = None;
            loop {
                let hdr = Client::connect()
                    .ok()
                    .and_then(|mut c| c.request(&Request::Outputs).ok())
                    .map(|r| matches!(r, Response::Outputs { outputs } if outputs.iter().any(|o| o.hdr)))
                    .unwrap_or(false);
                if last != Some(hdr) {
                    tracing::info!(hdr, "монитор в HDR-режиме");
                    syngui::gpu::set_hdr_output(hdr);
                    last = Some(hdr);
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        })
        .ok();
}
