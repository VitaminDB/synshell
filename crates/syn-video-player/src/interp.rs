//! «Плавность»: промежуточные кадры нейросетью RIFE (synaptix-vfx). Ступень цепочки после увеличения:
//! на каждый новый кадр — промежуточные между прошлым и ним (pts между их pts), затем он сам. На смене
//! сцены промежуточных нет (иначе RIFE «перетекает» одну сцену в другую). Не успевает — кадры как есть
//! до перемотки. Модели — `rife-*.syn` в папке `vfx` библиотеки synaptixd.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use syngui::async_runtime::run_on_main_thread;
use syngui::video::{FrameFilter, VideoPlayer};

use crate::chain::{self, Stage};

/// Модель RIFE из библиотеки.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub path: PathBuf,
}

impl Model {
    pub fn title(&self) -> String {
        match self.id.as_str() {
            "rife-4-25" => "RIFE 4.25".into(),
            "rife-4-25-lite" => "RIFE 4.25 lite".into(),
            "rife-4-26" => "RIFE 4.26".into(),
            id => id.into(),
        }
    }

    /// Порядок в списке: основная, лёгкая, новая.
    fn rank(&self) -> u32 {
        ["rife-4-25", "rife-4-25-lite", "rife-4-26"].iter().position(|s| *s == self.id).unwrap_or(9) as u32
    }
}

/// Найденные модели RIFE.
pub fn models() -> Vec<Model> {
    let mut out: Vec<Model> = crate::neural::bundles().into_iter().filter(|(_, _, arch)| arch == "rife").map(|(id, path, _)| Model { id, path }).collect();
    out.sort_by_key(Model::rank);
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// Режим частоты кадров.
pub const MODES: [(&str, u32); 2] = [("x2", 0), ("60", 60)];

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    Loading,
    /// Работает: время на исходный кадр (с промежуточными), бюджет, частота на выходе.
    Active { ms: f32, budget_ms: f32, fps: f32 },
    Slow { ms: f32, budget_ms: f32 },
    NoVram,
    Failed(String),
}

/// Разница яркости больше этой (0..255, среднее по сетке) — смена сцены.
const SCENE_CUT: f32 = 28.0;
const BUDGET: f32 = 0.9;

