//! DLSS 5 для видео: фильтр кадров декодера syngui ([`FrameFilter`]) отдаёт каждый кадр помощнику NGX
//! (`syn_dlss::video`) и показывает ответ модели. Модель видит только кадр ролика — без панели и
//! заголовка; векторы движения (оптический поток NVIDIA) и смены сцен считает помощник. Не успел или
//! не готов — кадр показывается как есть, причина — в чипе и настройках.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use syn_dlss::video::{FrameError, VideoClient};
use syngui::async_runtime::run_on_main_thread;
use syngui::video::{FrameFilter, VideoFrame, VideoPlayer};

/// Ждать ответа на обычный кадр; первый кадр (сборка модели под размер) — дольше.
const FRAME_TIMEOUT: Duration = Duration::from_millis(250);
const FIRST_TIMEOUT: Duration = Duration::from_millis(3000);

/// Что сейчас с DLSS 5 в плеере — для чипа и панели фильтров.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    /// Ждём помощник или сборку модели.
    Starting,
    /// Кадры идут через модель; `scale` — доля разрешения модели, если плеер снизил его, чтобы
    /// успевать (`None` — как в профиле); `budget_ms` — время кадра ролика.
    Active { total_ms: f32, helper_ms: f32, fps: f32, size: (u32, u32), scale: Option<f32>, budget_ms: f32 },
    /// Модель не успевает даже на самом низком разрешении: кадры идут как есть, без мигания,
    /// следующая попытка — после перемотки или через [`SLOW_RETRY`].
    Slow { total_ms: f32, budget_ms: f32 },
    /// Кадры идут как есть: причина.
    Bypass(FrameError),
}

/// Ступени разрешения модели (доля от выбранного в профиле), когда она не успевает за роликом.
const AUTO_STEPS: [f32; 4] = [1.0, 0.75, 0.5, 0.35];
/// Время модели больше этой доли времени кадра — не успевает (нужен запас на копирование и показ).
const BUDGET: f32 = 0.85;
/// Меньше этой доли — можно поднять разрешение обратно.
const HEADROOM: f32 = 0.5;
/// Не менять ступень чаще: модель после смены разрешения пару кадров дольше.
const STEP_HOLD: Duration = Duration::from_millis(1500);
/// Через сколько снова пробовать модель после «не успевает».
const SLOW_RETRY: Duration = Duration::from_secs(20);

/// Подстройка разрешения модели под время кадра ролика.
struct Pace {
    /// Время кадра по pts (сглаженное), мс.
    period_ms: f32,
    last_pts: Option<f64>,
    /// Время кадра через модель (сглаженное), мс.
    cost_ms: f32,
    /// Ступень `AUTO_STEPS`.
    step: usize,
    changed: Instant,
    /// Модель не успевает: с какого момента кадры идут мимо неё и на какой перемотке.
    slow: Option<(Instant, u64, f32)>,
    /// Таймауты подряд: их считаем как «не успевает».
    timeouts: u32,
    /// Время кадра до последнего снижения разрешения: не стало заметно быстрее — время уходит не на
    /// модель (копирование кадров), снижать дальше бессмысленно.
    before_step: f32,
}

impl Pace {
    fn new() -> Self {
        Self { period_ms: 0.0, last_pts: None, cost_ms: 0.0, step: 0, changed: Instant::now(), slow: None, timeouts: 0, before_step: 0.0 }
    }

    fn budget(&self) -> f32 {
        self.period_ms * BUDGET
    }

    fn note_pts(&mut self, pts: f64) {
        if let Some(prev) = self.last_pts {
            let d = ((pts - prev) * 1000.0) as f32;
            // перемотка и разрывы — не время кадра
            if d > 1.0 && d < 250.0 {
                self.period_ms = if self.period_ms == 0.0 { d } else { self.period_ms * 0.9 + d * 0.1 };
            }
        }
        self.last_pts = Some(pts);
    }

    fn note_cost(&mut self, ms: f32) {
        self.timeouts = 0;
        self.cost_ms = if self.cost_ms == 0.0 { ms } else { self.cost_ms * 0.8 + ms * 0.2 };
    }

