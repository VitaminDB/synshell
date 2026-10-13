//! Добавление пульта: тип техники → марка → автоподбор (или модель из списка) → сохранение.

use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, W};
use crate::db::{self, Model};
use crate::icons;
use crate::keys;
use crate::state::{Page, St};

/// Поле поиска: пишет в `st.query`, само при этом не перестраивается.
fn search_field(st: St, placeholder: String) -> W {
    boxed(TextField::new().text(st.query.get_untracked()).placeholder(placeholder).prefix_icon(icons::SEARCH).on_change(move |t| st.query.set(t.to_string())).class("field search"))
}

fn row_item(glyph: Option<&str>, title: String, sub: String, on_click: impl FnMut() + Send + 'static) -> W {
    let mut row = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(g) = glyph {
        row = row.child(DecoratedBox::new().class("row-badge").child(icon(g).class("ic row-badge-ic")));
    }
    let mut text = Column::new().gap(1.0).child(Text::new(title).max_lines(1).elide(Elide::End).class("row-title"));
    if !sub.is_empty() {
        text = text.child(Text::new(sub).max_lines(1).elide(Elide::End).class("hint"));
    }
    row = row.child(DecoratedBox::new().class("grow").child(text)).child(icon(icons::CHEVRON).class("ic chevron"));
    boxed(GestureDetector::new().on_click(on_click).child(DecoratedBox::new().class("row-item").child(row)))
}

fn page_column() -> Column {
    Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("page narrow-page")
}

/// Тип техники (и поиск модели по всей базе).
pub fn categories(st: St) -> W {
    if st.db.get_untracked().is_none() {
        return boxed(ScrollView::new().vertical().child(page_column().child(Text::new(t!("Чтобы добавить пульт, нужна база кодов.")).class("hint")).child(super::home::db_card(st))));
    }
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(db) = st.db.get() else { return vec![] };
        let q = st.query.get();
        if !q.trim().is_empty() {
            let found = db::search(&db, &q, 60);
            let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            if found.is_empty() {
                col = col.child(Text::new(t!("Ничего не найдено")).class("hint"));
            }
            for (c, b, m) in found {
                let (kind, glyph, _) = db::category_info(&c.dir);
                let (cat, brand, model) = (c.dir.clone(), b.name.clone(), m.clone());
                col = col.child(row_item(Some(glyph), m.name.clone(), format!("{} · {kind}", b.name), move || st.choose_model(&cat, &brand, &model)));
            }
            return vec![boxed(col)];
        }
        let mut grid = Grid::new(if viewport_size().get_untracked().width > 560.0 { 3 } else { 2 }).gap(10.0);
        for c in &db.categories {
            let (name, glyph, _) = db::category_info(&c.dir);
            let dir = c.dir.clone();
            grid = grid.child(
                GestureDetector::new().on_click(move || st.go(Page::Brands(dir.clone()))).child(
                    DecoratedBox::new().class("tile").child(
                        Column::new()
                            .gap(8.0)
                            .child(DecoratedBox::new().class("card-badge").child(icon(glyph).class("ic card-badge-ic")))
                            .child(Text::new(name).max_lines(2).elide(Elide::End).class("card-title"))
                            .child(Text::new(tn!(c.count(), "{n} пульт", "{n} пульта", "{n} пультов")).class("hint")),
                    ),
                ),
            );
        }
        vec![boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Что будем подключать?")).class("section")).child(grid))]
    });
    boxed(ScrollView::new().vertical().child(page_column().child(search_field(st, t!("Поиск по марке и модели"))).child(body)))
}

