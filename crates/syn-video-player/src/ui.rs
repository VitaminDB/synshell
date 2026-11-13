//! Экраны: библиотека (сетка карточек; столбцов — по ширине окна, от телефона в портрете до монитора)
//! и плеер во весь экран (виджет плеера syngui).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::video::{DecodeInfo, HwAccel, VideoPlayer};
use syngui::widgets::*;

use crate::library::{self, Item};
use crate::settings::{self, Settings};

type W = Box<dyn Widget>;

/// Миниатюра карточки.
#[derive(PartialEq)]
pub struct Pic {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<Vec<u8>>,
    pub duration: f64,
}

#[derive(Clone)]
pub struct Card {
    pub item: Item,
    pub pic: RwSignal<Option<Arc<Pic>>>,
}

impl PartialEq for Card {
    fn eq(&self, o: &Self) -> bool {
        self.item == o.item
    }
}

/// Открытый плеер.
#[derive(Clone)]
pub struct Playing {
    pub path: PathBuf,
    pub player: Arc<Mutex<VideoPlayer>>,
}

impl PartialEq for Playing {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.player, &o.player)
    }
}

#[derive(Clone, Copy)]
pub struct St {
    /// Ускорение из командной строки (`--hw`); `None` — по настройкам.
    pub hw: Option<HwAccel>,
    pub settings: RwSignal<Settings>,
    /// Как идёт воспроизведение открытого ролика (чип в шапке).
    pub info: RwSignal<Option<DecodeInfo>>,
    pub settings_open: RwSignal<bool>,
    /// Открыта панель выбора звуковой дорожки.
    pub tracks_open: RwSignal<bool>,
    /// Озвучка, выбранная в этом сеансе: следующие ролики открываются с ней.
    pub audio_pref: RwSignal<Option<crate::tracks::AudioPref>>,
    /// Настройки субтитров и оформление для слоя плеера (меняются вместе).
    pub subs: RwSignal<crate::subs::SubSettings>,
    pub sub_style: RwSignal<syngui::video::SubtitleStyle>,
    /// Открыта панель субтитров.
    pub subs_open: RwSignal<bool>,
    /// Открыта панель фильтров (DLSS 5 и нейросети).
    pub filters_open: RwSignal<bool>,
    /// Растёт, когда панель фильтров надо перестроить (включили фильтр, сравнение).
    pub filters_rev: RwSignal<u32>,
    /// Черта сравнения (доля ширины кадра) — пока её тянут, настройки не трогаются.
    pub split: RwSignal<f32>,
    /// Что из настроек нужно виджету плеера: «заполнить», шаг перемотки. Отдельно от настроек —
    /// ползунки фильтров не пересобирают плеер.
    pub view_key: RwSignal<(bool, u32)>,
    /// Дорожка субтитров, выбранная в этом сеансе (или «выключены»).
    pub sub_pref: RwSignal<Option<crate::subs::SubPref>>,
    /// Последняя включённая дорожка — её возвращает клавиша C.
    pub sub_last: RwSignal<Option<syngui::video::SubtitleSource>>,
    /// Растёт при смене дорожки и сдвига — перестроить панель и кнопку.
    pub sub_rev: RwSignal<u32>,
    /// Растёт при смене набора оформления — перестроить ползунки.
    pub sub_gen: RwSignal<u32>,
    /// Вопрос «освободить видеопамять от нейросетей?» (аппаратный декодер не запустился).
    pub vram: RwSignal<crate::vram::Vram>,
    /// Состояние DLSS 5 открытого ролика (чип в шапке, настройки).
    pub dlss: RwSignal<crate::dlss::Status>,
    /// Состояние увеличения разрешения нейросетью.
    pub sr: RwSignal<crate::neural::Status>,
    /// Состояние промежуточных кадров RIFE.
    pub rife: RwSignal<crate::interp::Status>,
    /// Состояние «плавности» без нейросетей.
    pub smooth: RwSignal<crate::smooth::Status>,
    pub cards: RwSignal<Vec<Card>>,
    /// Видео каталога открытого ролика: листание и лента миниатюр в плеере.
    pub siblings: RwSignal<Vec<Card>>,
    pub scanning: RwSignal<bool>,
    pub playing: RwSignal<Option<Playing>>,
    /// Файл открывается (поток открытия ещё не вернулся).
    pub opening: RwSignal<Option<PathBuf>>,
    pub error: RwSignal<Option<String>>,
    /// Во весь экран (выбор пользователя; следует за окном).
    pub fullscreen: RwSignal<bool>,
    /// Состояние окна (развёрнуто, во весь экран, в фокусе) — для своего заголовка.
    pub window: RwSignal<syngui::window::WindowState>,
    /// Программа запущена с файлом: «назад» из плеера закрывает окно, а не ведёт в библиотеку.
    pub direct: bool,
    /// Монтаж открытого ролика (ножницы в шапке плеера).
    pub edit: RwSignal<Option<crate::edit::Edit>>,
    /// Окно поверх экрана монтажа.
    pub sheet: RwSignal<crate::editor::Sheet>,
    /// Позиция плеера в секундах (от виджета плеера) — шкала кусков монтажа.
    pub pos: RwSignal<f32>,
    /// Плеер встроен в другую программу ([`crate::embed`]): её шапка, слои, серии и переоткрытие.
    pub host: Option<&'static dyn crate::embed::Host>,
}

