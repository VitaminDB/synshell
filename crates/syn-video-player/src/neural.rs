//! Увеличение разрешения нейросетью (synaptix-vfx, свои ядра CUDA): модели — `.syn` в папке `vfx`
//! библиотек synaptixd. Фильтр стоит в цепочке после DLSS 5; модель грузится в фоне, пока кадры идут
//! как есть. Видеомодели (NanoVSR) копят окно кадров — их pts берутся из очереди входа. Не успевает
//! за роликом — кадры как есть до перемотки, без мигания.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use syngui::async_runtime::run_on_main_thread;
use syngui::video::{FrameFilter, VideoFrame, VideoPlayer};

use crate::chain::{self, Stage};

/// На телефонах (aarch64) synaptix-vfx нет: тот же интерфейс без моделей — `models()` пуст, и до
/// загрузки дело не доходит.
#[cfg(not(target_arch = "x86_64"))]
mod synaptix_vfx {
    #[derive(Debug)]
    pub struct Error;
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("нейросетевые фильтры есть только на компьютере")
        }
    }
    pub struct Frame<'a> {
        pub rgba: &'a [u8],
        pub width: u32,
        pub height: u32,
    }
    pub trait Upscaler: Send {
        fn name(&self) -> &str;
        fn push(&mut self, rgba: &[u8], width: u32, height: u32, emit: &mut dyn FnMut(Frame)) -> Result<(), Error>;
        fn reset(&mut self);
    }
    pub fn open_upscaler(_path: &std::path::Path, _ordinal: usize) -> Result<Box<dyn Upscaler>, Error> {
        Err(Error)
    }
}

/// Модель повышения разрешения из библиотеки.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub path: PathBuf,
    pub arch: String,
    pub scale: u32,
    /// Для чего лучше: кино, аниме.
    pub anime: bool,
    /// Порядок «от быстрой к качественной» внутри семейства.
    pub rank: u32,
}

impl Model {
    /// «NanoVSR 644k · ×4».
    pub fn title(&self) -> String {
        let name = match self.arch.as_str() {
            "nanovsr" => format!("NanoVSR {}", self.id.trim_start_matches("nanovsr-").replace('-', ".")),
            "span" if self.id.ends_with("liveaction") => "SPAN LiveAction".into(),
            "span" if self.id.ends_with("nomosuni") => "SPAN NomosUni".into(),
            "realplksr" if self.id.ends_with("public-real") => "RealPLKSR Public".into(),
            "realplksr" if self.id.ends_with("vhs2hd") => "RealPLKSR VHS2HD".into(),
            "compact" if self.id.ends_with("animevideov3") => "Real-ESRGAN animevideov3".into(),
            "compact" if self.id.ends_with("superultracompact") => "AnimeJaNai V3 SuperUltraCompact".into(),
            "compact" if self.id.ends_with("ultracompact") => "AnimeJaNai V3 UltraCompact".into(),
            "compact" if self.id.ends_with("animejanai-compact") => "AnimeJaNai V3 Compact".into(),
            _ => self.id.clone(),
        };
        format!("{name} · ×{}", self.scale)
    }
}

