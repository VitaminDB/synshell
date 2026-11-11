//! Экран звонка: фото абонента фоном (или крупный аватар), имя, номер, таймер, запись; живая визуализация
//! голосов (собеседник — над чертой, вы — под ней, бегущие столбики как в Диктофоне); кнопки разговора.

use std::path::PathBuf;

use syn_modem::api::{CallState, Request};
use syn_modem::time as tm;
use syngui::core::canvas::CanvasContext;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::common::*;
use super::dial::keypad;
use crate::prefs::RecordMode;
use crate::state::{act, dtmf, start_recording, state_text, stop_recording, voice_cmd, Page, St};
use crate::voice;

/// Фото звонящего для фона (если включено в настройках).
fn caller_photo(st: St) -> Option<PathBuf> {
    if !st.prefs.get().caller_photo {
        return None;
    }
    let c = st.main_call()?;
    st.contact(&c.number)?.photo()
}

fn draw_vis(ctx: &mut CanvasContext, t: f32) {
    let fg = ctx.mss_color().unwrap_or_else(|| Color::from_hex("#ffffff"));
    let accent = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#4ade80"));
    let (w, h) = (ctx.width(), ctx.height());
    if w < 8.0 || h < 8.0 {
        return;
    }
    let mid = h * 0.5;
    let step = 6.0f32;
    let bar = 3.4f32;
    let n = (w / step) as usize + 2;
    let (them, me, live, frac) = {
        let v = voice::vis();
        let take = |q: &std::collections::VecDeque<f32>| -> Vec<f32> { q.iter().skip(q.len().saturating_sub(n)).copied().collect() };
        let frac = v.last.map(|l| (l.elapsed().as_secs_f32() * voice::LEVELS_PER_SEC as f32).min(1.0)).unwrap_or(0.0);
        (take(&v.them), take(&v.me), v.live, frac)
    };
    // черта середины
    ctx.set_color(fg.with_alpha(0.18));
    ctx.fill_rounded_rect(0.0, mid - 0.5, w, 1.0, 0.5);
    if !live {
        // ждём звук — тихая «дышащая» линия
        let a = 0.15 + 0.1 * (t * 2.0).sin().abs();
        ctx.set_color(fg.with_alpha(a));
        let mut x = step * 0.5;
        while x < w {
            ctx.fill_circle(x, mid, 1.2);
            x += step;
        }
        return;
    }
    let amp = mid - 4.0;
    for (i, (&a, &b)) in them.iter().rev().zip(me.iter().rev()).enumerate() {
        let x = w - (i as f32 + frac) * step - step;
        if x < -bar {
            break;
        }
        let fade = (x / (w * 0.25)).clamp(0.0, 1.0);
        let ha = (a * a * amp).max(1.5);
        let hb = (b * b * amp).max(1.5);
        ctx.set_color(accent.with_alpha(0.25 + 0.75 * fade));
        ctx.fill_rounded_rect(x, mid - 1.0 - ha, bar, ha, bar * 0.5);
        ctx.set_color(fg.with_alpha(0.15 + 0.6 * fade));
        ctx.fill_rounded_rect(x, mid + 1.0, bar, hb, bar * 0.5);
    }
}

fn visualization(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let show = st.prefs.get().visualize && voice::available() && st.main_call().is_some_and(|c| matches!(c.state, CallState::Active | CallState::Held | CallState::Dialing | CallState::Alerting));
        if !show {
            return vec![boxed(DecoratedBox::new().class("vis-space"))];
        }
        vec![boxed(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().child(Text::new(t!("Собеседник")).class("vis-label vis-them grow")).child(Text::new(t!("Вы")).class("vis-label")))
                .child(DecoratedBox::new().class("vis").child(Canvas::new(draw_vis).animated(true)))
                .class("vis-box"),
        )]
    }))
}