    /// Новая ступень, если пора её сменить: `Some(step)`.
    fn adjust(&mut self) -> Option<usize> {
        if self.period_ms == 0.0 || self.cost_ms == 0.0 || self.changed.elapsed() < STEP_HOLD {
            return None;
        }
        let budget = self.budget();
        let next = if self.cost_ms > budget && self.step + 1 < AUTO_STEPS.len() {
            self.before_step = self.cost_ms;
            self.step + 1
        } else if self.cost_ms < self.period_ms * HEADROOM && self.step > 0 {
            self.step - 1
        } else {
            return None;
        };
        self.step = next;
        self.changed = Instant::now();
        // время на новой ступени ещё не известно
        self.cost_ms = 0.0;
        Some(next)
    }

    /// Не успевает и снижать разрешение дальше нечего или бесполезно (стало быстрее меньше чем на 10%).
    fn hopeless(&self) -> bool {
        if self.period_ms == 0.0 || self.cost_ms == 0.0 || self.changed.elapsed() < STEP_HOLD || self.cost_ms <= self.budget() {
            return false;
        }
        self.step + 1 == AUTO_STEPS.len() || (self.step > 0 && self.cost_ms > self.before_step * 0.9)
    }
}

pub struct DlssFilter {
    client: VideoClient,
    opts: Mutex<Opts>,
    status: Mutex<Status>,
    /// Новый размер или пересборка — следующему кадру дать время на сборку модели.
    warm: Mutex<Option<(u32, u32)>>,
    /// Окно счёта кадров в секунду: начало и кадры с него.
    window: Mutex<(Instant, u32, f32)>,
    /// Кадры, отданные плееру: когда он их отпустит, память идёт под следующие (без page fault'ов на
    /// 8–33 МБ каждый кадр).
    pool: Mutex<Vec<Arc<[u8]>>>,
    pace: Mutex<Pace>,
    /// Фильтр включён: после [`DlssFilter::release`] кадр, уже ушедший в поток фильтра, не должен снова
    /// занять канал. Замок держится весь кадр — release ждёт кадр в работе.
    gate: Mutex<bool>,
}

/// Больше кадров в пуле не держать: очередь показа — 8, плюс показанный.
const POOL_MAX: usize = 12;

/// Как показывать ответ модели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    /// Доля правки модели: 1 — ответ как есть, 0.5 — пополам с исходным кадром.
    pub strength: f32,
    /// Сравнение: слева от `split` — исходный кадр, справа — DLSS 5.
    pub compare: bool,
    /// Где черта сравнения: доля ширины кадра.
    pub split: f32,
}

impl Default for Opts {
    fn default() -> Self {
        Self { strength: 1.0, compare: false, split: 0.5 }
    }
}

