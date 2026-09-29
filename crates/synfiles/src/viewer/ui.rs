//! Окно просмотрщика: заголовок, сцена, панель, лента, сведения, клавиши.

use std::path::PathBuf;

use synshell_common::{mime, xdg};
use syngui::input::CursorIcon;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::*;

use super::prepare::{self, Prepared};
use super::{v, Item, Orientation, Viewer};
use crate::model;
use crate::ui::{boxed, W};

/// Поля области просмотра под то, что лежит поверх неё: панель с отступом
/// от низа, лента миниатюр над ней, стрелки по бокам.
const INSET_EDGE: f32 = 16.0;
const INSET_TOOLBAR: f32 = 78.0;
const INSET_STRIP: f32 = 78.0;
const INSET_NAV: f32 = 76.0;

/// Сколько миниатюр видно в ленте разом: текущая держится в середине.
const STRIP_WINDOW: usize = 11;

mod glyph {
    pub const PREV: &str = "\u{e5cb}";
    pub const NEXT: &str = "\u{e5cc}";
    pub const ZOOM_IN: &str = "\u{e8ff}";
    pub const ZOOM_OUT: &str = "\u{e900}";
    pub const FIT: &str = "\u{ea10}";
    pub const FILL: &str = "\u{e56b}";
    pub const ROTATE_LEFT: &str = "\u{e419}";
    pub const ROTATE_RIGHT: &str = "\u{e41a}";
    pub const FLIP_H: &str = "\u{e3e8}";
    pub const FLIP_V: &str = "\u{e8d5}";
    pub const FULLSCREEN: &str = "\u{e5d0}";
    pub const FULLSCREEN_EXIT: &str = "\u{e5d1}";
    pub const DELETE: &str = "\u{e872}";
    pub const FOLDER: &str = "\u{e2c8}";
    pub const COPY: &str = "\u{e14d}";
    pub const INFO: &str = "\u{e88e}";
    pub const OPEN_WITH: &str = "\u{e89e}";
    pub const IMAGE: &str = "\u{e3f4}";
    pub const BROKEN: &str = "\u{e3ad}";
    pub const HOURGLASS: &str = "\u{e88b}";
}

pub fn root() -> W {
    let main = Reactive::new(move || -> Vec<W> {
        let full = v().window.get().fullscreen;
        let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("iv-window");
        if !full {
            col = col.child(titlebar());
        }
        col = col.child(DecoratedBox::new().class("grow").child(stage()));
        vec![boxed(col)]
    });
    let hook = EventHook::new()
        .on_key_down(|k, m| if on_key(k, m) { KeyReply::Handled } else { KeyReply::Ignore })
        .on_char(on_char)
        .child(Stack::new().fit(StackFit::Expand).child(main).child(menu_layer()));
    boxed(WindowResizeRegion::new().over_content(true).inset(5.0).child(hook))
}

// ─────────────────────────────────────────────────────────────── заголовок

fn title_button(glyph: &str, tip: &str, class: &str, on: impl FnMut() + Send + 'static) -> W {
    boxed(
        Tooltip::new(
            GestureDetector::new()
                .on_click(on)
                .child(DecoratedBox::new().class(format!("iv-title-btn {class}")).child(Icon::new(glyph).class("icon"))),
            tip.to_string(),
        )
        .delay_ms(500),
    )
}