pub fn view(st: St) -> W {
    // фон: фото звонящего или градиент
    let bg = Reactive::new(move || -> Vec<W> {
        match caller_photo(st) {
            Some(p) => {
                let key = format!("bg:{}:{}", p.display(), std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0));
                let data = std::fs::read(&p).unwrap_or_default();
                vec![boxed(
                    Stack::new()
                        .fit(StackFit::Expand)
                        .child(Image::from_bytes(key, data).fit(ImageFit::Cover).placeholder(false))
                        .child(DecoratedBox::new().class("photo-shade")),
                )]
            }
            None => vec![boxed(DecoratedBox::new().class("call-bg"))],
        }
    });
    let info = Reactive::new(move || -> Vec<W> {
        let now = st.tick.get();
        let Some(c) = st.main_call() else { return vec![] };
        let contact = st.contact(&c.number);
        let photo_bg = caller_photo(st).is_some();
        let name = contact.as_ref().map(|x| x.title()).unwrap_or_else(|| if c.number.is_empty() { t!("Скрытый номер").into() } else { c.number.clone() });
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if !photo_bg {
            col = col.child(avatar(contact.as_ref(), "av-call"));
        }
        col = col.child(Text::new(name).max_lines(2).class("call-number"));
        if contact.is_some() && !c.number.is_empty() {
            let kind = contact.as_ref().and_then(|x| x.phones.iter().find(|p| synshell_common::contacts::same_number(&p.number, &c.number)).map(|p| phone_kind(&p.kind))).unwrap_or_default();
            col = col.child(Text::new(format!("{} · {kind}", c.number)).max_lines(1).class("call-sub"));
        }
        let mut state = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(state_text(&c, now)).class("call-state"));
        if let Some((_, since)) = st.rec.get() {
            state = state.child(DecoratedBox::new().class("rec-pill").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("rec-dot")).child(Text::new(tm::duration((now - since).max(0) as u32)).class("rec-text"))));
        }
        col = col.child(state);
        // второй вызов (ожидание или удержание)
        if let Some(s) = st.status.get() {
            for o in s.calls.iter().filter(|o| o.id != c.id) {
                let who = st.contact(&o.number).map(|x| x.title()).unwrap_or(o.number.clone());
                col = col.child(DecoratedBox::new().class("other-call").child(Text::new(format!("{who} · {}", state_text(o, now))).max_lines(1).class("other-text")));
            }
        }
        vec![boxed(col.class("call-info"))]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let Some(s) = st.status.get() else { return vec![] };
        if let Some(c) = s.calls.iter().find(|c| matches!(c.state, CallState::Incoming | CallState::Waiting)).cloned() {
            let id = c.id;
            let number = c.number.clone();
            let mut col = Column::new().gap(22.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            if !number.is_empty() {
                col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(GestureDetector::new().on_click(move || {
                    let n = number.replace('\'', "");
                    act(st, Request::Hangup { id });
                    let _ = std::process::Command::new("synsms").args(["--chat", &n]).spawn();
                }).child(DecoratedBox::new().class("pill pill-glass").child(Row::new().gap(6.0).child(Icon::new(ic::SMS).class("pill-icon")).child(Text::new(t!("Ответить сообщением")).class("pill-text"))))));
            }
            col = col.child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::SpaceAround)
                    .child(round(ic::CALL_END, "round-red round-big", t!("Отклонить").into(), move || act(st, Request::Hangup { id })))
                    .child(round(ic::CALL, "round-green round-big round-ring", t!("Ответить").into(), move || act(st, Request::Answer { id }))),
            );
            return vec![boxed(col.class("call-buttons"))];
        }
        let active: Vec<u8> = s.calls.iter().map(|c| c.id).collect();
        let open = st.dtmf_open.get();
        let speaker = st.speaker.get();
        let muted = st.muted.get();
        let recording = st.rec.get().is_some();
        let can_rec = st.prefs.get().record != RecordMode::Off;
        let soft = |on: bool| if on { "round-soft round-soft-on" } else { "round-soft" };
        let mut col = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if open {
            col = col.child(keypad(move |c| dtmf(st, c), false, "key-call"));
        }
        let mut row1 = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround).class("call-row-btns");
        row1 = row1.child(round(if muted { ic::MIC_OFF } else { ic::MIC }, soft(muted), if muted { t!("Микрофон выкл.").into() } else { t!("Микрофон").into() }, move || {
            let on = !st.muted.get_untracked();
            st.muted.set(on);
            voice_cmd(st, format!("voice-mute {}", if on { "on" } else { "off" }));
        }));
        row1 = row1.child(round(ic::DIALPAD, soft(open), t!("Клавиши").into(), move || st.dtmf_open.set(!st.dtmf_open.get_untracked())));
        row1 = row1.child(round(ic::SPEAKER, soft(speaker), t!("Динамик").into(), move || {
            let on = !st.speaker.get_untracked();
            st.speaker.set(on);
            voice_cmd(st, format!("voice-route {}", if on { "speaker" } else { "auto" }));
        }));
        col = col.child(row1);
        if !open {
            let mut row2 = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround).class("call-row-btns");
            if can_rec {
                row2 = row2.child(round(if recording { ic::STOP } else { ic::REC }, if recording { "round-soft round-rec-on" } else { "round-soft" }, if recording { t!("Стоп запись").into() } else { t!("Запись").into() }, move || {
                    if st.rec.get_untracked().is_some() {
                        stop_recording(st);
                    } else {
                        start_recording(st);
                    }
                }));
            }
            let number = st.main_call().map(|c| c.number).unwrap_or_default();
            let known = st.contact(&number).map(|c| c.uid);
            row2 = row2.child(round(if known.is_some() { ic::PERSON } else { ic::PERSON_ADD }, "round-soft", if known.is_some() { t!("Контакт").into() } else { t!("В контакты").into() }, move || {
                st.open(match &known {
                    Some(u) => Page::Contact(u.clone()),
                    None => Page::Edit(None, number.clone()),
                });
                // карточка — поверх вкладок, звонок идёт: экран звонка вернётся по нажатию на плашку
                st.call_hidden.set(true);
            }));
            row2 = row2.child(round(ic::SMS, "round-soft", t!("Сообщение").into(), {
                let n = st.main_call().map(|c| c.number).unwrap_or_default();
                move || {
                    let _ = std::process::Command::new("synsms").args(["--chat", &n.replace('\'', "")]).spawn();
                }
            }));
            col = col.child(row2);
        }
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(round(ic::CALL_END, "round-red round-big", t!("Завершить").into(), move || {
            for id in active.clone() {
                act(st, Request::Hangup { id });
            }
        })));
        vec![boxed(col.class("call-buttons"))]
    });
    let content = Column::new()
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(info))
        .child(visualization(st))
        .child(controls)
        .class("grow in-call");
    boxed(DecoratedBox::new().class("call-screen").child(Stack::new().fit(StackFit::Expand).child(bg).child(content)))
}

/// Плашка «идёт разговор» поверх вкладок (окно открыли во время звонка — вернуться к экрану звонка).
pub fn return_bar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let now = st.tick.get();
        if !st.call_hidden.get() {
            return vec![];
        }
        let Some(c) = st.main_call() else { return vec![] };
        let who = st.contact(&c.number).map(|x| x.title()).unwrap_or(c.number.clone());
        vec![boxed(GestureDetector::new().on_click(move || st.call_hidden.set(false)).child(
            DecoratedBox::new().class("return-bar").child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(ic::CALL).class("return-icon"))
                    .child(Text::new(format!("{who} · {}", state_text(&c, now))).max_lines(1).elide(Elide::End).class("return-text grow"))
                    .child(Text::new(t!("Вернуться")).class("return-text")),
            ),
        ))]
    }))
}
