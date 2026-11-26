use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use synshell_common::screenrec::{self, Status};
use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::prelude::*;

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, rx, InputArea};

pub mod gl {
    pub const RECORD: &str = "\u{E061}";
    pub const STOP: &str = "\u{E047}";
    pub const VIDEOCAM: &str = "\u{E04B}";
    pub const SCREEN: &str = "\u{E30C}";
    pub const CROP: &str = "\u{E3BE}";
    pub const WINDOW: &str = "\u{F088}";
    pub const VOLUME: &str = "\u{E050}";
    pub const MIC: &str = "\u{E029}";
    pub const MOUSE: &str = "\u{E323}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const SPEED: &str = "\u{E9E4}";
}

thread_local! {
    static STATUS: RefCell<Option<RwSignal<Option<Status>>>> = const { RefCell::new(None) };
}

fn sig() -> RwSignal<Option<Status>> {
    STATUS.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(None)))
}

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn start() {
    let _ = sig();
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("shell-screenrec".into())
        .spawn(|| loop {
            let s = screenrec::current();
            let active = s.is_some();
            run_on_main_thread(move || set(s));
            std::thread::sleep(Duration::from_millis(if active { 1000 } else { 2000 }));
        })
        .ok();
}

fn set(s: Option<Status>) {
    let sig = sig();
    if sig.get_untracked() != s {
        sig.set(s);
    }
}

pub fn refresh() {
    std::thread::spawn(|| {
        let s = screenrec::current();
        run_on_main_thread(move || set(s));
    });
}

pub fn active() -> bool {
    sig().get_untracked().is_some()
}

fn synshot(args: &str) {
    crate::actions::spawn(&format!("synshot --record {args}"));
    for ms in [300u64, 900, 2500] {
        syngui_layer::add_timer(Duration::from_millis(ms), || {
            refresh();
            None
        });
    }
}

pub fn toggle() {
    if active() {
        synshot("stop");
    } else {
        synshot("start");
    }
}

pub fn start_mode(mode: &str) {
    remember("mode", toml_edit::Value::from(mode));
    synshot(&format!("start --{mode}"));
}

fn remember(key: &str, v: toml_edit::Value) {
    let key = key.to_string();
    std::thread::spawn(move || {
        if let Err(e) = synshell_common::config_edit::set_value(&["recording", &key], v) {
            log::warn!("[recording] {key}: {e:#}");
        }
        run_on_main_thread(crate::reload_after_write);
    });
}

pub fn command(name: &str, arg: &str) -> bool {
    let ctx = ShellCtx::get();
    match name {
        "record" | "screenrec" => match arg.trim() {
            "" | "toggle" => toggle(),
            "start" => synshot("start"),
            "stop" => synshot("stop"),
            "screen" | "area" | "window" => start_mode(arg.trim()),
            "menu" => ctx.open_popup(PopupKind::Record, crate::panel::applet_anchor("record").unwrap_or_else(crate::commands::centered)),
            other => log::warn!("record: неизвестно {other:?}"),
        },
        "screenrec-started" => refresh(),
        "screenrec-saved" => {
            refresh();
            let path = arg.trim().to_string();
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
            let (open, folder) = (t!("Открыть"), t!("Показать в папке"));
            crate::notifications::local_actions(
                ctx,
                &format!("screenrec:{path}"),
                &t!("Запись экрана сохранена"),
                &name,
                "video-x-generic",
                &[("default", open.as_str()), ("open", open.as_str()), ("folder", folder.as_str())],
                move |a| {
                    let p = std::path::Path::new(&path);
                    match a {
                        "folder" => crate::launchers::open_path(p.parent().unwrap_or(p)),
                        _ => crate::launchers::open_path(p),
                    }
                },
            );
        }
        "screenrec-failed" => {
            refresh();
            crate::notifications::local(ctx, &t!("Запись экрана не удалась"), arg.trim(), None);
        }
        _ => return false,
    }
    true
}

pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

pub fn applet(pc: &crate::panel::PanelCtx, explicit: bool) -> Box<dyn Widget> {
    start();
    let ctx = ShellCtx::get();
    let pc = pc.clone();
    let vertical = pc.vertical;
    let s = sig();
    Box::new(rx(move || {
        let st = s.get();
        let wanted = explicit || st.is_some() || ctx.config.get().recording.panel_button;
        if !wanted {
            return Box::new(DecoratedBox::new()) as Box<dyn Widget>;
        }
        let content: Box<dyn Widget> = match &st {
            Some(st) => {
                let started = st.started_ms;
                let saving = st.state == "saving";
                let mut row = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(if saving { "record-dot record-dot-saving" } else { "record-dot" }));
                if !vertical {
                    row = row.child(rx(move || {
                        let _ = ctx.now.get();
                        let secs = if started == 0 { 0.0 } else { screenrec::now_ms().saturating_sub(started) as f64 / 1000.0 };
                        Box::new(Text::new(clock(secs)).class("applet-label applet-mono record-time")) as Box<dyn Widget>
                    }));
                }
                Box::new(row)
            }
            None => Box::new(icon(gl::RECORD).class("record-idle")),
        };
        let pc = pc.clone();
        let class = if st.is_some() { "applet applet-record record-on" } else { "applet applet-record" };
        Box::new(
            InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(content)).class(class))
                .pointer()
                .buttons(&[MouseButton::Left, MouseButton::Middle, MouseButton::Right])
                .on_click(move |b, _, r| match b {
                    MouseButton::Left => toggle(),
                    MouseButton::Right | MouseButton::Middle => ShellCtx::get().open_popup(PopupKind::Record, pc.anchor(r)),
                    _ => {}
                }),
        ) as Box<dyn Widget>
    }))
}

