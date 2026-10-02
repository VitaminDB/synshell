//! syncamera — «Камера» synshell: фото (вспышка, таймер, сетка, 4:3/16:9/1:1, полное разрешение 200 Мп),
//! ночь (сцена NIGHT HAL), профи (ISO, выдержка, фокус, баланс белого), видео (720p/1080p/4K, H.264/HEVC,
//! звук, стабилизация, пауза, снимок во время записи, фонарик) и таймлапс; зум щипком и кнопками (переход на
//! широкоугольную), фокус и экспозиция касанием (удержание — блокировка), смена камеры, просмотр снятого.
//!
//! Кадры, снимки и управление — служба камеры syncamd платформы (`proto`), запись — аппаратный кодер
//! (`recorder`). Снимки — в «Изображения» (XDG_PICTURES_DIR), видео — в «Видео» (XDG_VIDEOS_DIR).

mod convert;
mod engine;
mod media;
mod modules;
mod night;
mod orientation;
mod portal;
mod prefs;
mod proto;
mod recorder;
mod sound;
mod ui;
mod viewer;

use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synshell_common::Config;

use engine::{Engine, Note, OpenSpec};
use media::Kind;
use prefs::Prefs;
use proto::{Camera, Control, Meta};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Night,
    Pro,
    Photo,
    Video,
    Timelapse,
}

pub const MODES: [(Mode, &str); 5] =
    [(Mode::Night, "Ночь"), (Mode::Pro, "Профи"), (Mode::Photo, "Фото"), (Mode::Video, "Видео"), (Mode::Timelapse, "Таймлапс")];

impl Mode {
    fn from_index(i: u32) -> Mode {
        MODES.get(i as usize).map(|m| m.0).unwrap_or(Mode::Photo)
    }
    fn index(self) -> u32 {
        MODES.iter().position(|m| m.0 == self).unwrap_or(2) as u32
    }
    pub fn video(self) -> bool {
        matches!(self, Mode::Video | Mode::Timelapse)
    }
}

/// Объектив задней камеры.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lens {
    Main,
    Wide,
    Macro,
}

impl Lens {
    fn index(self) -> u32 {
        match self {
            Lens::Main => 0,
            Lens::Wide => 1,
            Lens::Macro => 2,
        }
    }
    fn from_index(i: u32) -> Lens {
        match i {
            1 => Lens::Wide,
            2 => Lens::Macro,
            _ => Lens::Main,
        }
    }
}

/// Поле ручного режима, которое сейчас крутится ползунком.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProField {
    Iso,
    Shutter,
    Focus,
    Wb,
    Ev,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rec {
    Idle,
    On,
    Paused,
}

/// Миниатюра последнего снятого.
#[derive(Clone)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<[u8]>,
    pub key: u64,
    pub video: bool,
}

impl PartialEq for Thumb {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.rgba, &o.rgba) && self.key == o.key && self.video == o.video
    }
}

