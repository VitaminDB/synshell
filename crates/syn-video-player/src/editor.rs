//! Экран монтажа (ножницы в шапке плеера): сверху — полоса «закрыть · Монтаж · отменить ·
//! сохранить», посередине — плеер, снизу — шкала кусков и инструменты.
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ ✕  Монтаж · 1:24               ↶   [Сохранить]│
//! │                                              │
//! │                   кадр                       │
//! │                                              │
//! │ ▓▓▓▓▓▓▓▓|▓▓▓▓ ░░░░░░ ▓▓▓▓▓▓▓▓▓ ▒▒▒▒▒▒▒▒▒▒▒   │ ← куски; ░ — убран, | — где играет
//! │ Кусок 2 из 4 · 0:12–0:31          Итог 1:24  │
//! │   ✂        🗑        ◀        ▶       ＋     │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Куски — отрезки исходных файлов ([`Edit`]); плеер показывает исходный файл текущего куска. При
//! воспроизведении убранные куски перескакиваются, после куска идёт следующий по шкале (в том числе
//! из другого файла) — смотришь то, что получится. Тап по куску — перейти к его началу.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::buttons::Segment;
use syngui::widgets::*;

use crate::edit::{self, Edit, Mode};
use crate::library::format_time;
use crate::ui::{self, icons, Card, St};

type W = Box<dyn Widget>;

/// Окно поверх экрана монтажа.
#[derive(Clone, PartialEq, Default)]
pub enum Sheet {
    #[default]
    None,
    /// Выбор ролика для склейки.
    Add,
    /// Способ сохранения; `fast_ok` — можно ли без перекодирования (`None` — ещё проверяется).
    Save { mode: Mode, fast_ok: Option<bool> },
    Export { progress: f32, cancel: Cancel },
    Done(PathBuf),
    Failed(String),
    /// Выйти, бросив правки?
    Discard,
}

#[derive(Clone)]
pub struct Cancel(pub Arc<AtomicBool>);

