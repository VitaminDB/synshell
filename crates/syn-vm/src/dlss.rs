//! DLSS 5 для кадров гостя: кадр, который показывает экран машины (рабочий стол DOS/VGA, Glide,
//! Windows, окно игры syn-gpu), уходит помощнику NGX через общий фильтр syn-dlss (профиль
//! виртуальных машин, vm.ini), на экран — ответ модели. Модель видит только кадр гостя — без панели,
//! заголовка и подсказок; векторы движения и смены сцен помощник считает сам (оптический поток).
//!
//! Пока DLSS включён, окно просит у syn-gpu кадры через BAR2 (CAPS без UUID) — пиксели сразу в памяти;
//! выключен — снова общие изображения GPU без копий. Модель не успела — кадр показывается как есть.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use syn_dlss::config::Profile;
use syn_dlss::filter::{ensure_helper, Filter};
use syngui::widgets::LiveFrame;

pub use syn_dlss::filter::{reason, Status};
pub use syn_dlss::video::FrameError;

/// Кадр гостя на обработку: куда показать и что.
pub struct Job {
    pub frame: Arc<LiveFrame>,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub on_change: Arc<dyn Fn() + Send + Sync>,
}

/// Поток DLSS машины: берёт последний кадр (не успел — промежуточные пропускаются), отдаёт модели и
/// ставит на экран ответ или кадр как есть.
pub struct Worker {
    filter: Arc<Filter>,
    slot: Mutex<Option<Job>>,
    cv: Condvar,
    alive: AtomicBool,
    /// Размер кадра сменился — новое «поколение» для подстройки (снова пробовать после «не успевает»).
    generation: AtomicU64,
    /// Размер и отпечаток последнего кадра: тот же кадр (VGA и рабочий стол шлются с частотой
    /// монитора, даже когда картинка стоит) модели не отдаётся — на экране остаётся её прошлый ответ.
    last: Mutex<((u32, u32), u64)>,
    /// Счёт за секунду для журнала: кадров пришло, из них новых.
    counts: Mutex<(Instant, u32, u32)>,
    /// Когда пришли новые кадры за последнюю секунду — частота картинки гостя для бюджета модели.
    recent: Mutex<std::collections::VecDeque<Instant>>,
}

