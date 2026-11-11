//! Контакты: поиск, избранные, список по буквам; карточка (звонок, SMS, номера, почта, заметка, последние
//! звонки); редактор (фото из файла, имя, номера с типами, почта, заметка, избранное).

use std::path::Path;

use syn_modem::api::CallKind;
use syn_modem::time as tm;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::contacts::{self, Contact, Phone};

use super::common::*;
use crate::state::{dial, photo_jpeg, reload_contacts, Draft, Page, PhotoEdit, St};

fn sms(number: &str) {
    let n = number.replace('\'', "");
    let _ = std::process::Command::new("synsms").args(["--chat", &n]).spawn();
}

/// Подходит ли контакт под поиск (имя по словам или цифры номера).
fn matches(c: &Contact, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let name = c.name.to_lowercase();
    if name.contains(&q) {
        return true;
    }
    let d = contacts::digits(&q);
    !d.is_empty() && d.len() == q.chars().filter(|c| !c.is_whitespace() && *c != '+').count() && c.phones.iter().any(|p| contacts::digits(&p.number).contains(&d))
}

pub fn list_view(st: St) -> W {
    let search = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(ic::SEARCH).class("search-icon"))
        .child(TextField::new().text(st.query.get_untracked()).placeholder(t!("Поиск по имени или номеру")).on_change(move |t| st.query.set(t.to_string())).class("search-input grow"));
    let favorites = Reactive::new(move || -> Vec<W> {
        if !st.query.get().trim().is_empty() {
            return vec![];
        }
        let favs: Vec<Contact> = st.contacts.get().into_iter().filter(|c| c.favorite).collect();
        if favs.is_empty() {
            return vec![];
        }
        let mut row = Row::new().gap(14.0);
        for c in favs {
            let uid = c.uid.clone();
            let first = c.name.split_whitespace().next().unwrap_or("").to_string();
            row = row.child(
                GestureDetector::new().on_click(move || st.open(Page::Contact(uid.clone()))).child(
                    Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(avatar(Some(&c), "av-l")).child(Text::new(first).max_lines(1).elide(Elide::End).class("fav-name")).class("fav"),
                ),
            );
        }
        vec![section(t!("Избранные")), boxed(ScrollView::new().horizontal().child(DecoratedBox::new().class("favs").child(row)))]
    });
    let list = Reactive::new(move || -> Vec<W> {
        let q = st.query.get();
        let all: Vec<Contact> = st.contacts.get().into_iter().filter(|c| matches(c, &q)).collect();
        if all.is_empty() {
            return vec![empty(ic::CONTACTS, if q.trim().is_empty() { t!("Контактов пока нет") } else { t!("Ничего не найдено") })];
        }
        let mut out: Vec<W> = Vec::new();
        let mut letter = String::new();
        for c in all {
            let l = c.initial();
            if l != letter && q.trim().is_empty() {
                letter = l.clone();
                out.push(section(if l.is_empty() { "#".into() } else { l }));
            }
            let uid = c.uid.clone();
            let num = c.phones.first().map(|p| format!("{} · {}", p.number, phone_kind(&p.kind))).unwrap_or_default();
            out.push(boxed(
                GestureDetector::new().on_click(move || st.open(Page::Contact(uid.clone()))).child(
                    DecoratedBox::new().class("log-row").child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(avatar(Some(&c), "av-m"))
                            .child(Column::new().gap(2.0).child(Text::new(c.title()).max_lines(1).elide(Elide::End).class("log-name")).child(Text::new(num).max_lines(1).class("log-sub")).class("grow")),
                    ),
                ),
            ));
        }
        out
    });
    let add = GestureDetector::new().on_click(move || st.open(Page::Edit(None, String::new()))).child(DecoratedBox::new().class("fab").child(Icon::new(ic::PERSON_ADD).class("fab-icon")));
    boxed(
        Stack::new()
            .fit(StackFit::Expand)
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().class("tab-head").child(Text::new(t!("Контакты")).class("title")))
                    .child(DecoratedBox::new().class("search").child(search))
                    .child(ScrollView::new().vertical().child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(favorites).child(list).class("list list-fab")).class("grow")),
            )
            .child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::End).child(DecoratedBox::new().class("fab-place").child(add))),
    )
}

