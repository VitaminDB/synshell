//! Вкладка «Записи»: записи разговоров по дням (кто — по журналу звонков и контактам, время, длительность,
//! входящий/исходящий). Касание — проигрыватель прямо в строке: волна с перемоткой пальцем, пауза, ±10 с,
//! позвонить, сообщение, удалить (с подтверждением). Значок записи в журнале открывает её здесь.

use syn_modem::api::{CallKind, CallRecord};
use syn_modem::time as tm;
use syngui::prelude::*;
use syngui::visual::StaticWaveform;
use syngui::GestureDetector;

use super::common::*;
use crate::player;
use crate::state::{dial, Page, RecFile, St};

/// Звонок журнала, во время которого сделана запись (файл дописан к концу разговора).
pub fn call_of<'a>(log: &'a [CallRecord], r: &RecFile) -> Option<&'a CallRecord> {
    log.iter().find(|c| r.time >= c.time && r.time <= c.time + c.duration as i64 + 120)
}

/// Кто в записи по имени файла «<номер или имя контакта> ГГГГ-ММ-ДД ЧЧ-ММ-СС» (state::record_path) — когда звонка нет
/// в журнале. Возвращает (номер, подпись).
pub fn who_from_name(st: St, title: &str) -> (String, String) {
    let who = match title.rsplitn(3, ' ').collect::<Vec<_>>().as_slice() {
        [time, date, who] if date.len() == 10 && time.len() == 8 && date.as_bytes()[4] == b'-' => who.trim().to_string(),
        _ => title.trim().to_string(),
    };
    let looks_number = !who.is_empty() && who.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#' | ' ' | '-' | '(' | ')'));
    if looks_number {
        return (who.clone(), who);
    }
    let number = st
        .contacts
        .get_untracked()
        .iter()
        .find(|c| c.title() == who)
        .and_then(|c| c.phones.first().map(|p| p.number.clone()))
        .unwrap_or_default();
    (number, who)
}

fn secs(s: f64) -> String {
    tm::duration(s.max(0.0).round() as u32)
}

fn size_text(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} {}", b as f64 / (1 << 20) as f64, t!("МБ"))
    } else {
        format!("{} {}", b / 1024, t!("КБ"))
    }
}

/// Проигрыватель выбранной записи.
fn player_card(st: St, r: RecFile, number: String) -> W {
    let p = st.play;
    let wave = Reactive::new(move || -> Vec<W> {
        let pos = p.pos.get();
        let dur = p.dur.get();
        let prog = if dur > 0.0 { (pos / dur) as f32 } else { 0.0 };
        // волна ещё строится — пустая полоса с прогрессом и перемоткой
        let w: W = boxed(StaticWaveform::new().pcm(p.wave.get()).bins(72).progress(prog).height(56.0).class("rec-wave").on_seek(move |f| {
            let d = st.play.dur.get_untracked();
            player::seek(st, f as f64 * d);
        }));
        vec![
            w,
            boxed(Row::new().child(Text::new(secs(pos)).class("rec-time grow")).child(Text::new(secs(dur)).class("rec-time"))),
        ]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let playing = p.playing.get();
        vec![boxed(
            Row::new()
                .gap(18.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon_btn(ic::REPLAY_10, "rec-skip", move || player::skip(st, -10.0)))
                .child(GestureDetector::new().on_click(move || player::toggle(st)).child(DecoratedBox::new().class("rec-play").child(Icon::new(if playing { ic::PAUSE } else { ic::PLAY }).class("rec-play-icon"))))
                .child(icon_btn(ic::FORWARD_10, "rec-skip", move || player::skip(st, 10.0))),
        )]
    });
    let path = r.path.clone();
    let actions = Reactive::new(move || -> Vec<W> {
        let confirm = p.confirm_delete.get().as_ref() == Some(&path);
        let mut row = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::Center);
        if !number.is_empty() && !confirm {
            let (n1, n2) = (number.clone(), number.clone());
            row = row
                .child(pill(ic::CALL, t!("Позвонить").into(), "pill", move || dial(st, n1.clone())))
                .child(pill(ic::SMS, t!("Сообщение").into(), "pill", move || {
                    let _ = std::process::Command::new("synsms").args(["--chat", &n2.replace('\'', "")]).spawn();
                }));
        }
        let path = path.clone();
        if confirm {
            let pd = path.clone();
            row = row
                .child(pill(ic::CLOSE, t!("Отмена").into(), "pill", move || st.play.confirm_delete.set(None)))
                .child(pill(ic::DELETE, t!("Удалить запись").into(), "pill pill-danger", move || delete(st, pd.clone())));
        } else {
            row = row.child(pill(ic::DELETE, t!("Удалить").into(), "pill", move || st.play.confirm_delete.set(Some(path.clone()))));
        }
        vec![boxed(row)]
    });
    boxed(
        DecoratedBox::new()
            .class("rec-card")
            .child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(wave).child(controls).child(actions)),
    )
}

