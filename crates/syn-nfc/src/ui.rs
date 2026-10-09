//! Экраны «NFC»: чтение, мои метки, создать. Вкладки внизу (телефон) или сверху.

use syngui::mss::StyleValue;
use syngui::prelude::*;
use synnfc::api::Tag;
use synnfc::ndef::Record;

use crate::icons;
use crate::state::{Action, Saved, St};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Подпись записи NDEF для человека: значок, заголовок, подробность.
pub fn describe(r: &Record) -> (&'static str, String, String) {
    match r {
        Record::Uri { uri } if uri.starts_with("tel:") => (icons::PHONE, uri.trim_start_matches("tel:").to_string(), t!("Номер телефона")),
        Record::Uri { uri } if uri.starts_with("mailto:") => (icons::MAIL, uri.trim_start_matches("mailto:").to_string(), t!("Почта")),
        Record::Uri { uri } => (icons::LINK, uri.clone(), t!("Ссылка")),
        Record::Text { text, lang } => (icons::TEXT, text.clone(), if lang.is_empty() { t!("Текст") } else { t!("Текст · {lang}", lang = lang) }),
        Record::SmartPoster { uri, title } => (icons::LINK, if title.is_empty() { uri.clone() } else { title.clone() }, uri.clone()),
        Record::Mime { mime, .. } if mime.contains("vcard") => {
            let text = r.mime_text().unwrap_or_default();
            let name = text.lines().find_map(|l| l.strip_prefix("FN:")).unwrap_or("").to_string();
            let tel = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.starts_with("TEL")).map(|(_, v)| v.to_string())).unwrap_or_default();
            (icons::CONTACT, if name.is_empty() { t!("Контакт") } else { name }, tel)
        }
        Record::Mime { mime, data } => (icons::DATA, mime.clone(), t!("{n} байт", n = data.len() / 2)),
        Record::Raw { tnf, r#type, payload, .. } => (icons::DATA, t!("Запись TNF {tnf}", tnf = tnf), format!("{} · {}", r#type, payload.chars().take(32).collect::<String>())),
    }
}

/// Что открыть для записи: ссылку, звонок, письмо.
fn open_target(r: &Record) -> Option<String> {
    match r {
        Record::Uri { uri } | Record::SmartPoster { uri, .. } if !uri.is_empty() => Some(uri.clone()),
        _ => None,
    }
}

fn open(uri: &str) {
    let _ = std::process::Command::new("xdg-open").arg(uri).spawn();
}

pub fn root(st: St) -> impl Widget {
    let body = move || -> Box<dyn Widget> {
        syngui::i18n::subscribe();
        let vp = viewport_size().get();
        let phone = vp.width < 700.0;
        let content: Box<dyn Widget> = match st.service.get() {
            Err(e) => Box::new(missing(&e)),
            Ok(s) if !s.present => Box::new(missing(&t!("Контроллер NFC не найден"))),
            Ok(s) => match st.tab.get() {
                0 => Box::new(read_page(st, &s.controller, &s.error)),
                1 => Box::new(saved_page(st)),
                _ => Box::new(create_page(st)),
            },
        };
        let toast = st.toast.get();
        let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !phone {
            col = col.child(tabs(st, false));
        }
        col = col.child(DecoratedBox::new().child(ScrollView::new().vertical().child(content)).style("height", StyleValue::px(vp.height - 64.0 - if toast.is_empty() { 0.0 } else { 40.0 })));
        if !toast.is_empty() {
            col = col.child(DecoratedBox::new().child(Text::new(toast).max_lines(2).class("toast-text")).class("toast"));
        }
        if phone {
            col = col.child(tabs(st, true));
        }
        Box::new(col)
    };
    DecoratedBox::new().child(syngui::widgets::Reactive::new(move || vec![body()])).class("root")
}

fn tabs(st: St, bottom: bool) -> impl Widget {
    let cur = st.tab.get();
    let mut row = Row::new().gap(4.0).main_axis_alignment(MainAxisAlignment::SpaceAround);
    for (i, (glyph, label)) in [(icons::NFC, n_!("Чтение")), (icons::SAVED, n_!("Мои метки")), (icons::ADD, n_!("Создать"))].into_iter().enumerate() {
        let class = if cur == i { "tab tab-on" } else { "tab" };
        row = row.child(
            syngui::GestureDetector::new()
                .on_click(move || {
                    st.tab.set(i);
                    // чтение — на вкладке «Чтение»; запись и эмуляция живут, пока не отменены
                    if i == 0 && matches!(st.action.get_untracked(), Action::Read) {
                        st.apply();
                    }
                })
                .child(DecoratedBox::new().child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("tab-icon")).child(Text::new(syngui::i18n::t(label)).class("tab-label"))).class(class)),
        );
    }
    DecoratedBox::new().child(row).class(if bottom { "tabs tabs-bottom" } else { "tabs" })
}