/// Карточка контакта.
pub fn card_view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let Page::Contact(uid) = st.page.get() else { return vec![] };
        let Some(c) = st.contacts.get().into_iter().find(|c| c.uid == uid) else { return vec![empty(ic::PERSON, t!("Контакт удалён"))] };
        let first = c.phones.first().map(|p| p.number.clone()).unwrap_or_default();
        let (f1, f2) = (first.clone(), first.clone());
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(avatar(Some(&c), "av-xl")).child(Text::new(c.title()).max_lines(2).class("card-name")));
        // быстрые действия
        let mut acts = Row::new().gap(12.0).main_axis_alignment(MainAxisAlignment::Center);
        if !first.is_empty() {
            acts = acts.child(round(ic::CALL, "round-soft round-sm", t!("Позвонить").into(), move || dial(st, f1.clone())));
            acts = acts.child(round(ic::SMS, "round-soft round-sm", t!("Сообщение").into(), move || sms(&f2)));
        }
        let c_fav = c.clone();
        acts = acts.child(round(if c.favorite { ic::STAR } else { ic::STAR_OFF }, if c.favorite { "round-soft round-soft-on round-sm" } else { "round-soft round-sm" }, t!("Избранное").into(), move || {
            let mut x = c_fav.clone();
            x.favorite = !x.favorite;
            save(st, x, PhotoEdit::Keep, false);
        }));
        col = col.child(acts);
        // номера
        let mut rows: Vec<W> = Vec::new();
        for p in &c.phones {
            let (n1, n2) = (p.number.clone(), p.number.clone());
            rows.push(boxed(
                GestureDetector::new().on_click(move || dial(st, n1.clone())).child(
                    DecoratedBox::new().class("set-row").child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(label(ic::CALL, p.number.clone(), phone_kind(&p.kind)))
                            .child(icon_btn(ic::SMS, "row-btn", move || sms(&n2))),
                    ),
                ),
            ));
        }
        for e in &c.emails {
            rows.push(boxed(DecoratedBox::new().class("set-row").child(label(ic::EMAIL, e.clone(), t!("почта")))));
        }
        if !c.note.trim().is_empty() {
            rows.push(boxed(DecoratedBox::new().class("set-row").child(label(ic::NOTE, c.note.clone(), t!("заметка")))));
        }
        if !rows.is_empty() {
            col = col.child(card(rows));
        }
        // последние звонки
        let mut recent: Vec<_> = st.log.get().into_iter().filter(|r| c.has_number(&r.number)).collect();
        recent.sort_by(|a, b| b.time.cmp(&a.time));
        if !recent.is_empty() {
            let mut rr: Vec<W> = Vec::new();
            for r in recent.into_iter().take(6) {
                let (g, what) = match r.kind {
                    CallKind::Incoming => (ic::CALL_RECEIVED, t!("входящий").to_string()),
                    CallKind::Outgoing => (ic::CALL_MADE, t!("исходящий").to_string()),
                    CallKind::Missed => (ic::CALL_MISSED, t!("пропущенный").to_string()),
                    CallKind::Rejected => (ic::CALL_END, t!("отклонён").to_string()),
                };
                let dur = if r.duration > 0 { format!(" · {}", tm::duration(r.duration)) } else { String::new() };
                rr.push(boxed(DecoratedBox::new().class("set-row").child(label(g, format!("{}{dur}", tm::short(r.time)), format!("{what} · {}", r.number)))));
            }
            col = col.child(section(t!("Последние звонки"))).child(card(rr));
        }
        let c_del = c.clone();
        col = col.child(action_row(ic::DELETE, t!("Удалить контакт"), String::new(), move || {
            let uid = c_del.uid.clone();
            if !st.demo {
                let _ = contacts::delete(&uid);
            }
            st.contacts.update(|v| v.retain(|x| x.uid != uid));
            st.open(Page::None);
            st.toast(t!("Контакт удалён"));
        }));
        vec![boxed(col)]
    });
    let edit = icon_btn(ic::EDIT, "", move || {
        if let Page::Contact(uid) = st.page.get_untracked() {
            st.open(Page::Edit(Some(uid), String::new()));
        }
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header(String::new(), move || st.open(Page::None), vec![edit]))
            .child(ScrollView::new().vertical().child(DecoratedBox::new().class("page-body").child(body)).class("grow")),
    )
}