/// Сведения об известных архитектурах: масштаб, аниме ли, ранг по id.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
fn describe(id: &str, arch: &str) -> Option<(u32, bool, u32)> {
    match arch {
        "nanovsr" => {
            let rank = ["226k", "644k", "1-7m", "5-4m"].iter().position(|s| id.ends_with(s)).unwrap_or(9) as u32;
            Some((4, false, rank))
        }
        "span" => {
            let scale = if id.contains("-4x-") { 4 } else { 2 };
            let rank = ["liveaction", "nomosuni"].iter().position(|s| id.ends_with(s)).unwrap_or(9) as u32;
            Some((scale, false, rank))
        }
        "realplksr" => {
            let rank = ["public-real", "vhs2hd"].iter().position(|s| id.ends_with(s)).unwrap_or(9) as u32;
            Some((if id.contains("-4x-") { 4 } else { 2 }, false, rank))
        }
        "compact" => {
            let scale = if id.contains("-4x-") { 4 } else { 2 };
            let rank = ["superultracompact", "ultracompact", "animejanai-compact", "animevideov3"].iter().position(|s| id.ends_with(s)).unwrap_or(9) as u32;
            Some((scale, true, rank))
        }
        _ => None,
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn models() -> Vec<Model> {
    Vec::new()
}

/// Папки моделей: `vfx` в каждой библиотеке synaptixd.
#[cfg(target_arch = "x86_64")]
fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(mut c) = synaptixd::Client::connect() {
        if let Ok(libs) = c.libraries() {
            out.extend(libs.into_iter().map(|l| PathBuf::from(l.path).join("vfx")));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(home).join(".local/share/synaptix/vfx"));
    }
    out.dedup();
    out
}

/// Найденные модели, по семейству и рангу.
#[cfg(target_arch = "x86_64")]
pub fn models() -> Vec<Model> {
    let mut out = Vec::new();
    for d in dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "syn") {
                continue;
            }
            let Ok(b) = synaptix_bundle::Bundle::open(&p) else { continue };
            let (id, arch) = (b.id().to_string(), b.meta().arch.clone());
            if let Some((scale, anime, rank)) = describe(&id, &arch) {
                if !out.iter().any(|m: &Model| m.id == id) {
                    out.push(Model { id, path: p, arch, scale, anime, rank });
                }
            }
        }
    }
    out.sort_by(|a, b| (a.anime, &a.arch, a.rank).cmp(&(b.anime, &b.arch, b.rank)));
    out
}

/// Что с увеличением сейчас — для чипа и панели.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    /// Модель грузится (компиляция ядер в первый раз — секунды).
    Loading,
    /// Работает: время на кадр, кадр до и после.
    Active { ms: f32, budget_ms: f32, from: (u32, u32), to: (u32, u32) },
    /// Ролик и так крупный — увеличивать некуда.
    Skipped(u32),
    /// Не успевает за роликом: кадры как есть до перемотки.
    Slow { ms: f32, budget_ms: f32 },
    /// Не хватило видеопамяти (её держат другие программы, обычно synaptixd).
    NoVram,
    Failed(String),
}

fn failure(e: &synaptix_vfx::Error) -> Status {
    let t = e.to_string();
    if t.contains("OUT_OF_MEMORY") { Status::NoVram } else { Status::Failed(t) }
}

/// Больше этого по высоте кадр на выходе не делаем (4K).
const MAX_OUT_H: u32 = 2160;
/// Время кадра больше этой доли времени кадра ролика — не успевает.
const BUDGET: f32 = 0.9;

struct Inner {
    up: Option<Box<dyn synaptix_vfx::Upscaler>>,
    loading: bool,
    gen: u64,
    /// pts кадров, ушедших в модель и ещё не вышедших.
    queue: VecDeque<f64>,
    last_pts: Option<f64>,
    period_ms: f32,
    /// Время модели на кадр (сглаженное), мс.
    cost_ms: f32,
    /// Модель не успевает на этой перемотке.
    slow_gen: Option<u64>,
    failed: bool,
    /// Окна, посчитанные после перемотки: первое — с выделением памяти, его время не в счёт.
    batches: u32,
    /// Время модели на кадр за последние кадры — медиана не боится одиночных выбросов.
    recent: VecDeque<f32>,
}

pub struct UpscaleFilter {
    me: Weak<UpscaleFilter>,
    model: Model,
    inner: Mutex<Inner>,
    status: Mutex<Status>,
}

impl UpscaleFilter {
    fn new(model: Model) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            model,
            inner: Mutex::new(Inner { up: None, loading: false, gen: u64::MAX, queue: VecDeque::new(), last_pts: None, period_ms: 0.0, cost_ms: 0.0, slow_gen: None, failed: false, batches: 0, recent: VecDeque::new() }),
            status: Mutex::new(Status::Loading),
        })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s;
    }

    /// Модель — в фоне: компиляция ядер и загрузка весов не держат кадры.
    fn start_loading(&self, inner: &mut Inner) {
        if inner.loading || inner.up.is_some() || inner.failed {
            return;
        }
        inner.loading = true;
        let me = self.me.clone();
        let path = self.model.path.clone();
        std::thread::Builder::new()
            .name("svp-upscale-load".into())
            .spawn(move || {
                let t0 = Instant::now();
                let r = synaptix_vfx::open_upscaler(&path, 0);
                let Some(me) = me.upgrade() else { return };
                let mut inner = me.inner.lock().unwrap();
                inner.loading = false;
                match r {
                    Ok(up) => {
                        tracing::info!("увеличение: {} готова за {} мс", up.name(), t0.elapsed().as_millis());
                        inner.up = Some(up);
                    }
                    Err(e) => {
                        tracing::warn!("увеличение: {}: {e}", path.display());
                        inner.failed = true;
                        drop(inner);
                        me.set_status(failure(&e));
                    }
                }
            })
            .ok();
    }
}