/// Состояние программы (сигналы — в главном потоке, после запуска syngui).
pub fn new_state(hw: Option<HwAccel>, direct: bool, window: RwSignal<syngui::window::WindowState>, host: Option<&'static dyn crate::embed::Host>) -> St {
    let subs = crate::subs::SubSettings::load();
    let settings = Settings::load();
    let st = St {
        hw,
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
        vram: use_signal(crate::vram::Vram::None),
        dlss: use_signal(crate::dlss::Status::Off),
        sr: use_signal(crate::neural::Status::Off),
        rife: use_signal(crate::interp::Status::Off),
        smooth: use_signal(crate::smooth::Status::Off),
        cards: use_signal(Vec::new()),
        siblings: use_signal(Vec::new()),
        scanning: use_signal(host.is_none()),
        playing: use_signal(None),
        opening: use_signal(None),
        error: use_signal(None),
        fullscreen: use_signal(crate::chrome::start_fullscreen(&settings)),
        window,
        direct,
        edit: use_signal(None),
        sheet: use_signal(crate::editor::Sheet::None),
        pos: use_signal(0.0),
        host,
    };
    syngui::signal::create_detached_effect(move || crate::editor::follow(st));
    st
}

/// Закрыть открытую панель плеера (настройки, субтитры, дорожки, фильтры); `false` — их нет.
pub fn close_panels(st: St) -> bool {
    let mut any = false;
    for s in [st.settings_open, st.subs_open, st.tracks_open, st.filters_open] {
        if s.get_untracked() {
            s.set(false);
            any = true;
        }
    }
    any
}

pub fn root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        if st.edit.with(Option::is_some) {
            return vec![crate::editor::view(st)];
        }
        let playing = st.playing.get().is_some() || st.opening.get().is_some() || (st.direct && st.error.get().is_some());
        vec![if playing { crate::player::view(st) } else { library_view(st) }]
    });
    crate::chrome::frame(
        st,
        GestureDetector::new()
            .on_back(move || back(st))
            .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("root")).child(body)),
    )
}

/// «Назад»: из плеера — в библиотеку (или выход, если открыт файл из командной строки).
pub fn back(st: St) -> bool {
    if crate::editor::back(st) {
        return true;
    }
    if close_panels(st) {
        return true;
    }
    if st.playing.get_untracked().is_some() || st.opening.get_untracked().is_some() {
        close(st);
        if st.direct {
            syngui::signal::quit_app();
        }
        return true;
    }
    false
}

/// Ускорение по командной строке или настройкам.
fn hw_of(st: St) -> HwAccel {
    let hardware = st.settings.get_untracked().hardware;
    match st.host {
        Some(h) => st.hw.unwrap_or_else(|| h.hwaccel(hardware, crate::smooth::needs_cpu_frames(st))),
        None => st.hw.unwrap_or(if hardware { HwAccel::Auto } else { HwAccel::None }),
    }
}

