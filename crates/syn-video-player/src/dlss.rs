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

/// Что сейчас с DLSS 5 в плеере — для чипа и настроек.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    /// Ждём помощник или сборку модели.
    Starting,
    /// Кадры идут через модель.
    Active { total_ms: f32, helper_ms: f32, fps: f32, size: (u32, u32) },
    /// Кадры идут как есть: причина.
    Bypass(FrameError),
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
}

/// Больше кадров в пуле не держать: очередь показа — 8, плюс показанный.
const POOL_MAX: usize = 12;

/// Как показывать ответ модели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    /// Доля правки модели: 1 — ответ как есть, 0.5 — пополам с исходным кадром.
    pub strength: f32,
    /// Левая половина — исходный кадр, правая — DLSS 5.
    pub compare: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self { strength: 1.0, compare: false }
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
        }))
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
        self.client.release();
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
}

impl FrameFilter for DlssFilter {
    fn apply(&self, f: &mut VideoFrame) {
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
                self.set_status(Status::Active { total_ms: t.total_ms, helper_ms: t.helper_ms, fps, size });
            }
            (Ok(_), None) => {}
            (Err(e), _) => {
                if matches!(e, FrameError::Timeout | FrameError::Refused) {
                    // модель собирается или пересобирается — следующему кадру снова дать время
                    *self.warm.lock().unwrap() = None;
                }
                tracing::debug!("DLSS 5: кадр {}x{} как есть: {e:?}", f.width, f.height);
                self.set_status(Status::Bypass(e));
            }
        }
    }
}

/// Итоговый кадр: ответ модели с долей `strength` поверх исходного, при сравнении левая половина —
/// исходный кадр и светлая черта посередине (ширина — по ширине кадра).
fn compose(orig: &[u8], answer: &[u8], dst: &mut [u8], w: usize, o: Opts) {
    let k = (o.strength.clamp(0.0, 1.0) * 256.0).round() as i32;
    let row = w * 4;
    let mid = if o.compare { w / 2 } else { 0 };
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
    Opts { strength: s.dlss_strength.min(100) as f32 / 100.0, compare: s.dlss_compare }
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
        compose(&orig, &ans, &mut d, w, Opts { strength: 1.0, compare: false });
        assert_eq!(&d[0..4], &[200, 200, 200, 200]);
        compose(&orig, &ans, &mut d, w, Opts { strength: 0.5, compare: false });
        assert_eq!(&d[0..4], &[150, 150, 150, 100]);
        compose(&orig, &ans, &mut d, w, Opts { strength: 1.0, compare: true });
        assert_eq!(&d[0..4], &[100, 100, 100, 100]); // слева исходник
        assert_eq!(&d[3 * 4..3 * 4 + 4], &[235, 235, 235, 255]); // черта
        assert_eq!(&d[4 * 4..4 * 4 + 4], &[235, 235, 235, 255]);
        assert_eq!(&d[6 * 4..6 * 4 + 4], &[200, 200, 200, 200]); // справа модель
        assert_eq!(&d[w * 4..w * 4 + 4], &[100, 100, 100, 100]); // вторая строка
        // 4K: черта 8 px по центру
        let (w4, h4) = (3840usize, 1usize);
        let (o4, a4) = (vec![100u8; w4 * h4 * 4], vec![200u8; w4 * h4 * 4]);
        let mut d4 = vec![0u8; w4 * h4 * 4];
        compose(&o4, &a4, &mut d4, w4, Opts { strength: 1.0, compare: true });
        let line = d4.chunks_exact(4).filter(|p| p == &[235, 235, 235, 255]).count();
        assert_eq!(line, 8);
        assert_eq!(&d4[1915 * 4..1916 * 4], &[100, 100, 100, 100]);
        assert_eq!(&d4[1916 * 4..1917 * 4], &[235, 235, 235, 255]);
        assert_eq!(&d4[1924 * 4..1925 * 4], &[200, 200, 200, 200]);
        // тёмнее исходного — без переполнения
        let ans0 = vec![0u8; w * h * 4];
        compose(&orig, &ans0, &mut d, w, Opts { strength: 0.5, compare: false });
        assert_eq!(&d[0..4], &[50, 50, 50, 100]);
    }
}