/// Марки типа техники: «не знаю марку», популярные, все.
pub fn brands(st: St, cat: String) -> W {
    let cat2 = cat.clone();
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(db) = st.db.get() else { return vec![] };
        let Some(c) = db.category(&cat2) else { return vec![] };
        let q = st.query.get().trim().to_lowercase();
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if q.is_empty() {
            let cat = cat2.clone();
            col = col.child(
                GestureDetector::new().on_click(move || st.start_pick(cat.clone(), None)).child(
                    DecoratedBox::new().class("row-item accent").child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("row-badge on").child(icon(icons::AUTO).class("ic row-badge-ic")))
                            .child(DecoratedBox::new().class("grow").child(Column::new().gap(1.0).child(Text::new(t!("Не знаю марку")).class("row-title")).child(Text::new(t!("Перебрать самые распространённые коды")).max_lines(2).class("hint"))))
                            .child(icon(icons::CHEVRON).class("ic chevron")),
                    ),
                ),
            );
            let mut popular: Vec<_> = c.brands.iter().filter(|b| b.models.len() >= 3).collect();
            popular.sort_by(|a, b| b.models.len().cmp(&a.models.len()));
            if popular.len() >= 4 {
                let mut chips = Flex::new().direction(FlexDirection::Row).gap(8.0).wrap();
                for b in popular.into_iter().take(12) {
                    let (cat, brand) = (cat2.clone(), b.name.clone());
                    chips = chips.child(Button::new(b.name.clone()).class("btn chip").on_click(move || st.start_pick(cat.clone(), Some(brand.clone()))));
                }
                col = col.child(Text::new(t!("Популярные")).class("section")).child(chips);
            }
            col = col.child(Text::new(t!("Все марки")).class("section"));
        }
        let mut any = false;
        for b in c.brands.iter().filter(|b| q.is_empty() || b.name.to_lowercase().contains(&q)) {
            any = true;
            let (cat, brand) = (cat2.clone(), b.name.clone());
            col = col.child(row_item(None, b.name.clone(), tn!(b.models.len(), "{n} пульт", "{n} пульта", "{n} пультов"), move || st.start_pick(cat.clone(), Some(brand.clone()))));
        }
        if !any {
            col = col.child(Text::new(t!("Такой марки нет в базе — попробуйте «Не знаю марку»")).max_lines(2).class("hint"));
        }
        vec![boxed(col)]
    });
    let (name, _, _) = db::category_info(&cat);
    boxed(ScrollView::new().vertical().child(page_column().child(search_field(st, t!("Марка: {kind}", kind = name.to_lowercase()))).child(body)))
}

/// Модели марки — выбор вручную; кнопка ⏻ рядом проверяет питание.
pub fn models(st: St, cat: String, brand: String) -> W {
    let (cat2, brand2) = (cat.clone(), brand.clone());
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(db) = st.db.get() else { return vec![] };
        let Some(b) = db.category(&cat2).and_then(|c| c.brand(&brand2)) else { return vec![] };
        let q = st.query.get().trim().to_lowercase();
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for m in b.models.iter().filter(|m| q.is_empty() || m.name.to_lowercase().contains(&q)) {
            col = col.child(model_row(st, &cat2, &brand2, m));
        }
        vec![boxed(col)]
    });
    let (cat3, brand3) = (cat.clone(), brand.clone());
    boxed(
        ScrollView::new().vertical().child(
            page_column()
                .child(search_field(st, t!("Модель пульта или телевизора")))
                .child(Button::new(t!("Подобрать автоматически")).icon(icons::AUTO).class("btn ghost").on_click(move || st.start_pick(cat3.clone(), Some(brand3.clone()))))
                .child(body),
        ),
    )
}

fn model_row(st: St, cat: &str, brand: &str, m: &Model) -> W {
    let (cat, brand, model) = (cat.to_string(), brand.to_string(), m.clone());
    let path = m.path.clone();
    let test = super::icon_button(icons::POWER, t!("Проверить питание"), "row-test", move || {
        let signals = db::read(&path);
        match keys::power(&signals) {
            Some(s) => st.send(s),
            None => st.toast(t!("В этом пульте нет кнопок")),
        }
    });
    boxed(
        DecoratedBox::new().class("row-item").child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(GestureDetector::new().on_click(move || st.choose_model(&cat, &brand, &model)).child(DecoratedBox::new().class("row-hit").child(Text::new(m.name.clone()).max_lines(1).elide(Elide::End).class("row-title")))))
                .child(test),
        ),
    )
}

fn test_label(key: &str) -> String {
    match key {
        "mute" => t!("Без звука"),
        "vol_up" => t!("Громкость +"),
        _ => t!("Питание"),
    }
}

const TESTS: [&str; 3] = ["power", "mute", "vol_up"];