#[derive(Clone, Copy)]
pub struct St {
    pub engine: RwSignal<Option<Engine>>,
    pub cams: RwSignal<Vec<Camera>>,
    pub mode: RwSignal<Mode>,
    pub front: RwSignal<bool>,
    pub lens: RwSignal<Lens>,
    pub status: RwSignal<String>,
    pub frame_rev: RwSignal<u64>,
    /// Размер кадра потока (без поворота).
    pub stream_size: RwSignal<(u32, u32)>,
    pub flash: RwSignal<u32>,
    pub timer: RwSignal<u32>,
    pub grid: RwSignal<bool>,
    pub aspect: RwSignal<u32>,
    pub full_res: RwSignal<bool>,
    pub video_q: RwSignal<u32>,
    pub hevc: RwSignal<bool>,
    pub mic: RwSignal<bool>,
    pub stab: RwSignal<bool>,
    pub sound: RwSignal<bool>,
    pub timelapse: RwSignal<u32>,
    pub torch: RwSignal<bool>,
    /// Зум как на кнопках: 0.6 — широкоугольная, 1 — основная…
    pub zoom: RwSignal<f32>,
    pub zooming: RwSignal<bool>,
    /// Точка фокуса в долях превью; блокировка AE/AF.
    pub focus_pt: RwSignal<Option<(f32, f32)>>,
    pub focus_seq: RwSignal<u64>,
    pub locked: RwSignal<bool>,
    pub ev: RwSignal<i32>,
    pub meta: RwSignal<Meta>,
    pub pro_field: RwSignal<Option<ProField>>,
    pub iso: RwSignal<i32>,
    pub shutter_ns: RwSignal<i64>,
    pub focus_d: RwSignal<f32>,
    pub awb: RwSignal<u32>,
    pub rec: RwSignal<Rec>,
    pub rec_us: RwSignal<u64>,
    pub countdown: RwSignal<u32>,
    pub busy: RwSignal<bool>,
    pub blink: RwSignal<u64>,
    pub thumb: RwSignal<Option<Thumb>>,
    pub items: RwSignal<Vec<media::Item>>,
    pub viewer: RwSignal<Option<usize>>,
    pub settings: RwSignal<bool>,
    /// Выпадающий список задних камер открыт.
    pub lens_menu: RwSignal<bool>,
    /// Задние модули по пробе платформы (исправность).
    pub modules: RwSignal<Vec<modules::Module>>,
    pub start_cam: RwSignal<u32>,
    /// Папки снимков и видео (пусто — XDG).
    pub photo_dir: RwSignal<String>,
    pub video_dir: RwSignal<String>,
    pub toast: RwSignal<String>,
    /// Долгий снимок идёт: подпись над кадром (ночь, полное разрешение).
    pub progress: RwSignal<String>,
    /// QR-код в кадре и счётчик его появлений (подсказка гаснет, когда код ушёл из кадра).
    pub qr: RwSignal<Option<String>>,
    pub qr_seen: RwSignal<u64>,
    /// Поворот телефона по часовой (акселерометр).
    pub dev_rot: RwSignal<u32>,
    pub auto_rotate: bool,
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syncamera=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    let (cfg, _) = Config::load();
    synshell_common::haptics::set_config(&cfg.haptics);
    let auto_rotate = cfg.rotation.auto;
    let mss = theme(&cfg);
    App::new()
        .title("Камера")
        .app_id("syncamera")
        .size(420, 860)
        .min_size(320, 520)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let p = Prefs::load();
            // камера при запуске
            let (front, lens) = match p.start_cam {
                1 => (false, Lens::Main),
                2 => (false, Lens::Wide),
                3 => (true, Lens::Main),
                _ => (p.front, Lens::from_index(p.lens)),
            };
            let st = St {
                engine: use_signal(None),
                cams: use_signal(Vec::new()),
                mode: use_signal(Mode::from_index(p.mode)),
                front: use_signal(front),
                lens: use_signal(lens),
                status: use_signal("Включаю камеру…".to_string()),
                frame_rev: use_signal(0),
                stream_size: use_signal((0, 0)),
                flash: use_signal(p.flash),
                timer: use_signal(p.timer),
                grid: use_signal(p.grid),
                aspect: use_signal(p.aspect),
                full_res: use_signal(p.full_res),
                video_q: use_signal(p.video_q),
                hevc: use_signal(p.hevc),
                mic: use_signal(p.mic),
                stab: use_signal(p.stab),
                sound: use_signal(p.sound),
                timelapse: use_signal(p.timelapse),
                torch: use_signal(false),
                zoom: use_signal(if !front && lens == Lens::Wide { 0.6 } else { 1.0 }),
                zooming: use_signal(false),
                focus_pt: use_signal(None),
                focus_seq: use_signal(0),
                locked: use_signal(false),
                ev: use_signal(0),
                meta: use_signal(Meta::default()),
                pro_field: use_signal(None),
                iso: use_signal(0),
                shutter_ns: use_signal(0),
                focus_d: use_signal(-1.0),
                awb: use_signal(1),
                rec: use_signal(Rec::Idle),
                rec_us: use_signal(0),
                countdown: use_signal(0),
                busy: use_signal(false),
                blink: use_signal(0),
                thumb: use_signal(None),
                items: use_signal(Vec::new()),
                viewer: use_signal(None),
                settings: use_signal(false),
                lens_menu: use_signal(false),
                modules: use_signal(modules::back_modules()),
                start_cam: use_signal(p.start_cam),
                photo_dir: use_signal(p.photo_dir.clone()),
                video_dir: use_signal(p.video_dir.clone()),
                toast: use_signal(String::new()),
                progress: use_signal(String::new()),
                qr: use_signal(None),
                qr_seen: use_signal(0),
                dev_rot: use_signal(0),
                auto_rotate,
            };
            start(st);
            // отладка вида подсказки без камеры: SYNCAMERA_DEMO_QR=текст
            if let Ok(t) = std::env::var("SYNCAMERA_DEMO_QR") {
                std::thread::spawn(move || loop {
                    let t = t.clone();
                    run_on_main_thread(move || {
                        st.qr.set(Some(t));
                        st.qr_seen.set(st.qr_seen.get_untracked() + 1);
                    });
                    std::thread::sleep(Duration::from_secs(1));
                });
            }
            ui::root(st)
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syncamera.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

// ─── Запуск и сеанс ─────────────────────────────────────────────────────────

fn start(st: St) {
    let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let eng = Engine::new(move |n| match n {
        Note::Frame => {
            if !pending.swap(true, std::sync::atomic::Ordering::AcqRel) {
                let p = pending.clone();
                run_on_main_thread(move || {
                    p.store(false, std::sync::atomic::Ordering::Release);
                    if let Some(e) = st.engine.get_untracked() {
                        e.frame_taken();
                    }
                    st.frame_rev.set(st.frame_rev.get_untracked().wrapping_add(1));
                });
            }
        }
        Note::Meta(m) => run_on_main_thread(move || {
            if st.meta.get_untracked() != m {
                st.meta.set(m);
            }
        }),
        Note::Started(w, h) => run_on_main_thread(move || st.stream_size.set((w, h))),
        Note::Status(s) => run_on_main_thread(move || st.status.set(s)),
        Note::Qr(s) => run_on_main_thread(move || {
            if st.qr.get_untracked().as_deref() != Some(s.as_str()) {
                st.qr.set(Some(s));
            }
            st.qr_seen.set(st.qr_seen.get_untracked() + 1);
        }),
    });
    st.engine.set(Some(eng));
    orientation::start(move |d| run_on_main_thread(move || st.dev_rot.set(d)));
    // камеры — в фоне (служба могла ещё не подняться)
    std::thread::spawn(move || loop {
        match proto::cameras() {
            Ok(c) if !c.is_empty() => {
                run_on_main_thread(move || st.cams.set(c));
                break;
            }
            Ok(_) => run_on_main_thread(move || st.status.set("Камер нет".into())),
            Err(e) => {
                let m = e.to_string();
                run_on_main_thread(move || st.status.set(m));
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    // переоткрыть камеру при смене режима, камеры, объектива, соотношения, качества видео
    create_effect(move || {
        let _ = (st.cams.get(), st.mode.get(), st.front.get(), st.lens.get(), st.aspect.get(), st.video_q.get(), st.full_res.get());
        reopen(st);
    });
    // просмотр снятого открыт — камера выключена (не греет телефон под просмотром)
    let was_open = std::cell::Cell::new(false);
    create_effect(move || {
        let open = st.viewer.get().is_some();
        if open == was_open.replace(open) {
            return;
        }
        if open {
            if st.rec.get_untracked() == Rec::Idle {
                with_engine(st, |e| e.close());
            }
        } else {
            reopen(st);
        }
    });
    // поворот превью: окно, ориентация телефона, камера
    create_effect(move || {
        let _ = (st.dev_rot.get(), st.front.get(), st.lens.get(), st.cams.get(), viewport_size().get());
        apply_rotation(st);
    });
    // настройки — в файл
    create_effect(move || {
        let p = Prefs {
            flash: st.flash.get(),
            timer: st.timer.get(),
            grid: st.grid.get(),
            aspect: st.aspect.get(),
            full_res: st.full_res.get(),
            video_q: st.video_q.get(),
            hevc: st.hevc.get(),
            mic: st.mic.get(),
            stab: st.stab.get(),
            sound: st.sound.get(),
            timelapse: st.timelapse.get(),
            front: st.front.get(),
            mode: st.mode.get().index(),
            lens: st.lens.get().index(),
            start_cam: st.start_cam.get(),
            photo_dir: st.photo_dir.get(),
            video_dir: st.video_dir.get(),
        };
        p.save();
    });
    // стабилизация и фонарик — сразу в камеру
    create_effect(move || {
        let (stab, torch, video) = (st.stab.get(), st.torch.get(), st.mode.get().video());
        with_engine(st, |e| {
            e.control(proto::CTL_STAB | proto::CTL_TORCH, |c| {
                c.stabilization = (stab && video) as u32;
                c.torch = (torch && video) as u32;
            })
        });
    });
    // точка фокуса гаснет через 6 с без касаний (блокировка — держится)
    create_effect(move || {
        let seq = st.focus_seq.get();
        if st.focus_pt.get_untracked().is_none() {
            return;
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(6));
            run_on_main_thread(move || {
                if st.focus_seq.get_untracked() == seq && !st.locked.get_untracked() {
                    reset_focus(st);
                }
            });
        });
    });
    // QR — только в «Фото»; подсказка гаснет через 3 с после последнего распознавания
    create_effect(move || {
        let on = st.mode.get() == Mode::Photo && st.engine.get().is_some();
        with_engine(st, |e| e.set_qr(on));
        if !on {
            st.qr.set(None);
        }
    });
    create_effect(move || {
        let seq = st.qr_seen.get();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            run_on_main_thread(move || {
                if st.qr_seen.get_untracked() == seq {
                    st.qr.set(None);
                }
            });
        });
    });
    // всплывающее сообщение — 3 с
    create_effect(move || {
        let t = st.toast.get();
        if !t.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                run_on_main_thread(move || {
                    if st.toast.get_untracked() == t {
                        st.toast.set(String::new());
                    }
                });
            });
        }
    });
    // папки хранения — в media; сменились — снятое и миниатюра из новых папок
    let first = std::cell::Cell::new(true);
    create_effect(move || {
        let some = |s: String| (!s.trim().is_empty()).then(|| std::path::PathBuf::from(s.trim()));
        media::set_dirs(some(st.photo_dir.get()), some(st.video_dir.get()));
        if !first.replace(false) {
            refresh_thumb(st);
        }
    });
    reload_items(st, true);
}

