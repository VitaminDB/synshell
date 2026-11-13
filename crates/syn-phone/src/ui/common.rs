//! Общие части интерфейса: аватар контакта, шапка страницы, строки настроек, круглые кнопки.

use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::contacts::Contact;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub mod ic {
    pub const CALL: &str = "\u{E0B0}";
    pub const CALL_END: &str = "\u{E0B1}";
    pub const CALL_MADE: &str = "\u{E0B2}";
    pub const CALL_MISSED: &str = "\u{E0B4}";
    pub const CALL_RECEIVED: &str = "\u{E0B5}";
    pub const SMS: &str = "\u{E0B7}";
    pub const DIALPAD: &str = "\u{E0BC}";
    pub const HISTORY: &str = "\u{E889}";
    pub const CONTACTS: &str = "\u{E0BA}";
    pub const PERSON: &str = "\u{E7FD}";
    pub const PERSON_ADD: &str = "\u{E7FE}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const BACK: &str = "\u{E5C4}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const DELETE: &str = "\u{E872}";
    pub const STAR: &str = "\u{E838}";
    pub const STAR_OFF: &str = "\u{E83A}";
    pub const ADD: &str = "\u{E145}";
    pub const MIC: &str = "\u{E029}";
    pub const MIC_OFF: &str = "\u{E02B}";
    pub const SPEAKER: &str = "\u{E050}";
    pub const REC: &str = "\u{E061}";
    pub const STOP: &str = "\u{E047}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const PHOTO: &str = "\u{E412}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const PLAY: &str = "\u{E037}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const BACKSPACE: &str = "\u{E14A}";
    pub const EMAIL: &str = "\u{E0BE}";
    pub const NOTE: &str = "\u{E26C}";
    pub const CHECK: &str = "\u{E5CA}";
    pub const WAVES: &str = "\u{E1B8}";
    pub const IMAGE: &str = "\u{E3F4}";
    pub const FORWARD: &str = "\u{E5C8}";
    pub const UPLOAD: &str = "\u{E2C6}";
    pub const DOWNLOAD: &str = "\u{E2C4}";
    pub const CHEVRON: &str = "\u{E5CC}";
    pub const PAUSE: &str = "\u{E034}";
    pub const REPLAY_10: &str = "\u{E059}";
    pub const FORWARD_10: &str = "\u{E056}";
}

/// Цвет кружка без фото — по имени (стабильно).
fn tone(name: &str) -> usize {
    // FNV-1a: соседние имена расходятся по цветам
    let h = name.trim().bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    (h ^ (h >> 16)) as usize % 6
}

/// Фото кругом (ключ кэша — путь и время правки: новое фото видно сразу).
fn photo(path: &std::path::Path, class: &str) -> Option<W> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let data = std::fs::read(path).ok()?;
    Some(boxed(
        DecoratedBox::new()
            .class(format!("av {class}"))
            .clip(true)
            .child(Image::from_bytes(format!("{}:{mtime}", path.display()), data).fit(ImageFit::Cover).placeholder(false).class("av-img")),
    ))
}

/// Аватар: фото, иначе первая буква на цветном круге, иначе значок. `class` — размер (`av-s`, `av-m`, `av-l`, `av-xl`).
pub fn avatar(c: Option<&Contact>, class: &str) -> W {
    if let Some(w) = c.and_then(|c| c.photo()).and_then(|p| photo(&p, class)) {
        return w;
    }
    let letter = c.map(|c| c.initial()).unwrap_or_default();
    let inner: W = if letter.is_empty() { boxed(Icon::new(ic::PERSON).class("av-icon")) } else { boxed(Text::new(letter).class("av-letter")) };
    let t = c.map(|c| tone(&c.name)).unwrap_or(6);
    boxed(
        DecoratedBox::new()
            .class(format!("av {class} av-c{t}"))
            .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(inner).class("grow")),
    )
}

/// Кнопка-значок (круглая, без фона).
pub fn icon_btn(glyph: &str, class: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(glyph).class("ib-icon")).class(format!("ib {class}"))))
}

