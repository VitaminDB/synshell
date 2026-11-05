//! Проигрыватели в ленте: звук — строкой (▶/⏸, перемотка, время), видео —
//! постер с ▶, по щелчку — компактный проигрыватель на месте (декодер
//! аппаратный, где есть). Играет один файл за раз.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use syngui::prelude::*;

use super::{boxed, icon, W};

thread_local! {
    static AUDIO: std::cell::RefCell<Option<(PathBuf, syngui::audio::AudioPlayer)>> = const { std::cell::RefCell::new(None) };
    static VIDEO: std::cell::RefCell<Option<(PathBuf, Arc<Mutex<syngui::video::VideoPlayer>>)>> = const { std::cell::RefCell::new(None) };
    static REV: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
}

fn rev() -> RwSignal<u64> {
    REV.with(|r| match r.get() {
        Some(v) => v,
        None => {
            let v = use_signal(0u64);
            r.set(Some(v));
            v
        }
    })
}

fn bump() {
    let r = rev();
    r.set(r.get_untracked() + 1);
}

/// Остановить звук и ролик.
pub fn stop() {
    if let Some((_, p)) = AUDIO.with(|a| a.borrow_mut().take()) {
        p.stop();
    }
    VIDEO.with(|v| v.borrow_mut().take());
    bump();
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn decode_stereo(path: &Path) -> Option<Arc<[f32]>> {
    let out = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-vn", "-ac", "2", "-ar", "48000", "-f", "f32le", "-"]).output().ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect::<Vec<_>>().into())
}

/// Строка проигрывателя звука.
pub fn audio(path: PathBuf) -> W {
    let rev = rev();
    boxed(Reactive::new(move || -> Vec<W> {
        rev.get();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let state = AUDIO.with(|a| a.borrow().as_ref().filter(|(p, _)| *p == path).map(|(_, pl)| (pl.position_seconds(), pl.duration_seconds(), pl.is_paused() || pl.is_done())));
        let (pos, dur, paused) = state.unwrap_or((0.0, 0.0, true));
        let p1 = path.clone();
        let toggle = Button::new("").icon(if paused { "\u{E037}" } else { "\u{E034}" }).class("btn round play-btn").on_click(move || {
            let mine = AUDIO.with(|a| a.borrow().as_ref().is_some_and(|(p, _)| *p == p1));
            if mine {
                AUDIO.with(|a| {
                    if let Some((_, pl)) = a.borrow().as_ref() {
                        if pl.is_done() {
                            let _ = pl.seek_seconds(0.0);
                            pl.resume();
                        } else if pl.is_paused() {
                            pl.resume();
                        } else {
                            pl.pause();
                        }
                    }
                });
            } else {
                stop();
                if let Some(pcm) = decode_stereo(&p1) {
                    if let Ok(pl) = syngui::audio::AudioPlayer::start_stereo(pcm, 48_000) {
                        AUDIO.with(|a| *a.borrow_mut() = Some((p1.clone(), pl)));
                        // Пока играет — обновлять ползунок и время.
                        std::thread::spawn(|| loop {
                            std::thread::sleep(Duration::from_millis(250));
                            let (tx, rx) = std::sync::mpsc::channel();
                            run_on_main_thread(move || {
                                bump();
                                let _ = tx.send(AUDIO.with(|a| a.borrow().as_ref().is_some_and(|(_, pl)| !pl.is_done())));
                            });
                            if !rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
                                break;
                            }
                        });
                    }
                }
            }
            bump();
        });
        let p2 = path.clone();
        let slider = Slider::new().range(0.0, dur.max(0.01) as f32).value(pos as f32).width(280.0).on_change(move |v| {
            AUDIO.with(|a| {
                if let Some((p, pl)) = a.borrow().as_ref() {
                    if *p == p2 {
                        let _ = pl.seek_seconds(v as f64);
                    }
                }
            });
        });
        vec![boxed(
            DecoratedBox::new().class("player-audio").child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(toggle)
                    .child(Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("player-name")).child(slider))
                    .child(Text::new(if dur > 0.0 { format!("{} / {}", fmt_time(pos), fmt_time(dur)) } else { String::new() }).class("hint")),
            ),
        )]
    }))
}

/// Ролик: постер `poster` с ▶; по щелчку — проигрыватель шириной `width`.
pub fn video(path: PathBuf, poster: Option<PathBuf>, width: f32) -> W {
    let rev = rev();
    boxed(Reactive::new(move || -> Vec<W> {
        rev.get();
        let playing = VIDEO.with(|v| v.borrow().as_ref().filter(|(p, _)| *p == path).map(|(_, pl)| pl.clone()));
        if let Some(pl) = playing {
            return vec![boxed(
                DecoratedBox::new()
                    .class("player-video")
                    .style("width", syngui::StyleValue::px(width))
                    .style("height", syngui::StyleValue::px(width * 9.0 / 16.0))
                    .child(syngui::widgets::visual::video_view::VideoPlayerView::file(pl).compact(true).build()),
            )];
        }
        let base: W = match &poster {
            Some(img) => boxed(Image::new(img.to_string_lossy()).fit(ImageFit::Cover).class("att-tile")),
            None => boxed(DecoratedBox::new().class("att-tile att-tile-empty")),
        };
        let p = path.clone();
        let play = Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("att-tile")
            .child(DecoratedBox::new().class("att-play").child(icon("\u{E037}").class("att-play-ic")));
        vec![boxed(
            syngui::GestureDetector::new()
                .on_click(move || {
                    stop();
                    match syngui::video::VideoPlayer::open_with_hwaccel(&p.to_string_lossy(), syngui::video::HwAccel::platform_default()) {
                        Ok(pl) => VIDEO.with(|v| *v.borrow_mut() = Some((p.clone(), Arc::new(Mutex::new(pl))))),
                        Err(_) => {
                            let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
                        }
                    }
                    bump();
                })
                .child(
                    DecoratedBox::new()
                        .class("att-tile-box")
                        .clip(true)
                        .style("width", syngui::StyleValue::px(width))
                        .style("height", syngui::StyleValue::px(width * 9.0 / 16.0))
                        .child(syngui::widgets::Stack::new().child(base).child(play)),
                ),
        )]
    }))
}