/// Выбрать папку снимков (`video` — видео) окном портала.
pub fn choose_dir(st: St, video: bool) {
    let start = if video { media::videos_dir() } else { media::pictures_dir() };
    let title = if video { "Папка для видео" } else { "Папка для снимков" };
    std::thread::spawn(move || {
        let r = portal::choose_folder(title, &start);
        run_on_main_thread(move || match r {
            Ok(Some(dir)) => {
                if !media::writable(&dir) {
                    st.toast.set(format!("В папку «{}» нельзя записывать", dir.display()));
                    return;
                }
                let s = dir.to_string_lossy().into_owned();
                if video {
                    st.video_dir.set(s);
                } else {
                    st.photo_dir.set(s);
                }
            }
            Ok(None) => {}
            Err(e) => st.toast.set(format!("Выбор папки: {e}")),
        });
    });
}

pub fn with_engine(st: St, f: impl FnOnce(&Engine)) {
    if let Some(e) = st.engine.get_untracked() {
        f(&e);
    }
}

/// Камера HAL для выбранной стороны и объектива.
pub fn current_cam(st: St) -> Option<Camera> {
    let cams = st.cams.get_untracked();
    pick_cam(&cams, st.front.get_untracked(), st.lens.get_untracked())
}

pub fn pick_cam(cams: &[Camera], front: bool, lens: Lens) -> Option<Camera> {
    let phys = cams.iter().filter(|c| c.logical == 0);
    if front {
        return phys.filter(|c| c.front()).min_by_key(|c| c.id).copied();
    }
    let back: Vec<&Camera> = phys.filter(|c| !c.front()).collect();
    let main = back.iter().max_by_key(|c| (c.focal_um, std::cmp::Reverse(c.id))).copied().copied();
    let wide = back.iter().filter(|c| Some(c.id) != main.map(|m| m.id)).min_by_key(|c| c.focal_um).copied().copied();
    match lens {
        Lens::Main => main,
        Lens::Wide => wide.or(main),
        // макро — третий задний модуль, если он есть в HAL (неисправный в HAL не попадает)
        Lens::Macro => back
            .iter()
            .find(|c| Some(c.id) != main.map(|m| m.id) && Some(c.id) != wide.map(|m| m.id))
            .copied()
            .copied()
            .or(main),
    }
}

