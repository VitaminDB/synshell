//! Экран машины: кадр гостя во всю область, мышь и клавиатура уходят гостю;
//! сверху — плавающая панель (сочетания клавиш, буфер обмена, снимок, полный экран).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::widgets::{LiveInput, LiveView};
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, icon_btn, W};
use crate::keymap;
use crate::model;
use crate::state::{self, icons, Power, St};

static CTRL: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);
static RESIZE_SEQ: AtomicU64 = AtomicU64::new(0);

// ─── Положение панели над экраном гостя ─────────────────────────────────
// Сдвиг (логические пиксели) от места по умолчанию — сверху по центру; панель тянут за ручку
// слева, двойной щелчок по ручке — обратно в центр. Запоминается в `<данные>/syn-vm/screen-bar`.

fn bar_file() -> std::path::PathBuf {
    model::root().join("screen-bar")
}

fn bar_offset() -> &'static std::sync::Mutex<(f32, f32)> {
    static O: std::sync::OnceLock<std::sync::Mutex<(f32, f32)>> = std::sync::OnceLock::new();
    O.get_or_init(|| {
        let v = std::fs::read_to_string(bar_file()).ok().and_then(|s| {
            let mut it = s.split_whitespace().filter_map(|x| x.parse::<f32>().ok());
            Some((it.next()?, it.next()?))
        });
        std::sync::Mutex::new(v.unwrap_or((0.0, 0.0)))
    })
}

/// Сдвиг в пределах области экрана (`view_size` — физические пиксели).
fn clamp_offset(st: St, (dx, dy): (f32, f32)) -> (f32, f32) {
    let k = syngui::scale::pixel_ratio().max(0.1);
    let (w, h) = st.view_size.get_untracked();
    let (w, h) = (w as f32 / k, h as f32 / k);
    let mx = (w / 2.0 - 120.0).max(0.0);
    (dx.clamp(-mx, mx), dy.clamp(0.0, (h - 70.0).max(0.0)))
}

fn save_bar(o: (f32, f32)) {
    let _ = std::fs::write(bar_file(), format!("{:.0} {:.0}\n", o.0, o.1));
}

pub fn page(st: St, full: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(id) = st.sel.get() else { return vec![] };
        let power = st.live.get().get(&id).map(|l| l.power).unwrap_or_default();
        let busy = st.busy.get().get(&id).cloned();
        let connected = st.connected.get();
        let Some(vm) = st.vm(&id) else { return vec![] };
        if !power.on() {
            return vec![off(st, &vm, busy)];
        }
        let Some(sess) = state::session(&id).filter(|_| connected) else {
            return vec![waiting(t!("Подключение к экрану…"))];
        };
        let s = sess.clone();
        let view = Reactive::new(move || -> Vec<W> {
            st.frame_tick.get();
            let s2 = s.clone();
            let id2 = id.clone();
            // Разрешение под окно — кадр и так во всё окно; иначе — как выбрано (по умолчанию с пропорциями).
            let scale = st.vm(&id).map(|v| v.scale).unwrap_or_default();
            let fit = match scale {
                model::ScaleMode::Fit | model::ScaleMode::Integer => ImageFit::Contain,
                model::ScaleMode::Stretch => ImageFit::Fill,
                model::ScaleMode::Native => ImageFit::None,
            };
            let integer = scale == model::ScaleMode::Integer;
            // Сглаживание выключено — чёткие пиксели (DOS, старые игры).
            let sharp = st.vm(&id).is_some_and(|v| !v.smooth);
            // syn-gpu — основная видеокарта: изображение — рабочий стол с карты (кадры DESKTOP),
            // экран VNC пуст (`-vga none`) и служит только вводу (планшет — в долях кадра).
            // Видеокарта — та, с которой машина запущена (выбранная в настройках вступит после перезапуска).
            let syn_display = st.vm(&id).is_some_and(|v| state::display_now(&v) == model::DisplayModel::SynGpu);
            let (frame, guest) = if syn_display {
                let Some(d) = state::desktop_layer(&id) else { return vec![no_picture(st, &id)] };
                let size = (d.rect.2, d.rect.3);
                (d.frame.clone(), size)
            } else {
                (s.frame.clone(), st.guest_size.get_untracked())
            };
            // Над рабочим столом syn-gpu — аппаратный курсор гостя (форма от сервера syn-gpu).
            let guest_cursor = if syn_display { CursorIcon::Custom(crate::present::cursor_id(&id)) } else { CursorIcon::None };
            let screen = LiveView::new(frame)
                .fit(fit)
                .integer_scale(integer)
                .pixelated(sharp)
                .cursor(guest_cursor)
                .on_input(move |e| input(st, &id2, &s2, e))
                .on_resize(move |sz| on_resize(st, sz))
                .class("screen");
            // Кадр игры (syn-gpu) — поверх, в месте окна игры на экране гостя, тем же масштабом.
            let Some(layer) = state::game_layer(&id) else { return vec![boxed(screen)] };
            let Some((x, y, w, h)) = game_rect(st, fit, integer, guest, layer.rect) else { return vec![boxed(screen)] };
            // Ввод над слоем — гостю: доли слоя → доли кадра гостя (окно игры x,y,w,h).
            let (s3, id3) = (s.clone(), id.clone());
            let (gw, gh) = guest;
            let (lx, ly, lw, lh) = layer.rect;
            let map = move |fx: f32, fy: f32| -> (f32, f32) {
                ((lx as f32 + fx * lw as f32) / gw.max(1) as f32, (ly as f32 + fy * lh as f32) / gh.max(1) as f32)
            };
            let game_view = LiveView::new(layer.frame.clone()).fit(ImageFit::Fill).pixelated(sharp).cursor(guest_cursor).on_input(move |e| {
                let e = match e {
                    LiveInput::Move { x, y } => {
                        let (x, y) = map(x, y);
                        LiveInput::Move { x, y }
                    }
                    LiveInput::Down { button, x, y } => {
                        let (x, y) = map(x, y);
                        if let Some(vm) = st.vm(&id3) {
                            super::capture::log(&vm, &format!("щелчок по слою игры → гостю {:.0},{:.0}", x * gw as f32, y * gh as f32));
                        }
                        LiveInput::Down { button, x, y }
                    }
                    LiveInput::Up { button, x, y } => {
                        let (x, y) = map(x, y);
                        LiveInput::Up { button, x, y }
                    }
                    LiveInput::Wheel { dx, dy, x, y } => {
                        let (x, y) = map(x, y);
                        LiveInput::Wheel { dx, dy, x, y }
                    }
                    other => other,
                };
                input(st, &id3, &s3, e)
            });
            let game = Positioned::new(game_view).at(x, y).dimensions(w, h);
            vec![boxed(Stack::new().fit(StackFit::Expand).child(screen).child(game))]
        });
        let mut stack = Stack::new().fit(StackFit::Expand).child(view);
        if power == Power::Paused {
            stack = stack.child(paused_veil(st, &vm.id));
        }
        let tid = vm.id.clone();
        stack = stack.child(Reactive::new(move || -> Vec<W> {
            st.frame_tick.get();
            if super::capture::active(&tid) {
                return vec![];
            }
            vec![toolbar(st, &tid, full)]
        }));
        vec![boxed(DecoratedBox::new().child(stack).class(if full { "screen-wrap full" } else { "screen-wrap" }))]
    }))
}

