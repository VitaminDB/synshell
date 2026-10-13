//! Экран пульта. Раскладка собирается из кнопок, которые есть в пульте: у телевизоров и прочей
//! медиатехники — питание, крестовина, громкость и каналы качелями, медиа, цветные, цифры; у
//! остальной техники (кондиционеры, вентиляторы, свет) — питание и сетка кнопок. Что не легло в
//! раскладку — в «Других кнопках». Громкость, каналы и стрелки повторяются при удержании.

use std::collections::HashSet;

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, W};
use crate::icons;
use crate::ir::Signal;
use crate::keys;
use crate::state::{Page, St};
use crate::store::Remote;

enum Face {
    Icon(&'static str),
    Text(String),
}

/// Кнопка пульта; нет такого сигнала — неактивная заглушка.
fn key(st: St, sig: Option<&Signal>, face: Face, class: &str) -> W {
    let face_w = move |f: &Face| -> W {
        match f {
            Face::Icon(g) => boxed(icon(g).class("ic key-ic")),
            Face::Text(t) => boxed(Text::new(t.clone()).max_lines(1).elide(Elide::End).class("key-text")),
        }
    };
    let Some(sig) = sig.cloned() else {
        return boxed(DecoratedBox::new().class(&format!("key {class} off")).child(face_w(&face)));
    };
    let name = sig.name.clone();
    let class = class.to_string();
    let lit = Reactive::new(move || -> Vec<W> {
        let on = st.flash.get().as_deref() == Some(name.as_str());
        vec![boxed(DecoratedBox::new().class(&format!("key {class}{}", if on { " lit" } else { "" })).child(face_w(&face)))]
    });
    let tooltip = keys::pretty(&sig.name);
    let gd = if keys::repeats(keys::key_of(&sig.name)) {
        // удержание — повтор; отпускание (и уход пальца) — стоп
        GestureDetector::new().on_press(move |_| st.press(sig.clone())).on_release(move |_| st.release())
    } else {
        GestureDetector::new().on_click(move || st.send(&sig))
    };
    super::tip(gd.child(lit), tooltip)
}

struct Board<'a> {
    st: St,
    signals: &'a [Signal],
    used: HashSet<String>,
}

impl<'a> Board<'a> {
    fn get(&mut self, k: &str) -> Option<&'a Signal> {
        let s = keys::find(self.signals, k)?;
        self.used.insert(s.name.clone());
        Some(s)
    }
    fn has(&self, k: &str) -> bool {
        keys::find(self.signals, k).is_some()
    }
    /// Кнопка по ключу со значком из раскладки (или подписью).
    fn k(&mut self, k: &str, class: &str) -> W {
        let s = self.get(k);
        let face = match keys::glyph(k) {
            Some(g) => Face::Icon(g),
            None => Face::Text(s.map(|s| keys::pretty(&s.name)).unwrap_or_else(|| k.to_uppercase())),
        };
        key(self.st, s, face, class)
    }
    fn text(&mut self, k: &str, label: String, class: &str) -> W {
        let s = self.get(k);
        key(self.st, s, Face::Text(label), class)
    }
    fn any(&self, ks: &[&str]) -> bool {
        ks.iter().any(|k| self.has(k))
    }
}

fn spacer(class: &str) -> W {
    boxed(DecoratedBox::new().class(class))
}

/// Качели «+ / подпись / −».
fn rocker(b: &mut Board, up: &str, dn: &str, label: String) -> W {
    boxed(
        DecoratedBox::new().class("rocker").child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(b.k(up, "rock"))
                .child(Text::new(label).class("rock-label"))
                .child(b.k(dn, "rock")),
        ),
    )
}

fn power_row(b: &mut Board) -> Row {
    let mut row = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if b.has("power") || !b.any(&["power_on", "power_off"]) {
        row = row.child(b.k("power", "round power"));
    }
    if b.has("power_on") {
        let label = t!("Вкл");
        row = row.child(b.text("power_on", label, "pill power-soft"));
    }
    if b.has("power_off") {
        let label = t!("Выкл");
        row = row.child(b.text("power_off", label, "pill power-soft"));
    }
    row
}