/// Есть ли в HAL камера для объектива (макро — третья задняя).
pub fn lens_available(st: St, lens: Lens) -> bool {
    let cams = st.cams.get_untracked();
    let main = pick_cam(&cams, false, Lens::Main).map(|c| c.id);
    let wide = pick_cam(&cams, false, Lens::Wide).map(|c| c.id);
    match lens {
        Lens::Main => main.is_some(),
        Lens::Wide => wide.is_some() && wide != main,
        Lens::Macro => {
            let m = pick_cam(&cams, false, Lens::Macro).map(|c| c.id);
            m.is_some() && m != main && m != wide
        }
    }
}

/// Выбрать задний объектив из списка.
pub fn choose_lens(st: St, lens: Lens) {
    st.lens_menu.set(false);
    if st.rec.get_untracked() != Rec::Idle || !lens_available(st, lens) {
        return;
    }
    st.front.set(false);
    st.zoom.set(if lens == Lens::Wide { 0.6 } else { 1.0 });
    st.lens.set(lens);
}

/// Есть ли широкоугольная задняя.
pub fn has_wide(st: St) -> bool {
    let cams = st.cams.get_untracked();
    let m = pick_cam(&cams, false, Lens::Main);
    let w = pick_cam(&cams, false, Lens::Wide);
    m.is_some() && w.is_some() && m.map(|c| c.id) != w.map(|c| c.id)
}

/// Базовый зум объектива на кнопках.
pub fn lens_base(st: St) -> f32 {
    if !st.front.get_untracked() && st.lens.get_untracked() == Lens::Wide {
        0.6
    } else {
        1.0
    }
}

/// Соотношение сторон фото (ширина:высота кадра датчика).
pub fn aspect_ratio(a: u32) -> (u32, u32) {
    match a {
        1 => (16, 9),
        2 => (1, 1),
        _ => (4, 3),
    }
}

/// Размер видео по качеству.
pub fn video_size(q: u32) -> (u32, u32) {
    match q {
        0 => (1280, 720),
        2 => (3840, 2160),
        _ => (1920, 1080),
    }
}

