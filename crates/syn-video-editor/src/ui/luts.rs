//! Панель «LUT» (C7): библиотека `.cube` плитками, превью — кадр клипа
//! (выделенного, иначе верхнего под позицией; без клипа — демо-кадр) с этим
//! LUT. Ведение курсора по плитке — сравнение «до | после» по шторке;
//! щелчок — `lut.apply` к выделенным клипам. Плитки считаются в фоне
//! (таблицы разбираются без общего кэша, чтобы 3800 файлов не копились в
//! памяти), только видимые, свежие запросы — первыми.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::video::RgbaFrame;

use super::{ed, icons, St};
use crate::doc::{Source, TrackKind};
use crate::render::lut;

const TW: u32 = 256;
const TH: u32 = 144;
const PAGE: usize = 48;

/// Откуда кадр превью: клип и номер кадра (или демо).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BaseKey {
    clip: u64,
    frame: i64,
}

struct State {
    base: Option<(BaseKey, Arc<RgbaFrame>)>,
    /// Ждёт расчёта кадр-основа для ключа.
    base_job: Option<(BaseKey, crate::doc::Project, f64)>,
    tiles: HashMap<PathBuf, (BaseKey, Option<Arc<RgbaFrame>>)>,
    queue: VecDeque<PathBuf>,
    last_notify: Instant,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static S: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    S.get_or_init(|| {
        for i in 0..2 {
            let _ = std::thread::Builder::new().name(format!("lut-tiles-{i}")).spawn(worker);
        }
        (Mutex::new(State { base: None, base_job: None, tiles: HashMap::new(), queue: VecDeque::new(), last_notify: Instant::now() }), Condvar::new())
    })
}

/// Клип для превью: первый выделенный видеоклип медиа, иначе верхний под позицией.
fn target() -> Option<(u64, f64, crate::doc::Project)> {
    let e = ed();
    let p = &e.project;
    let is_media_video = |id: u64| p.find_clip(id).is_some_and(|(ti, ci)| p.tracks[ti].kind == TrackKind::Video && matches!(p.tracks[ti].clips[ci].source, Source::Media { .. }));
    let pick = e.selection.iter().copied().find(|id| is_media_video(*id)).or_else(|| {
        p.tracks.iter().rev().filter(|t| t.kind == TrackKind::Video && !t.hidden).find_map(|t| t.clips.iter().find(|c| c.start <= e.playhead && e.playhead < c.end() && matches!(c.source, Source::Media { .. })).map(|c| c.id))
    })?;
    let c = p.clip(pick)?;
    let fps = p.settings.fps.max(1.0);
    let t = e.playhead.clamp(c.start, (c.end() - 1.0 / fps).max(c.start));
    Some((pick, t, p.clone()))
}

/// Демо-кадр без клипа: небо, кожа, зелень, радуга и шкала серого.
fn demo() -> RgbaFrame {
    let mut px = vec![0u8; (TW * TH * 4) as usize];
    for y in 0..TH {
        for x in 0..TW {
            let (fx, fy) = (x as f32 / TW as f32, y as f32 / TH as f32);
            let c: [f32; 3] = if fy < 0.45 {
                // небо → горизонт
                let k = fy / 0.45;
                [0.35 + 0.5 * k, 0.55 + 0.35 * k, 0.85 + 0.1 * k]
            } else if fy < 0.75 {
                if fx < 0.34 {
                    [0.86, 0.64, 0.52] // кожа
                } else if fx < 0.67 {
                    [0.22, 0.5, 0.2] // зелень
                } else {
                    [0.75, 0.25, 0.15] // красный
                }
            } else if fy < 0.88 {
                let h = fx * 6.0;
                let f = h.fract();
                match h as i32 {
                    0 => [1.0, f, 0.0],
                    1 => [1.0 - f, 1.0, 0.0],
                    2 => [0.0, 1.0, f],
                    3 => [0.0, 1.0 - f, 1.0],
                    4 => [f, 0.0, 1.0],
                    _ => [1.0, 0.0, 1.0 - f],
                }
            } else {
                [fx, fx, fx]
            };
            let i = ((y * TW + x) * 4) as usize;
            px[i] = (c[0] * 255.0) as u8;
            px[i + 1] = (c[1] * 255.0) as u8;
            px[i + 2] = (c[2] * 255.0) as u8;
            px[i + 3] = 255;
        }
    }
    RgbaFrame { width: TW, height: TH, pts: 0.0, rgba: Arc::new(px) }
}

