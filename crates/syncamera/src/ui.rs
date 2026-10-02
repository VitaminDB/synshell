//! Экран камеры: превью во всю ширину (4:3, 16:9, 1:1), сверху — переключатели режима съёмки, снизу — зум,
//! режимы, затвор; касание — фокус и экспозиция (ползунок яркости), удержание — блокировка, щипок — зум,
//! смахивание вбок — соседний режим. В альбомном окне панели справа.

use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::widgets::{LiveView, PinchUpdate, SwipeDirection};
use syngui::{GestureDetector, ShowIf};

use crate::{media, proto, viewer, Lens, Mode, ProField, Rec, St, MODES};

pub type W = Box<dyn Widget>;

pub mod gl {
    pub const FLASH_OFF: &str = "\u{E3E6}";
    pub const FLASH_ON: &str = "\u{E3E7}";
    pub const FLASH_AUTO: &str = "\u{E3E5}";
    pub const TIMER_OFF: &str = "\u{E426}";
    pub const TIMER_3: &str = "\u{E424}";
    pub const TIMER_10: &str = "\u{E423}";
    pub const TIMER: &str = "\u{E425}";
    pub const GRID_ON: &str = "\u{E3EC}";
    pub const GRID_OFF: &str = "\u{E3EB}";
    pub const SWITCH: &str = "\u{EFEB}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const PAUSE: &str = "\u{E034}";
    pub const PLAY: &str = "\u{E037}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const DELETE: &str = "\u{E872}";
    pub const BACK: &str = "\u{E5C4}";
    pub const MIC: &str = "\u{E029}";
    pub const MIC_OFF: &str = "\u{E02B}";
    // фонарик — значки вспышки (flashlight_* в этом шрифте рисуются иначе)
    pub const TORCH_ON: &str = "\u{E3E7}";
    pub const TORCH_OFF: &str = "\u{E3E6}";
    pub const SUN: &str = "\u{E3AB}";
    pub const LOCK: &str = "\u{E897}";
    pub const PHOTO: &str = "\u{E412}";
    pub const OPEN: &str = "\u{E89E}";
    pub const INFO: &str = "\u{E88E}";
    pub const NIGHT: &str = "\u{EA46}";
    pub const CHEVRON_L: &str = "\u{E5CB}";
    pub const CHEVRON_R: &str = "\u{E5CC}";
    pub const WB: [&str; 5] = ["\u{E42C}", "\u{E42E}", "\u{E436}", "\u{E430}", "\u{E42D}"];
}

fn icon_btn(glyph: &'static str, class: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                crate::haptic();
                f()
            })
            .child(DecoratedBox::new().child(Icon::new(glyph).class("ib-icon")).class(format!("ib {class}"))),
    )
}

/// Верхняя панель высотой, логические px.
const TOP: f32 = 56.0;
/// Нижние панели (зум, режимы, затвор).
const BOTTOM: f32 = 236.0;

pub fn root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let vp = viewport_size().get();
        let _ = (st.aspect.get(), st.mode.get());
        let (sw, sh) = st.stream_size.get();
        if vp.width > vp.height {
            vec![landscape(st, vp, (sw, sh))]
        } else {
            vec![portrait(st, vp, (sw, sh))]
        }
    });
    let overlay_view = ShowIf::new(1, viewer::open_index(st)).child(viewer::view(st));
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![Box::new(DecoratedBox::new().child(Text::new(t).max_lines(3).class("toast-text")).class("toast"))]
        }))
        .class("toast-place");
    Box::new(
        GestureDetector::new()
            .on_back(move || {
                if st.viewer.get_untracked().is_some() {
                    st.viewer.set(None);
                    return true;
                }
                if st.settings.get_untracked() {
                    st.settings.set(false);
                    return true;
                }
                if st.pro_field.get_untracked().is_some() {
                    st.pro_field.set(None);
                    return true;
                }
                false
            })
            .child(
                Stack::new()
                    .fit(StackFit::Expand)
                    .child(fill("root"))
                    .child(body)
                    .child(settings_sheet(st))
                    .child(overlay_view)
                    .child(toast),
            ),
    )
}

/// Размер превью (повёрнутого кадра) в области `avail_w × avail_h`.
fn preview_size(st: St, stream: (u32, u32), avail_w: f32, avail_h: f32) -> (f32, f32) {
    let (sw, sh) = if stream.0 > 0 {
        stream
    } else if st.mode.get_untracked().video() {
        (16, 9)
    } else {
        crate::aspect_ratio(st.aspect.get_untracked())
    };
    let rot = crate::current_cam(st).map(|c| crate::preview_rot(st, &c)).unwrap_or(90);
    let (dw, dh) = if rot % 180 == 90 { (sh as f32, sw as f32) } else { (sw as f32, sh as f32) };
    let k = (avail_w / dw).min(avail_h / dh);
    (dw * k, dh * k)
}