fn pill(glyph: &str, text: String, class: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(GestureDetector::new().on_click(f).child(DecoratedBox::new().class(class.to_string()).child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("pill-icon")).child(Text::new(text).class("pill-text")))))
}

fn delete(st: St, path: std::path::PathBuf) {
    if st.play.sel.get_untracked().as_ref() == Some(&path) {
        player::close(st);
    }
    st.play.confirm_delete.set(None);
    if !st.demo {
        if let Err(e) = std::fs::remove_file(&path) {
            st.toast(t!("Не удалось удалить: {e}", e = e));
            return;
        }
    }
    st.recordings.update(|v| v.retain(|x| x.path != path));
}

fn row(st: St, r: RecFile, call: Option<CallRecord>, selected: bool) -> W {
    let (number, label) = match &call {
        Some(c) => (c.number.clone(), if c.number.is_empty() { t!("Скрытый номер").to_string() } else { c.number.clone() }),
        None => who_from_name(st, &r.title),
    };
    let contact = if number.is_empty() { None } else { st.contact(&number) };
    let name = contact.as_ref().map(|x| x.title()).unwrap_or(label);
    let mut meta = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(c) = &call {
        let (glyph, _) = match c.kind {
            CallKind::Outgoing => (ic::CALL_MADE, ()),
            _ => (ic::CALL_RECEIVED, ()),
        };
        meta = meta.child(Icon::new(glyph).class("log-icon"));
    }
    let mut sub = tm::hm(call.as_ref().map(|c| c.time).unwrap_or(r.time));
    if r.duration > 0.0 {
        sub.push_str(&format!(" · {}", secs(r.duration)));
    }
    sub.push_str(&format!(" · {}", size_text(r.size)));
    meta = meta.child(Text::new(sub).max_lines(1).class("log-sub"));
    let p = r.path.clone();
    let head = GestureDetector::new().on_click(move || player::play(st, p.clone())).child(
        DecoratedBox::new().class(if selected { "log-row rec-row-on" } else { "log-row" }).child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(avatar(contact.as_ref(), "av-m"))
                .child(Column::new().gap(2.0).child(Text::new(name).max_lines(1).elide(Elide::End).class("log-name")).child(meta).class("grow"))
                .child(Reactive::new({
                    let path = r.path.clone();
                    move || -> Vec<W> {
                        let on = st.play.sel.get().as_ref() == Some(&path) && st.play.playing.get();
                        vec![boxed(Icon::new(if on { ic::PAUSE } else { ic::PLAY }).class(if on { "rec-row-icon rec-row-icon-on" } else { "rec-row-icon" }))]
                    }
                })),
        ),
    );
    if !selected {
        return boxed(head);
    }
    boxed(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(player_card(st, r, number)))
}

pub fn view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let recs = st.recordings.get();
        let log = st.log.get();
        let _ = st.contacts.get();
        let sel = st.play.sel.get();
        if recs.is_empty() {
            return vec![empty(ic::WAVES, t!("Записей разговоров нет"))];
        }
        let mut out: Vec<W> = Vec::new();
        let mut day = i64::MIN;
        for r in recs {
            let call = call_of(&log, &r).cloned();
            let t = call.as_ref().map(|c| c.time).unwrap_or(r.time);
            let d = tm::local(t).days;
            if d != day {
                day = d;
                out.push(section(tm::day_title(t)));
            }
            let selected = sel.as_ref() == Some(&r.path);
            out.push(row(st, r, call, selected));
        }
        out
    });
    let dir = Reactive::new(move || -> Vec<W> {
        let d = st.prefs.get().record_dir();
        vec![boxed(Text::new(d.display().to_string()).max_lines(1).elide(Elide::Middle).class("muted dir-line"))]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DecoratedBox::new().class("tab-head").child(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(t!("Записи")).class("title grow"))
                        .child(icon_btn(ic::SETTINGS, "row-btn", move || st.open(Page::Settings))),
                ),
            )
            .child(dir)
            .child(ScrollView::new().vertical().child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(list).class("list")).class("grow")),
    )
}