fn reopen(st: St) {
    if st.rec.get_untracked() != Rec::Idle {
        return;
    }
    let Some(cam) = current_cam(st) else { return };
    let mode = st.mode.get_untracked();
    let spec = if mode == Mode::Night {
        // серия кадров полного размера: снимок собирается из них (night.rs)
        let (aw, ah) = aspect_ratio(st.aspect.get_untracked());
        let (w, h) = cam.yuv_for(aw, ah, 4096).or_else(|| cam.yuv_for(4, 3, 4096)).unwrap_or((1920, 1440));
        OpenSpec { cam, width: w, height: h, fps: 30, video: false, jpeg: None }
    } else if mode.video() {
        let (w, h) = video_size(st.video_q.get_untracked());
        let ok = cam.sizes().iter().any(|s| s.width == w && s.height == h);
        let (w, h) = if ok { (w, h) } else { cam.yuv_for(16, 9, 1920).unwrap_or((1280, 720)) };
        // снимок во время записи — поток JPEG 16:9 сразу (не больше 1080p: рядом с 4K HAL может не принять)
        let jpeg = if w <= 1920 { cam.jpeg_for(16, 9) } else { None };
        OpenSpec { cam, width: w, height: h, fps: 30, video: true, jpeg }
    } else {
        let (aw, ah) = aspect_ratio(st.aspect.get_untracked());
        // превью ≤ 1440 по ширине (1920×1440 — лишние 1,2 Мп пересчёта в RGBA на каждый кадр)
        let max = if aw == ah { 2000 } else if (aw, ah) == (4, 3) { 1440 } else { 1920 };
        let (w, h) = cam.yuv_for(aw, ah, max).or_else(|| cam.yuv_for(4, 3, 1920)).unwrap_or((1280, 960));
        let jpeg = cam.jpeg_for(aw, ah);
        OpenSpec { cam, width: w, height: h, fps: 30, video: false, jpeg }
    };
    let digital = (st.zoom.get_untracked() / lens_base(st)).max(1.0);
    let mut c = Control::default();
    c.zoom = digital;
    c.ev = st.ev.get_untracked();
    c.mode = match mode {
        Mode::Night => proto::MODE_NIGHT,
        Mode::Video | Mode::Timelapse => proto::MODE_VIDEO,
        _ => proto::MODE_PHOTO,
    };
    c.stabilization = (mode.video() && st.stab.get_untracked()) as u32;
    c.torch = (mode.video() && st.torch.get_untracked()) as u32;
    if mode == Mode::Pro {
        c.iso = st.iso.get_untracked();
        c.exposure_ns = if c.iso > 0 { st.shutter_ns.get_untracked().max(cam.exposure_min.max(1)) } else { 0 };
        c.focus = st.focus_d.get_untracked();
        c.awb = st.awb.get_untracked();
    }
    st.focus_pt.set(None);
    st.locked.set(false);
    st.stream_size.set((0, 0));
    if mode != Mode::Pro {
        st.ev.set(0);
        c.ev = 0;
    }
    with_engine(st, |e| {
        e.set_rotation(preview_rot(st, &cam), cam.front());
        e.open(spec, c);
    });
}

/// Поворот окна относительно естественного портрета (по часовой).
pub fn display_rot(st: St) -> u32 {
    let vp = viewport_size().get_untracked();
    let d = st.dev_rot.get_untracked();
    if vp.width > vp.height {
        if d == 90 || d == 270 {
            d
        } else {
            90
        }
    } else if st.auto_rotate && d == 180 {
        180
    } else {
        0
    }
}

/// Поворот кадра камеры для превью в окне: задняя — датчик + поворот окна, фронтальная — датчик − поворот
/// (потом зеркало).
pub fn preview_rot(st: St, cam: &Camera) -> u32 {
    let d = display_rot(st);
    let o = cam.orientation.rem_euclid(360) as u32;
    if cam.front() {
        (o + 360 - d) % 360
    } else {
        (o + d) % 360
    }
}

fn apply_rotation(st: St) {
    if let Some(cam) = current_cam(st) {
        let r = preview_rot(st, &cam);
        with_engine(st, |e| e.set_rotation(r, cam.front()));
    }
}

/// Поворот записи (пиксели видео) по ориентации телефона.
fn record_rot(st: St, cam: &Camera) -> u32 {
    let d = st.dev_rot.get_untracked();
    let o = cam.orientation.rem_euclid(360) as u32;
    if cam.front() {
        (o + 360 - d) % 360
    } else {
        (o + d) % 360
    }
}

// ─── Зум, фокус, экспозиция ────────────────────────────────────────────────

/// Зум «как на кнопках»: переход между объективами, цифровой — на текущем.
pub fn set_zoom(st: St, z: f32) {
    let wide = has_wide(st) && !st.front.get_untracked();
    let min = if wide { 0.6 } else { 1.0 };
    let z = z.clamp(min, 10.0);
    let want_lens = if wide && z < 1.0 { Lens::Wide } else { Lens::Main };
    st.zoom.set(z);
    if !st.front.get_untracked() && want_lens != st.lens.get_untracked() {
        if st.rec.get_untracked() == Rec::Idle {
            st.lens.set(want_lens); // переоткроет камеру с этим зумом
        }
        return;
    }
    let digital = (z / lens_base(st)).max(1.0);
    with_engine(st, |e| e.control(proto::CTL_ZOOM, |c| c.zoom = digital));
}

