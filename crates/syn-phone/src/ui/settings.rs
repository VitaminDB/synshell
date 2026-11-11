//! Настройки и записи разговоров.

use std::path::Path;

use syn_modem::api::{CallKind, Request};
use syn_modem::time as tm;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::contacts;

use super::common::*;
use crate::prefs::RecordMode;
use crate::state::{act, reload_contacts, reload_log, reload_recordings, Page, St};
use crate::voice;

fn forward_reason(r: &str) -> String {
    match r {
        "always" => t!("Всегда").into(),
        "busy" => t!("Если занято").into(),
        "no-reply" => t!("Если не отвечаю").into(),
        "unreachable" => t!("Если недоступен").into(),
        o => o.to_string(),
    }
}

fn pick_dir(st: St) {
    std::thread::spawn(move || {
        let start = st.prefs.get_untracked().record_dir();
        let r = syn_system::portal_files::pick(&t!("Папка записей разговоров"), true, &start, &[]);
        run_on_main_thread(move || match r {
            Ok(Some(p)) => {
                let s = p.to_string_lossy().into_owned();
                st.set_prefs(|x| x.record_dir = s);
                reload_recordings(st);
            }
            Ok(None) => {}
            Err(e) => st.toast(t!("Выбор папки: {e}", e = e)),
        });
    });
}

fn import(st: St) {
    std::thread::spawn(move || {
        let start = synshell_common::paths::user_dir_or_default("DOWNLOAD");
        let filters = [syn_system::portal_files::PickFilter { name: "vCard", patterns: &["*.vcf", "*.vcard", "text/vcard", "text/x-vcard"] }];
        let r = syn_system::portal_files::pick(&t!("Импорт контактов"), false, &start, &filters);
        let msg = match r {
            Ok(Some(p)) => match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| contacts::import(&s).map_err(|e| e.to_string())) {
                Ok(n) => Some(t!("Добавлено контактов: {n}", n = n).to_string()),
                Err(e) => Some(t!("Импорт не удался: {e}", e = e).to_string()),
            },
            Ok(None) => None,
            Err(e) => Some(t!("Выбор файла: {e}", e = e).to_string()),
        };
        run_on_main_thread(move || {
            if let Some(m) = msg {
                st.toast(m);
            }
            reload_contacts(st);
        });
    });
}

fn export(st: St) {
    std::thread::spawn(move || {
        let dir = synshell_common::paths::user_dir_or_default("DOCUMENTS");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{}.vcf", t!("Контакты")));
        let msg = match contacts::export(&p) {
            Ok(n) => t!("Сохранено {n} в {path}", n = n, path = p.display()).to_string(),
            Err(e) => t!("Экспорт не удался: {e}", e = e).to_string(),
        };
        run_on_main_thread(move || st.toast(msg));
    });
}

