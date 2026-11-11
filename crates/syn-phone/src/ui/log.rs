//! Журнал звонков: по дням, имена и фото из контактов, фильтр «пропущенные», значок записи разговора.
//! Нажатие на строку — перезвонить; аватар — карточка контакта (неизвестный — добавить в контакты).

use syn_modem::api::{CallKind, CallRecord};
use syn_modem::time as tm;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::common::*;
use crate::state::{dial, Page, St};

fn kind_icon(k: CallKind) -> (&'static str, &'static str) {
    match k {
        CallKind::Incoming => (ic::CALL_RECEIVED, "log-icon"),
        CallKind::Outgoing => (ic::CALL_MADE, "log-icon"),
        CallKind::Missed => (ic::CALL_MISSED, "log-icon log-missed"),
        CallKind::Rejected => (ic::CALL_END, "log-icon log-missed"),
    }
}

/// Есть ли запись этого разговора (файл появился за время звонка).
fn has_recording(st: St, c: &CallRecord) -> bool {
    let end = c.time + c.duration as i64 + 120;
    st.recordings.get().iter().any(|r| r.time >= c.time && r.time <= end)
}

fn row(st: St, c: CallRecord) -> W {
    let contact = st.contact(&c.number);
    let (glyph, class) = kind_icon(c.kind);
    let sub = match c.kind {
        CallKind::Missed => t!("пропущенный").to_string(),
        CallKind::Rejected => t!("отклонён").to_string(),
        _ if c.duration > 0 => tm::duration(c.duration),
        CallKind::Outgoing => t!("не ответили").to_string(),
        CallKind::Incoming => String::new(),
    };
    let name = contact.as_ref().map(|x| x.title()).unwrap_or_else(|| if c.number.is_empty() { t!("Скрытый номер").into() } else { c.number.clone() });
    let rec = has_recording(st, &c);
    let mut meta = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class(class));
    meta = meta.child(Text::new(format!("{}{}", tm::short(c.time), if sub.is_empty() { String::new() } else { format!(" · {sub}") })).max_lines(1).class("log-sub"));
    if rec {
        meta = meta.child(Icon::new(ic::WAVES).class("log-rec"));
    }
    let (n1, n2, n3) = (c.number.clone(), c.number.clone(), c.number.clone());
    let uid = contact.as_ref().map(|x| x.uid.clone());
    let av = GestureDetector::new()
        .on_click(move || match &uid {
            Some(u) => st.open(Page::Contact(u.clone())),
            None if !n1.is_empty() => st.open(Page::Edit(None, n1.clone())),
            None => {}
        })
        .child(avatar(contact.as_ref(), "av-m"));
    boxed(
        GestureDetector::new().on_click(move || dial(st, n3.clone())).child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(av)
                        .child(
                            Column::new()
                                .gap(2.0)
                                .child(Text::new(name).max_lines(1).elide(Elide::End).class(if c.kind == CallKind::Missed { "log-name log-name-missed" } else { "log-name" }))
                                .child(meta)
                                .class("grow"),
                        )
                        .child(icon_btn(ic::SMS, "row-btn", move || {
                            let n = n2.replace('\'', "");
                            let _ = std::process::Command::new("synsms").args(["--chat", &n]).spawn();
                        })),
                )
                .class("log-row"),
        ),
    )
}

pub fn view(st: St) -> W {
    let filter = Reactive::new(move || -> Vec<W> {
        let missed = st.missed_only.get();
        let chip = |on: bool, text: String, v: bool| -> W { boxed(GestureDetector::new().on_click(move || st.missed_only.set(v)).child(DecoratedBox::new().class(if on { "chip chip-on" } else { "chip" }).child(Text::new(text).class("chip-text")))) };
        vec![boxed(Row::new().gap(8.0).child(chip(!missed, t!("Все").into(), false)).child(chip(missed, t!("Пропущенные").into(), true)).class("chips"))]
    });
    let list = Reactive::new(move || -> Vec<W> {
        let mut log = st.log.get();
        if st.missed_only.get() {
            log.retain(|c| matches!(c.kind, CallKind::Missed | CallKind::Rejected));
        }
        log.sort_by(|a, b| b.time.cmp(&a.time));
        if log.is_empty() {
            return vec![empty(ic::HISTORY, if st.missed_only.get() { t!("Пропущенных нет") } else { t!("Звонков пока не было") })];
        }
        let mut out: Vec<W> = Vec::new();
        let mut day = i64::MIN;
        for c in log {
            let d = tm::local(c.time).days;
            if d != day {
                day = d;
                out.push(section(tm::day_title(c.time)));
            }
            out.push(row(st, c));
        }
        out
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("tab-head").child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(t!("Журнал")).class("title grow")).child(filter)))
            .child(ScrollView::new().vertical().child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(list).class("list")).class("grow")),
    )
}