/// Зум щипком: только цифровой на текущем объективе, смена объектива — по окончании жеста.
pub fn pinch_zoom(st: St, z: f32) {
    let base = lens_base(st);
    let wide = has_wide(st) && !st.front.get_untracked();
    let lo = if wide { 0.6 } else { 1.0 };
    let z = z.clamp(lo, 10.0);
    st.zoom.set(z);
    let digital = (z / base).max(1.0);
    with_engine(st, |e| e.control(proto::CTL_ZOOM, |c| c.zoom = digital));
}

pub fn pinch_end(st: St) {
    set_zoom(st, st.zoom.get_untracked());
}

/// Доли превью (после поворота и зеркала) → доли кадра потока.
fn to_stream(st: St, u: f32, v: f32) -> (f32, f32) {
    let Some(cam) = current_cam(st) else { return (u, v) };
    let rot = preview_rot(st, &cam);
    let mu = if cam.front() { 1.0 - u } else { u };
    match rot {
        90 => (v, 1.0 - mu),
        180 => (1.0 - mu, 1.0 - v),
        270 => (1.0 - v, mu),
        _ => (mu, v),
    }
}

/// Касание превью: фокус и экспозиция по точке; `lock` — ещё и блокировка AE.
pub fn tap_focus(st: St, u: f32, v: f32, lock: bool) {
    let (x, y) = to_stream(st, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    st.focus_pt.set(Some((u, v)));
    st.focus_seq.set(st.focus_seq.get_untracked() + 1);
    st.locked.set(lock);
    with_engine(st, |e| {
        e.control(proto::CTL_POINT | proto::CTL_AE_LOCK, |c| {
            c.x = x;
            c.y = y;
            c.ae_lock = lock as u32;
        })
    });
    if lock {
        st.toast.set("Блокировка экспозиции и фокуса".into());
    }
}

/// Точку фокуса сбросить (непрерывный автофокус); яркость — к нулю, кроме ручного режима.
pub fn reset_focus(st: St) {
    st.focus_pt.set(None);
    st.locked.set(false);
    let keep_ev = st.mode.get_untracked() == Mode::Pro;
    if !keep_ev {
        st.ev.set(0);
    }
    with_engine(st, |e| {
        e.control(proto::CTL_POINT | proto::CTL_AE_LOCK | if keep_ev { 0 } else { proto::CTL_EV }, |c| {
            c.x = -1.0;
            c.y = -1.0;
            c.ae_lock = 0;
            if !keep_ev {
                c.ev = 0;
            }
        })
    });
}

pub fn set_ev(st: St, ev: i32) {
    st.ev.set(ev);
    st.focus_seq.set(st.focus_seq.get_untracked() + 1);
    with_engine(st, |e| e.control(proto::CTL_EV, |c| c.ev = ev));
}

/// Ручной режим → камера.
pub fn apply_manual(st: St) {
    let Some(cam) = current_cam(st) else { return };
    let iso = st.iso.get_untracked();
    let exp = if iso > 0 { st.shutter_ns.get_untracked().clamp(cam.exposure_min.max(1), cam.exposure_max.max(1)) } else { 0 };
    let focus = st.focus_d.get_untracked();
    let awb = st.awb.get_untracked();
    with_engine(st, |e| {
        e.control(proto::CTL_MANUAL | proto::CTL_AWB, |c| {
            c.iso = iso;
            c.exposure_ns = exp;
            c.focus = focus;
            c.awb = awb;
        })
    });
}

// ─── Снимок ────────────────────────────────────────────────────────────────

pub fn haptic() {
    synshell_common::haptics::play(synshell_common::haptics::Feedback::Tick);
}

/// Кнопка затвора.
pub fn shutter(st: St) {
    let mode = st.mode.get_untracked();
    if mode.video() {
        match st.rec.get_untracked() {
            Rec::Idle => start_countdown(st, start_recording),
            _ => stop_recording(st),
        }
        return;
    }
    if st.busy.get_untracked() || st.countdown.get_untracked() > 0 {
        return;
    }
    start_countdown(st, take_photo);
}

fn start_countdown(st: St, then: fn(St)) {
    let n = st.timer.get_untracked();
    if n == 0 || st.countdown.get_untracked() > 0 {
        then(st);
        return;
    }
    st.countdown.set(n);
    std::thread::spawn(move || {
        for left in (1..=n).rev() {
            let go = std::sync::mpsc::channel::<bool>();
            let tx = go.0;
            run_on_main_thread(move || {
                let alive = st.countdown.get_untracked() > 0;
                if alive {
                    st.countdown.set(left);
                    if st.sound.get_untracked() {
                        sound::play(sound::Sound::Tick);
                    }
                }
                let _ = tx.send(alive);
            });
            if !go.1.recv().unwrap_or(false) {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        run_on_main_thread(move || {
            if st.countdown.get_untracked() > 0 {
                st.countdown.set(0);
                then(st);
            }
        });
    });
}

/// Отменить отсчёт таймера (касание во время отсчёта).
pub fn cancel_countdown(st: St) {
    st.countdown.set(0);
}

/// Миниатюра из последнего кадра превью.
fn grab_thumb(st: St, video: bool) -> Option<(u32, u32, Vec<u8>)> {
    let e = st.engine.get_untracked()?;
    let (w, h, rgba) = e.last_frame()?;
    let t = media::shrink(w, h, &rgba, 256);
    st.thumb.set(Some(Thumb { w: t.0, h: t.1, rgba: Arc::from(t.2.as_slice()), key: st.blink.get_untracked() + 1, video }));
    Some(t)
}

/// Поворот снимка для EXIF: датчик и телефон (у фронтальной — в обратную сторону).
fn photo_rot(st: St, cam: &Camera) -> u32 {
    record_rot(st, cam)
}

/// «Ночь»: серия кадров с потока → слияние → JPEG.
fn night_shot(st: St, cam: Camera) {
    let Some(e) = st.engine.get_untracked() else { return };
    st.busy.set(true);
    if st.sound.get_untracked() {
        sound::play(sound::Sound::Shutter);
    }
    haptic();
    let thumb = grab_thumb(st, false);
    // кадров: чем длиннее выдержка, тем меньше (серия ~1–1,5 с)
    let exp = st.meta.get_untracked().exposure_ns;
    let n = if exp >= 100_000_000 {
        5
    } else if exp >= 50_000_000 {
        7
    } else {
        10
    };
    let rot = photo_rot(st, &cam);
    st.progress.set("Съёмка — держите телефон неподвижно".into());
    let rx = e.burst(n);
    std::thread::spawn(move || {
        let r = (|| -> anyhow::Result<std::path::PathBuf> {
            let (frames, vu) = rx.recv_timeout(Duration::from_secs(15)).map_err(|_| anyhow::anyhow!("кадры серии не пришли"))?;
            run_on_main_thread(move || {
                st.blink.set(st.blink.get_untracked() + 1);
                st.progress.set("Обработка ночного снимка…".into());
            });
            let t = std::time::Instant::now();
            let mut m = night::merge(frames);
            night::tone(&mut m);
            let jpeg = night::to_jpeg(&m, vu, 95)?;
            let jpeg = night::add_exif(&jpeg, night::exif_orientation(rot));
            tracing::info!("ночь: {n} кадров {}×{} за {:.1} с", m.w, m.h, t.elapsed().as_secs_f32());
            let path = media::new_path(Kind::Photo);
            std::fs::write(&path, &jpeg)?;
            if let Some((w, h, rgba)) = &thumb {
                media::save_thumb(&path, *w, *h, rgba);
            }
            Ok(path)
        })();
        run_on_main_thread(move || {
            st.busy.set(false);
            st.progress.set(String::new());
            match r {
                Ok(p) => {
                    tracing::info!("снимок: {}", p.display());
                    reload_items(st, false);
                }
                Err(e) => st.toast.set(format!("Ночной снимок не получился: {e:#}")),
            }
        });
    });
}

pub fn take_photo(st: St) {
    let Some(cam) = current_cam(st) else { return };
    let mode = st.mode.get_untracked();
    if mode == Mode::Night && st.rec.get_untracked() == Rec::Idle {
        night_shot(st, cam);
        return;
    }
    st.busy.set(true);
    if st.sound.get_untracked() {
        sound::play(sound::Sound::Shutter);
    }
    haptic();
    let thumb = grab_thumb(st, false);
    st.blink.set(st.blink.get_untracked() + 1);
    let recording = st.rec.get_untracked() != Rec::Idle;
    let (aw, ah) = if recording { (16, 9) } else { aspect_ratio(st.aspect.get_untracked()) };
    let full = !recording && st.full_res.get_untracked() && (aw, ah) == (4, 3) && cam.full_width > 0 && mode != Mode::Night;
    let size = if full { Some((cam.full_width, cam.full_height)) } else { cam.jpeg_for(aw, ah) };
    let flash = if recording || mode == Mode::Night || mode == Mode::Pro || cam.flash == 0 { proto::FLASH_OFF } else { st.flash.get_untracked() };
    let manual_focus = mode == Mode::Pro && st.focus_d.get_untracked() >= 0.0;
    let flags = if manual_focus || recording { proto::SNAP_NO_AF } else { 0 };
    let rot = st.dev_rot.get_untracked() as i32;
    if full {
        st.progress.set("Полное разрешение — около 15 секунд…".into());
    }
    std::thread::spawn(move || {
        let r = proto::snapshot(cam.id, flash, size, rot, flags).and_then(|jpeg| {
            let path = media::new_path(Kind::Photo);
            std::fs::write(&path, &jpeg)?;
            if let Some((w, h, rgba)) = &thumb {
                media::save_thumb(&path, *w, *h, rgba);
            }
            Ok(path)
        });
        run_on_main_thread(move || {
            st.busy.set(false);
            st.progress.set(String::new());
            match r {
                Ok(p) => {
                    tracing::info!("снимок: {}", p.display());
                    reload_items(st, false);
                }
                Err(e) => st.toast.set(format!("Снимок не получился: {e}")),
            }
        });
    });
}

// ─── Видео ─────────────────────────────────────────────────────────────────

fn bitrate(q: u32, hevc: bool) -> u32 {
    let b = match q {
        0 => 10_000_000,
        2 => 48_000_000,
        _ => 20_000_000,
    };
    if hevc {
        b * 6 / 10
    } else {
        b
    }
}

pub fn start_recording(st: St) {
    let Some(cam) = current_cam(st) else { return };
    let Some(e) = st.engine.get_untracked() else { return };
    let mode = st.mode.get_untracked();
    let path = media::new_path(Kind::Video);
    let s = recorder::Settings {
        path: path.clone(),
        rot: record_rot(st, &cam),
        fps: 30,
        bitrate: bitrate(st.video_q.get_untracked(), st.hevc.get_untracked()),
        hevc: st.hevc.get_untracked(),
        audio: st.mic.get_untracked() && mode == Mode::Video,
        timelapse: if mode == Mode::Timelapse { st.timelapse.get_untracked() } else { 1 },
    };
    if let Err(err) = e.start_recording(s) {
        st.toast.set(format!("Запись не началась: {err:#}"));
        return;
    }
    if st.sound.get_untracked() {
        sound::play(sound::Sound::RecordStart);
    }
    haptic();
    if let Some((w, h, rgba)) = grab_thumb(st, true) {
        let p = path.clone();
        std::thread::spawn(move || media::save_thumb(&p, w, h, &rgba));
    }
    st.rec.set(Rec::On);
    st.rec_us.set(0);
    // время записи и ошибки кодера
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        run_on_main_thread(move || {
            if st.rec.get_untracked() == Rec::Idle {
                let _ = tx.send(false);
                return;
            }
            let stats = st.engine.get_untracked().and_then(|e| e.record_stats());
            if let Some((us, _, err)) = stats {
                st.rec_us.set(us);
                if let Some(err) = err {
                    st.toast.set(format!("Запись прервана: {err}"));
                    stop_recording(st);
                }
            }
            let _ = tx.send(true);
        });
        if !rx.recv().unwrap_or(false) {
            break;
        }
    });
}

pub fn pause_recording(st: St) {
    let on = st.rec.get_untracked() == Rec::On;
    with_engine(st, |e| e.pause_recording(on));
    st.rec.set(if on { Rec::Paused } else { Rec::On });
    haptic();
}

pub fn stop_recording(st: St) {
    if st.rec.get_untracked() == Rec::Idle {
        return;
    }
    st.rec.set(Rec::Idle);
    st.busy.set(true);
    if st.sound.get_untracked() {
        sound::play(sound::Sound::RecordStop);
    }
    haptic();
    let Some(e) = st.engine.get_untracked() else { return };
    std::thread::spawn(move || {
        let r = e.stop_recording();
        run_on_main_thread(move || {
            st.busy.set(false);
            match r {
                Ok(p) => {
                    tracing::info!("видео: {}", p.display());
                    st.toast.set("Видео сохранено".into());
                    reload_items(st, false);
                }
                Err(e) => st.toast.set(format!("Видео не сохранено: {e:#}")),
            }
            // объектив могли сменить во время записи — открыть как надо
            reopen(st);
        });
    });
}

// ─── Снятое ────────────────────────────────────────────────────────────────

pub fn reload_items(st: St, thumb: bool) {
    std::thread::spawn(move || {
        let items = media::list();
        let first = items.first().cloned();
        run_on_main_thread(move || st.items.set(items));
        if thumb {
            if let Some(it) = first {
                if let Some((w, h, rgba)) = media::thumb(&it.path, it.kind, 256) {
                    run_on_main_thread(move || {
                        if st.thumb.get_untracked().is_none() {
                            st.thumb.set(Some(Thumb { w, h, rgba: Arc::from(rgba.as_slice()), key: 0, video: it.kind == Kind::Video }));
                        }
                    });
                }
            }
        }
    });
}

/// Снятое удалили — миниатюра следующего.
pub fn refresh_thumb(st: St) {
    st.thumb.set(None);
    reload_items(st, true);
}