fn titlebar() -> W {
    let info = Reactive::new(move || -> Vec<W> {
        let v = v();
        let Some(it) = v.current() else { return vec![boxed(Text::new("Просмотр").class("iv-name"))] };
        let n = v.items.with(|i| i.len());
        let idx = v.index.get();
        let _ = v.rev.get();
        let natural = prepare::get(&it).map(|p| if p.exif_turns % 2 != 0 { (p.natural.1, p.natural.0) } else { p.natural }).unwrap_or_default();
        let mut meta: Vec<String> = Vec::new();
        if natural.0 > 0 {
            meta.push(format!("{}×{}", natural.0, natural.1));
        }
        meta.push(model::format_size(it.size));
        if n > 1 {
            meta.push(format!("{} из {}", idx + 1, n));
        }
        vec![boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(glyph::IMAGE).class("icon iv-app-icon"))
                .child(Text::new(it.name).elide(Elide::Middle).max_lines(1).class("iv-name"))
                .child(Text::new(meta.join("  ·  ")).max_lines(1).class("iv-meta")),
        )]
    });
    let buttons = Reactive::new(move || -> Vec<W> {
        let v = v();
        let info_on = v.show_info.get();
        vec![boxed(
            Row::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(title_button(glyph::INFO, "Сведения (I)", if info_on { "toggled" } else { "" }, move || v.show_info.update(|s| *s = !*s)))
                .child(title_button(glyph::FOLDER, "Показать в папке", "", show_in_folder))
                .child(open_with_button())
                .child(title_button(glyph::COPY, "Копировать (Ctrl+C)", "", copy_current))
                .child(title_button(glyph::DELETE, "Удалить (Delete)", "", delete_current)),
        )]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = v().window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("iv-titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(WindowDragRegion::new().child(DecoratedBox::new().class("iv-drag").child(info))),
                )
                .child(DecoratedBox::new().class("iv-title-tools").child(buttons))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

fn open_with_button() -> W {
    boxed(
        Tooltip::new(
            GestureDetector::new()
                .on_click_with_bounds(|_, r| {
                    let v = v();
                    let Some(it) = v.current_untracked() else { return };
                    let mut items: Vec<MenuItem> = mime::apps_for(&it.mime)
                        .into_iter()
                        .filter(|a| a.id != "synfiles-viewer" && a.takes_files())
                        .map(|a| MenuItem::new(format!("app:{}", a.id), a.name.clone()))
                        .collect();
                    if items.is_empty() {
                        items.push(MenuItem::new("none", "Нет подходящих программ").disabled(true));
                    }
                    v.menu.set_always(items);
                    v.menu_pos.set(Point::new(r.origin.x, r.origin.y + r.size.height + 4.0));
                    v.menu_open.set(true);
                })
                .child(DecoratedBox::new().class("iv-title-btn").child(Icon::new(glyph::OPEN_WITH).class("icon"))),
            "Открыть с помощью".to_string(),
        )
        .delay_ms(500),
    )
}

fn menu_layer() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let v = v();
        let items = v.menu.get();
        vec![boxed(
            PopupMenu::new()
                .items(items)
                .position(v.menu_pos)
                .is_open(v.menu_open)
                .min_width(220.0)
                .on_select(move |id| {
                    v.menu_open.set(false);
                    if let Some(app_id) = id.strip_prefix("app:") {
                        if let (Some(app), Some(it)) = (xdg::app_by_id(app_id), v.current_untracked()) {
                            for cmd in app.commands_for(&[it.path.clone()]) {
                                super::spawn("sh".as_ref(), &["-c".as_ref(), cmd.as_ref()]);
                            }
                        }
                    }
                }),
        )]
    }))
}

// ─────────────────────────────────────────────────────────────── сцена

fn stage() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let v = v();
        let _ = v.rev.get();
        let Some(it) = v.current() else {
            return vec![boxed(Center::new().child(Text::new("Картинок не осталось").class("iv-hint")))];
        };
        let total = v.items.with(|i| i.len());
        let full = v.window.get().fullscreen;
        let (seq, command) = v.cmd.get();
        let user = v.orientation(&it.path);
        let prepared = prepare::get(&it);
        preload_neighbours(v);

        let (side, below) = if total > 1 { (INSET_NAV, INSET_TOOLBAR + INSET_STRIP) } else { (INSET_EDGE, INSET_TOOLBAR) };
        let mut stack = Stack::new().fit(StackFit::Expand);
        match &prepared {
            Some(p) if p.display.is_some() => {
                stack = stack.child(backdrop(&it, p)).child(DecoratedBox::new().class("iv-scrim"));
                let (turns, flip_h, flip_v) = combine(p, user);
                let display = p.display.clone().unwrap_or_default();
                stack = stack.child(
                    ImageViewport::new(display.to_string_lossy().to_string())
                        .natural_size(p.natural.0, p.natural.1)
                        .insets(INSET_EDGE, side, below, side)
                        .command(seq, command)
                        .quarter_turns(turns)
                        .flip(flip_h, flip_v)
                        .info(v.info)
                        .class("iv-viewport"),
                );
            }
            Some(p) => {
                let msg = p.error.clone().unwrap_or_else(|| "Не удалось открыть".into());
                stack = stack.child(DecoratedBox::new().class("iv-scrim")).child(placeholder(glyph::BROKEN, &msg));
            }
            None => {
                stack = stack.child(DecoratedBox::new().class("iv-scrim")).child(placeholder(glyph::HOURGLASS, "Открываю…"));
            }
        }
        if total > 1 {
            stack = stack.child(nav_layer(-1)).child(nav_layer(1));
        }
        stack = stack.child(bottom_layer(v, total, full));
        if v.show_info.get() {
            stack = stack.child(info_panel(&it, prepared.as_ref()));
        }
        if let Some(m) = v.message.get() {
            stack = stack.child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("iv-message-wrap")
                    .child(DecoratedBox::new().class("iv-message").child(Text::new(m).class("iv-message-text"))),
            );
        }
        vec![boxed(DecoratedBox::new().class("iv-stage").child(stack))]
    }))
}