/// Щелчок по полноэкранной игре (flags&2) у машины с 3D — захват мыши; сам щелчок уходит гостю.
/// Окно игры не во весь экран — обычный щелчок (меню, лаунчеры): захват — кнопкой или Ctrl+Alt+G.
fn try_capture(st: St, id: &str) {
    // Гость сам перевёл мышь в относительную (DOS: игра через SYNMOUSE.COM, или мышь PS/2 без
    // драйвера) — захват на его мыши.
    if super::capture::rel_wanted(id) {
        if let Some(vm) = st.vm(id) {
            super::capture::start_own(st, &vm);
        }
        return;
    }
    let Some(vm) = st.vm(id).filter(|v| v.gpu_on()) else { return };
    if state::game_layer(id).is_some_and(|l| l.fullscreen) {
        super::capture::start(st, &vm);
    }
}

/// Место окна игры (физические пиксели экрана гостя) → координаты в виджете экрана:
/// как LiveView кладёт кадр гостя (с пропорциями / растянуть / 1:1, по центру).
fn game_rect(st: St, fit: ImageFit, integer: bool, (gw, gh): (u32, u32), (x, y, w, h): (i32, i32, u32, u32)) -> Option<(f32, f32, f32, f32)> {
    let k = syngui::scale::pixel_ratio();
    let (vw, vh) = st.view_size.get_untracked();
    if vw == 0 || vh == 0 || gw == 0 || gh == 0 || w == 0 || h == 0 {
        return None;
    }
    let (bw, bh) = (vw as f32 / k, vh as f32 / k);
    let (gw, gh) = (gw as f32, gh as f32);
    let (sx, sy) = match fit {
        ImageFit::Fill => (bw / gw, bh / gh),
        // LiveView кладёт кадр 1:1 в логических единицах (пиксель гостя — логический пиксель).
        ImageFit::None => (1.0, 1.0),
        _ => {
            let s = (bw / gw).min(bh / gh);
            // Целый масштаб — как LiveView::integer_scale: целое число физических пикселей.
            let n = (s * k + 1e-3).floor();
            let s = if integer && n >= 1.0 { n / k } else { s };
            (s, s)
        }
    };
    let (ox, oy) = ((bw - gw * sx) / 2.0, (bh - gh * sy) / 2.0);
    Some((ox + x as f32 * sx, oy + y as f32 * sy, w as f32 * sx, h as f32 * sy))
}

