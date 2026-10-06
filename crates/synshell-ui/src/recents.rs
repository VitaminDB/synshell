//! «Недавние» (телефон): лента карточек открытых окон поверх всего —
//! жест снизу с задержкой (`shell recents`). Тап по карточке — перейти к
//! окну, свайп карточки вверх — закрыть окно, «Закрыть все» — все окна.
//! Карточки въезжают и съезжаются (`Keyed` + `AnimatedPosition`), закрытая
//! улетает вверх (`Presence`). В карточке — миниатюра окна: при открытии
//! ленты композитор рисует последний кадр каждого окна вне экрана
//! (`Request::Thumbnails`); пока её нет — значок и заголовок.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use synshell_common::ipc::{Client, Request, Response, WindowInfo, WindowOp};
use syngui::prelude::*;
use syngui::widgets::{Motion, PanAxis, Presence, SwipeDirection};
use syngui::containers::Keyed;
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::launchers;
use crate::ui::{icon, mi, rx};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    /// Окна, которые уже смахнули (уходят с анимацией, ждут закрытия).
    static GONE: Cell<Option<RwSignal<Vec<u64>>>> = const { Cell::new(None) };
    /// Миниатюры окон этого открытия ленты.
    static THUMBS: Cell<Option<RwSignal<HashMap<u64, Thumb>>>> = const { Cell::new(None) };
}

/// Область картинки в карточке (логические px, как `.recents-preview`).
const PREVIEW: (f64, f64) = (226.0, 420.0);

#[derive(Clone)]
struct Thumb {
    key: String,
    width: u32,
    height: u32,
    rgba: Arc<Vec<u8>>,
}

impl PartialEq for Thumb {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key
    }
}

fn thumbs() -> RwSignal<HashMap<u64, Thumb>> {
    THUMBS.with(|t| match t.get() {
        Some(s) => s,
        None => {
            let s = use_signal(HashMap::new());
            t.set(Some(s));
            s
        }
    })
}

/// Запросить у композитора миниатюры окон (в фоне: рисование и чтение
/// кадров — десятки мс на окно).
fn load_thumbs(ctx: &ShellCtx) {
    let ids: Vec<u64> = visible_windows(ctx).iter().map(|w| w.id).collect();
    if ids.is_empty() {
        return;
    }
    let out = crate::manager::focused_output(ctx);
    let scale = ctx.comp_outputs.get_untracked().iter().find(|o| Some(&o.name) == out.as_ref()).map(|o| o.scale).unwrap_or(2.0).max(1.0);
    let max = [(PREVIEW.0 * scale).round() as u32, (PREVIEW.1 * scale).round() as u32];
    let sig = thumbs();
    std::thread::spawn(move || {
        let thumbs = match Client::connect().and_then(|mut c| c.request(&Request::Thumbnails { ids, max })) {
            Ok(Response::Thumbnails { thumbs }) => thumbs,
            Ok(other) => {
                log::debug!("миниатюры окон: {other:?}");
                return;
            }
            Err(e) => {
                log::warn!("миниатюры окон: {e}");
                return;
            }
        };
        let mut map = HashMap::new();
        for t in thumbs {
            let data = std::fs::read(&t.path);
            let _ = std::fs::remove_file(&t.path);
            match data {
                Ok(d) if d.len() == (t.width * t.height * 4) as usize => {
                    let key = format!("recents-thumb:{}", std::path::Path::new(&t.path).file_stem().and_then(|s| s.to_str()).unwrap_or_default());
                    map.insert(t.id, Thumb { key, width: t.width, height: t.height, rgba: Arc::new(d) });
                }
                _ => {}
            }
        }
        syngui::async_runtime::run_on_main_thread(move || sig.set(map));
    });
}

pub fn is_open() -> bool {
    SURFACE.with(|s| s.get()).is_some()
}

pub fn toggle() {
    if is_open() {
        close();
    } else {
        open();
    }
}

pub fn close() {
    let ctx = ShellCtx::get();
    match OPEN.with(|o| o.get()) {
        Some(s) if crate::anim::group_ms(&ctx, "pages", 100) > 0 => s.set(false),
        _ => close_now(),
    }
}

fn close_now() {
    if let Some(id) = SURFACE.with(|s| s.take()) {
        syngui_layer::close_surface(id);
    }
    // Кадры окон не держим, пока лента закрыта.
    if let Some(t) = THUMBS.with(|t| t.get()) {
        t.set(HashMap::new());
    }
    OPEN.with(|o| o.set(None));
}

pub(crate) fn visible_windows(ctx: &ShellCtx) -> Vec<WindowInfo> {
    ctx.windows.get().into_iter().filter(|w| !w.skip_taskbar && !w.app_id.is_empty() || !w.title.is_empty()).collect()
}

pub fn open() {
    if is_open() {
        return;
    }
    let ctx = ShellCtx::get();
    ctx.close_popup();
    crate::shade::close();
    // Экранная клавиатура поверх ленты/шторки не нужна.
    crate::actions::spawn("synkeyboard hide");
    let open = use_signal(true);
    OPEN.with(|o| o.set(Some(open)));
    thumbs().set(HashMap::new());
    load_thumbs(&ctx);
    GONE.with(|g| {
        if g.get().is_none() {
            g.set(Some(use_signal(Vec::new())));
        }
    });
    let spec = SurfaceSpec {
        namespace: "syndesktop-recents".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::OnDemand,
        output: crate::manager::focused_output(&ctx),
        auto_size: false,
        clear_color: [0.0; 4],
    };
    let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), open)));
    SURFACE.with(|s| s.set(Some(id)));
}