/// Итоговый поворот: EXIF, поверх него — пользовательский. `ImageViewport`
/// сначала поворачивает, потом отражает; отражение из EXIF меняет знак
/// пользовательского поворота (R·F = F·R⁻¹).
fn combine(p: &Prepared, u: Orientation) -> (i32, bool, bool) {
    let turns = if p.exif_flip { p.exif_turns - u.turns } else { p.exif_turns + u.turns };
    (turns, u.flip_h ^ p.exif_flip, u.flip_v)
}

fn placeholder(glyph: &str, text: &str) -> W {
    boxed(
        Column::new()
            .gap(12.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(glyph).class("icon iv-placeholder-icon"))
            .child(Text::new(text).class("iv-hint")),
    )
}

/// Фон вместо пустых полей: та же картинка, размытая, на всю сцену.
fn backdrop(it: &Item, p: &Prepared) -> W {
    match prepare::blur(it, p) {
        Some(path) => boxed(Image::new(path.to_string_lossy().to_string()).fit(ImageFit::Cover).placeholder(false).class("iv-backdrop")),
        None => boxed(DecoratedBox::new().class("iv-backdrop-empty")),
    }
}

/// Соседние картинки готовятся заранее (EXIF, преобразование HEIC) —
/// листание без «Открываю…».
fn preload_neighbours(v: Viewer) {
    let n = v.items.with_untracked(|i| i.len());
    if n < 2 {
        return;
    }
    let i = v.index.get_untracked();
    for d in [1isize, -1] {
        let j = (i as isize + d).rem_euclid(n as isize) as usize;
        if let Some(it) = v.items.with_untracked(|v| v.get(j).cloned()) {
            let _ = prepare::get(&it);
        }
    }
}

fn nav_layer(delta: isize) -> impl Widget {
    let (icon, tip, align) = if delta < 0 {
        (glyph::PREV, "Предыдущая (←)", MainAxisAlignment::Start)
    } else {
        (glyph::NEXT, "Следующая (→)", MainAxisAlignment::End)
    };
    Row::new()
        .main_axis_alignment(align)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(ToolButton::new(icon).tooltip(tip).on_click(move || v().step(delta)).class("iv-nav"))
}

fn bottom_layer(v: Viewer, total: usize, full: bool) -> impl Widget {
    let mut items: Vec<W> = Vec::new();
    if total > 1 {
        items.push(filmstrip(v));
    }
    items.push(toolbar(v, total, full));
    items.push(boxed(DecoratedBox::new().class("iv-bottom-gap")));
    Column::new()
        .gap(10.0)
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(items)
}

/// Окно ленты: до [`STRIP_WINDOW`] миниатюр вокруг текущей.
fn strip_range(index: usize, total: usize) -> std::ops::Range<usize> {
    if total <= STRIP_WINDOW {
        return 0..total;
    }
    let half = STRIP_WINDOW / 2;
    let start = index.saturating_sub(half).min(total - STRIP_WINDOW);
    start..start + STRIP_WINDOW
}

fn filmstrip(v: Viewer) -> W {
    let index = v.index.get_untracked();
    let items = v.items.get_untracked();
    let thumbs: Vec<W> = strip_range(index, items.len())
        .map(|i| {
            let it = &items[i];
            let class = if i == index { "iv-thumb iv-thumb-active" } else { "iv-thumb" };
            let inner: W = match thumb(it) {
                Some(p) => boxed(Image::new(p.to_string_lossy().to_string()).fit(ImageFit::Cover).placeholder(false).class("iv-thumb-img")),
                None => boxed(Center::new().child(Icon::new(glyph::IMAGE).class("icon iv-thumb-icon"))),
            };
            boxed(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || v.go_to(i))
                    .child(DecoratedBox::new().class(class).child(inner)),
            )
        })
        .collect();
    boxed(DecoratedBox::new().class("iv-strip").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(thumbs)))
}

fn thumb(it: &Item) -> Option<PathBuf> {
    if it.mime == "image/svg+xml" {
        return (it.size < 2 << 20).then(|| it.path.clone());
    }
    crate::thumbs::get(&it.path, &it.mime, it.mtime, 128)
}

fn tool(icon: &'static str, tip: &str, on_click: impl Fn() + Send + Sync + 'static) -> W {
    boxed(ToolButton::new(icon).tooltip(tip.to_string()).on_click(on_click).class("iv-tool"))
}