/// Ввод над кадром → гостю.
fn input(st: St, id: &str, s: &crate::vnc::Session, e: LiveInput) {
    // Захват для игр: мышь — относительной (смещения идут из syngui сами), кнопки и колесо —
    // туда же; Ctrl+Alt — выход.
    if super::capture::active(id) {
        match e {
            LiveInput::Move { .. } | LiveInput::TouchMove { .. } => return,
            LiveInput::Down { button, .. } => {
                if let Some(i) = button_index(button) {
                    super::capture::button(i, true);
                }
                return;
            }
            LiveInput::Up { button, .. } => {
                if let Some(i) = button_index(button) {
                    super::capture::button(i, false);
                }
                return;
            }
            LiveInput::Wheel { dy, .. } => {
                super::capture::wheel(dy.signum() as i32 * dy.abs().ceil() as i32);
                return;
            }
            LiveInput::Key { key, pressed } => {
                match key {
                    Key::Ctrl => CTRL.store(pressed, Ordering::Relaxed),
                    Key::Alt => ALT.store(pressed, Ordering::Relaxed),
                    _ => {}
                }
                if pressed && matches!(key, Key::Ctrl | Key::Alt) && CTRL.load(Ordering::Relaxed) && ALT.load(Ordering::Relaxed) {
                    s.key(0x1d, 0, false);
                    s.key(0x38, 0, false);
                    super::capture::stop("Ctrl+Alt");
                    state::toast(st, t!("Мышь освобождена"));
                    return;
                }
            }
            _ => {}
        }
    } else if let LiveInput::Down { button: MouseButton::Left, .. } = e {
        try_capture(st, id);
    }
    match e {
        LiveInput::Move { x, y } => s.pointer(x, y),
        LiveInput::Down { button, x, y } => {
            if let Some(i) = button_index(button) {
                if i == 0 {
                    state::offer_clipboard(id);
                }
                s.button(i, true, x, y);
            }
            if st.keys_menu.get_untracked() {
                st.keys_menu.set(false);
            }
            if st.modes_menu.get_untracked() {
                st.modes_menu.set(false);
            }
        }
        LiveInput::Up { button, x, y } => {
            if let Some(i) = button_index(button) {
                s.button(i, false, x, y);
            }
        }
        LiveInput::Wheel { dx, dy, .. } => s.wheel(dx, dy),
        LiveInput::TouchStart { x, y, .. } => {
            s.pointer(x, y);
            s.button(0, true, x, y);
        }
        LiveInput::TouchMove { x, y, .. } => s.pointer(x, y),
        LiveInput::TouchEnd { x, y, .. } => s.button(0, false, x, y),
        LiveInput::Key { key, pressed } => {
            match key {
                Key::Ctrl => CTRL.store(pressed, Ordering::Relaxed),
                Key::Alt => ALT.store(pressed, Ordering::Relaxed),
                _ => {}
            }
            // Ctrl+Alt+F — полный экран, Ctrl+Alt+V — вставить текст хоста набором.
            if pressed && CTRL.load(Ordering::Relaxed) && ALT.load(Ordering::Relaxed) {
                match key {
                    Key::F => {
                        // Модификаторы гостю — отпущенными, иначе залипнут.
                        s.key(0x1d, 0, false);
                        s.key(0x38, 0, false);
                        toggle_full(st);
                        return;
                    }
                    Key::V => {
                        s.key(0x1d, 0, false);
                        s.key(0x38, 0, false);
                        type_clipboard(id);
                        return;
                    }
                    // Ctrl+Alt+G — захват мыши для игры (машины с 3D).
                    Key::G => {
                        s.key(0x1d, 0, false);
                        s.key(0x38, 0, false);
                        if let Some(vm) = st.vm(id) {
                            super::capture::log(&vm, "нажато Ctrl+Alt+G");
                            if vm.gpu_on() {
                                super::capture::start(st, &vm);
                            } else {
                                state::toast(st, t!("Захват мыши — у машин с «3D-ускорением (syn-gpu)»"));
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
            let code = syngui::input::current_scancode().or_else(|| keymap::evdev_of(key)).and_then(keymap::qnum);
            if let Some(q) = code {
                s.key(q, keymap::keysym_of(key), pressed);
            } else {
                let ks = keymap::keysym_of(key);
                if ks != 0 {
                    s.key(0, ks, pressed);
                }
            }
        }
        LiveInput::Char(_) => {}
    }
}

fn button_index(b: MouseButton) -> Option<u8> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// Область экрана изменилась: через полсекунды покоя — разрешение гостя под неё.
fn on_resize(st: St, sz: Size) {
    let k = syngui::scale::pixel_ratio();
    let w = (sz.width * k).round() as u32;
    let h = (sz.height * k).round() as u32;
    if st.view_size.get_untracked() == (w, h) {
        return;
    }
    st.view_size.set((w, h));
    let seq = RESIZE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(450));
        if RESIZE_SEQ.load(Ordering::Relaxed) == seq {
            run_on_main_thread(move || state::fit_guest(st));
        }
    });
}

pub fn toggle_full(st: St) {
    let on = !st.fullscreen.get_untracked();
    st.fullscreen.set(on);
    syngui::signal::set_fullscreen(on);
}

fn type_clipboard(id: &str) {
    if let (Some(s), Some(text)) = (state::session(id), syngui::clipboard::paste()) {
        s.type_text(&text);
    }
}

fn combo(id: &str, keys: &[u32]) {
    if let Some(s) = state::session(id) {
        s.combo(keys);
    }
}

/// Запись экрана гостя: точка — начать; идёт запись — красная плашка со временем, нажатие — остановить.
/// Нажатие ловится снаружи перестраиваемой части: время обновляется каждую секунду, и
/// жест внутри неё терялся бы, попав на перестройку.
fn record_button(st: St, id: String) -> W {
    let id2 = id.clone();
    let face = Reactive::new(move || -> Vec<W> {
        let since = st.live.get().get(&id).map(|l| l.rec_since).unwrap_or(0);
        if since == 0 {
            return vec![boxed(DecoratedBox::new().child(Icon::new(icons::RECORD).class("ibtn-icon")).class("ibtn"))];
        }
        let secs = crate::model::now().saturating_sub(since);
        let label = format!("{}:{:02}", secs / 60, secs % 60);
        vec![boxed(
            DecoratedBox::new()
                .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("rec-dot")).child(Text::new(label).class("rec-time")).child(Icon::new(icons::STOP).class("rec-stop")))
                .class("rec-pill"),
        )]
    });
    let b = GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || state::toggle_record(st, &id2)).child(face);
    boxed(Tooltip::new(b, t!("Запись экрана гостя в видео (Видео/Записи экрана)")).delay_ms(500))
}