/// Кадр клипа без его LUT, обрезанный под плитку 16:9.
fn base_frame(mut p: crate::doc::Project, cid: u64, t: f64) -> RgbaFrame {
    if let Some(c) = p.clip_mut(cid) {
        c.effects.retain(|e| e.kind != "lut");
    }
    let mut src = crate::render::compose::FrameSource::new(TW * 2, TH * 2);
    let Some(f) = src.frames(&p, t).remove(&cid) else { return demo() };
    let Some(img) = image::RgbaImage::from_raw(f.width, f.height, (*f.rgba).clone()) else { return demo() };
    let out = image::imageops::resize(&img, TW.max(f.width * TH / f.height.max(1)), TH.max(f.height * TW / f.width.max(1)), image::imageops::FilterType::Triangle);
    let (w, h) = out.dimensions();
    let cropped = image::imageops::crop_imm(&out, (w - TW.min(w)) / 2, (h - TH.min(h)) / 2, TW.min(w), TH.min(h)).to_image();
    let (w, h) = cropped.dimensions();
    let mut px = cropped.into_raw();
    for a in px.iter_mut().skip(3).step_by(4) {
        *a = 255;
    }
    RgbaFrame { width: w, height: h, pts: t, rgba: Arc::new(px) }
}

fn worker() {
    loop {
        enum Job {
            Base(BaseKey, crate::doc::Project, f64),
            Tile(PathBuf, BaseKey, Arc<RgbaFrame>),
        }
        let job = {
            let (m, cv) = state();
            let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some((k, p, t)) = s.base_job.take() {
                    break Job::Base(k, p, t);
                }
                if let Some((k, b)) = s.base.clone() {
                    if let Some(path) = s.queue.pop_front() {
                        if s.tiles.get(&path).is_some_and(|(tk, _)| *tk == k) {
                            continue;
                        }
                        // пометка «считается», чтобы второй поток не взял тот же
                        s.tiles.insert(path.clone(), (k, None));
                        break Job::Tile(path, k, b);
                    }
                }
                s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        match job {
            Job::Base(k, p, t) => {
                let f = if k.clip == 0 { demo() } else { base_frame(p, k.clip, t) };
                let (m, cv) = state();
                let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
                s.base = Some((k, Arc::new(f)));
                cv.notify_all();
                drop(s);
                super::thumbs::notify();
            }
            Job::Tile(path, k, base) => {
                let out = std::fs::read_to_string(&path).ok().and_then(|t| lut::parse(&t).ok()).map(|l| {
                    let mut px = (*base.rgba).clone();
                    l.apply(&mut px, 1.0);
                    Arc::new(RgbaFrame { width: base.width, height: base.height, pts: 0.0, rgba: Arc::new(px) })
                });
                let (m, _) = state();
                let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
                // нет таблицы (1D, битый файл) — помечаем пустым основой-ключом: не пересчитывать
                let bad = out.is_none();
                s.tiles.insert(path, (k, out.or_else(|| bad.then(|| Arc::new(RgbaFrame { width: 0, height: 0, pts: -1.0, rgba: Arc::new(Vec::new()) })))));
                let now = Instant::now();
                if s.queue.is_empty() || now.duration_since(s.last_notify) > Duration::from_millis(120) {
                    s.last_notify = now;
                    drop(s);
                    super::thumbs::notify();
                }
            }
        }
    }
}

/// Текущий ключ кадра-основы; при смене — заказать расчёт.
fn ensure_base() -> (BaseKey, String) {
    let (key, label, job) = match target() {
        Some((cid, t, p)) => {
            let fps = p.settings.fps.max(1.0);
            let k = BaseKey { clip: cid, frame: (t * fps).round() as i64 };
            (k, format!("Превью на кадре клипа c{cid} ({})", crate::doc::tc(t)), Some((p, t)))
        }
        None => (BaseKey { clip: 0, frame: 0 }, "Превью на демо-кадре — выделите клип, чтобы видеть его".to_string(), None),
    };
    let (m, cv) = state();
    let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
    let have = s.base.as_ref().is_some_and(|(k, _)| *k == key) || s.base_job.as_ref().is_some_and(|(k, _, _)| *k == key);
    if !have {
        let (p, t) = job.unwrap_or_else(|| (crate::doc::Project::default(), 0.0));
        s.base_job = Some((key, p, t));
        cv.notify_all();
    }
    (key, label)
}

/// Плитка: готовый кадр с LUT, `Some(None)` — LUT не читается, `None` — в очереди.
fn tile_or_queue(path: &PathBuf, key: BaseKey) -> Option<Option<Arc<RgbaFrame>>> {
    let (m, cv) = state();
    let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, f)) = s.tiles.get(path) {
        if *k == key {
            if let Some(f) = f {
                return Some((f.pts >= 0.0).then(|| f.clone()));
            }
            return None; // считается
        }
    }
    if !s.queue.contains(path) {
        // свежие (видимые сейчас) — вперёд
        s.queue.push_front(path.clone());
        s.queue.truncate(PAGE * 3);
        cv.notify_one();
    }
    None
}

