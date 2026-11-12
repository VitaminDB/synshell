//! DLSS 5 для видео: фильтр кадров декодера syngui ([`FrameFilter`]) отдаёт каждый кадр помощнику NGX
//! (`syn_dlss::video`) и показывает ответ модели. Модель видит только кадр ролика — без панели и
//! заголовка; векторы движения (оптический поток NVIDIA) и смены сцен считает помощник. Не успел или
//! не готов — кадр показывается как есть, причина — в чипе и настройках.

use std::sync::{Arc, Mutex};

use syn_dlss::video::FrameError;
use syngui::async_runtime::run_on_main_thread;
use syngui::video::{FrameFilter, VideoFrame, VideoPlayer};

pub use syn_dlss::filter::{ensure_helper, reason, Opts, Status};

/// DLSS 5 на кадрах декодера: общий фильтр syn-dlss (видеопрофиль) в цепочке плеера.
pub struct DlssFilter(Arc<syn_dlss::filter::Filter>);

impl std::ops::Deref for DlssFilter {
    type Target = syn_dlss::filter::Filter;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DlssFilter {
    pub fn new(name: &str) -> std::io::Result<Arc<Self>> {
        Ok(Arc::new(Self(syn_dlss::filter::Filter::new(name, syn_dlss::config::Profile::Video)?)))
    }
}

impl FrameFilter for DlssFilter {
    fn apply(&self, f: &mut VideoFrame) {
        if let Some(px) = self.0.process(f.width, f.height, &f.rgba, f.pts_sec, f.seek_generation) {
            f.rgba = px;
        }
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
        crate::chain::set(p, crate::chain::Stage::Dlss, None);
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
            crate::chain::set(p, crate::chain::Stage::Dlss, Some(f.clone()));
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