fn toolbar(st: St, id: &str, full: bool) -> W {
    let (a, b, c, d, e) = (id.to_string(), id.to_string(), id.to_string(), id.to_string(), id.to_string());
    let auto = st.vm(id).map(|v| v.auto_resize).unwrap_or(true);
    let size = match state::desktop_layer(id).filter(|_| st.vm(id).is_some_and(|v| state::display_now(&v) == model::DisplayModel::SynGpu)) {
        Some(d) => (d.rect.2, d.rect.3),
        None => st.guest_size.get_untracked(),
    };
    let gpu3d = st.vm(id).is_some_and(|v| v.gpu_on());
    // Кадры от syn-gpu (основная видеокарта или 3D) — их можно пропустить через DLSS 5.
    let syn_gpu = gpu3d || st.vm(id).is_some_and(|v| state::display_now(&v) == model::DisplayModel::SynGpu);
    let cap_id = id.to_string();
    // Ручка: тянуть панель в любое место экрана; двойной щелчок — сверху по центру.
    let grip = GestureDetector::new()
        .cursor(CursorIcon::Move)
        .on_pan_update(move |u| {
            let mut o = bar_offset().lock().unwrap();
            *o = clamp_offset(st, (o.0 + u.delta.x, o.1 + u.delta.y));
            drop(o);
            st.frame_tick.update(|t| *t = t.wrapping_add(1));
        })
        .on_pan_end(|_| save_bar(*bar_offset().lock().unwrap()))
        .on_double_click(move || {
            *bar_offset().lock().unwrap() = (0.0, 0.0);
            save_bar((0.0, 0.0));
            st.frame_tick.update(|t| *t = t.wrapping_add(1));
        })
        .child(DecoratedBox::new().child(Icon::new(icons::DRAG).class("grip-icon")).class("grip"));
    let mut bar = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(grip);
    if gpu3d {
        // Захват мыши для игр (камера в 3D-играх читает сырые смещения); Ctrl+Alt — выход.
        bar = bar
            .child(icon_btn(icons::MOUSE, t!("Захватить мышь для игры (Ctrl+Alt+G; Ctrl+Alt — освободить)"), move || {
                if let Some(vm) = st.vm(&cap_id) {
                    super::capture::log(&vm, "нажата кнопка захвата");
                    super::capture::start(st, &vm);
                }
            }))
            .child(DecoratedBox::new().class("vsep"));
    }
    let bar = bar
        .child(icon_btn(icons::KEYBOARD, t!("Ctrl+Alt+Del"), move || combo(&a, keymap::CTRL_ALT_DEL)))
        .child(icon_btn(icons::MORE, t!("Другие сочетания"), move || {
            st.modes_menu.set(false);
            st.keys_menu.set(!st.keys_menu.get_untracked());
        }))
        .child(DecoratedBox::new().class("vsep"))
        .child(icon_btn(icons::PASTE, t!("Напечатать текст из буфера (Ctrl+Alt+V)"), move || type_clipboard(&b)))
        .child(icon_btn(icons::CAMERA, t!("Снимок экрана"), move || state::screenshot(st, &c)))
        .child(record_button(st, id.to_string()))
        .child(icon_btn(
            icons::FIT,
            if auto { t!("Разрешение под окно: включено") } else { t!("Разрешение под окно: выключено") },
            move || {
                state::edit(st, |v| v.auto_resize = !v.auto_resize);
                state::fit_guest(st);
                let on = st.vm(&d).map(|v| v.auto_resize).unwrap_or(false);
                state::toast(st, if on { t!("Разрешение гостя подстраивается под окно") } else { t!("Разрешение гостя больше не меняется") });
            },
        ))
        .child(mode_button(st, &e, size))
        .children(syn_gpu.then(|| dlss_button(st, id)))
        .child(DecoratedBox::new().class("vsep"))
        .child(icon_btn(if full { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }, t!("Во весь экран (Ctrl+Alt+F)"), move || toggle_full(st)));
    let bar = DecoratedBox::new().child(bar).class(if auto { "float-bar auto" } else { "float-bar" });
    let id2 = id.to_string();
    let menu = Reactive::new(move || -> Vec<W> {
        if !st.keys_menu.get() {
            return vec![];
        }
        let item = |label: &str, keys: &'static [u32]| -> W {
            let id = id2.clone();
            boxed(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || {
                        combo(&id, keys);
                        st.keys_menu.set(false);
                    })
                    .child(DecoratedBox::new().child(Text::new(label.to_string()).class("menu-label")).class("menu-item")),
            )
        };
        vec![boxed(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(item("Ctrl+Alt+Backspace", keymap::CTRL_ALT_BACKSPACE))
                        .child(item("Ctrl+Alt+F1", keymap::CTRL_ALT_F1))
                        .child(item("Ctrl+Alt+F2", keymap::CTRL_ALT_F2))
                        .child(item("Ctrl+Alt+F7", keymap::CTRL_ALT_F7))
                        .child(item("Alt+Tab", keymap::ALT_TAB))
                        .child(item("Alt+F4", keymap::ALT_F4))
                        .child(item("Super", keymap::SUPER))
                        .child(item("Print Screen", keymap::PRINT)),
                )
                .class("menu"),
        )]
    });
    // Сверху по центру со сдвигом: отступ слева/справа сдвигает центр, сверху — вниз.
    let (dx, dy) = clamp_offset(st, *bar_offset().lock().unwrap());
    boxed(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(bar)
            .child(menu)
            .child(modes_menu(st, id.to_string()))
            .class("float-wrap")
            .style("padding-top", StyleValue::px(10.0 + dy))
            .style("padding-left", StyleValue::px((2.0 * dx).max(0.0)))
            .style("padding-right", StyleValue::px((-2.0 * dx).max(0.0))),
    )
}