fn base_now(key: BaseKey) -> Option<Arc<RgbaFrame>> {
    let (m, _) = state();
    let s = m.lock().unwrap_or_else(|e| e.into_inner());
    s.base.as_ref().filter(|(k, _)| *k == key).map(|(_, f)| f.clone())
}

/// Шторка «до | после»: слева от `frac` — исходный кадр.
fn split(before: &RgbaFrame, after: &RgbaFrame, frac: f32) -> RgbaFrame {
    let mut px = (*after.rgba).clone();
    let cut = ((after.width as f32 * frac) as u32).min(after.width);
    if before.width == after.width && before.height == after.height {
        for y in 0..after.height {
            let i0 = (y * after.width * 4) as usize;
            let n = (cut * 4) as usize;
            px[i0..i0 + n].copy_from_slice(&before.rgba[i0..i0 + n]);
        }
    }
    RgbaFrame { width: after.width, height: after.height, pts: 0.0, rgba: Arc::new(px) }
}

/// LUT у первого выделенного клипа (для пометки плитки).
fn current_lut() -> Option<String> {
    let e = ed();
    let id = *e.selection.first()?;
    e.project.clip(id)?.effects.iter().find(|x| x.kind == "lut").and_then(|x| x.params.get("path")).and_then(|v| v.as_str()).map(str::to_string)
}

pub fn view(st: St) -> impl Widget {
    let query = use_signal(String::new());
    let theme = use_signal(String::new());
    let set = use_signal(String::new());
    let limit = use_signal(PAGE);
    let all = lut::catalog();

    let head = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("panel-head")
        .child(DecoratedBox::new().class("grow").child(
            TextField::new()
                .prefix_icon(icons::SEARCH)
                .placeholder("Поиск LUT…")
                .on_change(move |s: &str| {
                    query.set(s.to_string());
                    limit.set(PAGE);
                })
                .class("tpl-search"),
        ))
        .child(Slider::new().range(100.0, 280.0).value(st.tile_size.get_untracked().clamp(100.0, 280.0)).width(74.0).on_change(move |v| st.tile_size.set(v)));

    let mut themes: Vec<String> = all.iter().map(|e| e.theme.clone()).collect();
    themes.dedup();
    let cats = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = theme.get();
        let cur_set = set.get();
        let chip = move |label: String, key: String, on: bool| {
            GestureDetector::new()
                .on_click(move || {
                    theme.set(key.clone());
                    set.set(String::new());
                    limit.set(PAGE);
                })
                .child(DecoratedBox::new().class(if on { "cv-chip cv-chip-on" } else { "cv-chip" }).child(Text::new(label).class("cv-chip-text")))
        };
        let mut chips = Flex::new().wrap().gap(4.0).child(chip("Все".into(), String::new(), cur.is_empty()));
        for t in &themes {
            chips = chips.child(chip(t.clone(), t.clone(), cur == *t));
        }
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("tpl-cats").child(chips);
        if !cur.is_empty() {
            let mut sets: BTreeMap<String, usize> = BTreeMap::new();
            for e in all.iter().filter(|e| e.theme == cur) {
                *sets.entry(e.set.clone()).or_default() += 1;
            }
            if sets.len() > 1 {
                let mut items = vec![DropdownItem::new("", format!("Все наборы ({})", sets.values().sum::<usize>()))];
                for (n, c) in sets {
                    let label = if n.is_empty() { format!("Без набора ({c})") } else { format!("{n} ({c})") };
                    items.push(DropdownItem::new(n, label));
                }
                col = col.child(Dropdown::new().items(items).selected(cur_set).on_change(move |v: &str| {
                    set.set(v.to_string());
                    limit.set(PAGE);
                }).class("tpl-sub-select"));
            }
        }
        vec![Box::new(col)]
    });

    let body = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let _ = st.rev.get();
        let _ = st.ph.get();
        let (q, th, sv, lim) = (query.get(), theme.get(), set.get(), limit.get());
        let size = st.tile_size.get().clamp(100.0, 280.0);
        if all.is_empty() {
            return vec![Box::new(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("media-empty").child(Text::new("Библиотека LUT не найдена").class("row-title")).child(Text::new(format!("Ожидаются файлы .cube в\n{}", lut::library_dir().display())).max_lines(3).class("panel-empty")))];
        }
        let (key, label) = ensure_base();
        let hits = lut::filter(all, &q, &th, &sv);
        let cur = current_lut();
        let mut grid = Grid::new(16).min_column_width(size).gap(8.0);
        for e in hits.iter().take(lim) {
            grid = grid.child(lut_tile(st, e, key, cur.as_deref() == Some(&*e.path.to_string_lossy())));
        }
        let has_sel = !ed().selection.is_empty();
        let mut col = Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("panel-list")
            .child(Text::new(format!("{label}. Найдено {} · ведите курсором — до | после · {}", hits.len(), if has_sel { "щелчок — наложить на выделенные" } else { "выделите клип, чтобы наложить" })).max_lines(3).class("panel-hint"))
            .child(grid);
        if hits.len() > lim {
            let rem = hits.len() - lim;
            col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).class("tpl-more").child(Button::new(format!("Показать ещё {} (осталось {rem})", rem.min(PAGE))).on_click(move || limit.update(|n| *n += PAGE)).class("head-btn")));
        }
        vec![Box::new(col)]
    }));

    Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(head).child(cats).child(DecoratedBox::new().class("grow").child(body))
}