impl PartialEq for Cancel {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

/// Ножницы в шапке плеера: начать монтаж открытого ролика.
pub fn start(st: St) {
    let Some(p) = st.playing.get_untracked() else { return };
    let dur = p.player.lock().map(|pl| pl.duration_sec()).unwrap_or(0.0);
    if dur <= 0.0 {
        st.error.set(Some(t!("У ролика неизвестна длительность — монтаж недоступен")));
        return;
    }
    if st.fullscreen.get_untracked() {
        st.fullscreen.set(false);
        syngui::signal::set_fullscreen(false);
    }
    st.settings_open.set(false);
    st.sheet.set(Sheet::None);
    st.edit.set(Some(Edit::new(p.path.clone(), dur)));
}

/// «Назад» в монтаже: закрыть окно, иначе выйти (с вопросом, если есть правки).
pub fn back(st: St) -> bool {
    if st.edit.get_untracked().is_none() {
        return false;
    }
    match st.sheet.get_untracked() {
        // сохранение не бросаем — его отменяют кнопкой
        Sheet::Export { .. } => {}
        Sheet::None => leave(st, false),
        _ => st.sheet.set(Sheet::None),
    }
    true
}

fn leave(st: St, force: bool) {
    if !force && st.edit.with_untracked(|e| e.as_ref().is_some_and(Edit::modified)) {
        st.sheet.set(Sheet::Discard);
        return;
    }
    st.sheet.set(Sheet::None);
    st.edit.set(None);
}

fn change(st: St, f: impl FnOnce(&mut Edit)) {
    let Some(mut e) = st.edit.get_untracked() else { return };
    f(&mut e);
    st.edit.set(Some(e));
}

/// Открытый (или открывающийся) ролик и точка воспроизведения в нём.
fn playhead(st: St) -> Option<(PathBuf, f64)> {
    let path = ui::current(st)?;
    Some((path, st.pos.get_untracked() as f64))
}

/// Перейти к началу куска `i` (другой файл — открыть его).
fn jump(st: St, i: usize) {
    let Some(e) = st.edit.get_untracked() else { return };
    let Some(piece) = e.pieces.get(i).cloned() else { return };
    change(st, |e| e.sel = i);
    JUMPED.with(|j| j.set(Some(Instant::now())));
    match st.playing.get_untracked() {
        Some(p) if p.path == piece.path => {
            if let Ok(mut pl) = p.player.lock() {
                let _ = pl.seek(piece.start);
            }
            st.pos.set(piece.start as f32);
        }
        _ => ui::open_at(st, piece.path, Some(piece.start), true),
    }
}

thread_local! {
    /// Когда последний раз перескакивали: позиция плеера догоняет перемотку не сразу.
    static JUMPED: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Просмотр результата: пока ролик играет, убранные куски перескакиваются, по концу куска —
/// следующий по шкале. Эффект на позиции плеера (ставится в `main`).
pub fn follow(st: St) {
    let t = st.pos.get() as f64;
    let Some(e) = st.edit.get_untracked() else { return };
    if !matches!(st.sheet.get_untracked(), Sheet::None) {
        return;
    }
    let Some(p) = st.playing.get_untracked() else { return };
    let paused = p.player.lock().map(|pl| pl.is_paused()).unwrap_or(true);
    if paused || JUMPED.with(|j| j.get().is_some_and(|at| at.elapsed() < Duration::from_millis(500))) {
        return;
    }
    let Some(i) = e.locate(&p.path, t) else { return };
    let piece = &e.pieces[i];
    let next = e.next_kept(i);
    let leave_now = !piece.keep || (t >= piece.end - 0.08 && !next.is_some_and(|j| j == i + 1 && e.continues(i, j)));
    if !leave_now {
        if e.sel != i {
            change(st, |e| e.sel = i);
        }
        return;
    }
    match next {
        Some(j) => jump(st, j),
        // дальше по шкале ничего — пауза (конец файла плеер ловит сам)
        None if piece.keep => {
            if let Ok(mut pl) = p.player.lock() {
                pl.pause();
            }
        }
        None => {}
    }
}

// ─── экран ──────────────────────────────────────────────────────────────────

pub fn view(st: St) -> W {
    let preview = Reactive::new(move || -> Vec<W> {
        let Some(p) = st.playing.get() else {
            return vec![Box::new(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(CircularProgress::new().indeterminate().size(40.0).stroke_width(4.0)),
            )];
        };
        let fit = if st.settings.get().fill { ImageFit::Cover } else { ImageFit::Contain };
        let view = VideoPlayerView::file(p.player.clone())
            .fit(fit)
            .seek_step(st.settings.get_untracked().seek_step as f64)
            .position_signal(st.pos);
        vec![Box::new(view.build())]
    });
    let preview = DecoratedBox::new().child(preview).class("ed-preview");
    let body = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(top_bar(st))
        .child(preview)
        .child(panel(st))
        .class("grow");
    let sheet = Reactive::new(move || -> Vec<W> {
        match st.sheet.get() {
            Sheet::None => vec![],
            Sheet::Add => vec![add_sheet(st)],
            s => vec![dialog(st, s)],
        }
    });
    Box::new(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("player-bg")).child(body).child(sheet))
}

fn top_bar(st: St) -> W {
    let title = Reactive::new(move || -> Vec<W> {
        let total = st.edit.with(|e| e.as_ref().map(Edit::total).unwrap_or(0.0));
        vec![Box::new(Text::new(t!("Монтаж · {t}", t = format_time(total))).max_lines(1).class("player-title"))]
    });
    let undo = Reactive::new(move || -> Vec<W> {
        let can = st.edit.with(|e| e.as_ref().is_some_and(Edit::can_undo));
        vec![tool_icon(icons::UNDO, &t!("Отменить"), can, move || change(st, |e| {
            e.undo();
        }))]
    });
    Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть монтаж")).on_click(move || leave(st, false)).class("vp-btn"))
            .child(title)
            .child(DecoratedBox::new().class("grow"))
            .child(undo)
            .child(pill(icons::SAVE, &t!("Сохранить"), true, move || open_save(st)))
            .class("ed-top"),
    )
}