fn missing(msg: &str) -> impl Widget {
    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icons::NFC_OFF).class("big-icon"))
        .child(Text::new(t!("NFC недоступен")).class("title"))
        .child(Text::new(if msg.is_empty() { t!("Подключение к службе NFC…") } else { msg.to_string() }).max_lines(4).class("muted center"))
        .class("page center-page")
}

/// Состояние ожидания: чтение, запись, эмуляция.
fn waiting(st: St) -> impl Widget {
    let (glyph, title, sub, cancel) = match st.action.get() {
        Action::Read => (icons::NFC, t!("Поднесите метку"), t!("к задней панели телефона, рядом с камерой"), false),
        Action::Write(_, name) => (icons::EDIT, t!("Запись: поднесите метку"), t!("Содержимое «{name}» заменит записанное на метке", name = name), true),
        Action::Emulate(_, name) => (icons::CAST, t!("Телефон — метка «{name}»", name = name), t!("Поднесите телефон к считывателю или другому телефону"), true),
    };
    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(glyph).class("wait-icon")).class("wait-ring"))
        .child(Text::new(title).class("title"))
        .child(Text::new(sub).max_lines(3).class("muted center"));
    if cancel {
        col = col.child(Button::new(t!("Отменить")).on_click(move || st.set_action(Action::Read)));
    }
    DecoratedBox::new().child(col).class("wait")
}

fn read_page(st: St, controller: &str, error: &str) -> impl Widget {
    let mut col = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(waiting(st));
    if !error.is_empty() {
        col = col.child(Text::new(t!("Ошибка NFC: {e}", e = error)).max_lines(3).class("warn-text"));
    }
    if let Some(tag) = st.last.get() {
        col = col.child(tag_card(st, &tag));
    }
    if !controller.is_empty() {
        col = col.child(Text::new(t!("Контроллер: {c}", c = controller)).class("muted small center"));
    }
    col.class("page")
}

fn records_view(records: &[Record]) -> Column {
    let mut col = Column::new().gap(6.0);
    for r in records {
        let (glyph, title, sub) = describe(r);
        let target = open_target(r);
        let copy = match r {
            Record::Uri { uri } | Record::SmartPoster { uri, .. } => uri.clone(),
            Record::Text { text, .. } => text.clone(),
            other => other.mime_text().unwrap_or_default(),
        };
        let mut row = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(glyph).class("rec-icon"))
            .child(Column::new().gap(1.0).child(Text::new(title).max_lines(3).class("rec-title")).child(Text::new(sub).max_lines(2).class("muted small")).class("grow"));
        if let Some(t) = target {
            row = row.child(Button::new(t!("Открыть")).class("small").on_click(move || open(&t)));
        }
        if !copy.is_empty() {
            row = row.child(Button::new(String::new()).icon(icons::COPY).class("small icon-only").on_click(move || syngui::clipboard::copy(&copy)));
        }
        col = col.child(DecoratedBox::new().child(row).class("rec"));
    }
    col
}

fn tag_card(st: St, tag: &Tag) -> impl Widget {
    let mut info = vec![t!("{kind} · {tech}", kind = tag.kind, tech = tag.tech), t!("UID {uid}", uid = tag.uid)];
    if tag.capacity > 0 {
        info.push(t!("{n} байт · {w}", n = tag.capacity, w = if tag.writable { t!("запись разрешена") } else { t!("только чтение") }));
    }
    let mut col = Column::new()
        .gap(8.0)
        .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::NFC).class("card-icon")).child(Column::new().gap(1.0).child(Text::new(t!("Метка прочитана")).class("card-title")).child(Text::new(info.join("\n")).max_lines(4).class("muted small")).class("grow")));
    match &tag.ndef {
        Some(recs) if recs.is_empty() => col = col.child(Text::new(t!("Метка пуста — на неё можно записать")).class("muted")),
        Some(recs) => col = col.child(records_view(recs)),
        None => col = col.child(Text::new(if tag.protocol == "MIFARE" { t!("Карта MIFARE Classic: данные защищены ключами, читается только идентификатор") } else { t!("Нет данных NDEF") }).max_lines(3).class("muted")),
    }
    if !tag.error.is_empty() {
        col = col.child(Text::new(t!("Прочитано не полностью: {e}", e = tag.error)).max_lines(3).class("warn-text"));
    }
    if let Some(recs) = tag.ndef.clone().filter(|r| !r.is_empty()) {
        let (uid, kind) = (tag.uid.clone(), tag.kind.clone());
        let name = recs.first().map(|r| describe(r).1).unwrap_or_default();
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(Button::new(t!("Сохранить")).icon(icons::SAVE).class("primary").on_click(move || {
            st.save(Saved { name: name.chars().take(60).collect(), records: recs.clone(), uid: uid.clone(), kind: kind.clone(), time: now() });
            st.toast(t!("Сохранено в «Мои метки»"));
        })));
    }
    DecoratedBox::new().child(col).class("card")
}