fn portrait(st: St, vp: Size, stream: (u32, u32)) -> W {
    let (w, h) = (vp.width, vp.height);
    let (pw, ph) = preview_size(st, stream, w, h);
    // 4:3 и квадрат — под верхней панелью; высокое 16:9 — от верха, если не помещается
    let py = if ph + TOP + BOTTOM <= h { TOP } else { ((h - BOTTOM - ph) / 2.0).max(0.0).min(TOP) };
    let px = (w - pw) / 2.0;
    let top = Row::new()
        .main_axis_alignment(MainAxisAlignment::SpaceAround)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(top_controls(st, false))
        .width(w)
        .height(TOP)
        .class("topbar");
    let bottom = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(6.0)
        .child(pro_panel(st))
        .child(zoom_row(st, false))
        .child(modes_row(st, false))
        .child(shutter_row(st, false))
        .width(w)
        .height(BOTTOM + 120.0)
        .class("bottombar");
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(Positioned::new(preview(st, pw, ph, px, py)).at(px, py))
            .child(Positioned::new(rec_badge(st, w)).at(0.0, py + 8.0))
            .child(Positioned::new(top).at(0.0, 0.0))
            .child(Positioned::new(bottom).at(0.0, h - BOTTOM - 120.0)),
    )
}

fn landscape(st: St, vp: Size, stream: (u32, u32)) -> W {
    let (w, h) = (vp.width, vp.height);
    // слева — переключатели, справа — режимы и затвор столбиками; зум — столбиком у правого края кадра
    let (lw, side) = (76.0, 210.0);
    let (pw, ph) = preview_size(st, stream, w - side - lw, h);
    let px = lw + ((w - side - lw - pw) / 2.0).max(0.0);
    let py = (h - ph) / 2.0;
    let left = Column::new()
        .main_axis_alignment(MainAxisAlignment::SpaceAround)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(top_controls(st, true))
        .width(lw)
        .height(h)
        .class("topbar");
    let right = Row::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(6.0)
        .child(modes_row(st, true))
        .child(shutter_row(st, true))
        .width(side)
        .height(h)
        .class("bottombar");
    let zoom = Column::new().main_axis_alignment(MainAxisAlignment::Center).child(zoom_row(st, true)).height(h);
    let pro = Column::new().main_axis_alignment(MainAxisAlignment::End).child(pro_panel(st)).width(pw).height(h);
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(Positioned::new(preview(st, pw, ph, px, py)).at(px, py))
            .child(Positioned::new(rec_badge(st, pw)).at(px, py + 8.0))
            .child(Positioned::new(pro).at(px, -8.0))
            .child(Positioned::new(zoom).at(px + pw - 62.0, 0.0))
            .child(Positioned::new(left).at(0.0, 0.0))
            .child(Positioned::new(right).at(w - side, 0.0)),
    )
}

// ─── Верхняя панель ─────────────────────────────────────────────────────────