fn panel(st: St) -> W {
    let timeline = Reactive::new(move || -> Vec<W> {
        let Some(e) = st.edit.get() else { return vec![] };
        let cur = ui::current(st);
        let t = st.pos.get() as f64;
        let active = cur.as_ref().and_then(|c| e.locate(c, t));
        // цвет куска — по исходному файлу
        let mut sources: Vec<PathBuf> = Vec::new();
        let mut row = Row::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("ed-track");
        for (i, p) in e.pieces.iter().enumerate() {
            let src = sources.iter().position(|s| *s == p.path).unwrap_or_else(|| {
                sources.push(p.path.clone());
                sources.len() - 1
            });
            let mut class = format!("ed-seg ed-src{}", src % 4);
            if !p.keep {
                class.push_str(" ed-seg-off");
            }
            if Some(i) == active {
                class.push_str(" ed-seg-on");
            }
            let inner: W = if Some(i) == active {
                let done = (t - p.start).max(0.0);
                let rest = (p.end - t).max(0.0);
                Box::new(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(DecoratedBox::new().style("flex-grow", StyleValue::Number(done as f32)))
                        .child(DecoratedBox::new().class("ed-head"))
                        .child(DecoratedBox::new().style("flex-grow", StyleValue::Number(rest as f32)))
                        .class("grow"),
                )
            } else {
                Box::new(DecoratedBox::new().class("grow"))
            };
            // совсем короткий кусок всё равно виден и попадает под палец
            let grow = p.len().max(e.pieces.iter().map(edit::Piece::len).sum::<f64>() * 0.015) as f32;
            row = row.child(
                GestureDetector::new()
                    .on_click(move || jump(st, i))
                    .cursor(syngui::CursorIcon::Pointer)
                    .child(DecoratedBox::new().child(inner).class(class))
                    .style("flex-grow", StyleValue::Number(grow)),
            );
        }
        let info = match active {
            Some(i) => {
                let p = &e.pieces[i];
                let mut s = t!(
                    "Кусок {n} из {all} · {from}–{to}",
                    n = i + 1,
                    all = e.pieces.len(),
                    from = format_time(p.start),
                    to = format_time(p.end)
                );
                if !p.keep {
                    s.push_str(&t!(" · убран"));
                }
                s
            }
            None => t!("Кусков: {n}", n = e.pieces.len()),
        };
        let hint = if e.pieces.len() == 1 && !e.modified() {
            t!("Ножницы режут там, где сейчас кадр; лишний кусок — в корзину.")
        } else {
            t!("Итог {t}", t = format_time(e.total()))
        };
        vec![
            Box::new(row),
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(info).max_lines(1).class("ed-info"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(Text::new(hint).max_lines(1).class("ed-hint")),
            ),
        ]
    });
    let tools = Reactive::new(move || -> Vec<W> {
        let Some(e) = st.edit.get() else { return vec![] };
        let _ = st.pos.get();
        let head = playhead(st);
        let active = head.as_ref().and_then(|(p, t)| e.locate(p, *t));
        let can_split = head.as_ref().is_some_and(|(p, t)| e.clone().split(p, *t));
        let off = active.is_some_and(|i| !e.pieces[i].keep);
        let n = e.pieces.len();
        vec![Box::new(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(tool(icons::CUT, &t!("Разрезать"), can_split, move || {
                    if let Some((p, t)) = playhead(st) {
                        change(st, |e| {
                            e.split(&p, t);
                        });
                    }
                }))
                .child(tool(
                    if off { icons::RESTORE } else { icons::DELETE },
                    &if off { t!("Вернуть") } else { t!("Убрать") },
                    active.is_some(),
                    move || {
                        if let Some(i) = active {
                            change(st, |e| e.toggle(i));
                        }
                    },
                ))
                .child(tool(icons::LEFT, &t!("Раньше"), active.is_some_and(|i| i > 0), move || {
                    if let Some(i) = active {
                        change(st, |e| {
                            e.shift(i, -1);
                        });
                    }
                }))
                .child(tool(icons::RIGHT, &t!("Позже"), active.is_some_and(|i| i + 1 < n), move || {
                    if let Some(i) = active {
                        change(st, |e| {
                            e.shift(i, 1);
                        });
                    }
                }))
                .child(tool(icons::ADD, &t!("Склеить"), true, move || open_add(st))),
        )]
    });
    Box::new(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(timeline)
            .child(tools)
            .class("ed-panel"),
    )
}