fn separator() -> W {
    boxed(DecoratedBox::new().class("iv-tool-sep"))
}

fn toolbar(v: Viewer, total: usize, full: bool) -> W {
    use syngui::widgets::ImageViewCommand as C;
    let send = move |c: C| move || v.send(c);
    let orient = move |f: fn(&mut Orientation)| move || v.orient_current(f);
    let mut items: Vec<W> = Vec::new();
    if total > 1 {
        items.push(boxed(Text::new(format!("{} / {}", v.index.get_untracked() + 1, total)).class("iv-counter")));
        items.push(separator());
    }
    items.extend([
        tool(glyph::ZOOM_OUT, "Уменьшить (−)", send(C::ZoomOut)),
        zoom_label(v),
        tool(glyph::ZOOM_IN, "Увеличить (+)", send(C::ZoomIn)),
        separator(),
        tool(glyph::FIT, "Вписать (0)", send(C::Fit)),
        boxed(Tooltip::new(Button::new("1:1").on_click(send(C::Actual)).class("iv-tool-text"), "Пиксель в пиксель (1)".to_string()).delay_ms(500)),
        tool(glyph::FILL, "Заполнить (2)", send(C::Fill)),
        separator(),
        tool(glyph::ROTATE_LEFT, "Повернуть влево (Shift+R)", orient(|o| o.turns -= 1)),
        tool(glyph::ROTATE_RIGHT, "Повернуть вправо (R)", orient(|o| o.turns += 1)),
        tool(glyph::FLIP_H, "Отразить по горизонтали (H)", orient(|o| o.flip_h = !o.flip_h)),
        tool(glyph::FLIP_V, "Отразить по вертикали (V)", orient(|o| o.flip_v = !o.flip_v)),
        separator(),
        if full {
            tool(glyph::FULLSCREEN_EXIT, "Выйти из полноэкранного режима (F11)", syngui::signal::toggle_fullscreen)
        } else {
            tool(glyph::FULLSCREEN, "Во весь экран (F11)", syngui::signal::toggle_fullscreen)
        },
    ]);
    boxed(DecoratedBox::new().class("iv-toolbar").child(Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).children(items)))
}

/// Подпись масштаба — отдельный `Reactive`: во время анимации она меняется
/// каждый кадр, пересобирать ради неё сцену незачем.
fn zoom_label(v: Viewer) -> W {
    boxed(Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(format_zoom(v.info.get().scale)).class("iv-zoom"))] }))
}

fn format_zoom(scale: f32) -> String {
    let pct = scale * 100.0;
    if pct < 10.0 {
        format!("{pct:.1}%")
    } else {
        format!("{pct:.0}%")
    }
}

// ─────────────────────────────────────────────────────────────── сведения

fn info_row(label: &str, value: String) -> W {
    boxed(
        Column::new()
            .gap(2.0)
            .child(Text::new(label).class("iv-info-label"))
            .child(Text::new(value).max_lines(3).class("iv-info-value")),
    )
}

fn info_panel(it: &Item, p: Option<&Prepared>) -> W {
    let mut col = Column::new().gap(12.0).child(Text::new("Сведения").class("iv-info-title"));
    col = col.child(info_row("Имя", it.name.clone()));
    if let Some((w, h)) = p.map(|p| p.natural).filter(|n| n.0 > 0) {
        let (w, h) = if p.map(|p| p.exif_turns % 2 != 0).unwrap_or(false) { (h, w) } else { (w, h) };
        let mp = w as f64 * h as f64 / 1e6;
        col = col.child(info_row("Размер в пикселях", format!("{w} × {h} ({mp:.1} Мп)")));
    }
    col = col
        .child(info_row("Размер файла", model::format_size(it.size)))
        .child(info_row("Тип", mime::description(&it.mime)))
        .child(info_row("Изменён", model::format_time(it.mtime)))
        .child(info_row("Папка", it.path.parent().map(|d| d.display().to_string()).unwrap_or_default()));
    boxed(
        Row::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(DecoratedBox::new().class("iv-info").child(col)),
    )
}

// ─────────────────────────────────────────────────────────────── действия

fn show_in_folder() {
    if let Some(it) = v().current_untracked() {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("synfiles"));
        super::spawn(&exe, &[it.path.as_os_str()]);
    }
}