fn gone() -> RwSignal<Vec<u64>> {
    GONE.with(|g| g.get()).unwrap_or_else(|| use_signal(Vec::new()))
}

fn view(ctx: ShellCtx, open: RwSignal<bool>) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "pages", 260);
    Presence::signal(open, move || {
        let ctx = ShellCtx::get();
        let cards = rx(move || {
            let gone = gone().get();
            let list: Vec<WindowInfo> = visible_windows(&ctx).into_iter().filter(|w| !gone.contains(&w.id)).collect();
            if list.is_empty() {
                return Box::new(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new("Нет открытых приложений").class("recents-empty"))
                        .class("recents-empty-box"),
                ) as Box<dyn Widget>;
            }
            let mut row = Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center);
            for w in list {
                row = row.child(Keyed::new(w.id, 0, move || Box::new(AnimatedPosition::new(card(w.clone())))));
            }
            // Высота ленты — по карточкам: растянутая лента выталкивала
            // «Закрыть все» за нижний край.
            // ScrollView в колонке забирает всю высоту — держим его в блоке
            // высотой с карточку.
            Box::new(DecoratedBox::new().child(ScrollView::new().horizontal().child(row.class("recents-row"))).class("recents-scroll"))
        });
        let clear = GestureDetector::new()
            .on_click(|| {
                let ctx = ShellCtx::get();
                for w in visible_windows(&ctx) {
                    crate::actions::window_op(w.id, WindowOp::Close);
                }
                close();
            })
            .child(
                DecoratedBox::new()
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::CLEAR_ALL).class("recents-clear-icon")).child(Text::new("Закрыть все").class("recents-clear-text")))
                    .class("recents-clear"),
            );
        Box::new(
            GestureDetector::new().on_click(close).child(
                Column::new()
                    .gap(18.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("Недавние").class("recents-title"))
                    .child(cards)
                    .child(clear)
                    .class("recents")
                    // Presence меряет по содержимому — затемнение на весь экран.
                    .style("width", syngui::viewport::viewport_size().get_untracked().width)
                    .style("height", syngui::viewport::viewport_size().get_untracked().height),
            ),
        )
    })
    .enter(Motion::fade().scale(1.06))
    .exit(Motion::fade().scale(1.04))
    .duration_ms(dur)
    .initial(dur > 0)
    .on_exit_complete(close_now)
}

fn card(w: WindowInfo) -> impl Widget {
    let entry = crate::xdg::app_for_window(&w.app_id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| w.app_id.clone());
    let icon_path = entry.as_ref().and_then(|e| crate::xdg::lookup_icon(&e.icon)).or_else(|| crate::xdg::window_icon(&w.app_id));
    let id = w.id;
    let leaving = use_signal(true);
    let dur = crate::anim::group_ms(&ShellCtx::get(), "pages", 240);
    let title = crate::xdg::window_subtitle(entry.as_ref(), &w.title);
    let body = move || -> Box<dyn Widget> {
        Box::new(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(launchers::icon_widget(&icon_path, &None, "recents-app-icon", 28.0))
                                .child(Text::new(name.clone()).max_lines(1).class("recents-app-name")),
                        )
                        .child(preview(id, icon_path.clone(), title.clone())),
                )
                .class("recents-card"),
        )
    };
    let presence = Presence::signal(leaving, body)
        .exit(Motion::fade().slide(0.0, -260.0))
        .duration_ms(dur)
        .initial(false)
        .on_exit_complete(move || {
            crate::actions::window_op(id, WindowOp::Close);
            let g = gone();
            let mut v = g.get_untracked();
            v.push(id);
            g.set(v);
        });
    GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_click(move || {
            crate::actions::window_op(id, WindowOp::Activate);
            close();
        })
        .on_swipe(move |dir, _| {
            if dir == SwipeDirection::Up {
                let g = gone();
                if dur == 0 {
                    crate::actions::window_op(id, WindowOp::Close);
                    let mut v = g.get_untracked();
                    v.push(id);
                    g.set(v);
                } else {
                    leaving.set(false);
                }
            }
        })
        .child(presence)
}

/// Картинка карточки: миниатюра окна с его пропорциями (вписана в
/// [`PREVIEW`]) или, пока её нет, значок и заголовок.
fn preview(id: u64, icon_path: Option<std::path::PathBuf>, title: String) -> impl Widget {
    rx(move || -> Box<dyn Widget> {
        if let Some(t) = thumbs().get().get(&id).cloned() {
            let k = (PREVIEW.0 / t.width as f64).min(PREVIEW.1 / t.height as f64);
            let (w, h) = ((t.width as f64 * k).round(), (t.height as f64 * k).round());
            return Box::new(
                DecoratedBox::new()
                    .child(Image::from_rgba_shared(t.key, t.width, t.height, t.rgba).fit(ImageFit::Cover))
                    .class("recents-thumb")
                    .style("width", w as f32)
                    .style("height", h as f32),
            );
        }
        Box::new(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .gap(10.0)
                        .child(launchers::icon_widget(&icon_path, &None, "recents-big-icon", 72.0))
                        .child(Text::new(title.clone()).max_lines(3).class("recents-window-title")),
                )
                .class("recents-preview"),
        )
    })
}