/// Кнопка инструмента: значок и подпись под ним.
fn tool(icon: &'static str, label: &str, enabled: bool, f: impl Fn() + Send + Sync + 'static) -> W {
    let b = DecoratedBox::new()
        .child(
            Column::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class("ed-tool-icon"))
                .child(Text::new(label).max_lines(1).class("ed-tool-text")),
        )
        .class(if enabled { "ed-tool" } else { "ed-tool ed-tool-off" });
    if enabled {
        Box::new(GestureDetector::new().on_click(f).cursor(syngui::CursorIcon::Pointer).child(b))
    } else {
        Box::new(b)
    }
}

fn tool_icon(icon: &'static str, tip: &str, enabled: bool, f: impl Fn() + Send + Sync + 'static) -> W {
    let b = DecoratedBox::new().child(Icon::new(icon).class("ed-ib-icon")).class(if enabled { "ed-ib" } else { "ed-ib ed-tool-off" });
    if enabled {
        Box::new(GestureDetector::new().on_click(f).cursor(syngui::CursorIcon::Pointer).child(Tooltip::new(b, tip)))
    } else {
        Box::new(b)
    }
}

/// Кнопка-«таблетка»: основная (акцент) или обычная.
fn pill(icon: &'static str, label: &str, primary: bool, f: impl Fn() + Send + Sync + 'static) -> W {
    let class = if primary { "ed-btn ed-btn-primary" } else { "ed-btn" };
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !icon.is_empty() {
        row = row.child(Icon::new(icon).class("ed-btn-icon"));
    }
    Box::new(
        GestureDetector::new()
            .on_click(f)
            .cursor(syngui::CursorIcon::Pointer)
            .child(DecoratedBox::new().child(row.child(Text::new(label).max_lines(1).class("ed-btn-text"))).class(class)),
    )
}

// ─── сохранение ─────────────────────────────────────────────────────────────

fn open_save(st: St) {
    let Some(e) = st.edit.get_untracked() else { return };
    let kept = e.kept();
    if kept.is_empty() {
        st.sheet.set(Sheet::Failed(t!("Все куски убраны — сохранять нечего")));
        return;
    }
    if let Some(p) = st.playing.get_untracked() {
        if let Ok(mut pl) = p.player.lock() {
            pl.pause();
        }
    }
    st.sheet.set(Sheet::Save { mode: Mode::Precise, fast_ok: None });
    // без перекодирования — только одинаковые ролики: проверить в фоне
    let mut paths: Vec<PathBuf> = kept.into_iter().map(|p| p.path).collect();
    paths.dedup();
    std::thread::spawn(move || {
        let probes: Option<Vec<_>> = paths.iter().map(|p| edit::probe(p).ok()).collect();
        let ok = probes.is_some_and(|p| edit::copy_compatible(&p));
        run_on_main_thread(move || {
            if let Sheet::Save { mode, fast_ok: None } = st.sheet.get_untracked() {
                st.sheet.set(Sheet::Save { mode, fast_ok: Some(ok) });
            }
        });
    });
}

fn run_export(st: St, mode: Mode) {
    let Some(e) = st.edit.get_untracked() else { return };
    let pieces = e.kept();
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));
    st.sheet.set(Sheet::Export { progress: 0.0, cancel: cancel.clone() });
    std::thread::Builder::new()
        .name("svp-export".into())
        .spawn(move || {
            let c = cancel.clone();
            let r = edit::export(&pieces, mode, &cancel.0, move |f| {
                let c = c.clone();
                run_on_main_thread(move || {
                    if let Sheet::Export { cancel, .. } = st.sheet.get_untracked() {
                        if cancel == c {
                            st.sheet.set(Sheet::Export { progress: f, cancel });
                        }
                    }
                });
            });
            run_on_main_thread(move || {
                if cancel.0.load(Ordering::Relaxed) {
                    return;
                }
                match r {
                    Ok(out) => {
                        // правки сохранены: выход больше не спрашивает
                        change(st, Edit::mark_saved);
                        st.sheet.set(Sheet::Done(out));
                    }
                    Err(e) => {
                        tracing::warn!("монтаж: {e}");
                        st.sheet.set(Sheet::Failed(e));
                    }
                }
            });
        })
        .ok();
}