// ─── Режим экрана и масштаб: кнопка и выпадающий список ─────────────────
// Режимы приходят от сервера syn-gpu (MODES): DOS — текстовые режимы и «под окно», Windows —
// разрешения и «по размеру окна». У прочих машин в списке только масштаб.

use crate::present::{mode_kind, modes_flags, GuestModes, ModeEntry};

/// Цвета по биту на точку: 8 — «256 цветов», 15/16 — тысячи, 24/32 — миллионы.
fn colors(bpp: u16) -> String {
    match bpp {
        1 => t!("2 цвета"),
        2 => t!("4 цвета"),
        4 => t!("16 цветов"),
        8 => t!("256 цветов"),
        15 | 16 => t!("тысячи цветов"),
        24 | 32 => t!("миллионы цветов"),
        b => t!("{b} бит", b = b),
    }
}

/// Подпись текущего режима на кнопке: «Текст 80×25», «Графика 640×480, 256 цветов», «Windows 1920×1080».
fn current_title(m: &GuestModes) -> String {
    let c = m.current;
    if m.has(modes_flags::GLIDE) && m.game.0 > 0 {
        return t!("3D {w}×{h} → {rw}×{rh}", w = m.game.0, h = m.game.1, rw = m.render.0, rh = m.render.1);
    }
    match c.kind {
        mode_kind::TEXT => t!("Текст {w}×{h}", w = c.width, h = c.height),
        mode_kind::WINDOW if m.has(modes_flags::DESKTOP) => t!("Windows {w}×{h}", w = c.width, h = c.height),
        mode_kind::WINDOW => t!("Под окно ({w}×{h})", w = c.width, h = c.height),
        _ if m.has(modes_flags::DESKTOP) => t!("Windows {w}×{h}", w = c.width, h = c.height),
        _ if c.bpp > 0 => t!("Графика {w}×{h}, {c}", w = c.width, h = c.height, c = colors(c.bpp)),
        _ => t!("Графика {w}×{h}", w = c.width, h = c.height),
    }
}

/// Подпись пункта списка.
fn entry_label(m: &GuestModes, e: &ModeEntry) -> String {
    match e.kind {
        mode_kind::TEXT => t!("Текст {w}×{h}", w = e.width, h = e.height),
        mode_kind::WINDOW if m.has(modes_flags::DESKTOP) => t!("По размеру окна"),
        mode_kind::WINDOW if e.width > 0 => t!("Под окно ({w}×{h})", w = e.width, h = e.height),
        mode_kind::WINDOW => t!("Под окно"),
        _ if e.bpp > 0 && !m.has(modes_flags::DESKTOP) => format!("{}×{}, {}", e.width, e.height, colors(e.bpp)),
        _ => format!("{}×{}", e.width, e.height),
    }
}

/// Галочка у пункта: Windows — у выбранного разрешения при FIXED, иначе у «по размеру окна»;
/// DOS — у текущего текстового режима (режим «под окно» — если текущий совпал с ним, а не с готовым).
fn entry_checked(m: &GuestModes, e: &ModeEntry) -> bool {
    let c = m.current;
    if m.has(modes_flags::DESKTOP) {
        return if m.has(modes_flags::FIXED) { e.kind == mode_kind::GRAPHICS && (e.width, e.height) == (c.width, c.height) } else { e.kind == mode_kind::WINDOW };
    }
    let same = (e.width, e.height) == (c.width, c.height);
    match e.kind {
        mode_kind::TEXT => c.kind == mode_kind::TEXT && same,
        mode_kind::WINDOW => c.kind == mode_kind::WINDOW || (c.kind == mode_kind::TEXT && same && !m.list.iter().any(|x| x.kind == mode_kind::TEXT && (x.width, x.height) == (c.width, c.height))),
        _ => *e == c,
    }
}

/// Кнопка панели: текущий режим (или размер кадра, если сервер режимов не шлёт) и стрелка списка.
fn mode_button(st: St, id: &str, size: (u32, u32)) -> W {
    let label = match state::guest_modes(id) {
        Some(m) if m.current.width > 0 => current_title(&m),
        _ if size.0 > 0 => format!("{}×{}", size.0, size.1),
        _ => t!("Экран"),
    };
    let label = match st.vm(id).and_then(|v| v.gpu_arg("--max-fps")).filter(|v| v != "0") {
        Some(n) => format!("{label} · {}", t!("{n} к/с", n = n)),
        None => label,
    };
    let pending = state::guest_modes(id).is_some_and(|m| m.has(modes_flags::PENDING));
    let open = st.modes_menu.get_untracked();
    let face = DecoratedBox::new()
        .child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(label).max_lines(1).class("mode-label"))
                .child(Icon::new(icons::EXPAND).class("mode-arrow")),
        )
        .class(match (open, pending) {
            (true, _) => "mode-btn open",
            (_, true) => "mode-btn pending",
            _ => "mode-btn",
        });
    let b = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            st.keys_menu.set(false);
            st.modes_menu.set(!st.modes_menu.get_untracked());
            st.frame_tick.update(|t| *t = t.wrapping_add(1));
        })
        .child(face);
    boxed(Tooltip::new(b, t!("Режим экрана и масштаб")).delay_ms(500))
}