fn top_controls(st: St, vertical: bool) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let mode = st.mode.get();
        let rec = st.rec.get();
        let cam = {
            let _ = (st.cams.get(), st.front.get(), st.lens.get());
            crate::current_cam(st)
        };
        let has_flash = cam.map(|c| c.flash != 0).unwrap_or(false);
        let mut items: Vec<W> = Vec::new();
        if mode.video() {
            let torch = st.torch.get();
            if has_flash {
                items.push(icon_btn(if torch { gl::TORCH_ON } else { gl::TORCH_OFF }, if torch { "ib-on" } else { "" }, move || {
                    st.torch.set(!st.torch.get_untracked())
                }));
            }
            if mode == Mode::Video && rec == Rec::Idle {
                let mic = st.mic.get();
                items.push(icon_btn(if mic { gl::MIC } else { gl::MIC_OFF }, "", move || st.mic.set(!st.mic.get_untracked())));
                let q = st.video_q.get();
                items.push(chip_btn(["720p", "1080p", "4K"][q as usize % 3], move || st.video_q.set((st.video_q.get_untracked() + 1) % 3)));
            }
            if mode == Mode::Timelapse && rec == Rec::Idle {
                let k = st.timelapse.get();
                items.push(chip_btn(&format!("×{k}"), move || {
                    let n = match st.timelapse.get_untracked() {
                        5 => 10,
                        10 => 30,
                        30 => 60,
                        _ => 5,
                    };
                    st.timelapse.set(n);
                }));
            }
        } else {
            if has_flash && mode == Mode::Photo {
                let f = st.flash.get();
                let g = [gl::FLASH_OFF, gl::FLASH_AUTO, gl::FLASH_ON][f as usize % 3];
                items.push(icon_btn(g, if f == 2 { "ib-on" } else { "" }, move || st.flash.set((st.flash.get_untracked() + 1) % 3)));
            }
            let t = st.timer.get();
            let g = match t {
                3 => gl::TIMER_3,
                10 => gl::TIMER_10,
                0 => gl::TIMER_OFF,
                _ => gl::TIMER,
            };
            items.push(icon_btn(g, if t > 0 { "ib-on" } else { "" }, move || {
                st.timer.set(match st.timer.get_untracked() {
                    0 => 3,
                    3 => 10,
                    _ => 0,
                })
            }));
            let a = st.aspect.get();
            items.push(chip_btn(["4:3", "16:9", "1:1"][a as usize % 3], move || st.aspect.set((st.aspect.get_untracked() + 1) % 3)));
            let full = st.full_res.get();
            if cam.is_some_and(|c| c.full_width > 0) && a == 0 && mode != Mode::Night {
                let mp = cam.map(|c| (c.full_width as u64 * c.full_height as u64) / 1_000_000).unwrap_or(0);
                let normal = cam.map(|c| (c.photo_width as u64 * c.photo_height as u64) / 1_000_000).unwrap_or(0);
                items.push(chip_btn_on(&format!("{} Мп", if full { mp } else { normal }), full, move || st.full_res.set(!st.full_res.get_untracked())));
            }
        }
        let g = st.grid.get();
        items.push(icon_btn(if g { gl::GRID_ON } else { gl::GRID_OFF }, "", move || st.grid.set(!st.grid.get_untracked())));
        if rec == Rec::Idle {
            items.push(icon_btn(gl::SETTINGS, "", move || st.settings.set(true)));
        }
        if vertical {
            vec![Box::new(Column::new().main_axis_alignment(MainAxisAlignment::SpaceAround).cross_axis_alignment(CrossAxisAlignment::Center).gap(10.0).children(items).class("vbar"))]
        } else {
            vec![Box::new(Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround).cross_axis_alignment(CrossAxisAlignment::Center).gap(14.0).children(items))]
        }
    })
}

fn chip_btn(label: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    chip_btn_on(label, false, f)
}

fn chip_btn_on(label: &str, on: bool, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(
        GestureDetector::new()
            .on_click(move || {
                crate::haptic();
                f()
            })
            .child(DecoratedBox::new().child(Text::new(label.to_string()).max_lines(1).class("chip-text")).class(if on { "chip chip-on" } else { "chip" })),
    )
}

// ─── Превью ─────────────────────────────────────────────────────────────────

fn preview(st: St, pw: f32, ph: f32, px: f32, py: f32) -> W {
    let live = Reactive::new(move || -> Vec<W> {
        let _ = st.frame_rev.get();
        let Some(e) = st.engine.get_untracked() else { return vec![] };
        vec![Box::new(LiveView::new(e.preview()).fit(ImageFit::Cover).class("live"))]
    });
    let pinch_start = std::sync::Arc::new(std::sync::Mutex::new(None::<f32>));
    let ps2 = pinch_start.clone();
    let frac = move |p: Point| ((p.x - px) / pw, (p.y - py) / ph);
    let gest = GestureDetector::new()
        .on_click_at(move |p| {
            if st.countdown.get_untracked() > 0 {
                crate::cancel_countdown(st);
                return;
            }
            if st.pro_field.get_untracked().is_some() {
                st.pro_field.set(None);
            }
            let (u, v) = frac(p);
            crate::tap_focus(st, u, v, false);
        })
        .on_long_press(move |p| {
            let (u, v) = frac(p);
            crate::haptic();
            crate::tap_focus(st, u, v, true);
        })
        .on_pinch(move |u: PinchUpdate| {
            let mut s = pinch_start.lock().unwrap();
            let z0 = *s.get_or_insert_with(|| st.zoom.get_untracked());
            st.zooming.set(true);
            crate::pinch_zoom(st, z0 * u.scale);
        })
        .on_pinch_end(move || {
            *ps2.lock().unwrap() = None;
            st.zooming.set(false);
            crate::pinch_end(st);
        })
        .on_swipe(move |dir, _| {
            if st.rec.get_untracked() != Rec::Idle || st.zooming.get_untracked() {
                return;
            }
            let cur = st.mode.get_untracked();
            let i = MODES.iter().position(|m| m.0 == cur).unwrap_or(2);
            let n = match dir {
                SwipeDirection::Left => (i + 1).min(MODES.len() - 1),
                SwipeDirection::Right => i.saturating_sub(1),
                _ => return,
            };
            if n != i {
                crate::haptic();
                st.mode.set(MODES[n].0);
            }
        })
        .child(
            Stack::new()
                .fit(StackFit::Expand)
                .child(live)
                .child(grid(st, pw, ph))
                .child(focus_overlay(st, pw, ph))
                .child(blink(st, pw, ph))
                .child(switch_dim(st, pw, ph))
                .child(center_overlay(st))
                .child(ev_bar(st, pw, ph)),
        );
    Box::new(Column::new().width(pw).height(ph).clip(true).child(Stack::new().fit(StackFit::Expand).child(gest)).class("preview"))
}