impl DlssFilter {
    pub fn new(name: &str) -> std::io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            client: VideoClient::open(name)?,
            opts: Mutex::new(Opts::default()),
            status: Mutex::new(Status::Starting),
            warm: Mutex::new(None),
            window: Mutex::new((Instant::now(), 0, 0.0)),
            pool: Mutex::new(Vec::new()),
            pace: Mutex::new(Pace::new()),
            gate: Mutex::new(true),
        }))
    }

    /// Снова пускать кадры к модели (после [`DlssFilter::release`]).
    pub fn enable(&self) {
        *self.gate.lock().unwrap() = true;
    }

    pub fn set_opts(&self, o: Opts) {
        *self.opts.lock().unwrap() = o;
    }

    pub fn status(&self) -> Status {
        // помощник мог подняться или упасть, пока кадров не было (пауза)
        let s = self.status.lock().unwrap().clone();
        match (&s, self.client.ready()) {
            (Status::Bypass(_), Ok(())) => Status::Starting,
            (Status::Active { .. } | Status::Starting, Err(e)) => Status::Bypass(e),
            _ => s,
        }
    }

    pub fn ready(&self) -> Result<(), FrameError> {
        self.client.ready()
    }

    pub fn release(&self) {
        let mut gate = self.gate.lock().unwrap();
        *gate = false;
        self.client.set_auto_scale(None);
        *self.pace.lock().unwrap() = Pace::new();
        self.client.release();
    }

    /// Значения видеопрофиля модели (стиль, разрешение): в video.ini и, пока канал наш, сразу в
    /// общую память.
    pub fn set_profile(&self, pairs: &[(&str, String)], latch: bool) {
        self.client.set_profile(pairs, latch);
        if latch {
            *self.warm.lock().unwrap() = None;
        }
        // выбранное разрешение — новая точка отсчёта подстройки
        let mut p = self.pace.lock().unwrap();
        if p.step != 0 || p.slow.is_some() {
            self.client.set_auto_scale(None);
            *p = Pace::new();
        }
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s;
    }

    /// Свободный буфер `n` байт (единственная ссылка): из пула, если плеер его уже отпустил, или новый.
    fn take_buffer(&self, n: usize) -> Arc<[u8]> {
        let mut pool = self.pool.lock().unwrap();
        pool.retain(|b| b.len() == n);
        match pool.iter_mut().position(|b| Arc::get_mut(b).is_some()) {
            Some(i) => pool.swap_remove(i),
            None => Arc::from(vec![0u8; n]),
        }
    }

    /// Кадр ушёл плееру — пул следит, когда он освободится.
    fn keep_buffer(&self, b: &Arc<[u8]>) {
        let mut pool = self.pool.lock().unwrap();
        if pool.len() < POOL_MAX {
            pool.push(b.clone());
        }
    }

    fn fps(&self) -> f32 {
        let mut w = self.window.lock().unwrap();
        w.1 += 1;
        let dt = w.0.elapsed().as_secs_f32();
        if dt >= 1.0 {
            w.2 = w.1 as f32 / dt;
            *w = (Instant::now(), 0, w.2);
        }
        w.2
    }

    /// Кадр пропустить мимо модели: она не успевает и ждёт следующей попытки.
    fn skip_slow(&self, generation: u64) -> bool {
        let mut p = self.pace.lock().unwrap();
        let Some((since, gen, cost)) = p.slow else { return false };
        if gen == generation && since.elapsed() < SLOW_RETRY {
            let period = p.period_ms;
            drop(p);
            self.set_status(Status::Slow { total_ms: cost, budget_ms: period });
            return true;
        }
        // перемотка или время вышло — ещё попытка
        p.slow = None;
        p.changed = Instant::now();
        p.cost_ms = 0.0;
        false
    }

    /// После кадра: подстроить разрешение модели под время кадра ролика.
    fn pace_after(&self, generation: u64) -> Option<f32> {
        let mut p = self.pace.lock().unwrap();
        if p.hopeless() || p.timeouts >= 3 && p.step + 1 == AUTO_STEPS.len() {
            tracing::info!("DLSS 5: модель не успевает ({:.1} мс при кадре {:.1} мс) — кадры как есть", p.cost_ms, p.period_ms);
            p.slow = Some((Instant::now(), generation, p.cost_ms));
            // кадры пойдут мимо модели — разрешение вернуть выбранное, следующая попытка начнёт с него
            p.step = 0;
            self.client.set_auto_scale(None);
            return None;
        }
        if p.timeouts >= 3 && p.step + 1 < AUTO_STEPS.len() {
            // таймауты подряд — то же «не успевает»
            p.cost_ms = f32::MAX;
            p.changed = Instant::now() - STEP_HOLD;
        }
        if let Some(step) = p.adjust() {
            let f = AUTO_STEPS[step];
            tracing::info!("DLSS 5: разрешение модели ×{f} (кадр {:.1} мс)", p.period_ms);
            self.client.set_auto_scale(if step == 0 { None } else { Some(f) });
        }
        (p.step > 0).then(|| AUTO_STEPS[p.step])
    }
}

