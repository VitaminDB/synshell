//! Экраны: библиотека (сетка карточек; столбцов — по ширине окна, от телефона в портрете до монитора)
//! и плеер во весь экран (виджет плеера syngui).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widgets::*;

use crate::library::{self, Item};

type W = Box<dyn Widget>;

/// Миниатюра карточки.
#[derive(PartialEq)]
pub struct Pic {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
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
    pub hw: HwAccel,
    pub cards: RwSignal<Vec<Card>>,
    pub scanning: RwSignal<bool>,
    pub playing: RwSignal<Option<Playing>>,
    /// Файл открывается (поток открытия ещё не вернулся).
    pub opening: RwSignal<Option<String>>,
    pub error: RwSignal<Option<String>>,
    pub fullscreen: RwSignal<bool>,
    /// Программа запущена с файлом: «назад» из плеера закрывает окно, а не ведёт в библиотеку.
    pub direct: bool,
}

pub fn root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let playing = st.playing.get().is_some() || st.opening.get().is_some() || (st.direct && st.error.get().is_some());
        vec![if playing { crate::player::view(st) } else { library_view(st) }]
    });
    Box::new(
        GestureDetector::new()
            .on_back(move || back(st))
            .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("root")).child(body)),
    )
}

/// «Назад»: из плеера — в библиотеку (или выход, если открыт файл из командной строки).
pub fn back(st: St) -> bool {
    if st.playing.get_untracked().is_some() || st.opening.get_untracked().is_some() {
        close(st);
        if st.direct {
            syngui::signal::quit_app();
        }
        return true;
    }
    false
}

pub fn open(st: St, path: PathBuf) {
    close(st);
    st.error.set(None);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    st.opening.set(Some(name));
    let hw = st.hw;
    std::thread::Builder::new()
        .name("svp-open".into())
        .spawn(move || {
            let r = VideoPlayer::open_with_hwaccel(&path.to_string_lossy(), hw);
            run_on_main_thread(move || {
                // пока открывали, пользователь ушёл назад
                if st.opening.get_untracked().is_none() {
                    return;
                }
                st.opening.set(None);
                match r {
                    Ok(p) => {
                        let mut p = p;
                        p.play();
                        st.playing.set(Some(Playing { path, player: Arc::new(Mutex::new(p)) }));
                    }
                    Err(e) => {
                        tracing::warn!("{}: {e}", path.display());
                        st.error.set(Some(format!("Не удалось открыть «{}»: {e}", path.display())));
                    }
                }
            });
        })
        .ok();
}

pub fn close(st: St) {
    st.opening.set(None);
    if let Some(p) = st.playing.get_untracked() {
        st.playing.set(None);
        // остановка декодера ждёт его потоки — не в главном потоке
        std::thread::spawn(move || drop(p));
    }
    if st.fullscreen.get_untracked() {
        st.fullscreen.set(false);
        syngui::signal::set_fullscreen(false);
    }
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
                    let pic = Arc::new(Pic { w, h, rgba, duration });
                    run_on_main_thread(move || sig.set(Some(pic)));
                }
            }
        })
        .ok();
}

fn library_view(st: St) -> W {
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Видео").class("title"))
        .child(DecoratedBox::new().class("grow"))
        .child(icon_button(icons::REFRESH, "Обновить", move || rescan(st)))
        .class("header");
    let grid = Reactive::new(move || -> Vec<W> {
        let cards = st.cards.get();
        let vp = viewport_size().get();
        if cards.is_empty() {
            let text = if st.scanning.get() {
                "Ищу видео…".to_string()
            } else {
                let dirs: Vec<String> = library::roots().iter().map(|d| d.display().to_string()).collect();
                format!("Видео не найдено.\nПрограмма ищет в: {}", dirs.join(", "))
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
            layers = layers.child(Image::from_rgba(key.clone(), p.w, p.h, p.rgba.clone()).fit(ImageFit::Cover).class("pic"));
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
}