/// Телевизоры и медиатехника.
fn media_board(b: &mut Board) -> Column {
    let st = b.st;
    let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center);
    // питание, вход, без звука
    let mut right = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if b.has("source") {
        right = right.child(b.k("source", "round"));
    }
    right = right.child(b.k("mute", "round"));
    let top = Row::new().main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::Center).child(power_row(b)).child(right);
    col = col.child(DecoratedBox::new().class("board-row").child(top));

    // крестовина
    if b.any(&["up", "down", "left", "right", "ok"]) {
        let ok_label = t!("OK");
        let ok = b.text("ok", ok_label, "dpad-ok");
        col = col.child(
            DecoratedBox::new().class("dpad").child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(spacer("dpad-gap")).child(b.k("up", "dpad-key")).child(spacer("dpad-gap")))
                    .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(b.k("left", "dpad-key")).child(ok).child(b.k("right", "dpad-key")))
                    .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(spacer("dpad-gap")).child(b.k("down", "dpad-key")).child(spacer("dpad-gap"))),
            ),
        );
    }

    // навигация
    let nav: Vec<&str> = ["back", "home", "menu", "settings", "exit"].into_iter().filter(|k| b.has(k)).take(4).collect();
    if !nav.is_empty() {
        let mut row = Row::new().gap(14.0).main_axis_alignment(MainAxisAlignment::Center);
        for k in nav {
            row = row.child(b.k(k, "round small"));
        }
        col = col.child(row);
    }

    // громкость и каналы
    if b.any(&["vol_up", "vol_dn", "ch_up", "ch_dn"]) {
        let mut mid = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let extra: Vec<&str> = ["info", "guide", "list"].into_iter().filter(|k| b.has(k)).take(2).collect();
        for k in extra {
            mid = mid.child(b.k(k, "round small"));
        }
        if b.any(&["0", "1", "2", "5", "9"]) {
            // цифры живут на панели, даже когда она закрыта
            for d in ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"] {
                b.get(d);
            }
            let open = st.numpad.get_untracked();
            mid = mid.child(super::icon_button(icons::DIALPAD, t!("Цифры"), if open { "key-toggle on" } else { "key-toggle" }, move || st.numpad.set(!st.numpad.get_untracked())));
        }
        let row = Row::new()
            .gap(22.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(rocker(b, "vol_up", "vol_dn", t!("ГРОМК")))
            .child(mid)
            .child(rocker(b, "ch_up", "ch_dn", t!("КАН")));
        col = col.child(row);
    }

    // цифры
    if st.numpad.get_untracked() && b.any(&["0", "1", "2", "5", "9"]) {
        let mut grid = Grid::new(3).gap(10.0);
        for d in ["1", "2", "3", "4", "5", "6", "7", "8", "9"] {
            grid = grid.child(b.text(d, d.to_string(), "num"));
        }
        grid = grid.child(spacer("num-gap")).child(b.text("0", "0".into(), "num")).child(spacer("num-gap"));
        col = col.child(DecoratedBox::new().class("numpad").child(grid));
    }

    // медиа
    let media: Vec<&str> = ["prev", "rewind", "play", "play_pause", "pause", "stop", "ffwd", "next", "record"].into_iter().filter(|k| b.has(k)).collect();
    if !media.is_empty() {
        let mut row = Flex::new().direction(FlexDirection::Row).gap(6.0).wrap().main_axis_alignment(MainAxisAlignment::Center);
        for k in media {
            row = row.child(b.k(k, "round media"));
        }
        col = col.child(row);
    }

    // цветные
    if b.any(&["red", "green", "yellow", "blue"]) {
        let mut row = Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::Center);
        for k in ["red", "green", "yellow", "blue"] {
            row = row.child(b.text(k, String::new(), &format!("color {k}")));
        }
        col = col.child(row);
    }

    // приложения
    let apps: Vec<(&str, &str)> = [("netflix", "Netflix"), ("youtube", "YouTube"), ("prime", "Prime Video")].into_iter().filter(|(k, _)| b.has(k)).collect();
    if !apps.is_empty() {
        let mut row = Flex::new().direction(FlexDirection::Row).gap(8.0).wrap().main_axis_alignment(MainAxisAlignment::Center);
        for (k, label) in apps {
            row = row.child(b.text(k, label.to_string(), &format!("pill app {k}")));
        }
        col = col.child(row);
    }
    col
}