fn lut_tile(st: St, e: &lut::Entry, key: BaseKey, active: bool) -> Box<dyn Widget> {
    let hover = use_signal(None::<f32>);
    let width = use_signal(150.0f32);
    let path = e.path.clone();
    let id = format!("{}", e.path.display());
    let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let frac = hover.get();
        let mut stack = Stack::new().fit(StackFit::Expand);
        match tile_or_queue(&path, key) {
            Some(Some(after)) => {
                let (f, tag) = match (frac, base_now(key)) {
                    (Some(fr), Some(b)) => {
                        let q = (fr * 32.0).round() / 32.0;
                        (Arc::new(split(&b, &after, q)), format!("s{}", (q * 32.0) as i32))
                    }
                    _ => (after, String::new()),
                };
                stack = stack.child(Image::from_rgba_shared(format!("lut:{id}:{}:{}:{tag}", key.clip, key.frame), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false));
                if let Some(fr) = frac {
                    stack = stack.child(Positioned::new(DecoratedBox::new().class("tile-scrub")).at((fr * width.get_untracked() - 1.0).max(0.0), 0.0).dimensions(2.0, 4000.0));
                }
            }
            Some(None) => {
                stack = stack.child(Center::new().child(Text::new("не читается").class("panel-empty")));
            }
            None => {
                if let Some(b) = base_now(key) {
                    stack = stack.child(Image::from_rgba_shared(format!("lutbase:{}:{}", key.clip, key.frame), b.width, b.height, b.rgba.clone()).fit(ImageFit::Cover).placeholder(false).class("tile-pending"));
                }
                stack = stack.child(Positioned::new(DecoratedBox::new().class("tile-badge").child(Text::new("…").class("tile-badge-text"))).at(6.0, 6.0));
            }
        }
        vec![Box::new(stack)]
    });
    let sub = if e.set.is_empty() { e.theme.clone() } else { e.set.clone() };
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(EventHook::new().fill().on_hover_at(move |p| {
            if let Some((_, s)) = p {
                width.set(s.width);
            }
            hover.set(p.map(|(p, s)| (p.x / s.width.max(1.0)).clamp(0.0, 1.0)))
        }).child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb").child(thumb))))
        .child(Text::new(e.name.clone()).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(sub).max_lines(1).elide(Elide::End).class("row-sub"));
    let p = e.path.display().to_string();
    let apply = move || {
        if ed().selection.is_empty() {
            st.toast.set(Some("Сначала выделите клип на таймлайне".into()));
            return;
        }
        st.exec("lut.apply", json!({"path": p}));
    };
    Box::new(GestureDetector::new().on_click(apply).child(DecoratedBox::new().class(if active { "tile tile-on" } else { "tile" }).child(body)))
}
