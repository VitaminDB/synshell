//! Поверхности по выводам: обои и панели. Эффект пересобирает их при
//! смене выводов или перечитанном конфиге.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, OutputInfo, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::ui::InputArea;

thread_local! {
    /// (поверхность, ключ панели, таймер слайд-шоу).
    static SURFACES: RefCell<Vec<(SurfaceId, Option<u64>, Option<u64>)>> = const { RefCell::new(Vec::new()) };
    /// С чем поверхности построены в прошлый раз.
    static BUILT: RefCell<Option<(Vec<OutputInfo>, u64, Option<String>)>> = const { RefCell::new(None) };
}

/// Имя вывода с фокусом (из композитора), иначе основной.
pub fn focused_output(ctx: &ShellCtx) -> Option<String> {
    ctx.comp_outputs.get_untracked().iter().find(|o| o.focused).map(|o| o.name.clone()).or_else(|| primary_output(ctx))
}

/// Основной вывод: флаг композитора, `primary` в конфиге, первый.
pub fn primary_output(ctx: &ShellCtx) -> Option<String> {
    let outs = syngui_layer::outputs().get_untracked();
    if let Some(o) = ctx.comp_outputs.get_untracked().iter().find(|o| o.primary) {
        if outs.iter().any(|x| x.name == o.name) {
            return Some(o.name.clone());
        }
    }
    let cfg = ctx.cfg();
    if let Some(o) = cfg.outputs.iter().find(|o| o.primary) {
        if outs.iter().any(|x| x.name == o.name) {
            return Some(o.name.clone());
        }
    }
    outs.first().map(|o| o.name.clone())
}

/// Логический размер вывода.
pub fn output_size(name: Option<&str>) -> (f32, f32) {
    let outs = syngui_layer::outputs().get_untracked();
    let o = name.and_then(|n| outs.iter().find(|o| o.name == n)).or_else(|| outs.first());
    o.map(|o| (o.size.0 as f32, o.size.1 as f32)).unwrap_or((1920.0, 1080.0))
}

/// Обои и панели по мониторам (рабочий стол).
pub fn install(ctx: ShellCtx) {
    install_with(ctx, true);
}

/// Панели и доки по мониторам; `wallpaper` — ещё и поверхности обоев
/// (телефонная оболочка рисует обои в домашнем экране сама). Панели
/// берутся только те, что показываются на форм-факторе оболочки
/// (`[[panel]] form_factor`).
pub fn install_with(ctx: ShellCtx, with_wallpaper: bool) {
    create_effect(move || {
        let outputs = syngui_layer::outputs().get();
        let _gen = ctx.generation.get();
        let _ = ctx.comp_outputs.get();
        let primary = primary_output(&ctx);
        // Композитор шлёт выводы и при смене фокуса — пересобирать только
        // при настоящих изменениях.
        let key = (outputs.clone(), _gen, primary.clone());
        if BUILT.with(|b| b.borrow().as_ref() == Some(&key)) {
            return;
        }
        BUILT.with(|b| *b.borrow_mut() = Some(key));
        let cfg = ctx.cfg();
        // Всё заново: смена выводов и конфига — редкие события.
        SURFACES.with(|s| {
            for (id, key, timer) in s.borrow_mut().drain(..) {
                syngui_layer::close_surface(id);
                if let Some(k) = key {
                    crate::panel::forget(k);
                }
                if let Some(t) = timer {
                    syngui_layer::cancel_timer(t);
                }
            }
        });
        let mut made = Vec::new();
        if with_wallpaper {
            for out in &outputs {
                let (id, timer) = wallpaper(ctx, out);
                made.push((id, None, timer));
            }
        }
        for (index, panel) in cfg.panels.iter().enumerate().filter(|(_, p)| p.shows_on(ctx.form_factor)) {
            for out in outputs.iter().filter(|o| match panel.output.as_str() {
                "*" | "all" | "" => true,
                "primary" => primary.as_deref() == Some(o.name.as_str()),
                name => o.name == name || o.description.contains(name),
            }) {
                let (id, key) = crate::panel::create(ctx, index, panel, out);
                made.push((id, Some(key), None));
            }
        }
        SURFACES.with(|s| *s.borrow_mut() = made);
    });
}

// ─── Обои ────────────────────────────────────────────────────────────────────

use synshell_common::wallpaper::{is_image, pick};