pub fn open(st: St, path: PathBuf) {
    let start = if st.settings.get_untracked().resume { settings::saved_position(&path) } else { None };
    open_from(st, path, start);
}

/// Открыть ролик с секунды `start` (снимок `--at`) — со списком соседей каталога.
pub fn open_from(st: St, path: PathBuf, start: Option<f64>) {
    load_siblings(st, &path);
    open_at(st, path, start, false);
}

/// Перейти к другому ролику того же каталога, не выходя из плеера (полный экран остаётся).
pub fn go(st: St, path: PathBuf) {
    if current(st).as_ref() == Some(&path) {
        return;
    }
    let start = if st.settings.get_untracked().resume { settings::saved_position(&path) } else { None };
    open_at(st, path, start, true);
}

/// Соседний ролик каталога: `delta` −1 — предыдущий, +1 — следующий; по кругу.
pub fn step(st: St, delta: isize) {
    let list = st.siblings.get_untracked();
    let n = list.len() as isize;
    let Some(i) = current(st).and_then(|p| list.iter().position(|c| c.item.path == p)) else { return };
    if n < 2 {
        return;
    }
    go(st, list[(i as isize + delta).rem_euclid(n) as usize].item.path.clone());
}

/// Ролик `from` досмотрен — следующий ролик каталога; после последнего — ничего (без круга).
fn play_next(st: St, from: PathBuf) {
    let list = st.siblings.get_untracked();
    if let Some(next) = list.iter().position(|c| c.item.path == from).and_then(|i| list.get(i + 1)) {
        go(st, next.item.path.clone());
    }
}

/// Ролик, который открыт или открывается.
pub fn current(st: St) -> Option<PathBuf> {
    st.opening.get_untracked().or_else(|| st.playing.get_untracked().map(|p| p.path))
}

/// Видео каталога ролика — в фоне; миниатюры, уже загруженные библиотекой, берутся оттуда.
fn load_siblings(st: St, path: &std::path::Path) {
    if st.siblings.get_untracked().iter().any(|c| c.item.path == path) {
        return;
    }
    st.siblings.set(Vec::new());
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let items = library::siblings(&path);
        run_on_main_thread(move || {
            // пока читали каталог, открыли ролик из другого
            if current(st).is_some_and(|p| p.parent() != path.parent()) {
                return;
            }
            let known = st.cards.get_untracked();
            let cards: Vec<Card> = items
                .into_iter()
                .map(|item| {
                    let pic = known.iter().find(|c| c.item == item).map(|c| c.pic).unwrap_or_else(|| use_signal(None));
                    Card { item, pic }
                })
                .collect();
            let todo = cards.iter().filter(|c| c.pic.get_untracked().is_none()).map(|c| (c.item.clone(), c.pic)).collect();
            st.siblings.set(cards);
            load_thumbs(todo);
        });
    });
}

/// Открыть заново тот же ролик с того же места (сменилось декодирование).
pub fn reopen(st: St) {
    let Some(p) = st.playing.get_untracked() else { return };
    let pos = p.player.lock().map(|pl| pl.position_sec()).unwrap_or(0.0);
    reopen_at(st, &p.path, pos);
}

/// Открыть заново с секунды `pos`: у встроенного плеера — силами программы (свежие адреса потока).
fn reopen_at(st: St, path: &std::path::Path, pos: f64) {
    match st.host {
        Some(h) => h.reopen(pos),
        None => open_at(st, path.to_path_buf(), Some(pos), true),
    }
}

/// `keep_old`: старый плеер закрывается в потоке открытия до нового — у аппаратного
/// кодека общий лимит сессий (Venus: две сессии 1080p уже не помещаются).
pub fn open_at(st: St, path: PathBuf, start: Option<f64>, keep_old: bool) {
    let input = path.to_string_lossy().into_owned();
    open_with(st, path, start, keep_old, Box::new(move |hw| VideoPlayer::open_with_hwaccel(&input, hw).map_err(|e| e.to_string())));
}