#[cfg(target_arch = "x86_64")]
mod imp {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, Weak};
    use std::time::Instant;

    use syngui::video::{FrameFilter, VideoFrame};

    use super::{Model, Status, BUDGET, SCENE_CUT};

    struct Inner {
        rife: Option<synaptix_vfx::rife::Rife>,
        loading: bool,
        failed: bool,
        gen: u64,
        prev_pts: Option<f64>,
        period_ms: f32,
        recent: VecDeque<f32>,
        frames: u32,
        slow_gen: Option<u64>,
        /// Промежуточных кадров и склеек — для журнала.
        made: u64,
        cuts: u64,
    }

    pub struct InterpFilter {
        me: Weak<InterpFilter>,
        pub model: Model,
        /// 0 — ×2, иначе — целевая частота кадров.
        pub target: u32,
        inner: Mutex<Inner>,
        status: Mutex<Status>,
    }

    fn failure(e: &synaptix_vfx::Error) -> Status {
        let t = e.to_string();
        if t.contains("OUT_OF_MEMORY") { Status::NoVram } else { Status::Failed(t) }
    }

    impl InterpFilter {
        pub fn new(model: Model, target: u32) -> Arc<Self> {
            Arc::new_cyclic(|me| Self {
                me: me.clone(),
                model,
                target,
                inner: Mutex::new(Inner { rife: None, loading: false, failed: false, gen: u64::MAX, prev_pts: None, period_ms: 0.0, recent: VecDeque::new(), frames: 0, slow_gen: None, made: 0, cuts: 0 }),
                status: Mutex::new(Status::Loading),
            })
        }

        pub fn status(&self) -> Status {
            self.status.lock().unwrap().clone()
        }

        fn set_status(&self, s: Status) {
            *self.status.lock().unwrap() = s;
        }

        fn start_loading(&self, inner: &mut Inner) {
            if inner.loading || inner.rife.is_some() || inner.failed {
                return;
            }
            inner.loading = true;
            let me = self.me.clone();
            let path = self.model.path.clone();
            std::thread::Builder::new()
                .name("svp-rife-load".into())
                .spawn(move || {
                    let r = (|| {
                        let w = synaptix_vfx::weights::Weights::open(&path)?;
                        synaptix_vfx::rife::Rife::load(synaptix_vfx::gpu::Gpu::new(0)?, &w)
                    })();
                    let Some(me) = me.upgrade() else { return };
                    let mut inner = me.inner.lock().unwrap();
                    inner.loading = false;
                    match r {
                        Ok(rf) => {
                            tracing::info!("плавность: {} готова", rf.name());
                            inner.rife = Some(rf);
                        }
                        Err(e) => {
                            tracing::warn!("плавность: {}: {e}", path.display());
                            inner.failed = true;
                            drop(inner);
                            me.set_status(failure(&e));
                        }
                    }
                })
                .ok();
        }

        /// Моменты промежуточных кадров между pts `a` и `b` (доли интервала).
        fn times(&self, a: f64, b: f64) -> Vec<f32> {
            let d = b - a;
            if d <= 0.0 || d > 0.25 {
                return vec![];
            }
            if self.target == 0 {
                return vec![0.5];
            }
            let step = 1.0 / self.target as f64;
            if d <= step * 1.05 {
                return vec![];
            }
            let mut out = Vec::new();
            let mut k = (a / step).floor() + 1.0;
            while k * step < b - step * 0.25 {
                let t = ((k * step - a) / d) as f32;
                if t > 0.05 && t < 0.95 {
                    out.push(t);
                }
                k += 1.0;
            }
            out
        }
    }

    impl FrameFilter for InterpFilter {
        fn apply(&self, _frame: &mut VideoFrame) {}

        fn process(&self, frame: VideoFrame, emit: &mut dyn FnMut(VideoFrame)) {
            let mut inner = self.inner.lock().unwrap();
            if frame.rgba.is_empty() || inner.failed {
                drop(inner);
                emit(frame);
                return;
            }
            if frame.seek_generation != inner.gen {
                inner.gen = frame.seek_generation;
                inner.prev_pts = None;
                inner.recent.clear();
                inner.frames = 0;
                if let Some(r) = inner.rife.as_mut() {
                    r.reset();
                }
            }
            if let Some(p) = inner.prev_pts {
                let d = ((frame.pts_sec - p) * 1000.0) as f32;
                if d > 1.0 && d < 250.0 {
                    inner.period_ms = if inner.period_ms == 0.0 { d } else { inner.period_ms * 0.9 + d * 0.1 };
                }
            }
            if inner.slow_gen == Some(frame.seek_generation) {
                inner.prev_pts = Some(frame.pts_sec);
                drop(inner);
                emit(frame);
                return;
            }
            if inner.rife.is_none() {
                self.start_loading(&mut inner);
                inner.prev_pts = Some(frame.pts_sec);
                drop(inner);
                self.set_status(Status::Loading);
                emit(frame);
                return;
            }
            let t0 = Instant::now();
            let (w, h, gen) = (frame.width as usize, frame.height as usize, frame.seek_generation);
            let prev = inner.prev_pts;
            let times = prev.map(|p| self.times(p, frame.pts_sec)).unwrap_or_default();
            let mut mids: Vec<VideoFrame> = Vec::new();
            let mut cut = false;
            let r = (|| -> synaptix_vfx::Result<()> {
                let rf = inner.rife.as_mut().unwrap();
                rf.set_flow_scale(if h > 1440 { 0.5 } else { 1.0 });
                rf.push(&frame.rgba, w, h)?;
                if times.is_empty() || !rf.ready() {
                    return Ok(());
                }
                if rf.scene_diff()? > SCENE_CUT {
                    cut = true;
                    return Ok(());
                }
                let p = prev.unwrap();
                for t in &times {
                    let px = rf.interpolate(*t)?;
                    mids.push(VideoFrame { width: frame.width, height: frame.height, rgba: Arc::from(px), pts_sec: p + (frame.pts_sec - p) * *t as f64, seek_generation: gen, surface: None, yuv: None });
                }
                Ok(())
            })();
            inner.prev_pts = Some(frame.pts_sec);
            inner.made += mids.len() as u64;
            inner.cuts += u64::from(cut);
            if inner.frames % 240 == 0 {
                tracing::debug!("плавность: {} промежуточных кадров, склеек {}", inner.made, inner.cuts);
            }
            if let Err(e) = r {
                tracing::warn!("плавность: {e}");
                inner.failed = true;
                inner.rife = None;
                drop(inner);
                self.set_status(failure(&e));
                emit(frame);
                return;
            }
            // время на исходный кадр — медиана за последние кадры (после разгона ~2 с)
            let cost = t0.elapsed().as_secs_f32() * 1000.0;
            inner.frames += 1;
            if inner.frames > 2 {
                inner.recent.push_back(cost);
                if inner.recent.len() > 48 {
                    inner.recent.pop_front();
                }
            }
            let mut v: Vec<f32> = inner.recent.iter().copied().collect();
            v.sort_by(f32::total_cmp);
            let med = v.get(v.len() / 2).copied().unwrap_or(cost);
            let period = inner.period_ms;
            let warm = period > 0.0 && inner.frames as f32 * period > 2000.0;
            let fps = if period > 0.0 { if self.target == 0 { 2000.0 / period } else { self.target as f32 } } else { 0.0 };
            if warm && med > period * BUDGET {
                tracing::info!("плавность не успевает: {med:.1} мс при кадре {period:.1} мс — кадры как есть до перемотки");
                inner.slow_gen = Some(gen);
                drop(inner);
                self.set_status(Status::Slow { ms: med, budget_ms: period });
            } else {
                drop(inner);
                self.set_status(Status::Active { ms: med, budget_ms: period, fps });
            }
            for m in mids {
                emit(m);
            }
            emit(frame);
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod imp {
    use std::sync::Arc;

    use syngui::video::{FrameFilter, VideoFrame};

    use super::{Model, Status};

    /// На телефонах нейросетевых фильтров нет — фильтр не создаётся (моделей не находится).
    pub struct InterpFilter {
        pub model: Model,
        pub target: u32,
    }

    impl InterpFilter {
        pub fn new(model: Model, target: u32) -> Arc<Self> {
            Arc::new(Self { model, target })
        }
        pub fn status(&self) -> Status {
            Status::Off
        }
    }

    impl FrameFilter for InterpFilter {
        fn apply(&self, _frame: &mut VideoFrame) {}
    }
}

use imp::InterpFilter;

static FILTER: Mutex<Option<Arc<InterpFilter>>> = Mutex::new(None);

/// Модель по настройке: выбранная или основная (4.25).
pub fn pick(id: &str) -> Option<Model> {
    let list = models();
    list.iter().find(|m| m.id == id).cloned().or_else(|| list.into_iter().next())
}

pub fn target_of(mode: &str) -> u32 {
    MODES.iter().find(|(k, _)| *k == mode).map(|m| m.1).unwrap_or(0)
}

pub fn attach(st: crate::ui::St, p: &VideoPlayer) {
    let s = st.settings.get_untracked();
    let model = if s.rife { pick(&s.rife_model) } else { None };
    let Some(model) = model else {
        chain::set(p, Stage::Interpolate, None);
        *FILTER.lock().unwrap() = None;
        st.rife.set(if s.rife { Status::Failed(syngui::t!("Модели RIFE не установлены").into()) } else { Status::Off });
        return;
    };
    let target = target_of(&s.rife_mode);
    let f = {
        let mut g = FILTER.lock().unwrap();
        match g.as_ref() {
            Some(f) if f.model == model && f.target == target => f.clone(),
            _ => {
                let f = InterpFilter::new(model, target);
                *g = Some(f.clone());
                f
            }
        }
    };
    st.rife.set(f.status());
    chain::set(p, Stage::Interpolate, Some(f as Arc<dyn FrameFilter>));
}

pub fn poll(st: crate::ui::St) {
    let s = if chain::has(Stage::Interpolate) { FILTER.lock().unwrap().as_ref().map(|f| f.status()).unwrap_or_default() } else { Status::Off };
    run_on_main_thread(move || {
        if st.rife.get_untracked() != s {
            st.rife.set(s);
        }
    });
}

pub fn retry(st: crate::ui::St) {
    *FILTER.lock().unwrap() = None;
    apply(st);
}

pub fn apply(st: crate::ui::St) {
    let Some(playing) = st.playing.get_untracked() else { return };
    let Ok(p) = playing.player.lock() else { return };
    attach(st, &p);
}
