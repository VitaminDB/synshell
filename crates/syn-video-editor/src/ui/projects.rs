//! Панель «Проекты» — недавние проекты карточками: превью таймлайна (ведение
//! курсора листает кадры клипов по порядку), длительность, кадр и частота,
//! когда открывали и меняли, состав медиатеки, клипы, размер на диске.
//! Список — открытые раньше (`store::recent`) и все `*.syn` в папке проектов;
//! файлы читаются в фоне, сведения кэшируются по времени правки файла.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::json;
use syngui::prelude::*;
use syngui::widgets::overlay::MenuItem;
use syngui::widgets::*;

use super::{ed, icons, St};
use crate::doc::{tc, MediaKind, Source, TrackKind};

/// Сведения о проекте для карточки.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub path: PathBuf,
    pub name: String,
    /// Время правки файла и последнего открытия, с от эпохи.
    pub modified: u64,
    pub opened: u64,
    /// Файл проекта + папка его файлов (`<имя>.media`), байт.
    pub size: u64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub videos: usize,
    pub images: usize,
    pub audios: usize,
    pub other: usize,
    pub clips: usize,
    pub titles: usize,
    /// Файлы для превью — клипы таймлайна по времени (без повторов), иначе медиатека.
    pub frames: Vec<PathBuf>,
    pub error: Option<String>,
}

impl Info {
    pub fn media_total(&self) -> usize {
        self.videos + self.images + self.audios + self.other
    }
    pub fn is_empty(&self) -> bool {
        self.media_total() == 0 && self.clips == 0
    }
    fn stamp(&self) -> u64 {
        self.opened.max(self.modified)
    }
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn dir_size(d: &Path, budget: &mut usize) -> u64 {
    let Ok(rd) = std::fs::read_dir(d) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(md) = e.metadata() else { continue };
        n += if md.is_dir() { dir_size(&e.path(), budget) } else { md.len() };
    }
    n
}