/// Открыть сохранённый ролик в плеере (монтаж закрывается).
fn open_result(st: St, out: PathBuf) {
    leave(st, true);
    // в каталоге появился новый файл — перечитать
    st.siblings.set(Vec::new());
    if !st.cards.with_untracked(Vec::is_empty) {
        ui::rescan(st);
    }
    ui::open(st, out);
}

fn dialog(st: St, s: Sheet) -> W {
    let close = move || st.sheet.set(Sheet::None);
    let (title, body, actions): (String, Vec<W>, Vec<W>) = match s {
        Sheet::Save { mode, fast_ok } => {
            let fast_ok = fast_ok.unwrap_or(false);
            let hint = match mode {
                Mode::Precise => t!("Режет точно по кадру и склеивает любые ролики. Перекодирование в H.264 — займёт время."),
                Mode::Fast if fast_ok => t!("Мгновенно и без потери качества, но куски начинаются с ближайшего ключевого кадра (сдвиг до пары секунд)."),
                Mode::Fast => t!("Ролики разные (кодек, размер или звук) — без перекодирования не склеить."),
            };
            let picker = SegmentedButton::new(vec![
                Segment::with_icon(t!("Точно"), icons::CUT),
                Segment::with_icon(t!("Быстро"), icons::FAST_FORWARD),
            ])
            .selected(if mode == Mode::Precise { 0 } else { 1 })
            .on_change(move |i| {
                if let Sheet::Save { fast_ok, .. } = st.sheet.get_untracked() {
                    st.sheet.set(Sheet::Save { mode: if i == 0 { Mode::Precise } else { Mode::Fast }, fast_ok });
                }
            });
            let first = st.edit.with_untracked(|e| e.as_ref().and_then(|e| e.kept().first().map(|p| p.path.clone())));
            let target = first
                .map(|f| edit::output_path(&f, mode))
                .and_then(|o| o.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let can = mode == Mode::Precise || fast_ok;
            (
                t!("Сохранить ролик"),
                vec![
                    Box::new(picker),
                    Box::new(Text::new(hint).max_lines(4).class("set-hint")),
                    Box::new(Text::new(t!("Файл: {name}", name = target)).max_lines(2).class("ed-file")),
                ],
                vec![
                    pill("", &t!("Отмена"), false, close),
                    if can {
                        pill(icons::SAVE, &t!("Сохранить"), true, move || run_export(st, mode))
                    } else {
                        Box::new(DecoratedBox::new().child(Text::new(t!("Сохранить")).class("ed-btn-text")).class("ed-btn ed-tool-off"))
                    },
                ],
            )
        }
        Sheet::Export { progress, cancel } => (
            t!("Сохраняю…"),
            vec![
                Box::new(ProgressBar::with_value(progress).class("ed-progress")),
                Box::new(Text::new(format!("{:.0} %", progress * 100.0)).class("set-hint")),
            ],
            vec![pill("", &t!("Отменить"), false, move || {
                cancel.0.store(true, Ordering::Relaxed);
                st.sheet.set(Sheet::None);
            })],
        ),
        Sheet::Done(out) => {
            let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let dir = out.parent().map(|d| d.display().to_string()).unwrap_or_default();
            (
                t!("Сохранено"),
                vec![Box::new(Text::new(t!("«{name}» в {dir}", name = name, dir = dir)).max_lines(4).class("set-row-title"))],
                vec![pill("", &t!("Продолжить монтаж"), false, close), pill(icons::PLAY, &t!("Смотреть"), true, move || open_result(st, out.clone()))],
            )
        }
        Sheet::Failed(e) => (t!("Не получилось"), vec![Box::new(Text::new(e).max_lines(8).class("set-row-title"))], vec![pill("", &t!("Понятно"), true, close)]),
        Sheet::Discard => (
            t!("Выйти из монтажа?"),
            vec![Box::new(Text::new(t!("Правки не сохранены и пропадут.")).max_lines(3).class("set-row-title"))],
            vec![pill("", &t!("Остаться"), false, close), pill("", &t!("Выйти"), true, move || leave(st, true))],
        ),
        Sheet::None | Sheet::Add => return Box::new(DecoratedBox::new()),
    };
    let busy = matches!(st.sheet.get_untracked(), Sheet::Export { .. });
    let mut col = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("set-title"));
    for b in body {
        col = col.child(b);
    }
    col = col.child(Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).children(actions));
    let scrim: W = if busy {
        Box::new(DecoratedBox::new().class("set-scrim"))
    } else {
        Box::new(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("set-scrim")))
    };
    Box::new(
        Stack::new().fit(StackFit::Expand).child(scrim).child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(GestureDetector::new().on_click(|| {}).child(DecoratedBox::new().child(col).class("ed-dialog"))),
        ),
    )
}

