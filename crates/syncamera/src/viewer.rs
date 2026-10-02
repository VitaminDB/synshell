//! Просмотр снятого: снимок — масштаб пальцами (ImageViewport), видео — проигрыватель FFmpeg; листание
//! смахиванием и стрелками, удаление с подтверждением, открыть в другой программе, сведения о файле.

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::video::VideoPlayer;
use syngui::widgets::{ImageViewport, SwipeDirection, VideoView};
use syngui::GestureDetector;

use crate::media::{self, Item, Kind};
use crate::ui::{gl, W};
use crate::St;

thread_local! {
    static PLAYER: std::cell::RefCell<Option<(String, Arc<Mutex<VideoPlayer>>)>> = const { std::cell::RefCell::new(None) };
}

/// 1 — просмотр открыт (для ShowIf).
pub fn open_index(st: St) -> RwSignal<usize> {
    let s = use_signal(0usize);
    create_effect(move || {
        let open = st.viewer.get().is_some();
        s.set(open as usize);
        if !open {
            PLAYER.with(|p| *p.borrow_mut() = None);
        }
    });
    s
}

fn step(st: St, d: isize) {
    let n = st.items.get_untracked().len();
    if let Some(i) = st.viewer.get_untracked() {
        let j = i as isize + d;
        if j >= 0 && (j as usize) < n {
            st.viewer.set(Some(j as usize));
        }
    }
}

fn player_for(path: &str) -> Option<Arc<Mutex<VideoPlayer>>> {
    PLAYER.with(|p| {
        let mut p = p.borrow_mut();
        if let Some((cur, pl)) = p.as_ref() {
            if cur == path {
                return Some(pl.clone());
            }
        }
        *p = None;
        match VideoPlayer::open(path) {
            Ok(pl) => {
                let pl = Arc::new(Mutex::new(pl));
                *p = Some((path.to_string(), pl.clone()));
                Some(pl)
            }
            Err(e) => {
                tracing::warn!("видео {path}: {e}");
                None
            }
        }
    })
}

fn date_text(it: &Item) -> String {
    let secs = it.mtime.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // SAFETY: localtime_r в живую структуру
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = secs as libc::time_t;
        libc::localtime_r(&t, &mut tm);
        tm
    };
    const MONTHS: [&str; 12] = ["янв.", "февр.", "марта", "апр.", "мая", "июня", "июля", "авг.", "сент.", "окт.", "нояб.", "дек."];
    format!("{} {} {}, {:02}:{:02}", tm.tm_mday, MONTHS[tm.tm_mon.clamp(0, 11) as usize], tm.tm_year + 1900, tm.tm_hour, tm.tm_min)
}

fn size_text(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} МБ", b as f64 / (1u64 << 20) as f64).replace('.', ",")
    } else {
        format!("{} КБ", b.div_ceil(1024))
    }
}