impl FrameFilter for DlssFilter {
    fn apply(&self, f: &mut VideoFrame) {
        let gate = self.gate.lock().unwrap();
        if !*gate {
            return;
        }
        self.pace.lock().unwrap().note_pts(f.pts_sec);
        if self.skip_slow(f.seek_generation) {
            return;
        }
        let size = (f.width, f.height);
        let timeout = {
            let mut w = self.warm.lock().unwrap();
            if *w != Some(size) {
                *w = Some(size);
                FIRST_TIMEOUT
            } else {
                FRAME_TIMEOUT
            }
        };
        let first = timeout == FIRST_TIMEOUT;
        let opts = *self.opts.lock().unwrap();
        let n = f.width as usize * f.height as usize * 4;
        let mut composed: Option<Arc<[u8]>> = None;
        let r = self.client.process(f.width, f.height, &f.rgba, timeout, |answer| {
            // ответ сразу в кадр для показа — без промежуточного буфера
            let mut dst = self.take_buffer(n);
            if let Some(d) = Arc::get_mut(&mut dst) {
                compose(&f.rgba[..n], answer, d, f.width as usize, opts);
                self.keep_buffer(&dst);
                composed = Some(dst);
            }
        });
        match (r, composed) {
            (Ok(t), Some(px)) => {
                f.rgba = px;
                let fps = self.fps();
                // первый кадр размера — сборка модели, не время кадра
                if !first {
                    self.pace.lock().unwrap().note_cost(t.total_ms);
                }
                let scale = self.pace_after(f.seek_generation);
                let budget_ms = self.pace.lock().unwrap().period_ms;
                self.set_status(Status::Active { total_ms: t.total_ms, helper_ms: t.helper_ms, fps, size, scale, budget_ms });
            }
            (Ok(_), None) => {}
            (Err(e), _) => {
                match e {
                    FrameError::Timeout => {
                        self.pace.lock().unwrap().timeouts += 1;
                        *self.warm.lock().unwrap() = None;
                        self.pace_after(f.seek_generation);
                    }
                    // модель собирается или пересобирается — следующему кадру снова дать время
                    FrameError::Refused => *self.warm.lock().unwrap() = None,
                    _ => {}
                }
                tracing::debug!("DLSS 5: кадр {}x{} как есть: {e:?}", f.width, f.height);
                self.set_status(Status::Bypass(e));
            }
        }
    }
}

/// Итоговый кадр: ответ модели с долей `strength` поверх исходного, при сравнении слева от `split` —
/// исходный кадр и светлая черта (ширина — по ширине кадра).
fn compose(orig: &[u8], answer: &[u8], dst: &mut [u8], w: usize, o: Opts) {
    let k = (o.strength.clamp(0.0, 1.0) * 256.0).round() as i32;
    let row = w * 4;
    let mid = if o.compare { ((o.split.clamp(0.0, 1.0) * w as f32).round() as usize).min(w) } else { 0 };
    for ((dr, or), ar) in dst.chunks_exact_mut(row).zip(orig.chunks_exact(row)).zip(answer.chunks_exact(row)) {
        let cut = mid * 4;
        dr[..cut].copy_from_slice(&or[..cut]);
        let (d, oo, a) = (&mut dr[cut..], &or[cut..], &ar[cut..]);
        if k >= 256 {
            d.copy_from_slice(a);
        } else {
            for ((d, &o), &a) in d.iter_mut().zip(oo).zip(a) {
                *d = (o as i32 + (((a as i32 - o as i32) * k) >> 8)) as u8;
            }
            for px in d.chunks_exact_mut(4).zip(oo.chunks_exact(4)) {
                px.0[3] = px.1[3];
            }
        }
        if o.compare && w >= 2 {
            // черта видна и в уменьшенном окне: ~2 px на каждые 960 px ширины кадра (4K — 8 px)
            let half = (w / 960).max(1);
            let (l, r) = (mid.saturating_sub(half), (mid + half).min(w));
            for px in dr[l * 4..r * 4].chunks_exact_mut(4) {
                px.copy_from_slice(&[235, 235, 235, 255]);
            }
        }
    }
}

/// Помощник не отвечает — запустить (`dlssnr-helper start`), не чаще раза в 20 с: запуск поднимает
/// префикс Wine и модель, это секунды.
pub fn ensure_helper() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) || syn_dlss::paths::helper_running() {
        return;
    }
    *last = Some(Instant::now());
    match syn_dlss::video::start_helper() {
        Ok(()) => tracing::info!("DLSS 5: запускаю помощник"),
        Err(e) => tracing::warn!("DLSS 5: {e}"),
    }
}

// ─── плеер ──────────────────────────────────────────────────────────────────

/// Один клиент на процесс: общая память помощника одна.
static FILTER: Mutex<Option<Arc<DlssFilter>>> = Mutex::new(None);
/// Фильтр стоит на открытом плеере.
static ATTACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn filter() -> Result<Arc<DlssFilter>, String> {
    let mut g = FILTER.lock().unwrap();
    if let Some(f) = g.as_ref() {
        return Ok(f.clone());
    }
    let f = DlssFilter::new(&syngui::t!("Видеоплеер")).map_err(|e| e.to_string())?;
    *g = Some(f.clone());
    Ok(f)
}

