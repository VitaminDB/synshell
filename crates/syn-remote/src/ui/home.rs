//! «Мои пульты»: карточки сохранённых пультов, добавление, напоминание скачать базу.

use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, W};
use crate::icons;
use crate::state::{DbState, Page, St};

/// Карточка «нет базы кодов» — со скачиванием прямо здесь.
pub fn db_card(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        if st.db.get().is_some() {
            return vec![];
        }
        let mut col = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("card-badge").child(icon(icons::DOWNLOAD).class("ic card-badge-ic")))
                    .child(
                        Column::new()
                            .gap(2.0)
                            .child(Text::new(t!("Скачайте базу ИК-кодов")).class("card-title"))
                            .child(Text::new(t!("≈9 000 пультов: телевизоры, кондиционеры, проекторы, аудио и другое. Загрузка — около 20 МБ.")).max_lines(3).class("hint")),
                    ),
            );
        col = col.child(super::database::progress(st));
        if !matches!(st.db_state.get(), DbState::Busy(..)) {
            col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(Button::new(t!("Скачать")).icon(icons::DOWNLOAD).class("btn primary").on_click(move || st.download_db())));
        }
        vec![boxed(DecoratedBox::new().class("card").child(col))]
    }))
}

pub fn page(st: St, wide: bool) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let remotes = st.remotes.get();
        if remotes.is_empty() {
            return vec![boxed(
                Column::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("empty")
                    .child(DecoratedBox::new().class("hero").child(icon(icons::REMOTE).class("ic hero-ic")))
                    .child(Text::new(t!("Пультов пока нет")).class("title"))
                    .child(Text::new(t!("Добавьте технику — пульт подберётся автоматически: направьте телефон и отвечайте, сработало ли.")).max_lines(4).class("hint center"))
                    .child(Button::new(t!("Добавить пульт")).icon(icons::ADD).class("btn primary big").on_click(move || st.go(Page::Categories))),
            )];
        }
        let cols = if wide { 3 } else { 2 };
        let mut grid = Grid::new(cols).gap(12.0);
        for r in remotes {
            let id = r.id.clone();
            let (kind, glyph, _) = crate::db::category_info(&r.category);
            grid = grid.child(
                GestureDetector::new().on_click(move || st.go(Page::Remote(id.clone()))).child(
                    DecoratedBox::new().class("remote-card").child(
                        Column::new()
                            .gap(6.0)
                            .child(DecoratedBox::new().class("card-badge").child(icon(glyph).class("ic card-badge-ic")))
                            .child(Text::new(r.name.clone()).max_lines(2).elide(Elide::End).class("card-title"))
                            .child(Text::new(if r.model.is_empty() { kind } else { r.model.clone() }).max_lines(1).elide(Elide::End).class("hint")),
                    ),
                ),
            );
        }
        grid = grid.child(
            GestureDetector::new().on_click(move || st.go(Page::Categories)).child(
                DecoratedBox::new().class("remote-card add").child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .child(icon(icons::ADD).class("ic add-ic"))
                        .child(Text::new(t!("Добавить")).class("card-title muted")),
                ),
            ),
        );
        vec![boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Мои пульты")).class("section")).child(grid))]
    });
    let ir_warn = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        if st.ir.get().is_some() {
            return vec![];
        }
        vec![boxed(
            DecoratedBox::new().class("banner").child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(icons::WARNING).class("ic warn-ic"))
                    .child(Text::new(t!("ИК-передатчик не найден: кнопки работают вхолостую. Нужна программа syn-ir (телефон с ИК-светодиодом).")).max_lines(3).class("hint grow")),
            ),
        )]
    });
    boxed(ScrollView::new().vertical().child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("page").child(ir_warn).child(db_card(st)).child(list)))
}