pub fn view(st: St) -> W {
    let confirm = use_signal(false);
    let info = use_signal(false);
    let pos = use_signal(0.0f32);
    let paused = use_signal(false);
    // смена снимка — сброс подтверждения и сведений
    create_effect(move || {
        let _ = st.viewer.get();
        confirm.set(false);
        paused.set(false);
    });
    let content = Reactive::new(move || -> Vec<W> {
        let (Some(i), items) = (st.viewer.get(), st.items.get()) else { return vec![] };
        let Some(it) = items.get(i).cloned() else { return vec![] };
        let path = it.path.to_string_lossy().into_owned();
        let body: W = match it.kind {
            Kind::Photo => Box::new(ImageViewport::new(path).insets(64.0, 0.0, 72.0, 0.0).class("viewer-img")),
            Kind::Video => match player_for(&path) {
                Some(pl) => video_view(pl, pos, paused),
                None => Box::new(Column::new().center().child(Text::new("Видео не открывается").class("viewer-note"))),
            },
        };
        vec![Box::new(
            GestureDetector::new()
                .pan_axis(syngui::widgets::PanAxis::Horizontal)
                .on_swipe(move |d, _| match d {
                    SwipeDirection::Left => step(st, 1),
                    SwipeDirection::Right => step(st, -1),
                    _ => {}
                })
                .child(Stack::new().fit(StackFit::Expand).child(body)),
        )]
    });
    let top = Reactive::new(move || -> Vec<W> {
        let (Some(i), items) = (st.viewer.get(), st.items.get()) else { return vec![] };
        let Some(it) = items.get(i).cloned() else { return vec![] };
        let p_open = it.path.clone();
        vec![Box::new(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ib(gl::BACK, move || st.viewer.set(None)))
                .child(
                    Column::new()
                        .child(Text::new(date_text(&it)).max_lines(1).class("viewer-title"))
                        .child(Text::new(format!("{} из {}", i + 1, items.len())).class("viewer-sub"))
                        .class("grow"),
                )
                .child(ib(gl::INFO, move || info.set(!info.get_untracked())))
                .child(ib(gl::OPEN, move || {
                    let _ = std::process::Command::new("xdg-open").arg(&p_open).spawn();
                }))
                .child(ib(gl::DELETE, move || confirm.set(true)))
                .class("viewer-top"),
        )]
    });
    let arrows = Reactive::new(move || -> Vec<W> {
        let (Some(i), n) = (st.viewer.get(), st.items.get().len()) else { return vec![] };
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(if i > 0 { ib(gl::CHEVRON_L, move || step(st, -1)) } else { Box::new(Column::new().width(48.0)) });
        row = row.child(if i + 1 < n { ib(gl::CHEVRON_R, move || step(st, 1)) } else { Box::new(Column::new().width(48.0)) });
        vec![Box::new(row.class("viewer-arrows"))]
    });
    let info_card = Reactive::new(move || -> Vec<W> {
        if !info.get() {
            return vec![];
        }
        let (Some(i), items) = (st.viewer.get(), st.items.get()) else { return vec![] };
        let Some(it) = items.get(i).cloned() else { return vec![] };
        let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dims = if it.kind == Kind::Photo {
            image::ImageReader::open(&it.path).ok().and_then(|r| r.with_guessed_format().ok()).and_then(|r| r.into_dimensions().ok()).map(|(w, h)| {
                format!("{w} × {h} ({:.1} Мп)", w as f64 * h as f64 / 1e6).replace('.', ",")
            })
        } else {
            None
        };
        let mut col = Column::new()
            .gap(4.0)
            .child(Text::new(name).max_lines(2).class("info-name"))
            .child(Text::new(it.path.parent().map(|p| p.display().to_string()).unwrap_or_default()).max_lines(2).class("info-sub"))
            .child(Text::new(size_text(it.size)).class("info-sub"));
        if let Some(d) = dims {
            col = col.child(Text::new(d).class("info-sub"));
        }
        vec![Box::new(DecoratedBox::new().child(col).class("info-card"))]
    });
    let dialog = Reactive::new(move || -> Vec<W> {
        if !confirm.get() {
            return vec![];
        }
        let (Some(i), items) = (st.viewer.get_untracked(), st.items.get_untracked()) else { return vec![] };
        let Some(it) = items.get(i).cloned() else { return vec![] };
        let what = if it.kind == Kind::Photo { "снимок" } else { "видео" };
        let card = Column::new()
            .gap(14.0)
            .child(Text::new(format!("Удалить {what}?")).class("dlg-title"))
            .child(Text::new("Файл будет удалён без возможности восстановления").max_lines(3).class("dlg-text"))
            .child(
                Row::new()
                    .gap(10.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(btn("Отмена", "dlg-btn", move || confirm.set(false)))
                    .child(btn("Удалить", "dlg-btn dlg-danger", move || {
                        confirm.set(false);
                        PLAYER.with(|p| *p.borrow_mut() = None);
                        if let Err(e) = media::delete(&it.path) {
                            st.toast.set(format!("Не удалось удалить: {e}"));
                            return;
                        }
                        let mut items = st.items.get_untracked();
                        items.retain(|x| x.path != it.path);
                        let n = items.len();
                        st.items.set(items);
                        st.viewer.set(if n == 0 { None } else { Some(i.min(n - 1)) });
                        crate::refresh_thumb(st);
                    })),
            )
            .class("dlg");
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || confirm.set(false)).child(crate::ui::scrim()))
                .child(Column::new().center().child(card)),
        )]
    });
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(crate::ui::fill("viewer-bg"))
            .child(content)
            .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).child(arrows))
            .child(Column::new().child(top).child(info_card))
            .child(dialog),
    )
}

fn ib(glyph: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(crate::ui::centered(glyph, "ib-icon", 44.0)).class("ib")))
}

fn btn(label: &'static str, class: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Text::new(label).class("dlg-btn-text")).class(class)))
}

fn video_view(pl: Arc<Mutex<VideoPlayer>>, pos: RwSignal<f32>, paused: RwSignal<bool>) -> W {
    let duration = pl.lock().map(|p| p.duration_sec() as f32).unwrap_or(0.0);
    let video = VideoView::new(pl.clone()).position_signal(pos).fit(ImageFit::Contain).class("viewer-video");
    let p2 = pl.clone();
    let play = Reactive::new(move || -> Vec<W> {
        // позиция — чтобы заметить конец ролика
        let _ = (paused.get(), pos.get());
        let is_paused = p2.lock().map(|p| p.is_paused() || p.is_ended()).unwrap_or(true);
        let p = p2.clone();
        vec![ib(if is_paused { gl::PLAY } else { gl::PAUSE }, move || {
            if let Ok(mut p) = p.lock() {
                if p.is_paused() || p.is_ended() {
                    if p.is_ended() {
                        let _ = p.seek(0.0);
                    }
                    p.play();
                } else {
                    p.pause();
                }
                paused.set(p.is_paused());
            }
        })]
    });
    let p3 = pl.clone();
    let seek = Reactive::new(move || -> Vec<W> {
        let v = pos.get();
        let p = p3.clone();
        let t = |s: f32| format!("{}:{:02}", (s as u32) / 60, (s as u32) % 60);
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t(v)).class("viewer-time"))
                .child(
                    Slider::new()
                        .range(0.0, duration.max(0.1))
                        .value(v.min(duration.max(0.1)))
                        .width((viewport_size().get_untracked().width - 170.0).max(80.0))
                        .on_change(move |s| {
                            if let Ok(mut p) = p.lock() {
                                let _ = p.seek(s as f64);
                            }
                            pos.set(s);
                        }),
                )
                .child(Text::new(t(duration)).class("viewer-time"))
                .class("grow"),
        )]
    });
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(Column::new().child(Column::new().width(1.0).height(64.0)).child(DecoratedBox::new().child(video).class("grow")).child(Column::new().width(1.0).height(80.0)))
            .child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(play).child(seek).class("viewer-controls")),
            ),
    )
}