fn opts_of(s: &crate::settings::Settings) -> Opts {
    Opts { strength: s.dlss_strength.min(100) as f32 / 100.0, compare: s.dlss_compare, split: s.dlss_split }
}

/// Значения видеопрофиля модели из панели фильтров. Клиента ещё нет (DLSS 5 выключен) — только в файл.
pub fn set_profile(pairs: &[(&str, String)], latch: bool) {
    let f = FILTER.lock().unwrap().clone();
    match f {
        Some(f) => f.set_profile(pairs, latch),
        None => syn_dlss::config::set_video_values(pairs),
    }
}

/// Значение видеопрофиля (числом).
pub fn profile_value(key: &str) -> Option<f32> {
    syn_dlss::config::video_value(key).and_then(|v| v.parse().ok())
}

/// Сменилась только черта или сила: без перестройки фильтра и перерисовки паузы через перемотку.
pub fn set_opts(st: crate::ui::St) {
    if let Some(f) = FILTER.lock().unwrap().as_ref() {
        let mut o = opts_of(&st.settings.get_untracked());
        // черту тянут по кадру — в настройки она попадёт по отпусканию
        o.split = st.split.get_untracked();
        f.set_opts(o);
    }
}

/// Поставить или снять DLSS 5 на открытом плеере по настройкам.
pub fn attach(st: crate::ui::St, p: &VideoPlayer) {
    let s = st.settings.get_untracked();
    if !s.dlss {
        p.set_frame_filter(None);
        ATTACHED.store(false, std::sync::atomic::Ordering::Release);
        if let Some(f) = FILTER.lock().unwrap().as_ref() {
            f.release();
        }
        st.dlss.set(Status::Off);
        return;
    }
    match filter() {
        Ok(f) => {
            f.set_opts(opts_of(&s));
            f.enable();
            if f.ready().is_err() {
                std::thread::spawn(ensure_helper);
            }
            p.set_frame_filter(Some(f.clone()));
            ATTACHED.store(true, std::sync::atomic::Ordering::Release);
            st.dlss.set(f.status());
        }
        Err(e) => {
            tracing::warn!("DLSS 5: общая память: {e}");
            st.dlss.set(Status::Bypass(FrameError::NoHelper));
        }
    }
}

/// Программа закрывается (окно, SIGTERM): отпустить канал — в общую память вернутся настройки игр.
pub fn shutdown() {
    ATTACHED.store(false, std::sync::atomic::Ordering::Release);
    let f = FILTER.lock().unwrap().clone();
    if let Some(f) = f {
        f.release();
    }
}

/// Настройки DLSS 5 сменились: применить к открытому ролику; на паузе — перерисовать кадр.
pub fn apply(st: crate::ui::St) {
    let Some(playing) = st.playing.get_untracked() else {
        if !st.settings.get_untracked().dlss {
            st.dlss.set(Status::Off);
        }
        return;
    };
    let Ok(mut p) = playing.player.lock() else { return };
    attach(st, &p);
    if p.is_paused() {
        let pos = p.position_sec();
        let _ = p.seek(pos);
    }
}

/// Раз в полсекунды из наблюдателя плеера (не главный поток): состояние — в чип; помощник
/// упал — поднять снова.
pub fn poll(st: crate::ui::St) {
    let status = if ATTACHED.load(std::sync::atomic::Ordering::Acquire) {
        FILTER.lock().unwrap().as_ref().map(|f| f.status()).unwrap_or_default()
    } else {
        Status::Off
    };
    if matches!(status, Status::Bypass(FrameError::NoHelper)) {
        ensure_helper();
    }
    run_on_main_thread(move || {
        if st.dlss.get_untracked() != status {
            st.dlss.set(status);
        }
    });
}