impl FrameFilter for UpscaleFilter {
    fn apply(&self, _frame: &mut VideoFrame) {}

    fn process(&self, frame: VideoFrame, emit: &mut dyn FnMut(VideoFrame)) {
        let mut inner = self.inner.lock().unwrap();
        if frame.rgba.is_empty() || inner.failed {
            drop(inner);
            emit(frame);
            return;
        }
        // время кадра ролика по pts
        if let Some(prev) = inner.last_pts {
            let d = ((frame.pts_sec - prev) * 1000.0) as f32;
            if d > 1.0 && d < 250.0 {
                inner.period_ms = if inner.period_ms == 0.0 { d } else { inner.period_ms * 0.9 + d * 0.1 };
            }
        }
        inner.last_pts = Some(frame.pts_sec);
        let scale = self.model.scale;
        if frame.height * scale > MAX_OUT_H {
            drop(inner);
            self.set_status(Status::Skipped(frame.height));
            emit(frame);
            return;
        }
        if frame.seek_generation != inner.gen {
            inner.gen = frame.seek_generation;
            inner.queue.clear();
            inner.cost_ms = 0.0;
            inner.batches = 0;
            inner.recent.clear();
            if let Some(up) = inner.up.as_mut() {
                up.reset();
            }
        }
        if inner.slow_gen == Some(frame.seek_generation) {
            drop(inner);
            emit(frame);
            return;
        }
        inner.slow_gen = None;
        if inner.up.is_none() {
            self.start_loading(&mut inner);
            drop(inner);
            self.set_status(Status::Loading);
            emit(frame);
            return;
        }
        let (w, h, gen) = (frame.width, frame.height, frame.seek_generation);
        inner.queue.push_back(frame.pts_sec);
        let t0 = Instant::now();
        let mut outs: Vec<VideoFrame> = Vec::new();
        let Inner { up, queue, .. } = &mut *inner;
        let mut t_copy = 0f32;
        let r = up.as_mut().unwrap().push(&frame.rgba, w, h, &mut |f| {
            let pts = queue.pop_front().unwrap_or(frame.pts_sec);
            let tc = Instant::now();
            let rgba: Arc<[u8]> = Arc::from(f.rgba);
            t_copy += tc.elapsed().as_secs_f32() * 1000.0;
            outs.push(VideoFrame { width: f.width, height: f.height, rgba, pts_sec: pts, seek_generation: gen, surface: None, yuv: None });
        });
        tracing::trace!("увеличение: кадр {:.1} мс, из них копия {:.1} мс", t0.elapsed().as_secs_f32() * 1000.0, t_copy);
        if let Err(e) = r {
            tracing::warn!("увеличение: {e}");
            inner.failed = true;
            inner.up = None;
            drop(inner);
            self.set_status(failure(&e));
            emit(frame);
            return;
        }
        if !outs.is_empty() {
            // окно видеомодели считается разом — время делится на его кадры
            let per = t0.elapsed().as_secs_f32() * 1000.0 / outs.len() as f32;
            inner.batches += 1;
            if inner.batches > 1 {
                inner.recent.push_back(per);
                if inner.recent.len() > 48 {
                    inner.recent.pop_front();
                }
                let mut v: Vec<f32> = inner.recent.iter().copied().collect();
                v.sort_by(f32::total_cmp);
                inner.cost_ms = v[v.len() / 2];
            }
            let (cost, period) = (inner.cost_ms, inner.period_ms);
            let to = (outs[0].width, outs[0].height);
            // решать не раньше чем через ~2 с работы: ноутбучная карта на первых кадрах ещё на низких частотах
            let warm = period > 0.0 && inner.batches as f32 * period * outs.len() as f32 > 2000.0;
            if warm && cost > period * BUDGET {
                tracing::info!("увеличение не успевает: {cost:.1} мс при кадре {period:.1} мс — кадры как есть до перемотки");
                inner.slow_gen = Some(gen);
                inner.queue.clear();
                if let Some(up) = inner.up.as_mut() {
                    up.reset();
                }
                drop(inner);
                self.set_status(Status::Slow { ms: cost, budget_ms: period });
                // уже посчитанные кадры отдать — они по порядку и вовремя
                for o in outs {
                    emit(o);
                }
                return;
            }
            drop(inner);
            self.set_status(Status::Active { ms: cost, budget_ms: period, from: (w, h), to });
            for o in outs {
                emit(o);
            }
        }
    }
}

