//! Экраны: библиотека (список треков, внизу — «сейчас играет») и плеер во весь экран (обложка, позиция,
//! управление). Очередь — порядок библиотеки (или файлы из командной строки); по концу трека — следующий.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::video::AudioFilePlayer;
use syngui::widgets::*;

use crate::library::{self, format_time, Track};

type W = Box<dyn Widget>;

/// Открытый плеер.
#[derive(Clone)]
pub struct Shared(pub Arc<Mutex<AudioFilePlayer>>);

impl PartialEq for Shared {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    /// По очереди, в конце — стоп.
    Off,
    /// Очередь по кругу.
    All,
    /// Один трек.
    One,
}

#[derive(Clone, Copy)]
pub struct St {
    pub tracks: RwSignal<Vec<Track>>,
    pub scanning: RwSignal<bool>,
    pub current: RwSignal<Option<PathBuf>>,
    pub player: RwSignal<Option<Shared>>,
    pub cover: RwSignal<Option<Arc<Vec<u8>>>>,
    pub pos: RwSignal<f64>,
    pub dur: RwSignal<f64>,
    pub paused: RwSignal<bool>,
    pub opening: RwSignal<bool>,
    pub full: RwSignal<bool>,
    pub repeat: RwSignal<Repeat>,
    pub error: RwSignal<Option<String>>,
    /// Запущена с файлами: очередь — они, библиотека не сканируется.
    pub direct: bool,
}

/// Перетаскивание ползунка: пока поднят, «тик» не перезаписывает позицию; поколение — для отложенной перемотки.
static SEEKING: AtomicBool = AtomicBool::new(false);
static SEEK_GEN: AtomicU64 = AtomicU64::new(0);
/// Поколение открытия трека: ответ устаревшего открытия отбрасывается.
static OPEN_GEN: AtomicU64 = AtomicU64::new(0);

pub fn root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> { vec![if st.full.get() { player_view(st) } else { library_view(st) }] });
    Box::new(
        GestureDetector::new()
            .on_back(move || {
                if st.full.get_untracked() {
                    st.full.set(false);
                    return true;
                }
                false
            })
            .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("root")).child(body)),
    )
}

// ─── воспроизведение ───────────────────────────────────────────────────────

pub fn play_path(st: St, path: PathBuf) {
    let gen = OPEN_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(old) = st.player.get_untracked() {
        st.player.set(None);
        std::thread::spawn(move || drop(old));
    }
    st.current.set(Some(path.clone()));
    st.opening.set(true);
    st.error.set(None);
    st.pos.set(0.0);
    st.cover.set(None);
    std::thread::Builder::new()
        .name("sap-open".into())
        .spawn(move || {
            let r = AudioFilePlayer::open(&path.to_string_lossy());
            run_on_main_thread(move || {
                if OPEN_GEN.load(Ordering::Acquire) != gen {
                    if let Ok(p) = r {
                        std::thread::spawn(move || drop(p));
                    }
                    return;
                }
                st.opening.set(false);
                match r {
                    Ok(p) => {
                        let m = p.meta().clone();
                        st.dur.set(m.duration_sec);
                        st.cover.set(m.cover.map(Arc::new));
                        st.paused.set(false);
                        // теги трека — в список (если ещё не прочитаны)
                        st.tracks.update(|v| {
                            if let Some(t) = v.iter_mut().find(|t| t.path == path) {
                                if let Some(title) = m.title.clone() {
                                    t.title = title;
                                }
                                t.artist = m.artist.clone().or(t.artist.take());
                                t.album = m.album.clone().or(t.album.take());
                                t.duration = m.duration_sec;
                                t.probed = true;
                            }
                        });
                        st.player.set(Some(Shared(Arc::new(Mutex::new(p)))));
                    }
                    Err(e) => {
                        tracing::warn!("{}: {e}", path.display());
                        st.error.set(Some(t!("Не удалось открыть «{path}»: {e}", path = path.display(), e = e)));
                    }
                }
            });
        })
        .ok();
}

fn index_of_current(st: St) -> Option<usize> {
    let cur = st.current.get_untracked()?;
    st.tracks.get_untracked().iter().position(|t| t.path == cur)
}

pub fn next(st: St, auto: bool) {
    let tracks = st.tracks.get_untracked();
    if tracks.is_empty() {
        return;
    }
    let i = index_of_current(st);
    let n = match (i, st.repeat.get_untracked()) {
        (Some(i), Repeat::One) if auto => i,
        (Some(i), _) if i + 1 < tracks.len() => i + 1,
        (Some(_), Repeat::Off) if auto => {
            // конец очереди: остановиться на последнем
            st.paused.set(true);
            return;
        }
        (Some(_), _) => 0,
        (None, _) => 0,
    };
    play_path(st, tracks[n].path.clone());
}

