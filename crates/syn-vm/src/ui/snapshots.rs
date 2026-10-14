//! Снимки: у работающей машины — вместе с памятью (вернётся ровно в тот момент),
//! у выключенной — состояние дисков.

use syngui::prelude::*;

use super::parts::{self, boxed, btn, icon_btn, W};
use crate::state::{self, icons, Confirm, St};

pub fn page(st: St, narrow: bool) -> W {
    let field = TextField::new()
        .placeholder(t!("Название снимка, например «чистая установка»"))
        .on_change(move |s: &str| st.snap_name.set(s.to_string()))
        .on_submit(move |s: &str| state::snapshot_op(st, "create", s.to_string()))
        .class("field grow");
    let create = Reactive::new(move || -> Vec<W> {
        let Some(id) = st.sel.get() else { return vec![] };
        let on = st.live.get().get(&id).is_some_and(|l| l.power.on());
        let busy = st.busy.get().get(&id).cloned();
        let hint = if on {
            t!("Машина работает — снимок сохранит и память: после возврата программы продолжат с того же места.")
        } else {
            t!("Машина выключена — снимок сохранит состояние дисков.")
        };
        let action: W = match busy {
            Some(b) => boxed(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(20.0).stroke_width(2.5)).child(Text::new(b).class("muted"))),
            None => btn(icons::CAMERA, t!("Сохранить снимок"), "primary", move || {
                let mut name = st.snap_name.get_untracked();
                if name.trim().is_empty() {
                    name = state::chrono_like(crate::model::now());
                }
                state::snapshot_op(st, "create", name);
            }),
        };
        vec![boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(hint).max_lines(3).class("row-hint")).child(Row::new().child(action)))]
    });
    let list = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let snaps = st.snapshots.get();
        let Some(id) = st.sel.get() else { return vec![] };
        if snaps.is_empty() {
            return vec![parts::empty(icons::HISTORY, t!("Снимков пока нет"), t!("Сохраните состояние перед обновлением или экспериментом — вернуться к нему можно за секунды."))];
        }
        let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let n = snaps.len();
        for (i, s) in snaps.iter().enumerate().rev() {
            let (a, b) = ((id.clone(), s.name.clone()), (id.clone(), s.name.clone()));
            let line = Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(if i + 1 == n { "tl-dot last" } else { "tl-dot" }))
                .child(DecoratedBox::new().class(if i == 0 { "tl-line end" } else { "tl-line" }));
            let mem = if s.vm_size.starts_with("0 ") { t!("только диски") } else { t!("с памятью · {s}", s = s.vm_size) };
            col = col.child(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(line)
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(10.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Column::new().gap(3.0).child(Text::new(s.name.clone()).max_lines(1).class("row-title")).child(Text::new(format!("{} · {}", s.date, mem)).max_lines(1).class("row-hint")).class("grow"))
                                    .child(btn(icons::RESTORE, if narrow { String::new() } else { t!("Вернуться") }, "soft", move || {
                                        st.confirm.set(Some(Confirm::Restore(a.0.clone(), a.1.clone())));
                                        super::sync_grab(st);
                                    }))
                                    .child(icon_btn(icons::DELETE, t!("Удалить снимок"), move || {
                                        st.confirm.set(Some(Confirm::DeleteSnapshot(b.0.clone(), b.1.clone())));
                                        super::sync_grab(st);
                                    })),
                            )
                            .class("snap grow"),
                    ),
            );
        }
        vec![boxed(col)]
    });
    let body = Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(parts::section(
            icons::CAMERA,
            t!("Новый снимок"),
            vec![boxed(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Row::new().gap(10.0).child(field)).child(create).class("row"))],
        ))
        .child(parts::card(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(icons::HISTORY, None, 30.0)).child(Text::new(t!("История")).class("card-title grow")).child(icon_btn(icons::SYNC, t!("Обновить"), move || state::load_snapshots(st)))).child(list)));
    boxed(ScrollView::new().vertical().child(DecoratedBox::new().child(body).class(if narrow { "page narrow" } else { "page" })))
}