fn wallpaper(ctx: ShellCtx, out: &OutputInfo) -> (SurfaceId, Option<u64>) {
    let cfg = ctx.cfg();
    let w = cfg.wallpaper.clone();
    let out_name = out.name.clone();
    let slide = use_signal(0u64);
    let mut timer = None;
    if w.slideshow_minutes > 0 {
        timer = Some(syngui_layer::add_timer(Duration::from_secs(w.slideshow_minutes as u64 * 60), move || {
            slide.set(slide.get_untracked() + 1);
            Some(Duration::from_secs(w.slideshow_minutes as u64 * 60))
        }));
    }
    let icons = cfg.wallpaper.desktop_icons;
    let id = syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "syndesktop-wallpaper".into(),
            layer: Layer::Background,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            margin: [0; 4],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            output: Some(out.name.clone()),
            auto_size: false,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        move || {
            let menu_output = out_name.clone();
            let mut stack = Stack::new().fit(StackFit::Expand).child(wallpaper_view(out_name.clone(), slide, None));
            if icons {
                stack = stack.child(desktop_icons());
            }
            // Клик по рабочему столу закрывает открытые окна оболочки, правый —
            // меню рабочего стола.
            Box::new(InputArea::new(stack).on_press(move |b, p, _| {
                let ctx = ShellCtx::get();
                if b == MouseButton::Right {
                    crate::edit::open_desktop_menu(&ctx, Some(menu_output.clone()), p);
                } else {
                    ctx.close_popup();
                }
            }))
        },
    );
    (id, timer)
}

thread_local! {
    /// Размеры картинок обоев (по заголовку файла) — кадр считается каждый
    /// кадр листания, файл читать каждый раз незачем.
    static IMAGE_SIZES: RefCell<std::collections::HashMap<PathBuf, Option<(f32, f32)>>> = RefCell::new(Default::default());
}

fn image_size(p: &Path) -> Option<(f32, f32)> {
    IMAGE_SIZES.with(|m| {
        *m.borrow_mut()
            .entry(p.to_path_buf())
            .or_insert_with(|| syngui::gpu::image_file_size(&p.to_string_lossy()).map(|(w, h)| (w as f32, h as f32)))
    })
}

/// На каком столе вывод: дробное положение листания (`pos`, телефон),
/// иначе активный стол из композитора.
fn workspace_pos(pos: Option<RwSignal<f32>>) -> f32 {
    match pos {
        Some(p) => p.get(),
        None => ShellCtx::get().workspaces.get().iter().find(|w| w.active).map(|w| w.index as f32).unwrap_or(0.0),
    }
}

/// Обои вывода `out_name`: цвет, картинка из живого конфига (смена обоев
/// растворяет старую картинку в новую), слайд-шоу по сигналу `slide`.
/// Кадр (`zoom`/`center`) и панорама — при заполнении экрана; `pos` —
/// дробный номер стола от листающей столы карусели (фон едет за пальцем),
/// без него фон доезжает до активного стола анимацией.
pub fn wallpaper_view(out_name: String, slide: RwSignal<u64>, pos: Option<RwSignal<f32>>) -> impl Widget {
    // Готова JPEG-копия HEIC/AVIF — перестроить.
    let converted = use_signal(0u64);
    Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("wallpaper-color")).child(crate::ui::rx(move || {
        let slot = slide.get();
        let _ = converted.get();
        let cfg = ShellCtx::get().config.get();
        let w = &cfg.wallpaper;
        // Свои обои столов: картинка зависит от стола (при листании
        // меняется на середине пути и растворяется).
        let ws = if w.per_workspace() { workspace_pos(pos).round().max(0.0) as u32 } else { 0 };
        let frame = w.frame_for(&out_name, ws);
        let panorama = w.panorama();
        let framed = w.mode == "fill" || panorama;
        let fit = match w.mode.as_str() {
            "fit" => ImageFit::Contain,
            "stretch" => ImageFit::Fill,
            "center" | "tile" => ImageFit::None,
            _ => ImageFit::Cover,
        };
        // HEIC/AVIF — через JPEG-копию; пока её нет — цвет фона.
        let picked = pick(&frame.path, slot).and_then(|p| {
            let shown = synshell_common::wallpaper::displayable(&p);
            if shown.is_none() {
                synshell_common::wallpaper::prepare(&p, move || {
                    syngui::async_runtime::run_on_main_thread(move || converted.set(converted.get_untracked() + 1));
                });
            }
            shown
        });
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            picked.hash(&mut h);
            w.mode.hash(&mut h);
            framed.hash(&mut h);
            h.finish()
        };
        let dur = cfg.animations.theme_ms();
        // Доводка кадра до нового стола — в такт переключению столов в
        // композиторе; за пальцем — без задержки.
        let follow_ms = if pos.is_some() || cfg.animations.workspace_switch == "none" { 0 } else { cfg.animations.ms(260) };
        let pano = panorama.then_some((cfg.workspaces.count.max(1), w.panorama_shift, w.panorama_desks(cfg.workspaces.count.max(1))));
        // Кадр и участки панорамы меняются без перехода — версия, не ключ.
        let version = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?} {:?} {:?}", frame.zoom, frame.center, pano).hash(&mut h);
            h.finish()
        };
        let out = out_name.clone();
        Box::new(
            AnimatedSwitcher::new(key, move || match &picked {
                Some(p) if framed => Box::new(framed_image(p.clone(), out.clone(), frame.zoom, frame.center, pano.clone(), pos, follow_ms)),
                Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(fit).placeholder(false).class("wallpaper-image")),
                None => Box::new(DecoratedBox::new()),
            })
            .version(version)
            .exit_fade(false)
            .duration_ms(dur.max(1) * 2)
            .exit_duration_ms(dur.max(1) * 2)
            .easing(syngui::animation::Easing::EaseInOutSine)
            .animate_size(false)
            .directional(false),
        )
    }))
}