fn saved_page(st: St) -> impl Widget {
    let list = st.saved.get();
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !matches!(st.action.get(), Action::Read) {
        col = col.child(waiting(st));
    }
    if list.is_empty() {
        col = col.child(Text::new(t!("Сохранённых меток нет: прочитайте метку и нажмите «Сохранить» или создайте свою.")).max_lines(3).class("muted center"));
    }
    for (i, s) in list.into_iter().enumerate() {
        let (w, e) = (s.records.clone(), s.records.clone());
        let (wn, en) = (s.name.clone(), s.name.clone());
        let mut sub = vec![tn!(s.records.len(), "{n} запись", "{n} записи", "{n} записей")];
        if !s.kind.is_empty() {
            sub.push(s.kind.clone());
        }
        col = col.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::SAVED).class("card-icon")).child(Column::new().gap(1.0).child(Text::new(s.name.clone()).max_lines(2).class("card-title")).child(Text::new(sub.join(" · ")).class("muted small")).class("grow")))
                        .child(records_view(&s.records))
                        .child(
                            Row::new()
                                .gap(8.0)
                                .main_axis_alignment(MainAxisAlignment::End)
                                .child(Button::new(t!("Удалить")).icon(icons::DELETE).class("small").on_click(move || st.remove(i)))
                                .child(Button::new(t!("Эмулировать")).icon(icons::CAST).class("small").on_click(move || st.set_action(Action::Emulate(e.clone(), en.clone()))))
                                .child(Button::new(t!("Записать")).icon(icons::EDIT).class("small primary").on_click(move || st.set_action(Action::Write(w.clone(), wn.clone())))),
                        ),
                )
                .class("card"),
        );
    }
    col.class("page")
}

/// Новое содержимое: ссылка, текст, телефон, контакт.
fn create_page(st: St) -> impl Widget {
    let kind = use_signal(0usize);
    let a = use_signal(String::new());
    let b = use_signal(String::new());
    let c = use_signal(String::new());
    let build = move || -> Option<(String, Vec<Record>)> {
        let (av, bv, cv) = (a.get_untracked().trim().to_string(), b.get_untracked().trim().to_string(), c.get_untracked().trim().to_string());
        match kind.get_untracked() {
            0 if !av.is_empty() => {
                let uri = if av.contains(':') { av } else { format!("https://{av}") };
                Some((uri.clone(), vec![Record::Uri { uri }]))
            }
            1 if !av.is_empty() => Some((av.chars().take(40).collect(), vec![Record::Text { text: av, lang: syngui::i18n::language().base().to_string() }])),
            2 if !av.is_empty() => Some((av.clone(), vec![Record::Uri { uri: format!("tel:{}", av.replace(' ', "")) }])),
            3 if !av.is_empty() => Some((av.clone(), vec![Record::vcard(&av, &bv, &cv)])),
            _ => None,
        }
    };
    let form = move || -> Box<dyn Widget> {
        let k = kind.get();
        let field = |label: String, sig: RwSignal<String>, ph: &str| -> Box<dyn Widget> {
            Box::new(Column::new().gap(4.0).child(Text::new(label).class("muted small")).child(TextField::new().placeholder(ph).on_change(move |t| sig.set(t.to_string())).class("field")))
        };
        let mut col = Column::new().gap(12.0);
        match k {
            0 => col = col.child(field(t!("Ссылка"), a, "https://")),
            1 => col = col.child(field(t!("Текст"), a, "")),
            2 => col = col.child(field(t!("Номер телефона"), a, "+7")),
            _ => {
                col = col.child(field(t!("Имя"), a, "")).child(field(t!("Телефон"), b, "+7")).child(field(t!("Почта"), c, "name@example.org"));
            }
        }
        Box::new(col)
    };
    let kinds = SegmentedButton::new(vec![t!("Ссылка"), t!("Текст"), t!("Телефон"), t!("Контакт")]).selected(kind.get_untracked()).on_change(move |i| {
        kind.set(i);
        a.set(String::new());
        b.set(String::new());
        c.set(String::new());
    });
    Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(t!("Новая метка")).class("title"))
        .child(kinds)
        .child(syngui::widgets::Reactive::new(move || vec![form()]))
        .child(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::End)
                .child(Button::new(t!("Сохранить")).icon(icons::SAVE).on_click(move || match build() {
                    Some((name, records)) => {
                        st.save(Saved { name, records, uid: String::new(), kind: String::new(), time: now() });
                        st.toast(t!("Сохранено в «Мои метки»"));
                    }
                    None => st.toast(t!("Заполните поле")),
                }))
                .child(Button::new(t!("Записать на метку")).icon(icons::EDIT).class("primary").on_click(move || match build() {
                    Some((name, records)) => {
                        st.set_action(Action::Write(records, name));
                        st.tab.set(0);
                    }
                    None => st.toast(t!("Заполните поле")),
                })),
        )
        .class("page")
}
