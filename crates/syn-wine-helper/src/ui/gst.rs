//! «Нечем воспроизвести видео или звук»: пакеты GStreamer из репозиториев, установка через pacman (synpkg).

use std::sync::Arc;

use syngui::prelude::*;
use synsystem::packages::{self as pk, JobEvent, Op};

use super::{boxed, icon, icons, progress, Phase, St, W};
use crate::gst::{GstPkg, Missing};

/// Что спросить: процесс, форматы и пакеты к установке.
#[derive(Clone, Default, PartialEq)]
pub struct GstAsk {
    pub missing: Missing,
    pub pkgs: Vec<GstPkg>,
}

fn install(st: St, names: Vec<String>) {
    if st.busy() || names.is_empty() {
        return;
    }
    st.phase.set(Phase::Busy(t!("Установка {p}…", p = names.join(", ")), None));
    let (cfg, _) = synshell_common::Config::load();
    let job = pk::start(Op::Install(names), cfg.packages.build_user.clone());
    std::thread::spawn(move || {
        let mut last = String::new();
        while let Ok(ev) = job.rx.recv() {
            match ev {
                JobEvent::Line(l) | JobEvent::Stage(l) => {
                    let l = l.trim().to_string();
                    if l.is_empty() || l == last {
                        continue;
                    }
                    last = l.clone();
                    run_on_main_thread(move || st.phase.set(Phase::Busy(l, None)));
                }
                JobEvent::Done(r) => {
                    run_on_main_thread(move || match r {
                        Ok(()) => st.phase.set(Phase::Done(t!("Модули установлены. Перезапустите программу, чтобы видео и звук заработали."))),
                        Err(e) => st.phase.set(Phase::Error(e)),
                    });
                    break;
                }
            }
        }
    });
}

fn package_card(p: &GstPkg) -> W {
    boxed(
        DecoratedBox::new().class("card").child(
            Row::new().gap(12.0).child(DecoratedBox::new().class("badge").child(icon(icons::MOVIE).class("ic badge-ic"))).child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(4.0)
                        .child(Text::new(p.name).class("name"))
                        .child(Text::new((p.about)()).max_lines(3).class("muted small")),
                ),
            ),
        ),
    )
}

pub fn page(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let ask = st.gst.get();
        let ask: Arc<GstAsk> = ask.unwrap_or_default();
        let exe = if ask.missing.exe.is_empty() { t!("программа") } else { ask.missing.exe.clone() };
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("hero").child(icon(icons::MOVIE).class("ic hero-ic")))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(4.0)
                            .child(Text::new(t!("Нечем воспроизвести видео")).class("title"))
                            .child(Text::new(t!("«{p}» не может открыть видео или звук: в системе нет модулей GStreamer для этого формата.", p = &exe)).max_lines(4).class("muted")),
                    ),
                ),
        );
        let formats = ask.missing.formats();
        if !formats.is_empty() {
            col = col.child(Text::new(t!("Формат: {f}", f = formats.join(", "))).max_lines(3).class("dlls"));
        }
        for p in &ask.pkgs {
            col = col.child(package_card(p));
        }
        col = col.child(Text::new(t!("Пакеты ставятся в систему из репозиториев (pacman) и нужны всем программам Wine. Понадобится пароль администратора.")).max_lines(4).class("muted small"));
        vec![boxed(col)]
    });
    let buttons = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phase = st.phase.get();
        let busy = matches!(phase, Phase::Busy(..));
        let mut row = Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center);
        if matches!(phase, Phase::Done(_)) {
            row = row.child(Button::new(t!("Закрыть")).class("primary").on_click(syngui::signal::quit_app));
            return vec![boxed(row)];
        }
        row = row.child(Button::new(t!("Отмена")).disabled(busy).on_click(syngui::signal::quit_app));
        let names: Vec<String> = st.gst.get_untracked().map(|a| a.pkgs.iter().map(|p| p.name.to_string()).collect()).unwrap_or_default();
        let label = if matches!(phase, Phase::Error(_)) { t!("Повторить") } else { t!("Установить") };
        row = row.child(Button::new(label).icon(icons::DOWNLOAD).class("primary").disabled(busy || names.is_empty()).on_click(move || install(st, names.clone())));
        vec![boxed(row)]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(DecoratedBox::new().class("page").child(body))))
            .child(DecoratedBox::new().class("footer").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(progress(st)).child(buttons))),
    )
}