/// Прочитать проект и собрать сведения.
fn read(path: &Path) -> Info {
    let mut i = Info { path: path.to_path_buf(), modified: mtime(path), ..Default::default() };
    i.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut budget = 20_000;
    i.size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) + dir_size(&crate::store::media_dir(path), &mut budget);
    let p = match crate::store::load(path) {
        Ok(p) => p,
        Err(e) => {
            i.error = Some(e);
            return i;
        }
    };
    if !p.name.trim().is_empty() && p.name != "Без названия" {
        i.name = p.name.clone();
    }
    i.duration = p.duration();
    (i.width, i.height, i.fps) = (p.settings.width, p.settings.height, p.settings.fps);
    for m in &p.media {
        match m.kind {
            MediaKind::Video => i.videos += 1,
            MediaKind::Image => i.images += 1,
            MediaKind::Audio => i.audios += 1,
            _ => i.other += 1,
        }
    }
    let mut clips: Vec<(f64, PathBuf)> = Vec::new();
    for t in &p.tracks {
        i.clips += t.clips.len();
        for c in &t.clips {
            match &c.source {
                Source::Title { .. } => i.titles += 1,
                Source::Media { media } if t.kind == TrackKind::Video => {
                    if let Some(m) = p.media(*media).filter(|m| m.kind != MediaKind::Audio) {
                        clips.push((c.start, m.path.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    clips.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, f) in clips {
        if !i.frames.contains(&f) && i.frames.len() < 8 {
            i.frames.push(f);
        }
    }
    if i.frames.is_empty() {
        i.frames = p.media.iter().filter(|m| m.kind != MediaKind::Audio).map(|m| m.path.clone()).take(8).collect();
    }
    i.frames.retain(|f| f.exists());
    i
}

/// Все проекты: открытые раньше и лежащие в папке проектов (новые сверху).
pub fn scan() -> Vec<Info> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Info>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut paths: Vec<(PathBuf, u64)> = crate::store::recent();
    // проекты в своих папках (`~/Videos/X/X.syn`) и в прежнем общем каталоге
    let mut found: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::store::projects_dir()) {
        for e in rd.flatten() {
            let d = e.path();
            if let Some(name) = d.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|_| d.is_dir()) {
                let f = d.join(format!("{name}.{}", crate::store::EXT));
                if f.is_file() {
                    found.push(f);
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(crate::store::legacy_projects_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == crate::store::EXT) {
                found.push(p);
            }
        }
    }
    for p in found {
        if !paths.iter().any(|(q, _)| *q == p) {
            paths.push((p, 0));
        }
    }
    let auto = crate::store::autosave_path();
    let hidden = crate::store::hidden();
    // убранный вручную снова в списке, только если его открывали после этого
    paths.retain(|(p, opened)| *opened > 0 || !hidden.contains(p));
    let mut out = Vec::new();
    for (p, opened) in paths {
        if p == auto || !p.is_file() {
            continue;
        }
        let mt = mtime(&p);
        let hit = cache.lock().unwrap().get(&p).filter(|i| i.modified == mt).cloned();
        let mut i = hit.unwrap_or_else(|| {
            let i = read(&p);
            cache.lock().unwrap().insert(p.clone(), i.clone());
            i
        });
        i.opened = opened;
        out.push(i);
    }
    out.sort_by(|a, b| b.stamp().cmp(&a.stamp()));
    out
}

// ── время и размеры словами ─────────────────────────────────────────────

/// Смещение местного времени от UTC, с.
fn tz_offset() -> i64 {
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let s = std::process::Command::new("date").arg("+%z").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if s.len() == 5 {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let h: i64 = s[1..3].parse().unwrap_or(0);
            let m: i64 = s[3..5].parse().unwrap_or(0);
            sign * (h * 3600 + m * 60)
        } else {
            0
        }
    })
}

/// (год, месяц, день) из номера дня от эпохи.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// «сегодня, 14:32», «вчера, 09:10», «25 ноя, 18:00», «3 мар 2025».
pub fn when(secs: u64) -> String {
    if secs == 0 {
        return "—".into();
    }
    let off = tz_offset();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) + off;
    let t = secs as i64 + off;
    let (day, today) = (t.div_euclid(86_400), now.div_euclid(86_400));
    let hm = format!("{:02}:{:02}", t.rem_euclid(86_400) / 3600, t.rem_euclid(3600) / 60);
    const MON: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
    let (y, m, d) = civil(day);
    let (ny, _, _) = civil(today);
    match today - day {
        0 => format!("сегодня, {hm}"),
        1 => format!("вчера, {hm}"),
        _ if y == ny => format!("{d} {}, {hm}", MON[(m - 1) as usize]),
        _ => format!("{d} {} {y}", MON[(m - 1) as usize]),
    }
}

pub fn bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K * K {
        format!("{:.0} КБ", (f / K).max(1.0))
    } else if f < K * K * K {
        format!("{:.1} МБ", f / K / K)
    } else {
        format!("{:.2} ГБ", f / K / K / K)
    }
}

fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let (d, h) = (n % 10, n % 100);
    let w = if d == 1 && h != 11 {
        one
    } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        few
    } else {
        many
    };
    format!("{n} {w}")
}

// ── панель ─────────────────────────────────────────────────────────────

thread_local! {
    /// (список, идёт чтение, строка поиска, скрыть пустые)
    static STATE: std::cell::RefCell<Option<(RwSignal<Vec<Info>>, RwSignal<bool>, RwSignal<String>, RwSignal<bool>)>> = const { std::cell::RefCell::new(None) };
}

fn state() -> (RwSignal<Vec<Info>>, RwSignal<bool>, RwSignal<String>, RwSignal<bool>) {
    STATE.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(Vec::new()), use_signal(false), use_signal(String::new()), use_signal(true))))
}

/// Перечитать список в фоне.
pub fn refresh() {
    let (list, busy, _, _) = state();
    if busy.get_untracked() {
        return;
    }
    busy.set(true);
    std::thread::spawn(move || {
        let v = scan();
        syngui::async_runtime::run_on_main_thread(move || {
            list.set(v);
            busy.set(false);
        });
    });
}