pub fn view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let p = st.prefs.get();
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

        col = col.child(section(t!("Разговор")));
        col = col.child(card(vec![
            toggle_row(ic::WAVES, t!("Визуализация голосов"), t!("Живые волны собеседника и вашего голоса на экране разговора"), p.visualize, move |v| st.set_prefs(|x| x.visualize = v)),
            toggle_row(ic::IMAGE, t!("Фото абонента на весь экран"), t!("Если у контакта есть фото — фоном экрана звонка, иначе кружком"), p.caller_photo, move |v| st.set_prefs(|x| x.caller_photo = v)),
        ]));

        col = col.child(section(t!("Запись разговоров")));
        let cur = match p.record {
            RecordMode::Off => 0,
            RecordMode::Manual => 1,
            RecordMode::Auto => 2,
        };
        let hint = if voice::available() { t!("Файлы Opus; соблюдайте законы о записи разговоров").to_string() } else { t!("На этом устройстве нет отвода звука разговора — запись недоступна").to_string() };
        let mut rec_rows = vec![choice_row(ic::REC, t!("Записывать"), hint, vec![t!("Нет").into(), t!("Кнопкой").into(), t!("Все разговоры").into()], cur, move |i| {
            st.set_prefs(|x| {
                x.record = match i {
                    0 => RecordMode::Off,
                    2 => RecordMode::Auto,
                    _ => RecordMode::Manual,
                }
            })
        })];
        rec_rows.push(action_row(ic::FOLDER, t!("Папка"), p.record_dir().display().to_string(), move || pick_dir(st)));
        rec_rows.push(action_row(ic::PLAY, t!("Записи"), t!("Прослушать и удалить"), move || st.open(Page::Recordings)));
        col = col.child(card(rec_rows));

        col = col.child(section(t!("Услуги оператора")));
        let rows: Vec<W> = match st.services.get() {
            None => vec![boxed(DecoratedBox::new().class("set-row").child(label(ic::SETTINGS, t!("Запрос настроек у сети…"), String::new())))],
            Some(s) if !s.error.is_empty() && s.waiting.is_none() => vec![boxed(DecoratedBox::new().class("set-row").child(label(ic::SETTINGS, t!("Сеть не дала настройки"), s.error.clone())))],
            Some(s) => {
                let mut v: Vec<W> = Vec::new();
                if let Some(w) = s.waiting {
                    v.push(toggle_row(ic::CALL, t!("Ожидание вызова"), t!("Второй входящий во время разговора"), w, move |on| {
                        st.services.update(|x| {
                            if let Some(x) = x {
                                x.waiting = Some(on);
                            }
                        });
                        act(st, Request::SetCallWaiting { on });
                    }));
                }
                let clir = match s.clir.as_str() {
                    "show" => 1,
                    "hide" => 2,
                    _ => 0,
                };
                v.push(choice_row(ic::PERSON, t!("Мой номер у собеседника"), String::new(), vec![t!("Как в сети").into(), t!("Показывать").into(), t!("Скрывать").into()], clir, move |i| {
                    let mode = ["network", "show", "hide"][i].to_string();
                    st.services.update(|x| {
                        if let Some(x) = x {
                            x.clir = mode.clone();
                        }
                    });
                    act(st, Request::SetClir { mode });
                }));
                for f in &s.forwards {
                    let hint = if f.active && !f.number.is_empty() { t!("на {number}", number = f.number).to_string() } else { t!("выключена").to_string() };
                    let reason = f.reason.clone();
                    let active = f.active;
                    v.push(boxed(
                        DecoratedBox::new().class("set-row").child(
                            Row::new()
                                .gap(12.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(label(ic::FORWARD, t!("Переадресация: {when}", when = forward_reason(&f.reason)), hint))
                                .child(if active {
                                    icon_btn(ic::CLOSE, "row-btn", move || {
                                        let reason = reason.clone();
                                        st.services.update(|x| {
                                            if let Some(x) = x {
                                                for f in x.forwards.iter_mut().filter(|f| f.reason == reason) {
                                                    f.active = false;
                                                }
                                            }
                                        });
                                        act(st, Request::SetForward { reason, number: String::new(), timer: None });
                                    })
                                } else {
                                    boxed(DecoratedBox::new())
                                }),
                        ),
                    ));
                }
                v
            }
        };
        col = col.child(card(rows));

        col = col.child(section(t!("Контакты")));
        col = col.child(card(vec![
            action_row(ic::DOWNLOAD, t!("Импорт из файла"), t!("vCard (.vcf) из другого телефона или почты"), move || import(st)),
            action_row(ic::UPLOAD, t!("Экспорт в файл"), t!("Все контакты с фото — в «Документы»"), move || export(st)),
        ]));

        col = col.child(section(t!("Общие")));
        col = col.child(card(vec![
            choice_row(ic::DIALPAD, t!("Открывать вкладку"), String::new(), vec![t!("Набор").into(), t!("Журнал").into(), t!("Контакты").into()], p.start_tab as usize, move |i| st.set_prefs(|x| x.start_tab = i as u8)),
            action_row(ic::DELETE, t!("Очистить журнал звонков"), String::new(), move || {
                let ids: Vec<u64> = st.log.get_untracked().iter().map(|c| c.id).collect();
                if ids.is_empty() {
                    return;
                }
                st.log.set(Vec::new());
                if !st.demo {
                    std::thread::spawn(move || {
                        let _ = syn_modem::api::request(&Request::CallLogDelete { ids });
                        run_on_main_thread(move || reload_log(st));
                    });
                }
                st.toast(t!("Журнал очищен"));
            }),
        ]));
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header(t!("Настройки"), move || st.open(Page::None), vec![]))
            .child(ScrollView::new().vertical().child(DecoratedBox::new().class("page-body").child(body)).class("grow")),
    )
}

fn size_text(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} {}", b as f64 / (1 << 20) as f64, t!("МБ"))
    } else {
        format!("{} {}", b / 1024, t!("КБ"))
    }
}

pub fn recordings_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let recs = st.recordings.get();
        if recs.is_empty() {
            return vec![empty(ic::WAVES, t!("Записей разговоров нет"))];
        }
        // запись ↔ звонок журнала: тип звонка и номер для подписи
        let log = st.log.get();
        recs.into_iter()
            .map(|r| -> W {
                let call = log.iter().find(|c| r.time >= c.time && r.time <= c.time + c.duration as i64 + 120);
                let kind = call.map(|c| match c.kind {
                    CallKind::Outgoing => t!("исходящий").to_string(),
                    _ => t!("входящий").to_string(),
                });
                let sub = format!("{} · {}{}", tm::short(r.time), size_text(r.size), kind.map(|k| format!(" · {k}")).unwrap_or_default());
                let (p1, p2) = (r.path.clone(), r.path.clone());
                boxed(
                    GestureDetector::new()
                        .on_click(move || {
                            let _ = std::process::Command::new("syn-audio-player").arg(&p1).spawn();
                        })
                        .child(
                            DecoratedBox::new().class("log-row").child(
                                Row::new()
                                    .gap(12.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(DecoratedBox::new().class("av av-m av-rec").child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(ic::PLAY).class("av-icon")).class("grow")))
                                    .child(Column::new().gap(2.0).child(Text::new(r.title.clone()).max_lines(1).elide(Elide::End).class("log-name")).child(Text::new(sub).max_lines(1).class("log-sub")).class("grow"))
                                    .child(icon_btn(ic::DELETE, "row-btn", move || {
                                        let p = p2.clone();
                                        if !st.demo {
                                            let _ = std::fs::remove_file(&p);
                                        }
                                        st.recordings.update(|v| v.retain(|x| x.path != p));
                                    })),
                            ),
                        ),
                )
            })
            .collect()
    });
    let dir = Reactive::new(move || -> Vec<W> {
        let d = st.prefs.get().record_dir();
        vec![boxed(Text::new(d.display().to_string()).max_lines(1).elide(Elide::Middle).class("muted dir-line"))]
    });
    let _ = Path::new("");
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header(t!("Записи разговоров"), move || st.open(Page::Settings), vec![]))
            .child(dir)
            .child(ScrollView::new().vertical().child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(list).class("list")).class("grow")),
    )
}