/// Человеческая причина «кадры как есть».
pub fn reason(e: &FrameError) -> String {
    use syngui::t;
    match e {
        FrameError::NoHelper => t!("Запускаю помощник DLSS 5…").into(),
        FrameError::ModelFailed(r) if r.is_empty() => t!("Модель DLSS 5 не загрузилась — откройте окно DLSS 5.").into(),
        FrameError::ModelFailed(r) => t!("Модель DLSS 5 не загрузилась: {r}", r = r),
        FrameError::Busy(game) => t!("DLSS 5 сейчас занят: {game}", game = game),
        FrameError::BadSize => t!("Размер кадра не подходит модели").into(),
        FrameError::Timeout => t!("Модель не успевает — кадры идут как есть").into(),
        FrameError::Refused => t!("Модель пересобирается…").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_strength_and_split() {
        let (w, h) = (8usize, 2usize);
        let orig = vec![100u8; w * h * 4];
        let ans = vec![200u8; w * h * 4];
        let mut d = vec![0u8; w * h * 4];
        compose(&orig, &ans, &mut d, w, Opts { strength: 1.0, compare: false, split: 0.5 });
        assert_eq!(&d[0..4], &[200, 200, 200, 200]);
        compose(&orig, &ans, &mut d, w, Opts { strength: 0.5, compare: false, split: 0.5 });
        assert_eq!(&d[0..4], &[150, 150, 150, 100]);
        compose(&orig, &ans, &mut d, w, Opts { strength: 1.0, compare: true, split: 0.5 });
        assert_eq!(&d[0..4], &[100, 100, 100, 100]); // слева исходник
        assert_eq!(&d[3 * 4..3 * 4 + 4], &[235, 235, 235, 255]); // черта
        assert_eq!(&d[4 * 4..4 * 4 + 4], &[235, 235, 235, 255]);
        assert_eq!(&d[6 * 4..6 * 4 + 4], &[200, 200, 200, 200]); // справа модель
        assert_eq!(&d[w * 4..w * 4 + 4], &[100, 100, 100, 100]); // вторая строка
        // 4K: черта 8 px по центру
        let (w4, h4) = (3840usize, 1usize);
        let (o4, a4) = (vec![100u8; w4 * h4 * 4], vec![200u8; w4 * h4 * 4]);
        let mut d4 = vec![0u8; w4 * h4 * 4];
        compose(&o4, &a4, &mut d4, w4, Opts { strength: 1.0, compare: true, split: 0.5 });
        let line = d4.chunks_exact(4).filter(|p| p == &[235, 235, 235, 255]).count();
        assert_eq!(line, 8);
        assert_eq!(&d4[1915 * 4..1916 * 4], &[100, 100, 100, 100]);
        assert_eq!(&d4[1916 * 4..1917 * 4], &[235, 235, 235, 255]);
        assert_eq!(&d4[1924 * 4..1925 * 4], &[200, 200, 200, 200]);
        // тёмнее исходного — без переполнения
        let ans0 = vec![0u8; w * h * 4];
        compose(&orig, &ans0, &mut d, w, Opts { strength: 0.5, compare: false, split: 0.5 });
        assert_eq!(&d[0..4], &[50, 50, 50, 100]);
        // черта в четверти: пиксель исходника, черта на 1–2, дальше модель
        compose(&orig, &ans, &mut d, w, Opts { strength: 1.0, compare: true, split: 0.25 });
        assert_eq!(&d[0..4], &[100, 100, 100, 100]);
        assert_eq!(&d[4..8], &[235, 235, 235, 255]);
        assert_eq!(&d[4 * 4..4 * 4 + 4], &[200, 200, 200, 200]);
    }

    #[test]
    fn pace_steps_down_and_up() {
        let mut p = Pace::new();
        for i in 0..20 {
            p.note_pts(i as f64 / 24.0); // ~41,7 мс
        }
        p.changed = Instant::now() - STEP_HOLD;
        p.note_cost(60.0);
        assert_eq!(p.adjust(), Some(1));
        assert_eq!(p.adjust(), None); // держит ступень
        p.changed = Instant::now() - STEP_HOLD;
        p.note_cost(10.0);
        assert_eq!(p.adjust(), Some(0));
        p.step = AUTO_STEPS.len() - 1;
        p.changed = Instant::now() - STEP_HOLD;
        p.note_cost(80.0);
        assert!(p.hopeless());
        // снизили, а быстрее не стало — дальше не снижать
        let mut p = Pace::new();
        for i in 0..20 {
            p.note_pts(i as f64 / 24.0);
        }
        p.changed = Instant::now() - STEP_HOLD;
        p.note_cost(60.0);
        assert_eq!(p.adjust(), Some(1));
        p.changed = Instant::now() - STEP_HOLD;
        p.note_cost(58.0);
        assert!(p.hopeless());
    }
}