/// Открыть ролик `path` (имя; у встроенного плеера — ключ программы) своим способом `opener`.
pub fn open_with(st: St, path: PathBuf, start: Option<f64>, keep_old: bool, opener: crate::embed::Opener) {
    let old = if keep_old { st.playing.get_untracked() } else { None };
    if keep_old {
        st.playing.set(None);
        st.opening.set(None);
    } else {
        close(st);
    }
    st.error.set(None);
    st.info.set(None);
    st.opening.set(Some(path.clone()));
    let hw = hw_of(st);
    std::thread::Builder::new()
        .name("svp-open".into())
        .spawn(move || {
            if let Some(o) = old {
                save_progress(st.host, &o);
                drop(o);
            }
            let r = opener(hw);
            run_on_main_thread(move || {
                // пока открывали, пользователь ушёл назад или перешёл к другому ролику
                if st.opening.get_untracked().as_ref() != Some(&path) {
                    if let Ok(p) = r {
                        std::thread::spawn(move || drop(p));
                    }
                    return;
                }
                st.opening.set(None);
                match r {
                    Ok(p) => {
                        let mut p = p;
                        if let Some(t) = start.filter(|t| *t > 1.0 && *t < p.duration_sec() - 1.0) {
                            let _ = p.seek(t);
                        }
                        crate::tracks::apply_pref(st, &mut p);
                        crate::subs::on_open(st, &path, &mut p);
                        crate::dlss::attach(st, &p);
                        crate::neural::attach(st, &p);
                        crate::interp::attach(st, &p);
                        crate::smooth::attach(st, &p);
                        if let Some(h) = st.host {
                            h.opened(&mut p);
                        }
                        p.play();
                        let playing = Playing { path, player: Arc::new(Mutex::new(p)) };
                        watch(st, &playing);
                        st.playing.set(Some(playing));
                    }
                    Err(e) => {
                        tracing::warn!("{}: {e}", path.display());
                        let text = match st.host {
                            Some(_) => t!("Не удалось открыть видео: {e}", e = e),
                            None => t!("Не удалось открыть «{path}»: {e}", path = path.display(), e = e),
                        };
                        st.error.set(Some(text));
                    }
                }
            });
        })
        .ok();
}

pub fn close(st: St) {
    st.opening.set(None);
    st.settings_open.set(false);
    st.tracks_open.set(false);
    st.subs_open.set(false);
    st.filters_open.set(false);
    if st.vram.get_untracked() != crate::vram::Vram::Freeing {
        st.vram.set(crate::vram::Vram::None);
    }
    st.info.set(None);
    // ролик закрыт — канал DLSS 5 играм; следующий ролик займёт его снова
    if st.dlss.get_untracked() != crate::dlss::Status::Off {
        std::thread::spawn(crate::dlss::shutdown);
    }
    if let Some(p) = st.playing.get_untracked() {
        st.playing.set(None);
        // остановка декодера ждёт его потоки — не в главном потоке
        let host = st.host;
        std::thread::spawn(move || {
            save_progress(host, &p);
            drop(p)
        });
    }
}

/// Запомнить, где остановились (если включено «продолжать с места остановки»).
fn save_progress(host: Option<&'static dyn crate::embed::Host>, p: &Playing) {
    let (pos, dur, ended) = match p.player.lock() {
        Ok(pl) => (pl.position_sec(), pl.duration_sec(), pl.is_ended()),
        Err(_) => return,
    };
    if let Some(h) = host {
        h.progress(pos, dur, true);
        return;
    }
    // ролик досмотрен — забыть
    let pos = if ended { dur } else { pos };
    settings::save_position(&p.path, pos, dur);
}

