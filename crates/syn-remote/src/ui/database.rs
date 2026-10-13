//! «База кодов»: что скачано, обновление, состояние ИК-передатчика.

use syngui::prelude::*;

use super::{boxed, icon, W};
use crate::icons;
use crate::state::{DbState, St};

/// Ход скачивания или ошибка (пусто, если ничего не происходит).
pub fn progress(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        match st.db_state.get() {
            DbState::Idle => vec![],
            DbState::Busy(text, frac) => {
                let bar = match frac {
                    Some(f) => ProgressBar::new().value(f.clamp(0.0, 1.0)),
                    None => ProgressBar::new().indeterminate(),
                };
                vec![boxed(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(bar.class("bar")).child(Text::new(text).class("hint")))]
            }
            DbState::Error(e) => vec![boxed(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(icons::WARNING).class("ic warn-ic"))
                    .child(Text::new(t!("Не удалось скачать: {e}", e = e)).max_lines(4).class("hint grow")),
            )],
        }
    }))
}

/// «13.10.2026 17:40» по местному времени.
fn date(ts: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = ts as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}.{:02}.{} {:02}:{:02}", tm.tm_mday, tm.tm_mon + 1, tm.tm_year + 1900, tm.tm_hour, tm.tm_min)
}

fn stat(value: String, label: String) -> W {
    boxed(DecoratedBox::new().class("stat").child(Column::new().gap(2.0).child(Text::new(value).class("stat-value")).child(Text::new(label).class("hint"))))
}

pub fn page(st: St) -> W {
    let info = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let busy = matches!(st.db_state.get(), DbState::Busy(..));
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        match st.db.get() {
            Some(db) => {
                let brands: usize = db.categories.iter().map(|c| c.brands.len()).sum();
                col = col
                    .child(Text::new(t!("Flipper-IRDB · загружена {d}", d = date(db.meta.updated))).class("card-title"))
                    .child(Row::new().gap(10.0).child(stat(db.count().to_string(), t!("пультов"))).child(stat(brands.to_string(), t!("марок"))).child(stat(db.categories.len().to_string(), t!("типов техники"))));
            }
            None => {
                col = col.child(Text::new(t!("База не скачана")).class("card-title")).child(Text::new(t!("Без базы можно пользоваться уже сохранёнными пультами, но не добавлять новые.")).max_lines(3).class("hint"));
            }
        }
        col = col.child(progress(st));
        let label = if st.db.get_untracked().is_some() { t!("Обновить") } else { t!("Скачать") };
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(Button::new(label).icon(if st.db.get_untracked().is_some() { icons::REFRESH } else { icons::DOWNLOAD }).class("btn primary").disabled(busy).on_click(move || st.download_db())));
        vec![boxed(DecoratedBox::new().class("card").child(col))]
    });
    let ir = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let (glyph, title, sub) = match st.ir.get() {
            Some(p) => (icons::SENSORS, t!("ИК-передатчик готов"), t!("Сигналы уходят через {p}", p = p)),
            None => (icons::WARNING, t!("ИК-передатчик не найден"), t!("Нет программы syn-ir: на этом устройстве кнопки только подсвечиваются.")),
        };
        vec![boxed(
            DecoratedBox::new().class("card").child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("card-badge").child(icon(glyph).class("ic card-badge-ic")))
                    .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(title).class("card-title")).child(Text::new(sub).max_lines(3).class("hint")))),
            ),
        )]
    });
    let about = Column::new()
        .gap(6.0)
        .child(Text::new(t!("Источники")).class("section"))
        .child(Text::new(t!("Коды пультов — открытая база Flipper-IRDB (github.com/Lucaslhm/Flipper-IRDB), универсальные коды для подбора — из прошивки Flipper Zero. Поддерживаются протоколы NEC, NECext, NEC42, Samsung32, RC5, RC5X, RC6, SIRC, Kaseikyo, RCA, Pioneer и сырые записи.")).max_lines(8).class("hint"));
    boxed(ScrollView::new().vertical().child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("page narrow-page").child(info).child(ir).child(about)))
}