fn grid(st: St, pw: f32, ph: f32) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if !st.grid.get() {
            return vec![];
        }
        let mut s = Stack::new().fit(StackFit::Expand);
        for k in [1.0, 2.0] {
            s = s.child(Positioned::new(Column::new().width(1.0).height(ph).class("grid-line")).at(pw * k / 3.0, 0.0));
            s = s.child(Positioned::new(Column::new().width(pw).height(1.0).class("grid-line")).at(0.0, ph * k / 3.0));
        }
        vec![Box::new(s)]
    })
}

fn focus_overlay(st: St, pw: f32, ph: f32) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let Some((u, v)) = st.focus_pt.get() else { return vec![] };
        let af = st.meta.get().af;
        let locked = st.locked.get();
        let r = 36.0;
        let class = match af {
            proto::AF_FOCUSED => "focus-ring focus-ok",
            proto::AF_FAILED => "focus-ring focus-bad",
            _ => "focus-ring",
        };
        let ring = Column::new().width(r * 2.0).height(r * 2.0).class(class);
        let mut s = Stack::new().fit(StackFit::Expand).child(Positioned::new(ring).at(u * pw - r, v * ph - r));
        if locked {
            s = s.child(
                Positioned::new(
                    Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(gl::LOCK).class("lock-icon")).child(Text::new("AE/AF").class("lock-text")).class("lock-pill"),
                )
                .at((u * pw - 34.0).max(4.0), (v * ph - r - 30.0).max(4.0)),
            );
        }
        vec![Box::new(s)]
    })
}

fn blink(st: St, pw: f32, ph: f32) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let k = st.blink.get();
        if k == 0 {
            return vec![];
        }
        vec![Box::new(
            Animated::new(Column::new().width(pw).height(ph).class("blink"))
                .opacity(Animation::tween(Easing::EaseOutQuad).from(0.85).to(0.0).duration_ms(260).build()),
        )]
    })
}

/// Камера переключается (кадров нового сеанса ещё нет) — старый кадр притушить.
fn switch_dim(st: St, pw: f32, ph: f32) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if st.stream_size.get() != (0, 0) {
            return vec![];
        }
        vec![Box::new(
            Animated::new(Column::new().width(pw).height(ph).class("switch-dim"))
                .opacity(Animation::tween(Easing::EaseOutQuad).from(0.0).to(1.0).duration_ms(180).build()),
        )]
    })
}

/// Отсчёт таймера, состояние камеры, зум при щипке.
fn center_overlay(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let n = st.countdown.get();
        let status = st.status.get();
        let progress = st.progress.get();
        let zooming = st.zooming.get();
        let mut col = Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).gap(10.0);
        let mut any = false;
        if n > 0 {
            col = col.child(Text::new(n.to_string()).class("countdown"));
            any = true;
        }
        if !status.is_empty() {
            col = col.child(DecoratedBox::new().child(Text::new(status).max_lines(3).class("status-text")).class("status"));
            any = true;
        }
        if !progress.is_empty() {
            col = col.child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Column::new().width(22.0).height(22.0).child(CircularProgress::new().indeterminate().size(22.0).stroke_width(2.5)))
                            .child(Text::new(progress).max_lines(2).class("status-text"))
                            .height(40.0),
                    )
                    .class("status"),
            );
            any = true;
        }
        if zooming {
            col = col.child(DecoratedBox::new().child(Text::new(zoom_label(st.zoom.get()))).class("zoom-big"));
            any = true;
        }
        if !any {
            return vec![];
        }
        vec![Box::new(col.class("center-ov"))]
    })
}

fn zoom_label(z: f32) -> String {
    if (z - z.round()).abs() < 0.05 {
        format!("{}×", z.round() as i32)
    } else {
        format!("{:.1}×", z).replace('.', ",")
    }
}

