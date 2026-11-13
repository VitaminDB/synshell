//! syn-video-player — видеоплеер synshell: ffmpeg (демуксер, звук, программные кодеки) и виджет плеера
//! syngui. Видео декодируется аппаратно, где есть чем: на телефонах — V4L2-декодер SoC (`HwAccel::V4l2`,
//! Qualcomm `msm_vidc`), на компьютерах — VAAPI/NVDEC; иначе — программно.
//!
//!   syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [ФАЙЛ|URI]
//!   syn-video-player --bench [--hw …] [--dlss] ФАЙЛ   — декодировать полным ходом без окна, напечатать fps
//!                                                    (SVP_BENCH_FRAMES=N — только N кадров)
//!                                                    (`--dlss` — через DLSS 5, время модели)
//!
//!   syn-video-player --screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page library|player|subs|filters|settings|tracks|edit] [--at С] [ФАЙЛ]
//!
//! Без файла — список видео из «Видео» (XDG_VIDEOS_DIR).

use syn_video_player::{bench, chrome, dlss, editor, settings, ui};
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
    /// `--screenshot`: снимок интерфейса без окна.
    pub shot: Option<Shot>,
}

pub struct Shot {
    pub out: String,
    pub size: (u32, u32),
    pub scale: f64,
    pub page: String,
    /// `--at С`: с какой секунды открыть ролик.
    pub at: Option<f64>,
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
    let mut a = Args { file: None, hw: None, bench: false, yuv: false, dlss: false, shot: None };
    let mut size = (1400u32, 900u32);
    let mut scale = 1.0f64;
    let mut page = String::from("library");
    let mut out: Option<String> = None;
    let mut at: Option<f64> = None;
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--bench" => a.bench = true,
            "--yuv" => a.yuv = true,
            "--dlss" => a.dlss = true,
            "--screenshot" => out = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = it.next().unwrap_or(page),
            "--at" => at = it.next().and_then(|s| s.parse().ok()),
            "--hw" => {
                let v = it.next().unwrap_or_default();
                a.hw = Some(parse_hw(&v).unwrap_or_else(|| {
                    eprintln!("{}", t!("неизвестное ускорение «{v}» (auto|none|v4l2|vaapi|nvdec|vulkan)", v = v));
                    std::process::exit(2)
                }));
            }
            "-h" | "--help" => {
                println!("{}", t!("syn-video-player [--hw auto|none|v4l2|vaapi|nvdec|vulkan] [--bench [--yuv] [--dlss]] [ФАЙЛ|URI]"));
                println!("{}", t!("syn-video-player --screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page library|player|subs|filters|settings|tracks|edit] [--at С] [ФАЙЛ]"));
                std::process::exit(0)
            }
            _ if a.file.is_none() => a.file = Some(input_of(&s)),
            _ => {}
        }
    }
    a.shot = out.map(|out| Shot { out, size, scale, page, at });
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
    synshell_common::i18n::init(&syn_video_player::catalogs());
    let args = parse_args();
    if args.bench {
        let Some(file) = args.file.as_deref() else {
            eprintln!("{}", t!("--bench: нужен файл"));
            std::process::exit(2)
        };
        std::process::exit(bench::run(file, args.hw.unwrap_or(HwAccel::Auto), args.yuv, args.dlss));
    }
    if let Some(shot) = &args.shot {
        match screenshot(&args, shot) {
            Ok((w, h)) => println!("{}: {w}x{h}", shot.out),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        // потоки декодера не ждать
        std::process::exit(0);
    }
    hdr_watch();
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let win = use_signal(syngui::window::WindowState::default());
    App::new()
        .title(t!("Видео"))
        .app_id("syn-video-player")
        .size(960, 640)
        .min_size(320, 400)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .fullscreen(chrome::start_fullscreen(&settings::Settings::load()))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .on_close_request(|| {
            // настройки DLSS 5 в общей памяти — снова игровые
            dlss::shutdown();
            true
        })
        .run(move |_| {
            let st = new_state(args.hw, args.file.is_some(), win);
            chrome::follow_window(st);
            match &args.file {
                Some(f) => ui::open(st, f.into()),
                None => ui::rescan(st),
            }
            ui::root(st)
        });
}