pub fn prev(st: St) {
    // дальше 3 с от начала — в начало трека
    if st.pos.get_untracked() > 3.0 {
        seek_now(st, 0.0);
        return;
    }
    let tracks = st.tracks.get_untracked();
    if tracks.is_empty() {
        return;
    }
    let n = match index_of_current(st) {
        Some(0) | None => tracks.len() - 1,
        Some(i) => i - 1,
    };
    play_path(st, tracks[n].path.clone());
}

pub fn toggle(st: St) {
    if let Some(p) = st.player.get_untracked() {
        let mut g = p.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_ended() {
            drop(g);
            seek_now(st, 0.0);
            return;
        }
        if g.is_paused() {
            g.play();
        } else {
            g.pause();
        }
        st.paused.set(g.is_paused());
    } else if let Some(t) = st.tracks.get_untracked().first() {
        play_path(st, st.current.get_untracked().unwrap_or_else(|| t.path.clone()));
    }
}

fn seek_now(st: St, sec: f64) {
    if let Some(p) = st.player.get_untracked() {
        let mut g = p.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = g.seek(sec) {
            tracing::warn!("перемотка: {e}");
        }
        g.play();
        st.paused.set(false);
    }
    st.pos.set(sec);
}

/// Ползунок: позиция сразу на экране, перемотка — через 250 мс после последнего сдвига (каждая перемотка
/// перезапускает декодер).
fn seek_debounced(st: St, sec: f64) {
    SEEKING.store(true, Ordering::Release);
    st.pos.set(sec);
    let gen = SEEK_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        if SEEK_GEN.load(Ordering::Acquire) != gen {
            return;
        }
        run_on_main_thread(move || {
            seek_now(st, sec);
            SEEKING.store(false, Ordering::Release);
        });
    });
}

/// Раз в 250 мс: позиция, пауза, конец трека → следующий.
pub fn start_ticker(st: St) {
    std::thread::Builder::new()
        .name("sap-tick".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            run_on_main_thread(move || {
                let Some(p) = st.player.get_untracked() else { return };
                let (pos, ended, paused) = {
                    let g = p.0.lock().unwrap_or_else(|e| e.into_inner());
                    (g.position_sec(), g.is_ended(), g.is_paused())
                };
                if !SEEKING.load(Ordering::Acquire) && (pos - st.pos.get_untracked()).abs() > 0.2 {
                    st.pos.set(pos);
                }
                if st.paused.get_untracked() != paused {
                    st.paused.set(paused);
                }
                if ended && !paused && !st.opening.get_untracked() {
                    next(st, true);
                }
            });
        })
        .ok();
}

// ─── библиотека ─────────────────────────────────────────────────────────────

pub fn rescan(st: St) {
    st.scanning.set(true);
    std::thread::spawn(move || {
        let tracks = library::scan();
        let paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
        run_on_main_thread(move || {
            st.tracks.set(tracks);
            st.scanning.set(false);
        });
        probe_all(st, paths);
    });
}

/// Теги по одному в фоне; в список — пачками, в конце — сортировка.
pub fn probe_all(st: St, paths: Vec<PathBuf>) {
    std::thread::Builder::new()
        .name("sap-probe".into())
        .spawn(move || {
            let mut batch: Vec<Track> = Vec::new();
            let total = paths.len();
            for (i, p) in paths.into_iter().enumerate() {
                let mut t = Track::from_path(p);
                t.probe();
                batch.push(t);
                if batch.len() >= 25 || i + 1 == total {
                    let b = std::mem::take(&mut batch);
                    let last = i + 1 == total;
                    run_on_main_thread(move || {
                        st.tracks.update(|v| {
                            for t in b {
                                if let Some(slot) = v.iter_mut().find(|x| x.path == t.path) {
                                    *slot = t;
                                }
                            }
                            if last && !st.direct {
                                library::sort(v);
                            }
                        });
                    });
                }
            }
        })
        .ok();
}