/// Записать контакт и фото; `close` — вернуться к карточке.
fn save(st: St, mut c: Contact, photo: PhotoEdit, close: bool) {
    if st.demo {
        st.contacts.update(|v| {
            v.retain(|x| x.uid != c.uid);
            v.push(c.clone());
        });
        return;
    }
    std::thread::spawn(move || {
        let r = (|| -> std::io::Result<()> {
            contacts::save(&mut c)?;
            match &photo {
                PhotoEdit::Keep => {}
                PhotoEdit::Set(d) => contacts::set_photo(&c.uid, Some(d))?,
                PhotoEdit::Remove => contacts::set_photo(&c.uid, None)?,
            }
            Ok(())
        })();
        let uid = c.uid.clone();
        run_on_main_thread(move || match r {
            Ok(()) => {
                reload_contacts(st);
                if close {
                    st.open(Page::Contact(uid));
                }
            }
            Err(e) => st.toast(t!("Контакт не сохранён: {e}", e = e)),
        });
    });
}

/// Черновик из контакта (или новый с номером).
pub fn start_edit(st: St, uid: Option<&str>, number: &str) {
    let c = uid.and_then(|u| st.contacts.get_untracked().into_iter().find(|c| c.uid == u));
    let d = match c {
        Some(c) => Draft { name: c.name.clone(), phones: c.phones.clone(), email: c.emails.first().cloned().unwrap_or_default(), note: c.note.clone(), favorite: c.favorite, photo: PhotoEdit::Keep },
        None => Draft { phones: vec![Phone { number: number.to_string(), kind: "CELL".into() }], ..Default::default() },
    };
    st.draft.set(d);
    st.draft_rev.update(|r| *r += 1);
}