fn new_state(hw: Option<HwAccel>, direct: bool, window: RwSignal<syngui::window::WindowState>) -> ui::St {
    ui::new_state(hw, direct, window, None)
}

/// `--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page library|player|subs|filters|settings|tracks|edit] [--at С] [ФАЙЛ]`:
/// снимок интерфейса без окна. Без файла — библиотека из «Видео»; страницы плеера открывают `ФАЙЛ`
/// (или первый ролик библиотеки) и ждут первых кадров.
fn screenshot(args: &Args, shot: &Shot) -> std::result::Result<(u32, u32), String> {
    use std::cell::Cell;
    use std::rc::Rc;
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let cell: Rc<Cell<Option<ui::St>>> = Rc::new(Cell::new(None));
    let page = shot.page.clone();
    let player_page = page != "library";
    let file = args.file.clone();
    let hw = args.hw;
    let at = shot.at;
    let started = Rc::new(Cell::new(None::<std::time::Instant>));
    let ready = {
        let cell = cell.clone();
        let page = page.clone();
        let started = started.clone();
        let paused = Rc::new(Cell::new(None::<()>));
        Box::new(move || -> bool {
            let Some(st) = cell.get() else { return false };
            if !player_page {
                return !st.scanning.get_untracked() && st.cards.with_untracked(|c| c.iter().all(|c| c.pic.with_untracked(Option::is_some)));
            }
            // библиотека нашлась — открыть первый ролик
            if st.playing.get_untracked().is_none() && st.opening.get_untracked().is_none() && st.error.get_untracked().is_none() {
                let first = st.cards.with_untracked(|c| c.first().map(|c| c.item.path.clone()));
                match first {
                    Some(p) if !st.scanning.get_untracked() => ui::open_from(st, p, at),
                    _ => return false,
                }
                return false;
            }
            let Some(p) = st.playing.get_untracked() else { return st.error.get_untracked().is_some() };
            // первые кадры декодированы — пауза; дальше ждём сводку декодирования и ленту миниатюр
            let pos = p.player.lock().map(|pl| pl.position_sec()).unwrap_or(0.0);
            if paused.get().is_none() {
                // кадры пошли с места открытия, способ декодирования известен — пауза
                if pos < at.unwrap_or(0.0) + 0.3 || st.info.with_untracked(|i| i.as_ref().is_none_or(|i| !i.known)) {
                    return false;
                }
                if let Ok(mut pl) = p.player.lock() {
                    pl.pause();
                }
                paused.set(Some(()));
            }
            let thumbs = st.siblings.with_untracked(|c| c.iter().all(|c| c.pic.with_untracked(Option::is_some)));
            if !thumbs {
                return false;
            }
            if started.get().is_none() {
                started.set(Some(std::time::Instant::now()));
                match page.as_str() {
                    "subs" => st.subs_open.set(true),
                    "filters" => st.filters_open.set(true),
                    "settings" => st.settings_open.set(true),
                    "tracks" => st.tracks_open.set(true),
                    "edit" => editor::start(st),
                    _ => {}
                }
            }
            true
        }) as Box<dyn Fn() -> bool>
    };
    let opts = syngui::shot::ShotOptions { ready: Some(ready), settle: std::time::Duration::from_millis(1500), timeout: std::time::Duration::from_secs(60), ..Default::default() };
    syngui::shot::render_png_with(
        move || {
            let st = new_state(hw, false, use_signal(syngui::window::WindowState { focused: true, ..Default::default() }));
            cell.set(Some(st));
            match &file {
                Some(f) if player_page => {
                    ui::open_from(st, f.into(), at);
                }
                _ => ui::rescan(st),
            }
            ui::root(st)
        },
        &mss,
        shot.size,
        shot.scale,
        &shot.out,
        opts,
    )
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-video-player").font_selector("Text, Button").base(include_str!("../styles/syn-video-player.mss")).push(syn_video_player::PLAYER_STYLE).build()
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