fn chip(label: String, on: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label).class("chip-label")).class(if on { "chip chip-on" } else { "chip" })).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

fn switch_row(glyph: &str, title: String, on: bool, key: &'static str) -> impl Widget {
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(glyph).class("menu-icon"))
        .child(Text::new(title).class("menu-label grow"))
        .child(Toggle::with_state(on).on_change(move |v| remember(key, toml_edit::Value::from(v))))
        .class("record-row")
}

fn action_row(glyph: &str, title: String, hint: String, current: bool, mode: &'static str) -> impl Widget {
    InputArea::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class("menu-icon"))
                    .child(Column::new().gap(1.0).class("grow").child(Text::new(title).class("menu-label")).child(Text::new(hint).max_lines(1).class("net-hint")))
                    .child(if current { Box::new(icon(gl::RECORD).class("record-default")) as Box<dyn Widget> } else { Box::new(DecoratedBox::new()) }),
            )
            .class("menu-item"),
    )
    .pointer()
    .on_click(move |b, _, _| {
        if b == MouseButton::Left {
            ShellCtx::get().close_popup();
            start_mode(mode);
        }
    })
}

pub fn view(ctx: ShellCtx) -> impl Widget {
    start();
    refresh();
    let s = sig();
    Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(rx(move || {
        let st = s.get();
        let c = ctx.config.get().recording.clone();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(gl::VIDEOCAM).class("record-title-icon")).child(Text::new(t!("Запись экрана")).class("popup-title grow")));
        if let Some(st) = st {
            let started = st.started_ms;
            let info = format!("{}×{} · {} {} · {}", st.width, st.height, st.fps, t!("к/с"), st.encoder);
            col = col.child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("record-dot record-dot-big"))
                            .child(
                                Column::new()
                                    .gap(1.0)
                                    .class("grow")
                                    .child(rx(move || {
                                        let _ = ctx.now.get();
                                        let secs = if started == 0 { 0.0 } else { screenrec::now_ms().saturating_sub(started) as f64 / 1000.0 };
                                        Box::new(Text::new(t!("Идёт запись · {time}", time = clock(secs))).class("menu-label")) as Box<dyn Widget>
                                    }))
                                    .child(Text::new(info).max_lines(1).class("net-hint")),
                            )
                            .child(
                                InputArea::new(DecoratedBox::new().child(Text::new(t!("Остановить")).class("link-btn-text")).class("link-btn link-btn-primary record-stop")).pointer().on_click(|b, _, _| {
                                    if b == MouseButton::Left {
                                        ShellCtx::get().close_popup();
                                        synshot("stop");
                                    }
                                }),
                            ),
                    )
                    .class("record-live"),
            );
        } else {
            let mode = c.mode.as_str();
            col = col
                .child(action_row(gl::SCREEN, t!("Весь экран"), t!("Монитор, на котором окно в фокусе"), mode == "screen", "screen"))
                .child(action_row(gl::CROP, t!("Область…"), t!("Выделить рамкой или щёлкнуть по окну"), mode == "area", "area"))
                .child(action_row(gl::WINDOW, t!("Активное окно"), t!("Запись следует за окном"), mode == "window", "window"));
        }
        col = col
            .child(DecoratedBox::new().class("menu-sep"))
            .child(switch_row(gl::VOLUME, t!("Звук системы"), c.audio, "audio"))
            .child(switch_row(gl::MIC, t!("Микрофон"), c.mic, "mic"))
            .child(switch_row(gl::MOUSE, t!("Указатель мыши"), c.cursor, "cursor"))
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(gl::SPEED).class("menu-icon"))
                    .child(Text::new(t!("Частота кадров")).class("menu-label grow"))
                    .child(chip(t!("30 к/с"), c.fps == 30, || remember("fps", toml_edit::Value::from(30i64))))
                    .child(chip(t!("60 к/с"), c.fps != 30, || remember("fps", toml_edit::Value::from(60i64))))
                    .class("record-row"),
            )
            .child(DecoratedBox::new().class("menu-sep"))
            .child(crate::popup::menu_item(gl::FOLDER, t!("Открыть папку записей"), move || {
                let d = screenrec::dir(&ShellCtx::get().cfg().recording);
                let _ = std::fs::create_dir_all(&d);
                crate::launchers::open_path(&d);
            }))
            .child(crate::popup::menu_item(gl::SETTINGS, t!("Параметры записи…"), || crate::actions::spawn("synsettings general")));
        Box::new(col) as Box<dyn Widget>
    }))
}