pub fn panel(st: St) -> impl Widget {
    let (list, busy, query, hide_empty) = state();
    refresh();
    let head = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(
            TextField::new()
                .text(query.get_untracked())
                .placeholder("Поиск проекта…")
                .prefix_icon(icons::SEARCH)
                .on_change(move |s: &str| query.set(s.to_string())),
        ))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let on = hide_empty.get();
            vec![Box::new(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Toggle::with_state(on).on_change(move |v: bool| hide_empty.set(v))).child(Text::new("Скрыть пустые").class("row-sub")))]
        }))
        .child(ToolButton::new(icons::REFRESH).tooltip("Обновить список").on_click(refresh))
        .child(Button::new("Открыть…").icon(icons::FOLDER_OPEN).on_click(move || st.run_menu("project.open")).class("head-btn"))
        .child(Button::new("Новый проект").icon(icons::ADD).on_click(move || st.run_menu("project.new")).class("head-btn"));
    let grid = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let all = list.get();
        let q = query.get().to_lowercase();
        let hide = hide_empty.get();
        let cur = ed().path.clone();
        let shown: Vec<Info> = all
            .into_iter()
            .filter(|i| !hide || !i.is_empty() || Some(&i.path) == cur.as_ref())
            .filter(|i| q.is_empty() || i.name.to_lowercase().contains(&q) || i.path.to_string_lossy().to_lowercase().contains(&q))
            .collect();
        if shown.is_empty() {
            let t = if busy.get() { "Читаю проекты…" } else if q.is_empty() { "Проектов пока нет — «Новый проект» или «Открыть…»" } else { "Ничего не найдено" };
            return vec![Box::new(Padding::all(24.0).child(Center::new().child(Text::new(t).class("panel-empty"))))];
        }
        let mut g = Grid::new(16).min_column_width(260.0).gap(14.0);
        for i in shown {
            let on = Some(&i.path) == cur.as_ref();
            g = g.child(card(st, i, on));
        }
        vec![Box::new(Padding::all(12.0).child(g))]
    }));
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(DecoratedBox::new().class("panel-head").child(head))
        .child(DecoratedBox::new().class("grow").child(grid))
}

fn chip(icon: &str, text: String) -> Box<dyn Widget> {
    Box::new(DecoratedBox::new().class("proj-chip").child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("proj-chip-icon")).child(Text::new(text).class("proj-chip-text"))))
}