/// Кондиционеры, вентиляторы, свет и прочее — питание и сетка кнопок.
fn plain_board(b: &mut Board) -> Column {
    let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center);
    col = col.child(DecoratedBox::new().class("board-row").child(power_row(b).main_axis_alignment(MainAxisAlignment::Center)));
    if b.any(&["temp_up", "temp_dn"]) {
        col = col.child(rocker(b, "temp_up", "temp_dn", t!("ТЕМП")));
    }
    col
}

/// Остальные кнопки пульта — сеткой с подписями.
fn rest(b: &mut Board, title: String, cols: usize, width: f32) -> Option<W> {
    let left: Vec<&Signal> = b.signals.iter().filter(|s| !b.used.contains(&s.name)).collect();
    if left.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let mut grid = Grid::new(cols).gap(8.0);
    for s in left {
        if !seen.insert(s.name.clone()) {
            continue;
        }
        let face = match keys::key_of(&s.name).and_then(keys::glyph) {
            Some(g) if keys::pretty(&s.name).len() <= 3 => Face::Icon(g),
            _ => Face::Text(keys::pretty(&s.name)),
        };
        let cell = (width - 8.0 * (cols as f32 - 1.0)) / cols as f32;
        grid = grid.child(DecoratedBox::new().style("width", StyleValue::px(cell)).child(key(b.st, Some(s), face, "cell")));
    }
    Some(boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("section")).child(grid)))
}

fn is_media(r: &Remote) -> bool {
    let media_cat = !matches!(r.category.as_str(), "ACs" | "Fans" | "LED_Lighting" | "Air_Purifiers" | "Heaters" | "Fireplaces" | "Humidifiers" | "Vacuum_Cleaners" | "Window_cleaners");
    let keys_like = ["vol_up", "vol_dn", "up", "down", "ok", "ch_up"].iter().any(|k| keys::find(&r.signals, k).is_some());
    media_cat && keys_like
}

fn header(st: St, r: &Remote) -> W {
    let (kind, glyph, _) = crate::db::category_info(&r.category);
    let sub = if r.model.is_empty() { format!("{} · {kind}", r.brand) } else { format!("{} · {}", r.brand, r.model) };
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("remote-head")
            .child(DecoratedBox::new().class("card-badge").child(icon(glyph).class("ic card-badge-ic")))
            .child(DecoratedBox::new().class("grow").child(Column::new().gap(1.0).child(Text::new(r.name.clone()).max_lines(1).elide(Elide::End).class("card-title")).child(Text::new(sub).max_lines(1).elide(Elide::End).class("hint"))))
            .child(super::icon_button(icons::MORE, t!("Ещё"), "tb-btn", move || st.menu_open.set(!st.menu_open.get_untracked()))),
    )
}

pub fn page(st: St, id: String) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.numpad.get();
        let Some(r) = st.remotes.get().into_iter().find(|r| r.id == id) else {
            return vec![boxed(Text::new(t!("Пульт не найден")).class("hint page"))];
        };
        // пульт — колонкой не шире 480 по центру свободного места
        let vw = viewport_size().get().width;
        let free = if vw >= 720.0 && !st.window.get_untracked().fullscreen { vw - 260.0 } else { vw };
        let width = (free - 32.0).clamp(280.0, 480.0);
        let mut b = Board { st, signals: &r.signals, used: HashSet::new() };
        let media = is_media(&r);
        let board = if media { media_board(&mut b) } else { plain_board(&mut b) };
        let mut col = Column::new().gap(20.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(header(st, &r));
        if st.ir.get_untracked().is_none() {
            col = col.child(
                DecoratedBox::new().class("banner").child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(icons::WARNING).class("ic warn-ic"))
                        .child(Text::new(t!("ИК-передатчик не найден — сигналы не отправляются")).max_lines(2).class("hint grow")),
                ),
            );
        }
        col = col.child(DecoratedBox::new().class("remote-body").child(board));
        let title = if media { t!("Другие кнопки") } else { t!("Кнопки") };
        if let Some(w) = rest(&mut b, title, 3, width) {
            col = col.child(w);
        }
        let page = DecoratedBox::new().class("remote-page").style("width", StyleValue::px(width)).child(col);
        vec![boxed(ScrollView::new().vertical().child(Row::new().main_axis_alignment(MainAxisAlignment::Center).class("page").child(page)))]
    }))
}