/// Автоподбор: отправить код, спросить, сработало ли.
pub fn pick(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(p) = st.pick.get() else { return vec![] };
        let (kind, glyph, _) = db::category_info(&p.cat);
        let what = match &p.brand {
            Some(b) => format!("{b} · {kind}"),
            None => t!("{kind} · любая марка", kind = kind),
        };
        let mut col = page_column().cross_axis_alignment(CrossAxisAlignment::Center);
        if p.loading {
            col = col.child(CircularProgress::new().indeterminate().size(36.0).stroke_width(3.0)).child(Text::new(t!("Собираю варианты кодов…")).class("hint"));
            return vec![boxed(col)];
        }
        let n = p.cands.len();
        if n == 0 {
            col = col.child(DecoratedBox::new().class("hero").child(icon(icons::HELP).class("ic hero-ic"))).child(Text::new(t!("Для «{w}» в базе нет кодов", w = what)).max_lines(2).class("title center"));
            if p.brand.is_some() {
                let cat = p.cat.clone();
                col = col.child(Button::new(t!("Подобрать без марки")).icon(icons::AUTO).class("btn primary").on_click(move || st.start_pick(cat.clone(), None)));
            }
            return vec![boxed(col)];
        }
        if p.idx >= n {
            col = col
                .child(DecoratedBox::new().class("hero").child(icon(icons::HELP).class("ic hero-ic")))
                .child(Text::new(t!("Варианты закончились")).class("title"))
                .child(Text::new(t!("Ни один из {n} кодов не подошёл. Проверьте, что телефон направлен на технику и до неё не дальше 3–5 м, и попробуйте ещё раз или другую проверочную кнопку.", n = n)).max_lines(5).class("hint center"))
                .child(Button::new(t!("Начать сначала")).icon(icons::REFRESH).class("btn primary").on_click(move || {
                    st.pick_step(-(n as isize));
                }));
            if p.brand.is_some() {
                let cat = p.cat.clone();
                col = col.child(Button::new(t!("Подобрать без марки")).icon(icons::AUTO).class("btn ghost").on_click(move || st.start_pick(cat.clone(), None)));
            }
            return vec![boxed(col)];
        }
        let cand = &p.cands[p.idx];
        let test_key = if cand.signals.iter().any(|s| keys::key_of(&s.name) == Some(p.test)) { p.test } else { "power" };
        let test_glyph = keys::glyph(test_key).unwrap_or(icons::POWER);

        // заголовок и ход
        col = col
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("ic muted-ic")).child(Text::new(what).class("hint")))
            .child(Text::new(t!("Вариант {i} из {n}", i = p.idx + 1, n = n)).class("title"))
            .child(DecoratedBox::new().class("pick-bar").child(ProgressBar::new().value((p.idx + 1) as f32 / n as f32).class("bar")));

        // большая кнопка отправки
        let sig_name = cand.test(test_key).map(|s| s.name.clone()).unwrap_or_default();
        let lit = Reactive::new(move || -> Vec<W> {
            let on = st.flash.get().as_deref() == Some(sig_name.as_str());
            vec![boxed(DecoratedBox::new().class(if on { "send-ring lit" } else { "send-ring" }).child(DecoratedBox::new().class("send-core").child(icon(test_glyph).class("ic send-ic"))))]
        });
        col = col.child(GestureDetector::new().on_click(move || st.pick_send()).child(lit));
        col = col.child(
            Text::new(if p.auto {
                t!("Автоперебор: коды уходят каждые 3 секунды. Нажмите «Сработало», как только техника отреагирует.")
            } else if p.sent {
                t!("Сигнал отправлен. Техника отреагировала?")
            } else {
                t!("Направьте телефон на технику и нажмите круг")
            })
            .max_lines(3)
            .class("hint center"),
        );

        if p.auto {
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .child(Button::new(t!("Стоп")).icon(icons::CLOSE).class("btn ghost big").on_click(move || st.stop_auto()))
                    .child(Button::new(t!("Сработало!")).icon(icons::CHECK).class("btn primary big").on_click(move || st.pick_yes())),
            );
        } else {
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .child(Button::new(t!("Нет, дальше")).icon(icons::CLOSE).class("btn ghost big").on_click(move || st.pick_step(1)))
                    .child(Button::new(t!("Да, работает")).icon(icons::CHECK).class("btn primary big").disabled(!p.sent).on_click(move || st.pick_yes())),
            );
        }

        // проверочная кнопка и автоперебор
        let mut tests = Row::new().gap(2.0);
        for k in TESTS {
            let b = Button::new(test_label(k)).icon(keys::glyph(k).unwrap_or(icons::POWER)).class(if p.test == k { "btn test on" } else { "btn test" });
            tests = tests.child(b.on_click(move || st.set_test(k)));
        }
        col = col.child(Text::new(t!("Чем проверять")).class("section")).child(DecoratedBox::new().class("tests").child(tests));
        let mut tools = Flex::new().direction(FlexDirection::Row).gap(8.0).wrap().main_axis_alignment(MainAxisAlignment::Center);
        if p.idx > 0 && !p.auto {
            tools = tools.child(Button::new(t!("Предыдущий")).icon(icons::LEFT).class("btn flat").on_click(move || st.pick_step(-1)));
        }
        if !p.auto {
            tools = tools.child(Button::new(t!("Автоперебор")).icon(icons::AUTO).class("btn flat").on_click(move || st.toggle_auto()));
        }
        if let Some(b) = p.brand.clone() {
            let cat = p.cat.clone();
            tools = tools.child(Button::new(t!("Модель из списка")).icon(icons::LIST).class("btn flat").on_click(move || st.go(Page::Models(cat.clone(), b.clone()))));
        }
        col = col.child(tools);

        // какие пульты за этим кодом
        let about = match cand.models.len() {
            0 => t!("Код из универсальной библиотеки Flipper"),
            1 => cand.models[0].name.clone(),
            k => t!("{m} и ещё {k}", m = cand.models[0].name, k = k - 1),
        };
        col = col.child(Text::new(format!("{} · {}", about, cand.test(test_key).map(|s| crate::ir::describe(&s.code)).unwrap_or_default())).max_lines(2).class("hint center small"));
        vec![boxed(ScrollView::new().vertical().child(col))]
    }))
}