impl Worker {
    /// Кадр гостя — в очередь (вместо ещё не взятого).
    pub fn submit(&self, job: Job) {
        if !self.alive.load(Ordering::Acquire) {
            // DLSS уже выключен, а клиент ещё не снял поток — кадр как есть
            job.frame.set(job.width, job.height, job.rgba);
            (job.on_change)();
            return;
        }
        let size = (job.width, job.height);
        let print = fingerprint(&job.rgba);
        let mut last = self.last.lock().unwrap();
        let same = *last == (size, print);
        {
            let mut c = self.counts.lock().unwrap();
            c.1 += 1;
            c.2 += u32::from(!same);
            if c.0.elapsed().as_secs_f32() >= 5.0 {
                tracing::debug!("DLSS 5: кадров гостя {}×{} за 5 с: {}, новых {}", size.0, size.1, c.1, c.2);
                *c = (Instant::now(), 0, 0);
            }
        }
        if same {
            return;
        }
        if last.0 != size {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        *last = (size, print);
        drop(last);
        // бюджет модели — по частоте новых кадров за секунду, а не по тому, как часто их успевают взять
        let now = Instant::now();
        let mut r = self.recent.lock().unwrap();
        r.push_back(now);
        while r.front().is_some_and(|t| now.duration_since(*t).as_secs_f32() > 1.0) {
            r.pop_front();
        }
        if r.len() >= 2 {
            self.filter.set_frame_period(1000.0 / r.len() as f32);
        }
        drop(r);
        *self.slot.lock().unwrap() = Some(job);
        self.cv.notify_one();
    }

    fn run(self: Arc<Self>) {
        loop {
            let job = {
                let mut g = self.slot.lock().unwrap();
                loop {
                    if !self.alive.load(Ordering::Acquire) {
                        // кадр, не дошедший до модели, — на экран как есть
                        if let Some(j) = g.take() {
                            j.frame.set(j.width, j.height, j.rgba);
                            (j.on_change)();
                        }
                        return;
                    }
                    if let Some(j) = g.take() {
                        break j;
                    }
                    g = self.cv.wait(g).unwrap();
                }
            };
            let out = self.filter.process_frame(job.width, job.height, &job.rgba, self.generation.load(Ordering::Relaxed));
            job.frame.set(job.width, job.height, out.unwrap_or(job.rgba));
            (job.on_change)();
            if matches!(self.filter.status(), Status::Bypass(FrameError::NoHelper)) {
                ensure_helper();
            }
        }
    }

    fn stop(&self) {
        self.alive.store(false, Ordering::Release);
        self.cv.notify_all();
    }
}

/// Отпечаток кадра: все пиксели словами по 8 байт (≈1 мс на 8 МБ) — отличить новый кадр от повтора.
fn fingerprint(px: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ px.len() as u64;
    let mut chunks = px.chunks_exact(8);
    for c in &mut chunks {
        let v = u64::from_le_bytes(c.try_into().unwrap());
        h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(29);
    }
    for &b in chunks.remainder() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Канал помощника один на процесс: DLSS включён у одной машины.
struct Active {
    id: String,
    worker: Arc<Worker>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);
/// Клиент общей памяти: один на процесс (имя в окне syn-dlss — машина, для которой открыт).
static FILTER: Mutex<Option<(String, Arc<Filter>)>> = Mutex::new(None);

fn filter_for(name: &str) -> std::io::Result<Arc<Filter>> {
    let mut g = FILTER.lock().unwrap();
    if let Some((n, f)) = g.as_ref() {
        if n == name {
            return Ok(f.clone());
        }
        f.release();
    }
    let f = Filter::new(name, Profile::Vm)?;
    *g = Some((name.to_string(), f.clone()));
    Ok(f)
}

/// Включить или выключить DLSS у машины `id` (`name` — для окна syn-dlss) на её клиенте syn-gpu.
pub fn set(id: &str, name: &str, present: Option<&Arc<crate::present::Present>>, on: bool) {
    let mut act = ACTIVE.lock().unwrap();
    if !on {
        if act.as_ref().is_some_and(|a| a.id == id) {
            let a = act.take().unwrap();
            drop(act);
            stop(a, present);
        } else if let Some(p) = present {
            p.set_dlss(None);
        }
        return;
    }
    if let Some(a) = act.as_ref() {
        if a.id == id {
            if let Some(p) = present {
                p.set_dlss(Some(a.worker.clone()));
            }
            return;
        }
    }
    // другая машина держала канал — отпустить
    if let Some(a) = act.take() {
        stop(a, None);
    }
    let filter = match filter_for(&format!("syn-vm: {name}")) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("DLSS 5: общая память: {e}");
            return;
        }
    };
    filter.enable();
    if filter.ready().is_err() {
        std::thread::spawn(ensure_helper);
    }
    let worker = Arc::new(Worker {
        filter,
        slot: Mutex::new(None),
        cv: Condvar::new(),
        alive: AtomicBool::new(true),
        generation: AtomicU64::new(0),
        last: Mutex::new(((0, 0), 0)),
        counts: Mutex::new((Instant::now(), 0, 0)),
        recent: Mutex::new(std::collections::VecDeque::new()),
    });
    let w2 = worker.clone();
    if std::thread::Builder::new().name("syn-vm-dlss".into()).spawn(move || w2.run()).is_err() {
        return;
    }
    if let Some(p) = present {
        p.set_dlss(Some(worker.clone()));
    }
    tracing::info!("DLSS 5: включён у машины {id}");
    *act = Some(Active { id: id.to_string(), worker });
}

fn stop(a: Active, present: Option<&Arc<crate::present::Present>>) {
    if let Some(p) = present {
        p.set_dlss(None);
    }
    a.worker.stop();
    a.worker.filter.release();
    tracing::info!("DLSS 5: выключен у машины {}", a.id);
}

/// Состояние DLSS машины для кнопки: `Off` — выключен или включён у другой машины.
pub fn status(id: &str) -> Status {
    ACTIVE.lock().unwrap().as_ref().filter(|a| a.id == id).map(|a| a.worker.filter.status()).unwrap_or(Status::Off)
}

/// Окно закрывается: отпустить канал — в общую память вернутся настройки игр.
pub fn shutdown() {
    if let Some(a) = ACTIVE.lock().unwrap().take() {
        stop(a, None);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fingerprint_tells_frames_apart() {
        let a = vec![7u8; 640 * 480 * 4];
        let mut b = a.clone();
        assert_eq!(super::fingerprint(&a), super::fingerprint(&b));
        b[640 * 4 * 300 + 17] = 8; // один байт посреди кадра
        assert_ne!(super::fingerprint(&a), super::fingerprint(&b));
        assert_ne!(super::fingerprint(&a[..16]), super::fingerprint(&a[..24]));
    }
}