/// Ползунок яркости (EV) под точкой фокуса.
fn ev_bar(st: St, pw: f32, ph: f32) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if st.focus_pt.get().is_none() || st.mode.get() == Mode::Pro {
            return vec![];
        }
        let _ = st.cams.get();
        let Some(cam) = crate::current_cam(st) else { return vec![] };
        if cam.ev_max <= cam.ev_min {
            return vec![];
        }
        let ev = st.ev.get_untracked();
        let step = cam.ev_step();
        // подпись — отдельно: пересборка ползунка во время перетаскивания сорвала бы жест
        let label = Reactive::new(move || -> Vec<W> { vec![Box::new(Text::new(format!("{:+.1}", st.ev.get() as f32 * step).replace('.', ",")).class("ev-text"))] });
        let w = (pw - 48.0).min(320.0);
        let bar = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(gl::SUN).class("ev-icon"))
            .child(
                Slider::new()
                    .range(cam.ev_min as f32, cam.ev_max as f32)
                    .step(1.0)
                    .value(ev as f32)
                    .width(w - 90.0)
                    .on_change(move |v: f32| crate::set_ev(st, v.round() as i32)),
            )
            .child(label)
            .width(w)
            .class("ev-bar");
        vec![Box::new(Stack::new().fit(StackFit::Expand).child(Positioned::new(bar).at((pw - w) / 2.0, ph - 64.0)))]
    })
}

fn rec_badge(st: St, w: f32) -> impl Widget {
    Row::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let rec = st.rec.get();
            if rec == Rec::Idle {
                return vec![];
            }
            let s = st.rec_us.get() / 1_000_000;
            let text = if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) };
            vec![Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class(if rec == Rec::On { "rec-dot" } else { "rec-dot rec-dot-paused" }))
                    .child(Text::new(text).class("rec-text"))
                    .class("rec-pill"),
            )]
        }))
        .width(w)
}

// ─── Нижняя панель ──────────────────────────────────────────────────────────

/// Ряд или столбик (альбомное окно).
fn line(vertical: bool, gap: f32, items: Vec<W>) -> W {
    if vertical {
        Box::new(Column::new().gap(gap).cross_axis_alignment(CrossAxisAlignment::Center).children(items))
    } else {
        Box::new(Row::new().gap(gap).cross_axis_alignment(CrossAxisAlignment::Center).children(items))
    }
}

fn zoom_row(st: St, vertical: bool) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let _ = (st.cams.get(), st.front.get(), st.lens.get());
        let z = st.zoom.get();
        let wide = crate::has_wide(st) && !st.front.get_untracked();
        let mut stops: Vec<f32> = Vec::new();
        if wide {
            stops.push(0.6);
        }
        stops.extend([1.0, 2.0, 5.0]);
        if st.front.get_untracked() {
            stops = vec![1.0, 2.0];
        }
        // активная кнопка — ближайшая снизу; на ней — точное значение
        let active = stops.iter().rposition(|s| z + 0.01 >= *s).unwrap_or(0);
        let mut items: Vec<W> = Vec::new();
        for (i, s) in stops.iter().copied().enumerate() {
            let on = i == active;
            let label = if on { zoom_label(z) } else if s < 1.0 { ",6".into() } else { format!("{}", s as i32) };
            items.push(Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        crate::haptic();
                        crate::set_zoom(st, s)
                    })
                    .child(DecoratedBox::new().child(Text::new(label).class("zoom-text")).class(if on { "zoom-btn zoom-on" } else { "zoom-btn" })),
            ));
        }
        vec![Box::new(DecoratedBox::new().child(line(vertical, 6.0, items)).class("zoom-row"))]
    })
}

fn modes_row(st: St, vertical: bool) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let cur = st.mode.get();
        let rec = st.rec.get() != Rec::Idle;
        let mut items: Vec<W> = Vec::new();
        for (m, name) in MODES {
            if rec && m != cur {
                continue;
            }
            items.push(Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        if st.rec.get_untracked() == Rec::Idle && st.mode.get_untracked() != m {
                            crate::haptic();
                            st.mode.set(m);
                        }
                    })
                    .child(DecoratedBox::new().child(Text::new(name).class(if m == cur { "mode-text mode-on" } else { "mode-text" })).class(if m == cur { "mode mode-sel" } else { "mode" })),
            ));
        }
        vec![line(vertical, 4.0, items)]
    })
}

