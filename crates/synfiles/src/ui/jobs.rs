//! Задания (копирование, перемещение, удаление): карточки с прогрессом в
//! углу окна, пауза, отмена, вопрос о совпадающих именах.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::*;

use super::{boxed, bx, icon_button, icons, W};
use crate::model::format_size;
use crate::ops::{self, Job, Resolution};
use crate::state;

fn eta(job: &Job) -> String {
    let p = &job.progress;
    let rate = p.rate.load(Ordering::Relaxed);
    let total = p.total_bytes.load(Ordering::Relaxed);
    let done = p.done_bytes.load(Ordering::Relaxed);
    if rate == 0 || total == 0 {
        return String::new();
    }
    let left = total.saturating_sub(done) / rate.max(1);
    let t = if left >= 3600 {
        format!("{} ч {} мин", left / 3600, left % 3600 / 60)
    } else if left >= 60 {
        format!("{} мин {} с", left / 60, left % 60)
    } else {
        format!("{left} с")
    };
    format!("{}/с · осталось {t}", format_size(rate))
}

fn conflict_card(job: Arc<Job>, c: ops::Conflict) -> W {
    let all = use_signal(false);
    let j1 = job.clone();
    let j2 = job.clone();
    let j3 = job.clone();
    let j4 = job.clone();
    let meta = |p: &std::path::Path| -> String {
        std::fs::metadata(p)
            .map(|m| {
                use std::os::unix::fs::MetadataExt;
                if m.is_dir() {
                    format!("папка, изменена {}", crate::model::format_time(m.mtime()))
                } else {
                    format!("{}, изменён {}", format_size(m.len()), crate::model::format_time(m.mtime()))
                }
            })
            .unwrap_or_default()
    };
    let name = ops::name_of(&c.dst);
    let title = if c.dir {
        format!("Папка «{name}» уже есть в «{}»", c.dst.parent().map(ops::name_of).unwrap_or_default())
    } else {
        format!("Файл «{name}» уже есть в «{}»", c.dst.parent().map(ops::name_of).unwrap_or_default())
    };
    let replace = Button::new(if c.dir { "Объединить" } else { "Заменить" }).class("primary").on_click(move || j1.answer(Resolution::Replace, all.get_untracked()));
    let both = Button::new("Оставить оба").on_click(move || j2.answer(Resolution::KeepBoth, all.get_untracked()));
    let skip = Button::new("Пропустить").on_click(move || j3.answer(Resolution::Skip, all.get_untracked()));
    let cancel = Button::new("Отмена").class("flat").on_click(move || j4.cancel());
    // Телефон: четыре кнопки в строку не влезают — по две.
    let buttons: W = if state::is_phone() {
        boxed(
            Column::new()
                .gap(6.0)
                .child(Row::new().gap(6.0).child(replace).child(both))
                .child(Row::new().gap(6.0).child(skip).child(cancel)),
        )
    } else {
        boxed(Row::new().gap(6.0).child(replace).child(both).child(skip).child(cancel))
    };
    boxed(
        Column::new()
            .gap(8.0)
            .class("conflict")
            .child(Text::new(title).max_lines(2).class("conflict-title"))
            .child(Text::new(format!("Новый: {}", meta(&c.src))).class("meta"))
            .child(Text::new(format!("Имеющийся: {}", meta(&c.dst))).class("meta"))
            .child(buttons)
            .child(Checkbox::new().label("Для всех совпадений").on_change(move |v| all.set(v))),
    )
}

fn job_card(job: Arc<Job>) -> W {
    let p = &job.progress;
    let finished = p.finished.load(Ordering::SeqCst);
    let cancelled = p.cancel.load(Ordering::SeqCst);
    let paused = p.paused.load(Ordering::SeqCst);
    let errors = p.errors.lock().unwrap().clone();
    let current = p.current.lock().unwrap().clone();
    let conflict = p.conflict.lock().unwrap().clone();
    let frac = job.fraction();
    let status = if finished && cancelled {
        "Отменено".to_string()
    } else if finished && !errors.is_empty() {
        format!("Готово с ошибками: {}", errors.len())
    } else if finished {
        "Готово".to_string()
    } else if paused {
        "Пауза".to_string()
    } else {
        let files = format!("{} из {}", p.done_files.load(Ordering::Relaxed), p.total_files.load(Ordering::Relaxed));
        let e = eta(&job);
        if e.is_empty() {
            files
        } else {
            format!("{files} · {e}")
        }
    };
    let jp = job.clone();
    let jc = job.clone();
    let mut col = Column::new()
        .gap(6.0)
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(job.title.clone()).max_lines(1).class("job-title grow"))
                .child(if finished {
                    boxed(DecoratedBox::new())
                } else {
                    icon_button(if paused { icons::PLAY } else { icons::PAUSE }, if paused { "Продолжить" } else { "Пауза" }, "small", true, move || jp.toggle_pause())
                })
                .child(if finished {
                    boxed(DecoratedBox::new())
                } else {
                    icon_button(icons::CLOSE, "Отменить", "small", true, move || jc.cancel())
                }),
        )
        .child(ProgressBar::new().value(if finished { 1.0 } else { frac }).class("job-progress"))
        .child(Text::new(status).max_lines(1).class("meta"));
    if !finished && !current.is_empty() && conflict.is_none() {
        col = col.child(Text::new(current).max_lines(1).class("meta dim"));
    }
    for e in errors.iter().take(3) {
        col = col.child(Text::new(e.clone()).max_lines(2).class("job-error"));
    }
    if let Some(c) = conflict {
        col = col.child(conflict_card(job.clone(), c));
    }
    bx("job-card", col)
}

/// Карточки заданий в правом нижнем углу.
pub fn jobs_panel() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let _ = state::ctx().jobs_rev.get();
        let jobs = ops::jobs();
        if jobs.is_empty() {
            return vec![];
        }
        let any_done = jobs.iter().any(|j| j.progress.finished.load(Ordering::SeqCst));
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for j in jobs.into_iter().rev().take(4) {
            col = col.child(job_card(j));
        }
        if any_done {
            col = col.child(
                GestureDetector::new()
                    .on_click(ops::clear_finished)
                    .child(DecoratedBox::new().class("link-btn").child(Text::new("Скрыть завершённые"))),
            );
        }
        let mut jobs = DecoratedBox::new().class("jobs");
        if state::is_phone() {
            // Телефон: карточки во всю ширину экрана.
            jobs = jobs.style("width", syngui::viewport::viewport_size().get_untracked().width - 24.0);
        }
        vec![boxed(jobs.child(col))]
    }))
}