fn pick_photo(st: St) {
    std::thread::spawn(move || {
        let start = synshell_common::paths::user_dir_or_default("PICTURES");
        let filters = [syn_system::portal_files::PickFilter { name: &t!("Изображения"), patterns: &["image/*"] }];
        let r = syn_system::portal_files::pick(&t!("Фото контакта"), false, &start, &filters);
        let res = match r {
            Ok(Some(p)) => photo_jpeg(Path::new(&p)).map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        run_on_main_thread(move || match res {
            Ok(Some(j)) => st.draft.update(|d| d.photo = PhotoEdit::Set(j)),
            Ok(None) => {}
            Err(e) => st.toast(t!("Фото не открылось: {e}", e = e)),
        });
    });
}

pub fn edit_view(st: St) -> W {
    // фото: читает черновик (меняется только фото — поля ввода вне этого Reactive)
    let photo = Reactive::new(move || -> Vec<W> {
        let Page::Edit(uid, _) = st.page.get() else { return vec![] };
        let d = st.draft.get();
        let cur: W = match &d.photo {
            PhotoEdit::Set(j) => boxed(
                DecoratedBox::new().class("av av-xl").clip(true).child(Image::from_bytes(format!("draft:{}", j.len()), j.clone()).fit(ImageFit::Cover).placeholder(false).class("av-img")),
            ),
            PhotoEdit::Remove => avatar(None, "av-xl"),
            PhotoEdit::Keep => {
                let c = uid.as_ref().and_then(|u| st.contacts.get_untracked().into_iter().find(|c| &c.uid == u));
                avatar(c.as_ref(), "av-xl")
            }
        };
        let has = match &d.photo {
            PhotoEdit::Set(_) => true,
            PhotoEdit::Remove => false,
            PhotoEdit::Keep => uid.as_ref().is_some_and(|u| contacts::photo_path(u).is_file()),
        };
        let mut btns = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::Center);
        btns = btns.child(GestureDetector::new().on_click(move || pick_photo(st)).child(DecoratedBox::new().class("pill").child(Row::new().gap(6.0).child(Icon::new(ic::PHOTO).class("pill-icon")).child(Text::new(if has { t!("Сменить фото") } else { t!("Добавить фото") }).class("pill-text")))));
        if has {
            btns = btns.child(GestureDetector::new().on_click(move || st.draft.update(|d| d.photo = PhotoEdit::Remove)).child(DecoratedBox::new().class("pill").child(Row::new().gap(6.0).child(Icon::new(ic::DELETE).class("pill-icon")).child(Text::new(t!("Убрать")).class("pill-text")))));
        }
        vec![boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(GestureDetector::new().on_click(move || pick_photo(st)).child(cur)).child(btns))]
    });
    let field = move |glyph: &str, placeholder: String, text: String, set: fn(&mut Draft, &str)| -> W {
        boxed(
            DecoratedBox::new().class("field").child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(glyph).class("field-icon"))
                    .child(TextField::new().text(text).placeholder(placeholder).on_change(move |t| st.draft.update(|d| set(d, t))).class("input grow")),
            ),
        )
    };
    // поля перестраиваются только при открытии редактора и добавлении/удалении номера
    let fields = Reactive::new(move || -> Vec<W> {
        let _ = st.draft_rev.get();
        let d = st.draft.get_untracked();
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(field(ic::PERSON, t!("Имя").into(), d.name.clone(), |d, t| d.name = t.to_string()));
        for (i, p) in d.phones.iter().enumerate() {
            let kind_next = |k: &str| -> String {
                let order = ["CELL", "HOME", "WORK", "MAIN", ""];
                let i = order.iter().position(|o| *o == k).unwrap_or(4);
                order[(i + 1) % order.len()].to_string()
            };
            let k = p.kind.clone();
            col = col.child(
                DecoratedBox::new().class("field").child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(ic::CALL).class("field-icon"))
                        .child(
                            TextField::new()
                                .text(p.number.clone())
                                .placeholder(t!("Номер"))
                                .on_change(move |t| {
                                    st.draft.update(|d| {
                                        if let Some(p) = d.phones.get_mut(i) {
                                            p.number = t.to_string();
                                        }
                                    })
                                })
                                .class("input grow"),
                        )
                        .child(
                            GestureDetector::new()
                                .on_click(move || {
                                    let next = kind_next(&k);
                                    st.draft.update(|d| {
                                        if let Some(p) = d.phones.get_mut(i) {
                                            p.kind = next.clone();
                                        }
                                    });
                                    st.draft_rev.update(|r| *r += 1);
                                })
                                .child(DecoratedBox::new().class("kind").child(Text::new(phone_kind(&p.kind)).class("kind-text"))),
                        )
                        .child(icon_btn(ic::CLOSE, "row-btn", move || {
                            st.draft.update(|d| {
                                if i < d.phones.len() {
                                    d.phones.remove(i);
                                }
                            });
                            st.draft_rev.update(|r| *r += 1);
                        })),
                ),
            );
        }
        col = col.child(GestureDetector::new().on_click(move || {
            st.draft.update(|d| d.phones.push(Phone { number: String::new(), kind: "CELL".into() }));
            st.draft_rev.update(|r| *r += 1);
        }).child(DecoratedBox::new().class("add-row").child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(ic::ADD).class("field-icon")).child(Text::new(t!("Добавить номер")).class("add-text")))));
        col = col.child(field(ic::EMAIL, t!("Почта").into(), d.email.clone(), |d, t| d.email = t.to_string()));
        col = col.child(field(ic::NOTE, t!("Заметка").into(), d.note.clone(), |d, t| d.note = t.to_string()));
        let fav = d.favorite;
        col = col.child(toggle_row(ic::STAR, t!("Избранный"), t!("Наверху списка контактов"), fav, move |v| {
            st.draft.update(|d| d.favorite = v);
            st.draft_rev.update(|r| *r += 1);
        }));
        vec![boxed(col)]
    });
    let done = icon_btn(ic::CHECK, "ib-accent", move || {
        let Page::Edit(uid, _) = st.page.get_untracked() else { return };
        let d = st.draft.get_untracked();
        let phones: Vec<Phone> = d.phones.iter().filter(|p| !p.number.trim().is_empty()).map(|p| Phone { number: p.number.trim().to_string(), kind: p.kind.clone() }).collect();
        if d.name.trim().is_empty() && phones.is_empty() {
            st.toast(t!("Нужно имя или номер"));
            return;
        }
        let mut c = uid.as_ref().and_then(|u| st.contacts.get_untracked().into_iter().find(|c| &c.uid == u)).unwrap_or_default();
        c.name = d.name.trim().to_string();
        c.phones = phones;
        c.emails = if d.email.trim().is_empty() { vec![] } else { vec![d.email.trim().to_string()] };
        c.note = d.note.trim().to_string();
        c.favorite = d.favorite;
        if st.demo && c.uid.is_empty() {
            c.uid = "demo-new".into();
        }
        save(st, c, d.photo.clone(), true);
    });
    let title = Reactive::new(move || -> Vec<W> {
        let new = matches!(st.page.get(), Page::Edit(None, _));
        vec![header(if new { t!("Новый контакт") } else { t!("Изменить контакт") }, move || match st.page.get_untracked() {
            Page::Edit(Some(u), _) => st.open(Page::Contact(u)),
            _ => st.open(Page::None),
        }, vec![])]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(title)).child(DecoratedBox::new().class("header-act").child(done)))
            .child(ScrollView::new().vertical().child(DecoratedBox::new().class("page-body").child(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(photo).child(fields))).class("grow")),
    )
}