/// Поток оборвался посреди ролика (сеть: HLS после исчерпанных повторов сегмента дошёл до «конца»):
/// до настоящего конца больше этого — открыть заново, а не считать ролик досмотренным.
const ENDED_MARGIN: f64 = 60.0;
/// Столько буферизации подряд — поток завис (мёртвый узел CDN, сменилась сеть): открыть заново
/// с той же позиции, как сделал бы выход и повторный запуск.
const STALL_REOPEN: std::time::Duration = std::time::Duration::from_secs(12);

/// Конец потока раньше настоящего конца ролика — обрыв, а не досмотренный ролик.
fn broken_end(pos: f64, dur: f64, network: bool) -> bool {
    network && dur > 0.0 && pos < dur - ENDED_MARGIN
}

/// Сетевой адрес (у встроенного плеера ролики — потоки программы).
fn is_network(st: St, path: &std::path::Path) -> bool {
    st.host.is_some() || path.to_str().is_some_and(|s| s.contains("://"))
}

/// Ролик дошёл до конца (виджет плеера встал на паузу): повтор или следующий из каталога.
pub fn ended(st: St, playing: &Playing) {
    if current(st).as_ref() != Some(&playing.path) {
        return;
    }
    if let Some(h) = st.host {
        let (pos, dur) = playing.player.lock().map(|p| (p.position_sec(), p.duration_sec())).unwrap_or((0.0, 0.0));
        if broken_end(pos, dur, true) {
            tracing::warn!("поток оборвался на {pos:.1} с из {dur:.1} — открываю заново");
            h.reopen(pos);
        } else {
            h.ended();
        }
        return;
    }
    let s = st.settings.get_untracked();
    if broken_end(playing.player.lock().map(|p| p.position_sec()).unwrap_or(0.0), playing.player.lock().map(|p| p.duration_sec()).unwrap_or(0.0), is_network(st, &playing.path)) {
        reopen(st);
        return;
    }
    if s.repeat {
        if let Ok(mut p) = playing.player.lock() {
            let _ = p.seek(0.0);
            p.play();
        }
    } else if s.autoplay_next {
        play_next(st, playing.path.clone());
    }
}

/// Пока ролик открыт: сводка декодирования для чипа, место остановки раз в 5 с.
/// Плеер — по слабой ссылке: закрыли — наблюдатель заканчивается сам.
fn watch(st: St, playing: &Playing) {
    let weak = Arc::downgrade(&playing.player);
    let path = playing.path.clone();
    let host = st.host;
    let network = is_network(st, &path);
    std::thread::Builder::new()
        .name("svp-watch".into())
        .spawn(move || {
            let mut last: Option<DecodeInfo> = None;
            let mut tick = 0u32;
            let mut stall_since: Option<std::time::Instant> = None;
            let mut recovered = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let Some(player) = weak.upgrade() else { break };
                let s = st_settings(st);
                let (info, pos, dur, stalled) = match player.lock() {
                    Ok(p) => (p.decode_info(), p.position_sec(), p.duration_sec(), p.is_buffering() && !p.is_paused()),
                    Err(_) => break,
                };
                drop(player);
                // Зависший сетевой поток — открыть заново с того же места (один раз на ролик).
                if network && stalled {
                    let since = *stall_since.get_or_insert_with(std::time::Instant::now);
                    if !recovered && since.elapsed() >= STALL_REOPEN {
                        recovered = true;
                        tracing::warn!("поток завис на {pos:.1} с — открываю заново");
                        let path = path.clone();
                        run_on_main_thread(move || {
                            if current(st).as_ref() == Some(&path) {
                                reopen_at(st, &path, pos);
                            }
                        });
                    }
                } else {
                    stall_since = None;
                }
                if last.as_ref() != Some(&info) {
                    if let (Some(reason), None, true) = (&info.hw_fallback, &info.hardware, info.known) {
                        if last.as_ref().is_none_or(|l| l.hw_fallback.is_none() || !l.known) {
                            crate::vram::on_fallback(st, &path, reason.clone());
                        }
                    }
                    last = Some(info.clone());
                    run_on_main_thread(move || st.info.set(Some(info)));
                }
                crate::dlss::poll(st);
                crate::neural::poll(st);
                crate::interp::poll(st);
                crate::smooth::poll(st);
                tick += 1;
                if tick % 10 == 0 {
                    match host {
                        Some(h) => h.progress(pos, dur, false),
                        None if s.resume => settings::save_position(&path, pos, dur),
                        None => {}
                    }
                }
            }
        })
        .ok();
}