/// Меню пульта: переименовать, другая модель с тем же кодом, удалить.
pub fn menu_layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        if !st.menu_open.get() {
            return vec![];
        }
        let Page::Remote(id) = st.page.get() else { return vec![] };
        let Some(r) = st.remote(&id) else { return vec![] };
        let item = |glyph: &'static str, label: String, class: &'static str, f: Box<dyn FnMut() + Send>| -> W {
            let f = std::sync::Mutex::new(f);
            boxed(GestureDetector::new().on_click(move || (f.lock().unwrap())()).child(DecoratedBox::new().class(&format!("menu-item {class}")).child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("ic menu-ic")).child(Text::new(label).max_lines(1).elide(Elide::End).class("menu-text")))))
        };
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let id2 = id.clone();
        col = col.child(item(
            icons::EDIT,
            t!("Переименовать"),
            "",
            Box::new(move || {
                st.menu_open.set(false);
                st.rename.set(Some(id2.clone()));
            }),
        ));
        if !r.alternatives.is_empty() {
            col = col.child(Text::new(t!("Другие модели с тем же кодом")).class("section menu-section"));
            for path in r.alternatives.iter().take(8) {
                let (id3, p) = (id.clone(), path.clone());
                let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
                col = col.child(item(
                    icons::SWAP,
                    name,
                    "",
                    Box::new(move || {
                        st.menu_open.set(false);
                        st.switch_model(&id3, &p);
                    }),
                ));
            }
        }
        col = col.child(item(
            icons::DELETE,
            t!("Удалить пульт"),
            "danger",
            Box::new(move || {
                st.menu_open.set(false);
                st.delete_open.set(true);
            }),
        ));
        let scrim = GestureDetector::new().on_click(move || st.menu_open.set(false)).child(DecoratedBox::new().class("scrim-clear"));
        let panel = Column::new().cross_axis_alignment(CrossAxisAlignment::End).class("menu-wrap").child(DecoratedBox::new().class("menu").child(col));
        vec![boxed(Stack::new().fit(StackFit::Expand).child(scrim).child(panel))]
    }))
}

/// Подтверждение удаления и переименование.
pub fn dialogs(st: St) -> W {
    let del = ConfirmDialog::new(t!("Удалить пульт?"), t!("Пульт исчезнет из «Моих пультов». Его всегда можно подобрать заново."), st.delete_open, move |ok| {
        if ok {
            if let Page::Remote(id) = st.page.get_untracked() {
                st.delete(&id);
            }
        }
    })
    .icon(icons::DELETE)
    .width(360.0);
    let rename = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = st.rename.get() else { return vec![] };
        let Some(r) = st.remote(&id) else { return vec![] };
        let text = use_signal(r.name.clone());
        let (id1, id2) = (id.clone(), id.clone());
        let card = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(t!("Название пульта")).class("card-title"))
            .child(
                TextField::new()
                    .text(r.name.clone())
                    .autofocus(true)
                    .on_change(move |t| text.set(t.to_string()))
                    .on_submit(move |t| {
                        st.set_name(&id1, t);
                        st.rename.set(None);
                    })
                    .on_escape(move || st.rename.set(None))
                    .class("field"),
            )
            .child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new(t!("Отмена")).class("btn flat").on_click(move || st.rename.set(None)))
                    .child(Button::new(t!("Сохранить")).class("btn primary").on_click(move || {
                        st.set_name(&id2, &text.get_untracked());
                        st.rename.set(None);
                    })),
            );
        let scrim = GestureDetector::new().on_click(move || st.rename.set(None)).child(DecoratedBox::new().class("scrim"));
        let centered = Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("dialog-wrap").child(DecoratedBox::new().class("dialog-card").child(card));
        vec![boxed(Stack::new().fit(StackFit::Expand).child(scrim).child(centered))]
    });
    boxed(Stack::new().fit(StackFit::Expand).child(del).child(rename))
}