// ─── плеер ──────────────────────────────────────────────────────────────────

static FILTER: Mutex<Option<Arc<UpscaleFilter>>> = Mutex::new(None);

/// Модель по настройке; пусто — «Авто» под высоту кадра `h`: до 540p — ×4, до 1080p — ×2; кино —
/// NanoVSR 644k / SPAN LiveAction, аниме — animevideov3 / AnimeJaNai UltraCompact. Тяжёлая RealPLKSR
/// сама не выбирается.
pub fn pick(id: &str, h: u32, anime: bool) -> Option<Model> {
    let list = models();
    if !id.is_empty() {
        if let Some(m) = list.iter().find(|m| m.id == id) {
            return Some(m.clone());
        }
    }
    let want = if h > 0 && h <= 540 { 4 } else { 2 };
    let prefer: &[&str] = match (anime, want) {
        (false, 4) => &["nanovsr-644k", "nanovsr-226k"],
        (false, _) => &["span-2x-liveaction", "span-2x-nomosuni"],
        (true, 4) => &["compact-4x-animevideov3"],
        (true, _) => &["compact-2x-animejanai-ultracompact", "compact-2x-animejanai-superultracompact"],
    };
    let ok = |m: &&Model| m.anime == anime && m.arch != "realplksr";
    prefer
        .iter()
        .find_map(|p| list.iter().find(|m| m.id == *p))
        .or_else(|| list.iter().find(|m| ok(m) && m.scale == want))
        .or_else(|| list.iter().find(|m| ok(m) && h > 0 && h * m.scale <= MAX_OUT_H))
        .cloned()
}

/// Поставить или снять увеличение на плеере по настройкам.
pub fn attach(st: crate::ui::St, p: &VideoPlayer) {
    let s = st.settings.get_untracked();
    let model = if s.sr { pick(&s.sr_model, p.meta().height, s.sr_anime) } else { None };
    let Some(model) = model else {
        chain::set(p, Stage::Upscale, None);
        *FILTER.lock().unwrap() = None;
        st.sr.set(if s.sr { Status::Failed(syngui::t!("Модели увеличения не установлены").into()) } else { Status::Off });
        return;
    };
    let f = {
        let mut g = FILTER.lock().unwrap();
        match g.as_ref() {
            Some(f) if f.model == model => f.clone(),
            _ => {
                let f = UpscaleFilter::new(model);
                *g = Some(f.clone());
                f
            }
        }
    };
    st.sr.set(f.status());
    chain::set(p, Stage::Upscale, Some(f as Arc<dyn FrameFilter>));
}

/// Из наблюдателя плеера (не главный поток): состояние — в панель и чип.
pub fn poll(st: crate::ui::St) {
    let s = if chain::has(Stage::Upscale) { FILTER.lock().unwrap().as_ref().map(|f| f.status()).unwrap_or_default() } else { Status::Off };
    run_on_main_thread(move || {
        if st.sr.get_untracked() != s {
            st.sr.set(s);
        }
    });
}

/// Повторить с нуля (после освобождения видеопамяти): новый фильтр с той же моделью.
pub fn retry(st: crate::ui::St) {
    *FILTER.lock().unwrap() = None;
    apply(st);
}

/// Настройки сменились: применить к открытому ролику.
pub fn apply(st: crate::ui::St) {
    let Some(playing) = st.playing.get_untracked() else { return };
    let Ok(p) = playing.player.lock() else { return };
    attach(st, &p);
}