/// Кнопка DLSS 5: выключен — «DLSS» приглушённо; включён — состояние модели (запуск, мс на кадр,
/// не успевает, как есть), причина — в подсказке. Нажатие включает и выключает (запоминается у машины).
fn dlss_button(st: St, id: &str) -> W {
    use crate::dlss::{reason, FrameError, Status};
    let on = st.vm(id).is_some_and(|v| v.dlss);
    let s = crate::dlss::status(id);
    let (text, class, tip) = match (&s, on) {
        (_, false) | (Status::Off, _) => ("DLSS".to_string(), "dlss-btn", t!("DLSS 5: картинка гостя через нейрорендеринг NVIDIA (syn-dlss)")),
        (Status::Starting, _) => (t!("DLSS 5 · запуск…"), "dlss-btn wait", t!("Помощник DLSS 5 готовит модель")),
        (Status::Active { total_ms, fps, size, cut: Some(n), .. }, _) => (
            t!("DLSS 5 · {ms} мс · 1 проход", ms = format!("{total_ms:.0}")),
            "dlss-btn on",
            t!("Кадры {w}×{h} идут через DLSS 5 в 1 проход вместо {n}, чтобы успевать: {ms} мс на кадр, {fps} кадр/с", w = size.0, h = size.1, n = n, ms = format!("{total_ms:.1}"), fps = format!("{fps:.0}")),
        ),
        (Status::Active { total_ms, fps, size, .. }, _) => (
            t!("DLSS 5 · {ms} мс", ms = format!("{total_ms:.0}")),
            "dlss-btn on",
            t!("Кадры {w}×{h} идут через DLSS 5: {ms} мс на кадр, {fps} кадр/с", w = size.0, h = size.1, ms = format!("{total_ms:.1}"), fps = format!("{fps:.0}")),
        ),
        (Status::Slow { total_ms, budget_ms }, _) => (
            t!("DLSS 5 · не успевает"),
            "dlss-btn warn",
            t!("Модели нужно {a} мс, а кадр гостя приходит раз в {b} мс: кадры идут как есть, новая попытка — через 20 с или при смене разрешения", a = format!("{total_ms:.0}"), b = format!("{budget_ms:.0}")),
        ),
        (Status::Bypass(FrameError::Busy(game)), _) => (t!("DLSS 5 · занят"), "dlss-btn warn", t!("DLSS 5 сейчас занят: {game}", game = game.clone())),
        (Status::Bypass(e), _) => (t!("DLSS 5 · как есть"), "dlss-btn warn", reason(e)),
    };
    let tip = if on { format!("{tip}\n{}", t!("Нажмите, чтобы выключить")) } else { tip };
    let face = DecoratedBox::new()
        .child(
            Row::new()
                .gap(5.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::DLSS).class("dlss-icon"))
                .child(Text::new(text).max_lines(1).class("dlss-label")),
        )
        .class(class);
    let id = id.to_string();
    let b = GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || state::set_dlss(st, &id, !on)).child(face);
    boxed(Tooltip::new(b, tip).delay_ms(300))
}

/// Пункт выпадающего списка: галочка, подпись; `on` — нажатие (`None` — неактивен).
fn menu_row(label: String, checked: bool, on: Option<Box<dyn Fn() + Send + Sync>>) -> W {
    let check: W = if checked { boxed(Icon::new(icons::CHECK).class("menu-check")) } else { boxed(DecoratedBox::new().class("menu-check-gap")) };
    let row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(check).child(Text::new(label).max_lines(1).class("menu-label"));
    match on {
        Some(f) => boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || f()).child(DecoratedBox::new().child(row).class(if checked { "menu-item checked" } else { "menu-item" }))),
        None => boxed(DecoratedBox::new().child(row).class(if checked { "menu-item off checked" } else { "menu-item off" })),
    }
}

fn menu_head(text: String) -> W {
    boxed(Text::new(text).class("menu-head"))
}

fn menu_hint(icon: &str, text: String) -> W {
    boxed(
        DecoratedBox::new()
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Start).child(Icon::new(icon).class("menu-hint-icon")).child(Text::new(text).max_lines(3).class("menu-hint-text grow")))
            .class("menu-hint"),
    )
}