fn copy_current() {
    let v = v();
    let Some(it) = v.current_untracked() else { return };
    let paths = vec![it.path.clone()];
    let uris = syngui::clipboard::uri_list(&paths);
    let gnome = format!("copy\n{}", uris.replace("\r\n", "\n"));
    let mut rich: Vec<(&str, Vec<u8>)> = vec![("text/uri-list", uris.clone().into_bytes()), ("x-special/gnome-copied-files", gnome.into_bytes())];
    // Сама картинка — чтобы вставлялась в редакторы и чаты.
    if it.mime == "image/png" && it.size < 64 << 20 {
        if let Ok(b) = std::fs::read(&it.path) {
            rich.push(("image/png", b));
        }
    }
    let refs: Vec<(&str, &[u8])> = rich.iter().map(|(m, b)| (*m, b.as_slice())).collect();
    syngui::clipboard::copy_rich(&it.path.display().to_string(), &refs);
    v.flash("Скопировано");
}

fn delete_current() {
    let v = v();
    let Some(it) = v.current_untracked() else { return };
    match crate::trash::trash(&it.path) {
        Ok(_) => {
            let mut items = (*v.items.get_untracked()).clone();
            let i = v.index.get_untracked();
            items.retain(|x| x.path != it.path);
            if items.is_empty() {
                std::process::exit(0);
            }
            v.index.set(i.min(items.len() - 1));
            v.items.set(std::sync::Arc::new(items));
            v.send(syngui::widgets::ImageViewCommand::Fit);
            v.flash(format!("«{}» в корзине", it.name));
        }
        Err(e) => v.flash(format!("Не удалось удалить: {e}")),
    }
}

// ─────────────────────────────────────────────────────────────── клавиши

fn on_key(key: Key, m: Modifiers) -> bool {
    use syngui::widgets::ImageViewCommand as C;
    let v = v();
    if v.menu_open.get_untracked() {
        return false;
    }
    let full = v.window.get_untracked().fullscreen;
    let total = v.items.with_untracked(|i| i.len());
    // Приближенную картинку стрелки двигают, вписанную — листают.
    let zoomed = {
        let info = v.info.get_untracked();
        !info.fit && info.scale > info.fit_scale * 1.01
    };
    const PAN: f32 = 0.15;
    match key {
        Key::Escape if full => syngui::signal::toggle_fullscreen(),
        Key::Escape => std::process::exit(0),
        Key::C if m.ctrl => copy_current(),
        Key::W | Key::Q if m.ctrl => std::process::exit(0),
        Key::Left if zoomed => v.send(C::PanBy(-PAN, 0.0)),
        Key::Right if zoomed => v.send(C::PanBy(PAN, 0.0)),
        Key::Up => v.send(C::PanBy(0.0, -PAN)),
        Key::Down => v.send(C::PanBy(0.0, PAN)),
        Key::Left | Key::PageUp | Key::Backspace => v.step(-1),
        Key::Right | Key::PageDown | Key::Space => v.step(1),
        Key::Home if total > 1 => v.go_to(0),
        Key::End if total > 1 => v.go_to(total - 1),
        Key::Num0 => v.send(C::Fit),
        Key::Num1 => v.send(C::Actual),
        Key::Num2 => v.send(C::Fill),
        Key::R if m.shift => v.orient_current(|o| o.turns -= 1),
        Key::R => v.orient_current(|o| o.turns += 1),
        Key::H => v.orient_current(|o| o.flip_h = !o.flip_h),
        Key::V => v.orient_current(|o| o.flip_v = !o.flip_v),
        Key::I => v.show_info.update(|s| *s = !*s),
        Key::F | Key::F11 => syngui::signal::toggle_fullscreen(),
        Key::Delete => delete_current(),
        _ => return false,
    }
    true
}

/// «+» и «−» своих `Key` не имеют — приходят вводом символа.
fn on_char(c: char) -> bool {
    use syngui::widgets::ImageViewCommand as C;
    match c {
        '+' | '=' => v().send(C::ZoomIn),
        '-' | '_' => v().send(C::ZoomOut),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_current_in_window() {
        assert_eq!(strip_range(0, 3), 0..3);
        assert_eq!(strip_range(20, 40), 15..26);
        assert_eq!(strip_range(39, 40), 29..40);
    }

    #[test]
    fn exif_flip_reverses_user_turns() {
        let p = Prepared { exif_turns: 1, exif_flip: true, ..Default::default() };
        assert_eq!(combine(&p, Orientation { turns: 1, ..Default::default() }), (0, true, false));
        let p = Prepared { exif_turns: 1, ..Default::default() };
        assert_eq!(combine(&p, Orientation { turns: 1, flip_h: true, ..Default::default() }), (2, true, false));
    }

    #[test]
    fn zoom_label() {
        assert_eq!(format_zoom(1.0), "100%");
        assert_eq!(format_zoom(0.034), "3.4%");
    }
}