fn library_view(st: St) -> W {
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(if st.direct { t!("Очередь") } else { t!("Музыка") }).class("title"))
        .child(DecoratedBox::new().class("grow"))
        .child(icon_button(icons::REFRESH, &t!("Обновить"), move || rescan(st)))
        .class("header");
    let list = Reactive::new(move || -> Vec<W> {
        let tracks = st.tracks.get();
        let cur = st.current.get();
        if tracks.is_empty() {
            let text = if st.scanning.get() {
                t!("Ищу музыку…").to_string()
            } else {
                let dirs: Vec<String> = library::roots().iter().map(|d| d.display().to_string()).collect();
                t!("Музыка не найдена.\nПрограмма ищет в: {v}", v = dirs.join(", "))
            };
            return vec![Box::new(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::LIBRARY_MUSIC).class("empty-icon"))
                    .child(Text::new(text).max_lines(4).class("empty-text"))
                    .class("empty"),
            )];
        }
        let rows = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(tracks.into_iter().map(|t| {
                let active = cur.as_ref() == Some(&t.path);
                row(st, t, active)
            }))
            .class("list");
        vec![Box::new(ScrollView::new().vertical().child(rows).class("grow"))]
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
    let bar = Reactive::new(move || -> Vec<W> {
        if st.current.get().is_none() {
            return vec![];
        }
        vec![mini_bar(st)]
    });
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(error)
            .child(DecoratedBox::new().child(list).class("grow"))
            .child(bar)
            .class("grow"),
    )
}

fn row(st: St, t: Track, active: bool) -> W {
    let path = t.path.clone();
    let sub = t.subtitle();
    let mut text = Column::new().gap(2.0).child(Text::new(t.title.clone()).max_lines(1).class(if active { "row-title active" } else { "row-title" }));
    if !sub.is_empty() {
        text = text.child(Text::new(sub).max_lines(1).class("row-sub"));
    }
    let mut r = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(if active { icons::EQUALIZER } else { icons::MUSIC_NOTE }).class(if active { "row-icon active" } else { "row-icon" })).class("row-icon-box"))
        .child(DecoratedBox::new().child(text).class("grow"));
    if t.duration > 0.0 {
        r = r.child(Text::new(format_time(t.duration)).class("row-dur"));
    }
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                if st.current.get_untracked().as_ref() == Some(&path) && st.player.get_untracked().is_some() {
                    st.full.set(true);
                } else {
                    play_path(st, path.clone());
                }
            })
            .cursor(syngui::CursorIcon::Pointer)
            .child(DecoratedBox::new().child(r).class(if active { "row row-active" } else { "row" })),
    )
}

fn current_track(st: St) -> Option<Track> {
    let cur = st.current.get()?;
    st.tracks.get().into_iter().find(|t| t.path == cur).or_else(|| Some(Track::from_path(cur)))
}

fn cover_widget(st: St, class: &'static str, icon_class: &'static str) -> W {
    // заглушка — прямой слой Stack (растянут на всю обложку, центрируется), картинка — поверх
    let placeholder = Column::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icons::ALBUM).class(icon_class));
    let pic = Reactive::new(move || -> Vec<W> {
        let key = st.current.get().map(|p| format!("sap:{}", p.display())).unwrap_or_default();
        match st.cover.get() {
            Some(c) => vec![Box::new(Image::from_bytes(key, (*c).clone()).fit(ImageFit::Cover).class("cover-img"))],
            None => vec![],
        }
    });
    Box::new(
        DecoratedBox::new()
            .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("cover-bg")).child(placeholder).child(pic))
            .class(class),
    )
}

fn mini_bar(st: St) -> W {
    let info = Reactive::new(move || -> Vec<W> {
        let t = current_track(st).unwrap_or_default();
        let mut c = Column::new().gap(2.0).child(Text::new(t.title.clone()).max_lines(1).class("bar-title"));
        let sub = t.subtitle();
        if !sub.is_empty() {
            c = c.child(Text::new(sub).max_lines(1).class("row-sub"));
        }
        vec![Box::new(c)]
    });
    let play = Reactive::new(move || -> Vec<W> {
        let icon = if st.opening.get() { icons::HOURGLASS } else if st.paused.get() || st.player.get().is_none() { icons::PLAY } else { icons::PAUSE };
        vec![icon_button(icon, &t!("Играть / пауза"), move || toggle(st))]
    });
    let progress = Reactive::new(move || -> Vec<W> {
        let d = st.dur.get();
        let f = if d > 0.0 { (st.pos.get() / d).clamp(0.0, 1.0) as f32 } else { 0.0 };
        vec![Box::new(ProgressBar::new().value(f).class("bar-progress"))]
    });
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(progress)
            .child(
                GestureDetector::new().on_click(move || st.full.set(true)).cursor(syngui::CursorIcon::Pointer).child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(cover_widget(st, "bar-cover", "bar-cover-icon"))
                        .child(DecoratedBox::new().child(info).class("grow"))
                        .child(play)
                        .child(icon_button(icons::NEXT, &t!("Следующий"), move || next(st, false)))
                        .class("bar"),
                ),
            )
            .class("bar-wrap"),
    )
}