/// Пульт найден: название, модель, проверка других кнопок, сохранение.
pub fn found(st: St) -> W {
    let Some(f0) = st.found.get_untracked() else { return boxed(Column::new()) };
    let name = TextField::new().text(f0.name.clone()).placeholder(t!("Название")).on_change(move |t| st.found_name(t)).class("field");
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(f) = st.found.get() else { return vec![] };
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if f.models.len() > 1 {
            let items: Vec<DropdownItem> = f.models.iter().enumerate().map(|(i, m)| DropdownItem::new(i.to_string(), m.name.clone())).collect();
            col = col
                .child(Text::new(t!("Модель пульта · {n} с таким кодом", n = f.models.len())).class("section"))
                .child(Dropdown::with_items(items).selected(f.sel.to_string()).max_height(320.0).on_change(move |v| st.found_select(v.parse().unwrap_or(0))).class("field"));
        } else if let Some(m) = f.models.first() {
            col = col.child(Text::new(t!("Модель пульта: {m}", m = m.name)).class("hint"));
        }
        // проверка основных кнопок
        let mut tests = Flex::new().direction(FlexDirection::Row).gap(8.0).wrap();
        for key in ["power", "vol_up", "vol_dn", "mute", "ch_up", "ch_dn", "ok", "source"] {
            if let Some(s) = keys::find(&f.signals, key) {
                let s = s.clone();
                let label = keys::pretty(&s.name);
                let mut b = Button::new(label).class("btn chip");
                if let Some(g) = keys::glyph(key) {
                    b = b.icon(g);
                }
                tests = tests.child(b.on_click(move || st.send(&s)));
            }
        }
        col = col
            .child(Text::new(t!("Проверьте кнопки")).class("section"))
            .child(tests)
            .child(Text::new(tn!(f.signals.len(), "В пульте {n} кнопка", "В пульте {n} кнопки", "В пульте {n} кнопок")).class("hint"));
        vec![boxed(col)]
    });
    boxed(
        ScrollView::new().vertical().child(
            page_column()
                .child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("hero ok").child(icon(icons::CHECK).class("ic hero-ic"))).child(Text::new(t!("Пульт подобран")).class("title")))
                .child(Text::new(t!("Название")).class("section"))
                .child(name)
                .child(body)
                .child(Button::new(t!("Сохранить пульт")).icon(icons::CHECK).class("btn primary big").on_click(move || st.save_found())),
        ),
    )
}