fn shutter_row(st: St, vertical: bool) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let mode = st.mode.get();
        let rec = st.rec.get();
        let busy = st.busy.get();
        let thumb = st.thumb.get();
        // слева: миниатюра снятого или пауза записи
        let left: W = if rec != Rec::Idle && mode == Mode::Video {
            icon_btn(if rec == Rec::Paused { gl::PLAY } else { gl::PAUSE }, "side-btn", move || crate::pause_recording(st))
        } else if rec != Rec::Idle {
            Box::new(Column::new().width(56.0).height(56.0))
        } else {
            let inner: W = match &thumb {
                Some(t) => Box::new(
                    Stack::new()
                        .fit(StackFit::Expand)
                        .child(Image::from_rgba(format!("thumb:{}:{}", t.key, t.w), t.w, t.h, t.rgba.to_vec()).fit(ImageFit::Cover).class("thumb-img"))
                        .child(if t.video {
                            Box::new(Column::new().center().child(Icon::new(gl::PLAY).class("thumb-play"))) as W
                        } else {
                            Box::new(Column::new()) as W
                        }),
                ),
                None => Box::new(Column::new().center().child(Icon::new(gl::PHOTO).class("thumb-empty"))),
            };
            Box::new(
                GestureDetector::new()
                    .on_click(move || {
                        if !st.items.get_untracked().is_empty() {
                            st.viewer.set(Some(0));
                        }
                    })
                    .child(DecoratedBox::new().clip(true).child(inner).class("thumb")),
            )
        };
        // затвор
        let (outer, inner) = match (mode.video(), rec) {
            (true, Rec::Idle) => ("shutter", "shutter-in shutter-rec"),
            (true, _) => ("shutter", "shutter-in shutter-stop"),
            (false, _) if busy => ("shutter shutter-busy", "shutter-in"),
            (false, _) if mode == Mode::Night => ("shutter", "shutter-in shutter-night"),
            _ => ("shutter", "shutter-in"),
        };
        let mut shutter_inner = DecoratedBox::new().class(inner.to_string());
        if mode == Mode::Night && rec == Rec::Idle {
            shutter_inner = DecoratedBox::new().child(Column::new().center().child(Icon::new(gl::NIGHT).class("night-icon"))).class(inner.to_string());
        }
        let shutter = GestureDetector::new()
            .on_click(move || crate::shutter(st))
            .child(DecoratedBox::new().child(Column::new().center().child(shutter_inner)).class(outer.to_string()));
        // справа: смена камеры или снимок во время записи
        let right: W = if rec != Rec::Idle {
            icon_btn(gl::PHOTO, "side-btn", move || {
                if !st.busy.get_untracked() {
                    crate::take_photo(st)
                }
            })
        } else {
            let front_exists = st.cams.get().iter().any(|c| c.front()) && st.cams.get().iter().any(|c| !c.front());
            if front_exists {
                icon_btn(gl::SWITCH, "side-btn", move || {
                    st.zoom.set(1.0);
                    st.lens.set(Lens::Main);
                    st.front.set(!st.front.get_untracked());
                })
            } else {
                Box::new(Column::new().width(56.0).height(56.0))
            }
        };
        if vertical {
            // столбик: смена камеры сверху, миниатюра снизу
            return vec![Box::new(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::SpaceAround)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(right)
                    .child(shutter)
                    .child(left)
                    .height(viewport_size().get_untracked().height)
                    .class("shutter-col"),
            )];
        }
        vec![Box::new(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::SpaceAround)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(left)
                .child(shutter)
                .child(right)
                .class("shutter-row"),
        )]
    })
}

// ─── Профи ──────────────────────────────────────────────────────────────────

const SHUTTERS: [(i64, &str); 16] = [
    (125_000, "1/8000"),
    (250_000, "1/4000"),
    (500_000, "1/2000"),
    (1_000_000, "1/1000"),
    (2_000_000, "1/500"),
    (4_000_000, "1/250"),
    (8_000_000, "1/125"),
    (16_666_667, "1/60"),
    (33_333_333, "1/30"),
    (66_666_667, "1/15"),
    (125_000_000, "1/8"),
    (250_000_000, "1/4"),
    (500_000_000, "1/2"),
    (1_000_000_000, "1\""),
    (2_000_000_000, "2\""),
    (4_000_000_000, "4\""),
];

const WB_NAMES: [(u32, &str); 5] = [(1, "Авто"), (2, "Лампа"), (3, "Флуор."), (5, "День"), (6, "Облачно")];

fn shutter_name(ns: i64) -> String {
    SHUTTERS.iter().min_by_key(|(v, _)| (v - ns).abs()).map(|(_, n)| n.to_string()).unwrap_or_default()
}

