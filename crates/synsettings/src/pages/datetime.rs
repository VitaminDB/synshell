//! Дата и время: часовой пояс (вручную или по сети), синхронизация часов по
//! сети (NTP), ручная установка даты и времени. Пояс и часы — системные
//! (systemd-timedated); автоопределение пояса ведёт оболочка по
//! `[time] auto_timezone`.

use std::cell::Cell;
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::GestureDetector;
use synsystem::time::{self as systime, TimeStatus};

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

thread_local! {
    static STATUS: Cell<Option<RwSignal<Option<TimeStatus>>>> = const { Cell::new(None) };
    static ZONES: std::cell::OnceCell<Arc<Vec<Zone>>> = const { std::cell::OnceCell::new() };
}

/// Пояс для списка: имя, город, регион, смещение (зимнее) и строка поиска.
struct Zone {
    tz: String,
    city: String,
    region: String,
    offset: Option<i32>,
    hay: String,
}

fn zones() -> Arc<Vec<Zone>> {
    ZONES.with(|z| {
        z.get_or_init(|| {
            let sys = synsystem::Sys::host();
            Arc::new(
                systime::zones(&sys)
                    .into_iter()
                    .map(|tz| {
                        let (city, region) = systime::zone_label(&tz);
                        let offset = systime::std_offset(&sys, &tz);
                        let off = offset.map(|o| format!("{} {}", systime::format_offset(o), short_offset(o))).unwrap_or_default();
                        let hay = format!("{tz} {city} {region} {off}").to_lowercase();
                        Zone { tz, city, region, offset, hay }
                    })
                    .collect(),
            )
        })
        .clone()
    })
}

/// `+5`, `-3:30`, `+0` — как вводят в поиск.
fn short_offset(secs: i32) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    if a % 3600 == 0 {
        format!("{sign}{}", a / 3600)
    } else {
        format!("{sign}{}:{:02}", a / 3600, a % 3600 / 60)
    }
}

fn status_sig() -> RwSignal<Option<TimeStatus>> {
    STATUS.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(None);
            s.set(Some(x));
            x
        }
    })
}

fn refresh() {
    let sig = status_sig();
    std::thread::spawn(move || {
        let st = systime::status();
        run_on_main_thread(move || {
            if sig.get_untracked().as_ref() != Some(&st) {
                sig.set(Some(st));
            }
        });
    });
}

/// Действие с часами системы в фоне: итог — подсказкой, затем перечитать.
fn act(label: String, f: impl FnOnce() -> std::result::Result<(), String> + Send + 'static) {
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = f();
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            refresh();
        });
    });
}

fn detect_now() {
    act(t!("Определение пояса по сети").into(), || {
        let tz = systime::detect_timezone()?;
        if systime::status().timezone != tz {
            systime::set_timezone(&tz)?;
        }
        Ok(())
    });
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Карточка «сейчас»: время, дата, пояс и смещение.
fn summary(st: RwSignal<Option<TimeStatus>>) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let s = st.get();
        let (y, mo, d, h, mi, off) = systime::local_parts(now_unix());
        let zone = match &s {
            None => "…".to_string(),
            Some(s) if s.timezone.is_empty() => t!("пояс не задан (UTC)").to_string(),
            Some(s) => s.timezone.clone(),
        };
        let sync = match &s {
            Some(s) if !s.service => t!("служба systemd-timedated недоступна"),
            Some(s) if s.ntp && s.synced => t!("сверено с сервером времени"),
            Some(s) if s.ntp => t!("синхронизация по сети ещё не прошла"),
            Some(_) => t!("часы идут вручную"),
            None => "".to_string(),
        };
        vec![group(
            "",
            vec![row_inline(
                &format!("{h:02}:{mi:02} · {d:02}.{mo:02}.{y}"),
                &format!("{zone} · {} · {sync}", systime::format_offset(off)),
                icon_button(icons::REFRESH, refresh),
            )],
        )]
    }))
}

fn column(items: Vec<W>) -> Vec<W> {
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for w in items {
        col = col.child(w);
    }
    vec![boxed(col)]
}