/// Настройки из фонового потока (сигнал читается в главном).
fn st_settings(st: St) -> Settings {
    let (tx, rx) = std::sync::mpsc::channel();
    run_on_main_thread(move || {
        let _ = tx.send(st.settings.get_untracked());
    });
    rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap_or_default()
}

// ─── библиотека ─────────────────────────────────────────────────────────────

pub fn rescan(st: St) {
    st.scanning.set(true);
    std::thread::spawn(move || {
        let items = library::scan();
        run_on_main_thread(move || {
            let old = st.cards.get_untracked();
            let cards: Vec<Card> = items
                .into_iter()
                .map(|item| {
                    let pic = old.iter().find(|c| c.item == item).map(|c| c.pic).unwrap_or_else(|| use_signal(None));
                    Card { item, pic }
                })
                .collect();
            let todo: Vec<(Item, RwSignal<Option<Arc<Pic>>>)> =
                cards.iter().filter(|c| c.pic.get_untracked().is_none()).map(|c| (c.item.clone(), c.pic)).collect();
            st.cards.set(cards);
            st.scanning.set(false);
            load_thumbs(todo);
        });
    });
}

/// Миниатюры по одной в фоне, по порядку карточек.
fn load_thumbs(todo: Vec<(Item, RwSignal<Option<Arc<Pic>>>)>) {
    if todo.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("svp-thumbs".into())
        .spawn(move || {
            for (item, sig) in todo {
                if let Some((w, h, rgba, duration)) = library::thumb(&item, 480) {
                    let pic = Arc::new(Pic { w, h, rgba: Arc::new(rgba), duration });
                    run_on_main_thread(move || sig.set(Some(pic)));
                }
            }
        })
        .ok();
}

fn library_view(st: St) -> W {
    // шапка библиотеки — она же заголовок окна
    let header = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(crate::chrome::drag(Text::new(t!("Видео")).max_lines(1).elide(Elide::End).class("title")))
        .child(icon_button(icons::REFRESH, &t!("Обновить"), move || rescan(st)))
        .child(crate::chrome::fullscreen_button(st, false))
        .child(crate::chrome::controls(st, "window-controls"))
        .class("header");
    let grid = Reactive::new(move || -> Vec<W> {
        let cards = st.cards.get();
        let vp = viewport_size().get();
        if cards.is_empty() {
            let text = if st.scanning.get() {
                t!("Ищу видео…").to_string()
            } else {
                let dirs: Vec<String> = library::roots().iter().map(|d| d.display().to_string()).collect();
                t!("Видео не найдено.\nПрограмма ищет в: {v}", v = dirs.join(", "))
            };
            return vec![Box::new(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::VIDEO_LIBRARY).class("empty-icon"))
                    .child(Text::new(text).max_lines(4).class("empty-text"))
                    .class("empty"),
            )];
        }
        // Столбцы: ~220 px на карточку, не меньше двух.
        let cols = ((vp.width - 16.0) / 220.0).floor().clamp(2.0, 8.0) as usize;
        let grid = Grid::new(cols).gap(12.0).children(cards.into_iter().map(|c| card(st, c)));
        vec![Box::new(ScrollView::new().vertical().child(DecoratedBox::new().child(grid).class("grid-pad")).class("grow"))]
    });
    let error = Reactive::new(move || -> Vec<W> {
        match st.error.get() {
            Some(e) => vec![Box::new(
                GestureDetector::new()
                    .on_click(move || st.error.set(None))
                    .child(DecoratedBox::new().child(Text::new(e).max_lines(4).class("error-text")).class("error")),
            )],
            None => vec![],
        }
    });
    Box::new(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(header).child(error).child(grid).class("grow"))
}