fn pro_panel(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if st.mode.get() != Mode::Pro {
            return vec![];
        }
        let _ = st.cams.get();
        let Some(cam) = crate::current_cam(st) else { return vec![] };
        let field = st.pro_field.get();
        let iso = st.iso.get();
        let sh = st.shutter_ns.get();
        let fd = st.focus_d.get();
        let awb = st.awb.get();
        let ev = st.ev.get();
        let meta = st.meta.get();
        let auto_iso = iso <= 0;
        let chips: [(ProField, String, String); 5] = [
            (ProField::Iso, "ISO".into(), if auto_iso { format!("А {}", meta.iso) } else { iso.to_string() }),
            (ProField::Shutter, "Выдержка".into(), if auto_iso { format!("А {}", shutter_name(meta.exposure_ns)) } else { shutter_name(sh) }),
            (ProField::Focus, "Фокус".into(), if fd < 0.0 { "А".into() } else if fd == 0.0 { "∞".into() } else { format!("{:.2} м", 1.0 / fd) }),
            (ProField::Wb, "ББ".into(), WB_NAMES.iter().find(|w| w.0 == awb).map(|w| w.1).unwrap_or("Авто").into()),
            (ProField::Ev, "EV".into(), format!("{:+.1}", ev as f32 * cam.ev_step()).replace('.', ",")),
        ];
        let mut row = Row::new().gap(4.0).main_axis_alignment(MainAxisAlignment::SpaceAround);
        for (f, name, val) in chips {
            let on = field == Some(f);
            row = row.child(
                GestureDetector::new()
                    .on_click(move || st.pro_field.set(if st.pro_field.get_untracked() == Some(f) { None } else { Some(f) }))
                    .child(
                        DecoratedBox::new()
                            .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(name).class("pro-name")).child(Text::new(val).max_lines(1).class("pro-val")))
                            .class(if on { "pro-chip pro-on" } else { "pro-chip" }),
                    ),
            );
        }
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(f) = field {
            col = col.child(pro_slider(st, f, &cam, iso, sh, fd, awb, ev));
        }
        col = col.child(DecoratedBox::new().child(row).class("pro-row"));
        vec![Box::new(col)]
    })
}

#[allow(clippy::too_many_arguments)]
fn pro_slider(st: St, f: ProField, cam: &proto::Camera, iso: i32, sh: i64, fd: f32, awb: u32, ev: i32) -> W {
    let auto = |label: &str, on: bool, act: Box<dyn Fn() + Send + Sync>| chip_btn_on(label, on, move || act());
    let width = 250.0;
    match f {
        ProField::Iso => {
            let (lo, hi) = (cam.iso_min.max(50) as f32, cam.iso_max.max(100) as f32);
            let v = if iso > 0 { iso as f32 } else { 400f32.clamp(lo, hi) };
            // логарифмическая шкала
            let (l0, l1) = (lo.ln(), hi.ln());
            let ex = st.shutter_ns.get_untracked();
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(auto("А", iso <= 0, Box::new(move || {
                        st.iso.set(0);
                        crate::apply_manual(st)
                    })))
                    .child(Slider::new().range(0.0, 1.0).value((v.ln() - l0) / (l1 - l0)).width(width).on_change(move |t: f32| {
                        let iso = (l0 + t * (l1 - l0)).exp();
                        let iso = ((iso / 10.0).round() * 10.0) as i32;
                        st.iso.set(iso);
                        if ex <= 0 {
                            st.shutter_ns.set(16_666_667);
                        }
                        crate::apply_manual(st)
                    }))
                    .class("pro-slider"),
            )
        }
        ProField::Shutter => {
            let list: Vec<(i64, &str)> = SHUTTERS.iter().copied().filter(|(v, _)| *v >= cam.exposure_min && *v <= cam.exposure_max.max(cam.exposure_min)).collect();
            let n = list.len().max(1);
            let cur = list.iter().position(|(v, _)| *v >= sh).unwrap_or(n / 2);
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(auto("А", iso <= 0, Box::new(move || {
                        st.iso.set(0);
                        crate::apply_manual(st)
                    })))
                    .child(Slider::new().range(0.0, (n - 1) as f32).step(1.0).value(cur as f32).width(width).on_change(move |t: f32| {
                        let i = (t.round() as usize).min(n - 1);
                        if let Some((v, _)) = list.get(i) {
                            st.shutter_ns.set(*v);
                            if st.iso.get_untracked() <= 0 {
                                let m = st.meta.get_untracked().iso;
                                st.iso.set(if m > 0 { m } else { 400 });
                            }
                            crate::apply_manual(st)
                        }
                    }))
                    .class("pro-slider"),
            )
        }
        ProField::Focus => {
            if cam.min_focus <= 0.0 {
                return Box::new(Text::new("У этой камеры постоянный фокус").class("pro-note"));
            }
            let mf = cam.min_focus;
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(auto("А", fd < 0.0, Box::new(move || {
                        st.focus_d.set(-1.0);
                        crate::apply_manual(st)
                    })))
                    .child(Slider::new().range(0.0, mf).value(fd.max(0.0)).width(width).on_change(move |v: f32| {
                        st.focus_d.set(v);
                        crate::apply_manual(st)
                    }))
                    .class("pro-slider"),
            )
        }
        ProField::Wb => {
            let mut row = Row::new().gap(6.0);
            for (i, (mode, name)) in WB_NAMES.iter().copied().enumerate() {
                row = row.child(
                    GestureDetector::new()
                        .on_click(move || {
                            st.awb.set(mode);
                            crate::apply_manual(st)
                        })
                        .child(
                            DecoratedBox::new()
                                .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(gl::WB[i]).class("wb-icon")).child(Text::new(name).class("pro-name")))
                                .class(if awb == mode { "pro-chip pro-on" } else { "pro-chip" }),
                        ),
                );
            }
            Box::new(row.class("pro-slider"))
        }
        ProField::Ev => Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(auto("0", ev == 0, Box::new(move || crate::set_ev(st, 0))))
                .child(Slider::new().range(cam.ev_min as f32, cam.ev_max as f32).step(1.0).value(ev as f32).width(width).on_change(move |v: f32| crate::set_ev(st, v.round() as i32)))
                .class("pro-slider"),
        ),
    }
}