fn modes_menu(st: St, id: String) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if !st.modes_menu.get() {
            return vec![];
        }
        let Some(vm) = st.vm(&id) else { return vec![] };
        let mut row = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Start);
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(m) = state::guest_modes(&id).filter(|m| !m.list.is_empty()) {
            let dos = m.has(modes_flags::DOS);
            // DOS без SYNMOUSE: список только для сведения.
            let can = !dos || m.has(modes_flags::AGENT);
            col = col.child(menu_head(if dos { t!("Режим экрана DOS") } else { t!("Разрешение экрана") }));
            for e in m.list.iter().copied().filter(|e| e.kind != mode_kind::RENDER) {
                let id2 = id.clone();
                let on: Option<Box<dyn Fn() + Send + Sync>> = can.then(|| {
                    Box::new(move || {
                        state::set_guest_mode(st, &id2, e);
                        st.modes_menu.set(false);
                    }) as Box<dyn Fn() + Send + Sync>
                });
                col = col.child(menu_row(entry_label(&m, &e), entry_checked(&m, &e), on));
            }
            // Разрешение 3D-игр (Voodoo): кадр хоста — по окну, как в игре, ×2, ×3; действует сразу.
            let render: Vec<ModeEntry> = m.list.iter().copied().filter(|e| e.kind == mode_kind::RENDER).collect();
            if !render.is_empty() {
                col = col.child(menu_head(t!("3D (Voodoo): разрешение")));
                for e in render {
                    let id2 = id.clone();
                    let k = e.width;
                    let label = match k {
                        0 => t!("По размеру окна"),
                        1 if m.game.0 > 0 => t!("Как в игре ({w}×{h})", w = m.game.0, h = m.game.1),
                        1 => t!("Как в игре"),
                        _ => format!("×{k}"),
                    };
                    let f: Box<dyn Fn() + Send + Sync> = Box::new(move || {
                        state::edit(st, |v| v.glide_res = k as u8);
                        crate::present::set_render_pref(&id2, k);
                        state::set_guest_mode(st, &id2, e);
                    });
                    col = col.child(menu_row(label, e.bpp != 0, Some(f)));
                }
            }
            if m.has(modes_flags::PENDING) {
                col = col.child(menu_hint(icons::INFO, t!("Применится в командной строке DOS — после выхода из программы")));
            }
            if dos && !can {
                col = col.child(menu_hint(icons::INFO, t!("Чтобы менять режим, загрузите SYNMOUSE: запустите SYNGPU → Мышь и звук → автозагрузка")));
            }
            row = row.child(col.class("menu-col"));
            col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        }
        col = col.child(menu_head(t!("Масштаб")));
        for s in [model::ScaleMode::Fit, model::ScaleMode::Integer, model::ScaleMode::Stretch, model::ScaleMode::Native] {
            let f: Box<dyn Fn() + Send + Sync> = Box::new(move || {
                state::edit(st, |v| v.scale = s);
                st.frame_tick.update(|t| *t = t.wrapping_add(1));
            });
            col = col.child(menu_row(syngui::i18n::t(s.title()), vm.scale == s, Some(f)));
        }
        let f: Box<dyn Fn() + Send + Sync> = Box::new(move || {
            state::edit(st, |v| v.smooth = !v.smooth);
            st.frame_tick.update(|t| *t = t.wrapping_add(1));
        });
        col = col.child(menu_row(t!("Сглаживание"), vm.smooth, Some(f)));
        // Частота кадров гостя (syn-gpu): по монитору окна или не больше N — сразу и со следующего запуска.
        let syn_gpu = vm.gpu_on() || state::display_now(&vm) == model::DisplayModel::SynGpu;
        if syn_gpu {
            let cur: u32 = vm.gpu_arg("--max-fps").and_then(|v| v.parse().ok()).unwrap_or(0);
            col = col.child(menu_head(t!("Частота кадров")));
            let monitor = match state::monitor_refresh(&id) {
                Some(mhz) => t!("По монитору ({hz} Гц)", hz = (mhz + 500) / 1000),
                None => t!("По монитору"),
            };
            let mut items: Vec<u32> = FPS_LIMITS.to_vec();
            if cur > 0 && !items.contains(&cur) {
                items.push(cur);
                items.sort_unstable();
            }
            for n in std::iter::once(0).chain(items) {
                let id2 = id.clone();
                let f: Box<dyn Fn() + Send + Sync> = Box::new(move || state::set_max_fps(st, &id2, n));
                let label = if n == 0 { monitor.clone() } else { t!("{n} кадров/с", n = n) };
                col = col.child(menu_row(label, cur == n, Some(f)));
            }
        }
        vec![boxed(DecoratedBox::new().child(row.child(col.class("menu-col"))).class("menu modes-menu"))]
    }))
}

/// Ограничения частоты кадров в списке панели.
const FPS_LIMITS: [u32; 5] = [30, 60, 75, 120, 144];

fn paused_veil(st: St, id: &str) -> W {
    let id = id.to_string();
    boxed(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(14.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::PAUSE).class("veil-icon"))
                    .child(Text::new(t!("Машина на паузе")).class("veil-title"))
                    .child(btn(icons::PLAY, t!("Продолжить"), "primary", move || state::toggle_pause(st, &id))),
            )
            .class("veil"),
    )
}

/// Кадров рабочего стола syn-gpu ещё нет. Первые секунды — просто ожидание (прошивка, GOP и
/// VGA тоже идут через syn-gpu); дальше — что случилось и что сделать.
fn no_picture(st: St, id: &str) -> W {
    const GRACE: std::time::Duration = std::time::Duration::from_secs(8);
    let wait = state::desktop_wait(id);
    if wait.is_none_or(|w| w < GRACE) && crate::present::server_up(id) {
        return waiting(t!("Ждём изображение с видеокарты syn-gpu…"));
    }
    let (title, text) = if !crate::present::server_up(id) {
        (t!("Видеокарта syn-gpu не работает"), t!("Сервер видеокарты syn-gpu этой машины не запущен (журнал — syn-gpu.log в папке машины). Перезапустите машину."))
    } else {
        let secs = wait.map(|w| w.as_secs()).unwrap_or(0);
        (
            t!("Гость не выводит изображение на syn-gpu ({s} с)", s = secs),
            t!("Обычно картинка идёт через syn-gpu сразу — прошивка, затем базовый видеоадаптер гостя, затем наш драйвер. Если экрана нет: гость завис или выключил монитор — перезапустите машину; если гость на связи — поставьте драйвер syn-gpu."),
        )
    };
    let (a, b) = (id.to_string(), id.to_string());
    let busy = st.busy.get_untracked().get(id).cloned();
    let agent = st.guest.get_untracked().is_some() && crate::present::server_up(id);
    let mut acts = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(b) = busy {
        acts = acts.child(CircularProgress::new().indeterminate().size(20.0).stroke_width(2.5)).child(Text::new(b).class("muted"));
    } else {
        acts = acts.child(btn(icons::RESTART, t!("Перезапустить машину"), "primary", move || state::restart(st, &a)));
        if agent {
            acts = acts.child(btn(icons::SYNC, t!("Поставить драйвер syn-gpu"), "soft", move || state::install_gpu_driver(st, &b)));
        }
    }
    let col = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icons::MONITOR).class("veil-icon"))
        .child(Text::new(title).class("veil-title"))
        .child(Text::new(text).max_lines(5).class("veil-sub nopic-text"))
        .child(acts);
    boxed(DecoratedBox::new().child(Center::new().child(col)).class("screen-wrap"))
}

