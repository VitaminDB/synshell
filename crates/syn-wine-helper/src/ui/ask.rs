//! «Программе не хватает компонентов»: что поставить, сколько скачать, установка и повторный запуск.

use std::sync::atomic::{AtomicBool, Ordering};

use syngui::prelude::*;

use super::{boxed, db, icon, icons, progress, size_mb, Mode, Phase, St, W};
use crate::launch::Failed;

/// Пользователь согласился запускать (для `run` — main запускает Wine после окна).
pub static ACCEPTED: AtomicBool = AtomicBool::new(false);

fn finish(st: St) {
    match st.mode {
        Mode::AskBeforeRun => {
            ACCEPTED.store(true, Ordering::SeqCst);
            syngui::signal::quit_app();
        }
        _ => {
            let Some(f) = st.failed.get_untracked() else { return syngui::signal::quit_app() };
            if f.running {
                st.phase.set(Phase::Done(t!("Компоненты установлены. Закройте программу и запустите её снова.")));
                return;
            }
            if Failed::under_steam() {
                st.phase.set(Phase::Done(t!("Готово. Запустите игру снова из Steam.")));
                return;
            }
            match f.relaunch() {
                Ok(()) => syngui::signal::quit_app(),
                Err(e) => st.phase.set(Phase::Error(t!("установлено, но программа не запустилась: {e}", e = e))),
            }
        }
    }
}

fn package_card(id: &str, arch: crate::pe::Arch) -> W {
    let db = db();
    let Some(p) = db.package(id) else { return boxed(DecoratedBox::new()) };
    let mut col = Column::new().gap(4.0).child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(p.name.clone()).class("name"))
            .child(DecoratedBox::new().class("chip").child(Text::new(arch.name()).class("chip-text"))),
    );
    if !p.description.is_empty() {
        col = col.child(Text::new(p.description.clone()).max_lines(3).class("muted small"));
    }
    col = col.child(Text::new(p.dlls.join(", ")).max_lines(2).class("dlls"));
    boxed(DecoratedBox::new().class("card").child(Row::new().gap(12.0).child(DecoratedBox::new().class("badge").child(icon(icons::EXTENSION).class("ic badge-ic"))).child(DecoratedBox::new().class("grow").child(col))))
}

fn unknown_card(dlls: &[String]) -> W {
    boxed(
        DecoratedBox::new().class("card warn-card").child(
            Row::new().gap(12.0).child(icon(icons::WARNING).class("ic warn-ic")).child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(4.0)
                        .child(Text::new(dlls.join(", ")).max_lines(3).class("name"))
                        .child(Text::new(t!("Этого нет ни в Wine, ни в базе компонентов. Обычно такие библиотеки лежат рядом с программой — проверьте, что она скопирована целиком.")).max_lines(5).class("muted small")),
                ),
            ),
        ),
    )
}

pub fn page(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let plan = st.plan.get();
        let failed = st.failed.get();
        let title = failed.as_ref().map(|f| f.title()).unwrap_or_default();
        let tried = failed.as_ref().is_some_and(|f| f.already_tried()) || !plan.present.is_empty();
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("hero").child(icon(icons::EXTENSION).class("ic hero-ic")))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(4.0)
                            .child(Text::new(t!("Не хватает компонентов")).class("title"))
                            .child(Text::new(t!("Программе «{p}» нужны библиотеки, которых нет в Wine.", p = &title)).max_lines(4).class("muted")),
                    ),
                ),
        );
        if tried {
            col = col.child(boxed(
                DecoratedBox::new().class("card warn-card").child(
                    Row::new()
                        .gap(12.0)
                        .child(icon(icons::WARNING).class("ic warn-ic"))
                        .child(DecoratedBox::new().class("grow").child(Text::new(t!("Эти библиотеки уже поставлены, но Wine их не нашёл. Нужен Wine с поддержкой общего каталога компонентов (патч syn-wine-helper).")).max_lines(6).class("small"))),
                ),
            ));
        }
        for (id, arch) in &plan.items {
            col = col.child(package_card(id, *arch));
        }
        if !plan.unknown.is_empty() {
            col = col.child(unknown_card(&plan.unknown));
        }
        if !plan.items.is_empty() {
            let size = plan.download_size(&db());
            let note = if size > 0 { t!("Скачать {s} — один раз: компоненты ставятся для всех программ.", s = size_mb(size)) } else { t!("Файлы уже скачаны: компоненты ставятся для всех программ.") };
            col = col.child(Text::new(note).max_lines(3).class("muted small"));
        }
        vec![boxed(col)]
    });
    let buttons = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phase = st.phase.get();
        let plan = st.plan.get_untracked();
        let busy = matches!(phase, Phase::Busy(..));
        let done = matches!(phase, Phase::Done(_));
        let mut row = Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center);
        if done {
            row = row.child(Button::new(t!("Закрыть")).class("primary").on_click(syngui::signal::quit_app));
            return vec![boxed(row)];
        }
        let close = if plan.items.is_empty() { t!("Закрыть") } else { t!("Отмена") };
        row = row.child(Button::new(close).disabled(busy).on_click(syngui::signal::quit_app));
        if st.mode == Mode::AskBeforeRun {
            row = row.child(Button::new(t!("Запустить так")).disabled(busy).on_click(move || {
                ACCEPTED.store(true, Ordering::SeqCst);
                syngui::signal::quit_app();
            }));
        }
        if !plan.items.is_empty() {
            let running = st.failed.get_untracked().is_some_and(|f| f.running);
            let label = if matches!(phase, Phase::Error(_)) {
                t!("Повторить")
            } else if running {
                t!("Установить")
            } else {
                t!("Установить и запустить")
            };
            let items = plan.items.clone();
            row = row.child(Button::new(label).icon(icons::DOWNLOAD).class("primary").disabled(busy).on_click(move || st.install(items.clone(), finish)));
        }
        vec![boxed(row)]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(DecoratedBox::new().class("page").child(body))))
            .child(DecoratedBox::new().class("footer").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(progress(st)).child(buttons))),
    )
}