/// Шапка страницы: «назад», заголовок, действия справа.
pub fn header(title: impl Into<String>, back: impl Fn() + Send + Sync + 'static, actions: Vec<W>) -> W {
    let mut row = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_btn(ic::BACK, "", back))
        .child(DecoratedBox::new().class("grow").child(Text::new(title.into()).max_lines(1).elide(Elide::End).class("page-title")));
    for a in actions {
        row = row.child(a);
    }
    boxed(DecoratedBox::new().class("page-header").child(row))
}

/// Заголовок раздела списка.
pub fn section(title: impl Into<String>) -> W {
    boxed(Text::new(title.into()).class("section"))
}

/// Подпись строки: значок, заголовок и пояснение.
pub fn label(glyph: &str, title: impl Into<String>, hint: impl Into<String>) -> W {
    let hint = hint.into();
    let mut texts = Column::new().gap(2.0).child(Text::new(title.into()).max_lines(2).class("row-title"));
    if !hint.is_empty() {
        texts = texts.child(Text::new(hint).max_lines(4).class("row-hint"));
    }
    boxed(
        DecoratedBox::new().class("grow").child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("row-badge").child(Icon::new(glyph).class("row-badge-icon")))
                .child(DecoratedBox::new().class("grow").child(texts)),
        ),
    )
}

/// Строка-переключатель.
pub fn toggle_row(glyph: &str, title: impl Into<String>, hint: impl Into<String>, on: bool, f: impl Fn(bool) + Send + Sync + 'static) -> W {
    let f = std::sync::Arc::new(f);
    let f2 = f.clone();
    boxed(
        GestureDetector::new().on_click(move || f2(!on)).child(
            DecoratedBox::new()
                .class("set-row")
                .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(Toggle::new().on(on).on_change(move |v| f(v)))),
        ),
    )
}

/// Строка-действие (нажатие).
pub fn action_row(glyph: &str, title: impl Into<String>, hint: impl Into<String>, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(
        GestureDetector::new()
            .on_click(f)
            .child(DecoratedBox::new().class("set-row").child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(Icon::new(ic::CHEVRON).class("row-chevron")))),
    )
}

/// Выбор из вариантов (сегменты); `cur` — индекс.
pub fn choice_row(glyph: &str, title: impl Into<String>, hint: impl Into<String>, options: Vec<String>, cur: usize, f: impl Fn(usize) + Send + Sync + Clone + 'static) -> W {
    let mut seg = Row::new().gap(4.0).class("seg");
    for (i, o) in options.into_iter().enumerate() {
        let f = f.clone();
        seg = seg.child(GestureDetector::new().on_click(move || f(i)).child(DecoratedBox::new().class(if i == cur { "seg-item seg-on" } else { "seg-item" }).child(Text::new(o).max_lines(1).class("seg-text"))));
    }
    boxed(DecoratedBox::new().class("set-row").child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(glyph, title, hint)).child(seg)))
}

/// Карточка-группа строк.
pub fn card(rows: Vec<W>) -> W {
    let mut c = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for r in rows {
        c = c.child(r);
    }
    boxed(DecoratedBox::new().class("card").child(c))
}

/// Пустой список: значок и подпись.
pub fn empty(glyph: &str, text: impl Into<String>) -> W {
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("empty-icon")).child(Text::new(text.into()).class("muted")).class("empty"))
}

/// Круглая кнопка с подписью (экран разговора).
pub fn round(glyph: &str, class: &str, label: String, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(glyph).class("round-icon")).class(format!("round {class}"))))
            .child(Text::new(label).max_lines(1).class("round-label")),
    )
}

/// Подпись типа номера.
pub fn phone_kind(kind: &str) -> String {
    match kind {
        "CELL" => t!("мобильный").into(),
        "HOME" => t!("домашний").into(),
        "WORK" => t!("рабочий").into(),
        "MAIN" => t!("основной").into(),
        "FAX" => t!("факс").into(),
        "" => t!("телефон").into(),
        o => o.to_lowercase(),
    }
}