fn card(st: St, i: Info, on: bool) -> Box<dyn Widget> {
    let hover = use_signal(None::<f32>);
    let frames = i.frames.clone();
    let (dur, res) = (i.duration, if i.width > 0 { format!("{}×{} · {} к/с", i.width, i.height, (i.fps * 100.0).round() / 100.0) } else { String::new() });
    let err = i.error.clone();
    let cover = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let frac = hover.get();
        let mut stack = Stack::new().fit(StackFit::Expand);
        // ведение курсора по превью — клипы таймлайна по порядку и кадры внутри
        let (file_i, inner) = match frac {
            Some(f) if !frames.is_empty() => {
                let x = f * frames.len() as f32;
                ((x as usize).min(frames.len() - 1), x.fract())
            }
            _ => (0, 0.33),
        };
        match frames.get(file_i).and_then(|p| super::thumbs::strip(p).map(|s| (p, s))).filter(|(_, s)| !s.is_empty()) {
            Some((p, s)) => {
                let k = ((inner * s.len() as f32) as usize).min(s.len() - 1);
                let f = &s[k];
                stack = stack.child(Image::from_rgba_shared(format!("proj:{}:{k}:{:x}", p.display(), std::sync::Arc::as_ptr(&f.rgba) as *const u8 as usize), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false));
            }
            None => {
                let (icon, text) = if err.is_some() { (icons::DESCRIPTION, "не читается") } else if frames.is_empty() { (icons::MOVIE, "пустой проект") } else { (icons::MOVIE, "") };
                stack = stack.child(Center::new().child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("proj-empty-icon")).child(Text::new(text).class("row-sub"))));
            }
        }
        // подписи поверх кадра
        let mut top = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if on {
            top = top.child(DecoratedBox::new().class("proj-open-badge").child(Text::new("открыт").class("proj-open-text")));
        }
        top = top.child(DecoratedBox::new().class("grow"));
        if !res.is_empty() {
            top = top.child(DecoratedBox::new().class("tile-badge").child(Text::new(res.clone()).class("tile-badge-text")));
        }
        stack = stack.child(Padding::all(6.0).child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(top)));
        if dur > 0.0 {
            stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::End).child(DecoratedBox::new().class("tile-badge").child(Text::new(tc(dur)).class("tile-badge-text"))));
        }
        if let Some(f) = frac.filter(|_| frames.len() > 1) {
            // какой по счёту клип листается
            let n = frames.len();
            let mut dots = Row::new().gap(3.0).main_axis_alignment(MainAxisAlignment::Center);
            let cur = ((f * n as f32) as usize).min(n - 1);
            for k in 0..n {
                dots = dots.child(DecoratedBox::new().class(if k == cur { "proj-dot proj-dot-on" } else { "proj-dot" }));
            }
            stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(Padding::only(0.0, 0.0, 0.0, 8.0).child(dots)));
        }
        vec![Box::new(stack)]
    });
    let mut chips = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if i.videos > 0 {
        chips = chips.child(chip(icons::MOVIE, i.videos.to_string()));
    }
    if i.images > 0 {
        chips = chips.child(chip(icons::IMAGE, i.images.to_string()));
    }
    if i.audios > 0 {
        chips = chips.child(chip(icons::AUDIO, i.audios.to_string()));
    }
    if i.titles > 0 {
        chips = chips.child(chip(icons::TITLE, i.titles.to_string()));
    }
    chips = chips.child(DecoratedBox::new().class("grow")).child(Text::new(bytes(i.size)).class("row-sub"));
    let media_line = if i.error.is_some() {
        "файл не читается".to_string()
    } else if i.is_empty() {
        "пусто".to_string()
    } else {
        format!("{} в медиатеке · {}", plural(i.media_total(), "файл", "файла", "файлов"), plural(i.clips, "клип", "клипа", "клипов"))
    };
    let when_line = if i.opened > 0 && i.opened.abs_diff(i.modified) > 120 { format!("изменён {} · открыт {}", when(i.modified), when(i.opened)) } else { format!("изменён {}", when(i.modified)) };
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(i.name.clone()).max_lines(1).elide(Elide::End).class("proj-title"))
        .child(Text::new(when_line).max_lines(1).elide(Elide::End).class("row-sub"))
        .child(Text::new(media_line).max_lines(1).elide(Elide::End).class("row-sub"))
        .child(chips);
    let card = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(EventHook::new().fill().on_hover_at(move |p| hover.set(p.map(|(p, s)| (p.x / s.width.max(1.0)).clamp(0.0, 0.999)))).child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("proj-cover").child(cover))))
        .child(DecoratedBox::new().class("proj-body").child(body));
    let (p_open, p_menu) = (i.path.clone(), i.path.clone());
    let tip = format!("{}\n{}", i.name, i.path.display());
    let menu = vec![MenuItem::new("open", "Открыть"), MenuItem::new("folder", "Показать в папке"), MenuItem::new("forget", "Убрать из списка")];
    Box::new(
        ContextMenu::new()
            .items(menu)
            .on_select(move |id: &str| match id {
                "open" => open(st, &p_menu),
                "folder" => {
                    if let Some(d) = p_menu.parent() {
                        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                    }
                }
                "forget" => {
                    crate::store::forget(&p_menu);
                    let (list, ..) = state();
                    list.update(|v| v.retain(|x| x.path != p_menu));
                }
                _ => {}
            })
            .child(Tooltip::new(GestureDetector::new().on_click(move || open(st, &p_open)).child(DecoratedBox::new().class(if on { "proj-card proj-on" } else { "proj-card" }).child(card)), tip).delay_ms(600).max_width(420.0)),
    )
}

fn open(st: St, path: &Path) {
    if ed().path.as_deref() == Some(path) {
        st.open_panel("viewer");
        st.dock.activate("viewer");
        return;
    }
    st.exec("project.open", json!({"path": path.display().to_string()}));
    st.open_panel("viewer");
    st.dock.activate("viewer");
}