// ─── Настройки ──────────────────────────────────────────────────────────────

/// Фон на весь экран (пустой DecoratedBox размера не имеет).
pub fn fill(class: &'static str) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let vp = viewport_size().get();
        vec![Box::new(Column::new().width(vp.width).height(vp.height).class(class))]
    }))
}

/// Затемнение под панелью — на весь экран.
pub fn scrim() -> W {
    let vp = viewport_size().get_untracked();
    Box::new(Column::new().width(vp.width).height(vp.height).class("scrim"))
}

fn setting_toggle(label: &'static str, sub: &'static str, sig: RwSignal<bool>) -> W {
    Box::new(
        GestureDetector::new().on_click(move || sig.set(!sig.get_untracked())).child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Column::new().gap(2.0).child(Text::new(label).class("set-label")).child(Text::new(sub).max_lines(2).class("set-sub")).class("grow"))
                .child(Reactive::new(move || -> Vec<W> { vec![Box::new(Toggle::new().on(sig.get()).on_change(move |b| sig.set(b)))] }))
                .class("set-row"),
        ),
    )
}

fn setting_choice(label: &'static str, sig: RwSignal<u32>, opts: &'static [(u32, &'static str)]) -> W {
    Box::new(
        Column::new()
            .gap(8.0)
            .child(Text::new(label).class("set-label"))
            .child(Reactive::new(move || -> Vec<W> {
                let cur = sig.get();
                let mut row = Row::new().gap(6.0);
                for (v, name) in opts.iter().copied() {
                    row = row.child(chip_btn_on(name, cur == v, move || sig.set(v)));
                }
                vec![Box::new(row)]
            }))
            .class("set-row"),
    )
}

fn settings_sheet(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if !st.settings.get() {
            return vec![];
        }
        let pics = media::pictures_dir();
        let vids = media::videos_dir();
        let body = Column::new()
            .gap(4.0)
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("Настройки камеры").class("sheet-title grow"))
                    .child(icon_btn(gl::CLOSE, "", move || st.settings.set(false))),
            )
            .child(Text::new("Фото").class("set-head"))
            .child(setting_choice("Соотношение сторон", st.aspect, &[(0, "4:3"), (1, "16:9"), (2, "1:1")]))
            .child(setting_toggle("Полное разрешение", "Основная камера 200 Мп, фронтальная 20 Мп (4:3); снимок — около 15 секунд", st.full_res))
            .child(setting_choice("Таймер", st.timer, &[(0, "Выкл."), (3, "3 с"), (10, "10 с")]))
            .child(setting_toggle("Сетка", "Линии третей поверх кадра", st.grid))
            .child(setting_toggle("Звук затвора", "Щелчок при съёмке и сигналы записи", st.sound))
            .child(Text::new("Видео").class("set-head"))
            .child(setting_choice("Качество", st.video_q, &[(0, "720p"), (1, "1080p"), (2, "4K")]))
            .child(setting_toggle("HEVC (H.265)", "Файлы меньше на 40 %; старые проигрыватели могут не открыть", st.hevc))
            .child(setting_toggle("Звук", "Запись с микрофона", st.mic))
            .child(setting_toggle("Стабилизация", "Цифровая стабилизация видео", st.stab))
            .child(setting_choice("Ускорение таймлапса", st.timelapse, &[(5, "×5"), (10, "×10"), (30, "×30"), (60, "×60")]))
            .child(Text::new("Хранение").class("set-head"))
            .child(Text::new(format!("Снимки — {}\nВидео — {}", pics.display(), vids.display())).max_lines(4).class("set-sub set-row"));
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || st.settings.set(false)).child(scrim()))
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .child(
                            Animated::new(DecoratedBox::new().child(ScrollView::new().vertical().child(body)).class("sheet"))
                                .translate_y(Animation::tween(Easing::EaseOutCubic).from(80.0).to(0.0).duration_ms(220).build())
                                .opacity(Animation::tween(Easing::EaseOutQuad).from(0.0).to(1.0).duration_ms(160).build()),
                        ),
                ),
        )]
    })
}