/// Полоса над страницей машины: настройки ждут перезапуска, в госте нет драйвера syn-gpu.
pub fn notices(st: St, id: String) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let vms = st.vms.get();
        let on = st.live.get().get(&id).is_some_and(|l| l.power.on());
        let busy = st.busy.get().get(&id).cloned();
        let presence = st.gpu_presence.get();
        let Some(vm) = vms.iter().find(|v| v.id == id) else { return vec![] };
        if !on {
            return vec![];
        }
        let mut out: Vec<W> = Vec::new();
        let bar = |icon: &str, cls: &str, title: String, text: String, action: W| -> W {
            boxed(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(icon).class("banner-icon"))
                            .child(Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("row-title")).child(Text::new(text).max_lines(3).class("row-hint")).class("grow"))
                            .child(action),
                    )
                    .class(format!("banner {cls} notice")),
            )
        };
        let progress = |label: &str| -> W {
            boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(18.0).stroke_width(2.5)).child(Text::new(label.to_string()).max_lines(1).class("muted")))
        };
        let pending = state::pending_restart(vm);
        let launched = state::running_vm(vm);
        if !pending.is_empty() {
            let id2 = id.clone();
            out.push(bar(
                icons::INFO,
                "info",
                t!("Применится после перезапуска машины"),
                pending.join("; "),
                match &busy {
                    Some(b) => progress(b),
                    None => btn(icons::RESTART, t!("Перезапустить сейчас"), "primary", move || state::restart(st, &id2)),
                },
            ));
        }
        // Драйвер syn-gpu: устройство в госте есть, нашего драйвера на нём нет.
        let need = presence.as_ref().is_some_and(|(pid, p)| *pid == id && p.needs_driver());
        if need && launched.gpu_on() && vm.gpu_on() && crate::present::server_up(&id) {
            let id2 = id.clone();
            let text = if launched.os.is_windows() {
                t!("Сейчас экран идёт через «Базовый видеоадаптер (Microsoft)»: без 3D, разрешение под окно не меняется. Установка — через гостевой агент, может понадобиться перезагрузка гостя.")
            } else {
                t!("Без драйвера нет 3D и разрешение под окно не меняется. Установка — через гостевой агент (DKMS: нужны gcc, make, dkms и заголовки ядра).")
            };
            out.push(bar(
                icons::WARNING,
                "warn",
                t!("В госте нет драйвера syn-gpu"),
                text,
                match &busy {
                    Some(b) => progress(b),
                    None => btn(icons::SYNC, t!("Поставить драйвер syn-gpu"), "primary", move || state::install_gpu_driver(st, &id2)),
                },
            ));
        }
        if out.is_empty() {
            return vec![];
        }
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for w in out {
            col = col.child(w);
        }
        vec![boxed(col.class("notices"))]
    }))
}

fn waiting(text: String) -> W {
    boxed(
        Column::new()
            .gap(14.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(CircularProgress::new().indeterminate().size(36.0).stroke_width(3.5))
            .child(Text::new(text).class("muted"))
            .class("screen-wrap"),
    )
}

/// Машина выключена: последний кадр под вуалью и большая кнопка.
fn off(st: St, vm: &crate::model::Vm, busy: Option<String>) -> W {
    let id = vm.id.clone();
    let mut stack = Stack::new().fit(StackFit::Expand);
    if let Some(f) = state::thumb(vm) {
        stack = stack.child(DecoratedBox::new().child(LiveView::new(f).class("screen")).style("opacity", StyleValue::Number(0.35)));
    }
    let action: W = match busy {
        Some(b) => boxed(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(CircularProgress::new().indeterminate().size(40.0).stroke_width(3.5))
                .child(Text::new(b).class("veil-title")),
        ),
        None => boxed(
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    GestureDetector::new()
                        .cursor(CursorIcon::Pointer)
                        .on_click(move || state::start(st, &id))
                        .child(DecoratedBox::new().child(Icon::new(icons::PLAY).class("big-play-icon")).class("big-play")),
                )
                .child(Text::new(t!("Машина выключена")).class("veil-title"))
                .child(Text::new(if vm.cdrom.is_empty() { t!("Нажмите, чтобы запустить") } else { t!("В приводе: {f}", f = parts::file_name(&vm.cdrom)) }).max_lines(2).class("veil-sub")),
        ),
    };
    stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(action));
    boxed(DecoratedBox::new().child(stack).class("screen-wrap"))
}