/// Картинка с кадром: область под пропорции вывода, в панораме — окно
/// стола на общем холсте.
#[allow(clippy::type_complexity)]
fn framed_image(path: PathBuf, out: String, zoom: f32, center: [f32; 2], pano: Option<(u32, f32, Vec<Option<(f32, [f32; 2])>>)>, pos: Option<RwSignal<f32>>, follow_ms: u32) -> impl Widget {
    let size = image_size(&path);
    crate::ui::rx(move || {
        let img = Image::new(path.to_string_lossy()).placeholder(false);
        let Some(size) = size else {
            return Box::new(img.fit(ImageFit::Cover).class("wallpaper-image"));
        };
        let view = output_size(Some(&out));
        let desks = pano.as_ref().map(|p| p.2.clone()).unwrap_or_default();
        let pano = pano.as_ref().map(|(n, shift, _)| (*n, *shift, workspace_pos(pos)));
        let [x, y, w, h] = synshell_common::wallpaper::screen_uv_desks(size, view, zoom, center, pano, &desks);
        Box::new(img.fit(ImageFit::Fill).crop_uv(x, y, w, h).crop_transition_ms(follow_ms).class("wallpaper-image"))
    })
}

// ─── Значки рабочего стола ───────────────────────────────────────────────────

fn desktop_dir() -> PathBuf {
    // XDG_DESKTOP_DIR из user-dirs.dirs, иначе ~/Desktop.
    let home = synshell_common::paths::expand_tilde("~");
    let conf = home.join(".config/user-dirs.dirs");
    if let Ok(s) = std::fs::read_to_string(conf) {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("XDG_DESKTOP_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
                return PathBuf::from(v);
            }
        }
    }
    home.join("Desktop")
}

fn desktop_icons() -> impl Widget {
    let dir = desktop_dir();
    let mut items: Vec<(String, Option<PathBuf>, String)> = Vec::new(); // (подпись, значок, команда)
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
        entries.sort();
        for p in entries {
            if p.extension().is_some_and(|e| e == "desktop") {
                if let Some(e) = crate::xdg::parse_desktop_file(&p, String::new(), &[]) {
                    let cmd = e.command();
                    items.push((e.name.clone(), crate::xdg::lookup_icon(&e.icon), cmd));
                    continue;
                }
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let icon = if p.is_dir() { "folder" } else if is_image(&p) { "image-x-generic" } else { "text-x-generic" };
            items.push((name, crate::xdg::lookup_icon(icon), format!("xdg-open '{}'", p.to_string_lossy().replace('\'', "'\\''"))));
        }
    }
    let mut col = Flex::new().direction(FlexDirection::Column).wrap().gap(6.0);
    for (name, icon, cmd) in items {
        let mut c = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(p) = icon {
            c = c.child(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("desk-icon"));
        }
        c = c.child(Text::new(name).max_lines(2).class("desk-label"));
        col = col.child(InputArea::new(DecoratedBox::new().child(c).class("desk-item")).pointer().on_click(move |b, _, _| {
            if b == MouseButton::Left {
                crate::actions::spawn(&cmd);
            }
        }));
    }
    DecoratedBox::new().child(col).class("desk-icons")
}