// ─── добавить ролик ─────────────────────────────────────────────────────────

fn open_add(st: St) {
    if st.cards.with_untracked(Vec::is_empty) {
        ui::rescan(st);
    }
    st.sheet.set(Sheet::Add);
}

fn append(st: St, path: PathBuf) {
    st.sheet.set(Sheet::None);
    std::thread::spawn(move || {
        let r = edit::probe(&path);
        run_on_main_thread(move || match r {
            Ok(p) if p.duration > 0.0 => change(st, |e| e.append(path, p.duration)),
            Ok(_) => st.sheet.set(Sheet::Failed(t!("У «{name}» неизвестна длительность", name = path.display()))),
            Err(e) => st.sheet.set(Sheet::Failed(t!("Не удалось открыть «{path}»: {e}", path = path.display(), e = e))),
        });
    });
}

fn add_sheet(st: St) -> W {
    let close = move || st.sheet.set(Sheet::None);
    let list = Reactive::new(move || -> Vec<W> {
        // сначала ролики той же папки, затем остальные из библиотеки
        let mut cards: Vec<Card> = st.siblings.get();
        for c in st.cards.get() {
            if !cards.iter().any(|k| k.item.path == c.item.path) {
                cards.push(c);
            }
        }
        if cards.is_empty() {
            let text = if st.scanning.get() { t!("Ищу видео…") } else { t!("Видео не найдено") };
            return vec![Box::new(Text::new(text).class("set-hint"))];
        }
        cards.into_iter().map(|c| add_row(st, c)).collect()
    });
    let panel = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Склеить с…")).class("set-title"))
                .child(DecoratedBox::new().class("grow"))
                .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
                .class("set-head"),
        )
        .child(Text::new(t!("Ролик добавится в конец; переставить можно стрелками.")).max_lines(2).class("ed-add-hint"))
        .child(
            ScrollView::new()
                .vertical()
                .child(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(list).class("set-body"))
                .class("grow"),
        )
        .class("set-panel");
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("set-scrim")))
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(GestureDetector::new().on_click(|| {}).child(panel)),
            ),
    )
}

fn add_row(st: St, c: Card) -> W {
    let pic = c.pic;
    let key = format!("svp:{}", c.item.path.display());
    let thumb = Reactive::new(move || -> Vec<W> {
        let mut layers = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("pic-bg"));
        let mut dur = None;
        layers = match pic.get() {
            Some(p) => {
                dur = (p.duration > 0.0).then_some(p.duration);
                layers.child(Image::from_rgba_shared(key.clone(), p.w, p.h, p.rgba.clone()).fit(ImageFit::Cover).class("pic"))
            }
            None => layers.child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::MOVIE).class("strip-icon")),
            ),
        };
        if let Some(d) = dur {
            layers = layers.child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .cross_axis_alignment(CrossAxisAlignment::End)
                    .child(DecoratedBox::new().child(Text::new(format_time(d)).class("dur-text")).class("dur"))
                    .class("ed-dur-place"),
            );
        }
        vec![Box::new(DecoratedBox::new().child(layers).class("ed-add-thumb"))]
    });
    let path = c.item.path.clone();
    Box::new(
        GestureDetector::new().on_click(move || append(st, path.clone())).cursor(syngui::CursorIcon::Pointer).child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(thumb)
                .child(Text::new(c.item.name()).max_lines(2).class("set-row-title grow"))
                .class("ed-add-row"),
        ),
    )
}