/// Список поясов с поиском (при выключенном автоопределении).
fn zone_picker(st: RwSignal<Option<TimeStatus>>) -> W {
    let query = use_signal(String::new());
    let field = TextField::new().placeholder(t!("Город, регион или смещение (+5)")).on_change(move |s| query.set(s.to_string()));
    let list = Reactive::new(move || -> Vec<W> {
        let q = query.get().to_lowercase();
        let cur = st.get().map(|s| s.timezone).unwrap_or_default();
        let all = zones();
        let cur_off = all.iter().find(|z| z.tz == cur).and_then(|z| z.offset);
        let words: Vec<&str> = q.split_whitespace().collect();
        let hits: Vec<&Zone> = if words.is_empty() {
            // Без запроса — соседи текущего пояса по смещению.
            all.iter().filter(|z| cur_off.is_some() && z.offset == cur_off).collect()
        } else {
            all.iter().filter(|z| words.iter().all(|w| z.hay.contains(w))).collect()
        };
        let mut out: Vec<W> = Vec::new();
        if words.is_empty() {
            out.push(note(&t!("Начните вводить город (латиницей, как в базе поясов: Moscow, Almaty) или смещение. Ниже — пояса с тем же смещением, что и текущий.")));
        }
        if hits.is_empty() {
            if !words.is_empty() {
                out.push(note(&t!("Ничего не найдено")));
            }
            return column(out);
        }
        let mut rows: Vec<W> = Vec::new();
        const MAX: usize = 60;
        for z in hits.iter().take(MAX) {
            let tz = z.tz.clone();
            let hint = format!("{} · {}", z.region, z.offset.map(systime::format_offset).unwrap_or_default());
            let mark: W = if z.tz == cur { boxed(Text::new(t!("✓ текущий")).class("row-value")) } else { boxed(DecoratedBox::new()) };
            rows.push(boxed(
                GestureDetector::new()
                    .on_click(move || {
                        let tz = tz.clone();
                        act(t!("Часовой пояс {tz}", tz = tz), move || systime::set_timezone(&tz));
                    })
                    .child(row_inline(&z.city, &hint, mark)),
            ));
        }
        out.push(group("", rows));
        if hits.len() > MAX {
            out.push(note(&t!("И ещё {v} — уточните запрос", v = hits.len() - MAX)));
        }
        column(out)
    });
    boxed(Column::new().gap(8.0).child(row_wide(&t!("Выбрать пояс"), "", field)).child(list))
}

/// Синхронизация по сети и ручная установка часов.
fn clock_rows(st: RwSignal<Option<TimeStatus>>) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(s) = st.get() else { return vec![note(&t!("Чтение настроек часов…"))] };
        if !s.service {
            return vec![note(&t!("systemd-timedated недоступен: пояс и часы не настроить отсюда (timedatectl)."))];
        }
        let ntp_hint = if !s.can_ntp {
            t!("Служба синхронизации не установлена (systemd-timesyncd)")
        } else if s.synced {
            t!("По сети (NTP) · сверено")
        } else {
            t!("По сети (NTP)")
        };
        let mut rows: Vec<W> = vec![row_inline(
            t!("Время автоматически"),
            ntp_hint,
            Toggle::with_state(s.ntp).disabled(!s.can_ntp).on_change(|on| {
                act(if on { t!("Синхронизация по сети включается").into() } else { t!("Синхронизация по сети выключается").into() }, move || systime::set_ntp(on))
            }),
        )];
        if s.ntp {
            return vec![group(&t!("Время"), rows)];
        }
        let (y, mo, d, h, mi, _) = systime::local_parts(now_unix());
        let date = use_signal(Date::new(y, mo, d));
        let time = use_signal(Time::new(h, mi));
        rows.push(row(
            &t!("Дата"),
            "",
            DatePicker::new().selected(Date::new(y, mo, d)).width(200.0).on_change(move |v| {
                if let Some(v) = v {
                    date.set(v);
                }
            }),
        ));
        rows.push(row(
            &t!("Время"),
            "",
            TimePicker::new().selected(Time::new(h, mi)).use_24h(true).width(200.0).on_change(move |v| {
                if let Some(v) = v {
                    time.set(v);
                }
            }),
        ));
        rows.push(row_inline(
            t!("Установить часы"),
            t!("Дата и время выше — в текущем поясе"),
            primary_button(&t!("Установить"), move || {
                let (dt, tm) = (date.get_untracked(), time.get_untracked());
                match systime::local_to_unix(dt.year, dt.month, dt.day, tm.hour, tm.minute) {
                    Some(t) => act(t!("Часы: {day}.{month}.{year} {format}", day = format!("{:02}", dt.day), month = format!("{:02}", dt.month), year = dt.year, format = tm.format()), move || systime::set_time(t)),
                    None => state::toast(t!("Неверные дата или время")),
                }
            }),
        ));
        vec![group(&t!("Время"), rows)]
    }))
}

pub fn datetime() -> W {
    let st = status_sig();
    refresh();
    let c = store::config();
    let auto_tz = c.time.auto_timezone(c.process_form_factor());
    let mut body: Vec<W> = vec![summary(st), clock_rows(st)];
    body.push(group(
        &t!("Часовой пояс"),
        vec![
            row_inline(
                t!("Часовой пояс автоматически"),
                t!("По местоположению в сети (внешний IP): при входе, при подключении к сети и раз в 3 часа"),
                Toggle::with_state(auto_tz).on_change(|v| {
                    set(&op!["time", "auto_timezone"], v);
                    state::bump();
                }),
            ),
            row_inline(t!("Определить сейчас"), t!("Узнать пояс по сети и сразу поставить"), button(&t!("Определить"), detect_now)),
        ],
    ));
    if auto_tz {
        body.push(note(&t!("Чтобы выбрать пояс вручную, выключите «Часовой пояс автоматически».")));
    } else {
        body.push(zone_picker(st));
    }
    body.push(note(&t!("Пояс и часы — общие для системы (systemd-timedated); оболочка применяет их без перезапуска.")));
    page(t!("Дата и время"), t!("Часовой пояс, синхронизация часов по сети и ручная установка."), body)
}