fn player_view(st: St) -> W {
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_button(icons::EXPAND_MORE, &t!("К списку"), move || st.full.set(false)))
        .child(Text::new(t!("Сейчас играет")).class("now"))
        .child(DecoratedBox::new().class("grow"))
        .class("header");
    let info = Reactive::new(move || -> Vec<W> {
        let t = current_track(st).unwrap_or_default();
        let mut c = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(t.title.clone()).max_lines(2).class("big-title"));
        let sub = t.subtitle();
        if !sub.is_empty() {
            c = c.child(Text::new(sub).max_lines(2).class("big-sub"));
        }
        vec![Box::new(c)]
    });
    let slider = Reactive::new(move || -> Vec<W> {
        let d = st.dur.get().max(0.0);
        let p = st.pos.get().clamp(0.0, d.max(0.0));
        vec![Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Slider::new()
                        .range(0.0, d.max(0.001) as f32)
                        .value(p as f32)
                        .disabled(d <= 0.0)
                        .on_change(move |v| seek_debounced(st, v as f64))
                        .class("seek"),
                )
                .child(
                    Row::new()
                        .child(Text::new(format_time(p)).class("time"))
                        .child(DecoratedBox::new().class("grow"))
                        .child(Text::new(format_time(d)).class("time")),
                ),
        )]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let rep = st.repeat.get();
        let (rep_icon, rep_class) = match rep {
            Repeat::Off => (icons::REPEAT, "ctl-icon dim"),
            Repeat::All => (icons::REPEAT, "ctl-icon active"),
            Repeat::One => (icons::REPEAT_ONE, "ctl-icon active"),
        };
        let playing = !(st.paused.get() || st.player.get().is_none());
        let main_icon = if st.opening.get() { icons::HOURGLASS } else if playing { icons::PAUSE } else { icons::PLAY };
        vec![Box::new(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ctl_button(rep_icon, rep_class, "ctl", &t!("Повтор"), move || {
                    st.repeat.set(match st.repeat.get_untracked() {
                        Repeat::Off => Repeat::All,
                        Repeat::All => Repeat::One,
                        Repeat::One => Repeat::Off,
                    })
                }))
                .child(ctl_button(icons::PREV, "ctl-icon", "ctl", &t!("Предыдущий"), move || prev(st)))
                .child(ctl_button(main_icon, "ctl-main-icon", "ctl-main", &t!("Играть / пауза"), move || toggle(st)))
                .child(ctl_button(icons::NEXT, "ctl-icon", "ctl", &t!("Следующий"), move || next(st, false)))
                .child(DecoratedBox::new().class("ctl")),
        )]
    });
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(
                Column::new()
                    .gap(18.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().child(AspectRatio::new(1.0).child(cover_widget(st, "big-cover", "big-cover-icon"))).class("big-cover-box"))
                    .child(info)
                    .class("player-body"),
            )
            .child(DecoratedBox::new().child(slider).class("seek-box"))
            .child(DecoratedBox::new().child(controls).class("controls"))
            .class("grow"),
    )
}

fn ctl_button(icon: &'static str, icon_class: &'static str, class: &'static str, tip: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        GestureDetector::new()
            .on_click(f)
            .cursor(syngui::CursorIcon::Pointer)
            .child(Tooltip::new(DecoratedBox::new().child(Icon::new(icon).class(icon_class)).class(class), tip)),
    )
}

pub fn icon_button(icon: &'static str, tip: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    ctl_button(icon, "ib-icon", "ib", tip, f)
}

pub mod icons {
    pub const REFRESH: &str = "\u{E5D5}";
    pub const LIBRARY_MUSIC: &str = "\u{E030}";
    pub const MUSIC_NOTE: &str = "\u{E405}";
    pub const EQUALIZER: &str = "\u{E01D}";
    pub const ALBUM: &str = "\u{E019}";
    pub const PLAY: &str = "\u{E037}";
    pub const PAUSE: &str = "\u{E034}";
    pub const NEXT: &str = "\u{E044}";
    pub const PREV: &str = "\u{E045}";
    pub const REPEAT: &str = "\u{E040}";
    pub const REPEAT_ONE: &str = "\u{E041}";
    pub const EXPAND_MORE: &str = "\u{E5CF}";
    pub const HOURGLASS: &str = "\u{E88B}";
}