fn card(st: St, c: Card) -> W {
    let item = c.item.clone();
    let pic = c.pic;
    let key = format!("svp:{}", c.item.path.display());
    let picture = Reactive::new(move || -> Vec<W> {
        let mut layers = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("pic-bg"));
        if let Some(p) = pic.get() {
            layers = layers.child(Image::from_rgba_shared(key.clone(), p.w, p.h, p.rgba.clone()).fit(ImageFit::Cover).class("pic"));
            if p.duration > 0.0 {
                layers = layers.child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::End)
                        .child(DecoratedBox::new().child(Text::new(library::format_time(p.duration)).class("dur-text")).class("dur"))
                        .class("dur-place"),
                );
            }
        } else {
            layers = layers.child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::MOVIE).class("pic-icon")),
            );
        }
        vec![Box::new(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().child(layers).class("pic-box")))]
    });
    let path = item.path.clone();
    Box::new(
        GestureDetector::new()
            .on_click(move || open(st, path.clone()))
            .cursor(syngui::CursorIcon::Pointer)
            .child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(picture)
                    .child(Text::new(item.name()).max_lines(2).class("card-title"))
                    .class("card"),
            ),
    )
}

pub fn icon_button(icon: &'static str, tip: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        GestureDetector::new()
            .on_click(f)
            .cursor(syngui::CursorIcon::Pointer)
            .child(Tooltip::new(DecoratedBox::new().child(Icon::new(icon).class("ib-icon")).class("ib"), tip)),
    )
}

pub mod icons {
    pub const REFRESH: &str = "\u{E5D5}";
    pub const VIDEO_LIBRARY: &str = "\u{E04A}";
    pub const MOVIE: &str = "\u{E02C}";
    pub const ARROW_BACK: &str = "\u{E5C4}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const CLOSE: &str = "\u{E5CD}";
    /// chip: аппаратный кодек
    pub const MEMORY: &str = "\u{E322}";
    /// developer_board: процессор
    pub const CPU: &str = "\u{E30D}";
    pub const HOURGLASS: &str = "\u{E88B}";
    pub const ASPECT: &str = "\u{E85B}";
    pub const FIT: &str = "\u{E3C2}";
    pub const FILL: &str = "\u{E56B}";
    pub const FAST_FORWARD: &str = "\u{E01F}";
    pub const PLAY_CIRCLE: &str = "\u{E1C4}";
    pub const INFO: &str = "\u{E88E}";
    /// content_cut: монтаж
    pub const CUT: &str = "\u{E14E}";
    pub const UNDO: &str = "\u{E166}";
    pub const SAVE: &str = "\u{E161}";
    pub const DELETE: &str = "\u{E872}";
    /// restore_from_trash
    pub const RESTORE: &str = "\u{E938}";
    pub const LEFT: &str = "\u{E5CB}";
    pub const RIGHT: &str = "\u{E5CC}";
    pub const ADD: &str = "\u{E145}";
    pub const PLAY: &str = "\u{E037}";
    /// record_voice_over: озвучка
    pub const VOICE: &str = "\u{E91F}";
    pub const RADIO_ON: &str = "\u{E837}";
    pub const RADIO_OFF: &str = "\u{E836}";
    /// auto_awesome: DLSS 5
    pub const DLSS: &str = "\u{E65F}";
    /// tune — кнопка фильтров
    pub const TUNE: &str = "\u{E429}";
    pub const COMPARE: &str = "\u{E3B9}";
    pub const SPEED: &str = "\u{E9E4}";
    /// hd — увеличение разрешения
    pub const SR: &str = "\u{E052}";
    /// slow_motion_video — плавность
    pub const SMOOTH: &str = "\u{E068}";
    pub const OPEN_IN_NEW: &str = "\u{E89E}";
    /// subtitles
    pub const SUBTITLES: &str = "\u{E048}";
    pub const SYNC: &str = "\u{E627}";
    pub const PALETTE: &str = "\u{E40A}";
    /// auto_mode → hdr_auto нет в шрифте: brightness_auto
    pub const AUTO: &str = "\u{E1AB}";
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const FULLSCREEN_EXIT: &str = "\u{E5D1}";
}
