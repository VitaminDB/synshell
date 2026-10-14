//! Список компонентов: что есть в базе, что поставлено, установка и удаление.

use syngui::prelude::*;

use super::{boxed, db, icon, icons, progress, Phase, St, W};
use crate::pe::Arch;

fn row(st: St, p: &crate::db::Package) -> W {
    let archs = p.archs();
    let done: Vec<Arch> = archs.iter().copied().filter(|a| crate::store::installed(p, *a)).collect();
    let mut chips = Row::new().gap(6.0);
    for a in &archs {
        let on = done.contains(a);
        chips = chips.child(DecoratedBox::new().class(if on { "chip on" } else { "chip" }).child(Text::new(if on { format!("{} ✓", a.name()) } else { a.name().to_string() }).class("chip-text")));
    }
    let info = Column::new()
        .gap(4.0)
        .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(p.name.clone()).class("name")).child(chips))
        .child(Text::new(p.description.clone()).max_lines(3).class("muted small"))
        .child(Text::new(p.dlls.join(", ")).max_lines(2).class("dlls"));
    let id = p.id.clone();
    let action: W = if done.len() == archs.len() {
        boxed(Button::new("").icon(icons::DELETE).class("flat round").on_click(move || {
            if st.busy() {
                return;
            }
            match crate::install::remove(&db(), &id) {
                Ok(_) => st.phase.set(Phase::Idle),
                Err(e) => st.phase.set(Phase::Error(format!("{e:#}"))),
            }
            st.changed.update(|n| *n += 1);
        }))
    } else {
        let items: Vec<(String, Arch)> = archs.iter().filter(|a| !done.contains(a)).map(|a| (id.clone(), *a)).collect();
        boxed(Button::new(t!("Установить")).icon(icons::DOWNLOAD).class("primary").on_click(move || st.install(items.clone(), |_| {})))
    };
    boxed(DecoratedBox::new().class("card").child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(info)).child(action)))
}

pub fn page(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.changed.get();
        let db = db();
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("hero").child(icon(icons::EXTENSION).class("ic hero-ic")))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(4.0)
                            .child(Text::new(t!("Компоненты Wine")).class("title"))
                            .child(Text::new(t!("Библиотеки, которых нет в самом Wine. Ставятся один раз — для всех программ и префиксов; когда программе чего-то не хватит, Wine предложит поставить сам.")).max_lines(5).class("muted")),
                    ),
                ),
        );
        for p in &db.packages {
            col = col.child(row(st, p));
        }
        col = col.child(Text::new(t!("Каталог: {d}", d = crate::store::dll_root().display())).max_lines(3).class("muted small"));
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(DecoratedBox::new().class("page").child(body))))
            .child(DecoratedBox::new().class("footer slim").child(progress(st))),
    )
}
