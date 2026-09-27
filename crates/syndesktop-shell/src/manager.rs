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

pub fn install(ctx: ShellCtx) {
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
        for out in &outputs {
            let (id, timer) = wallpaper(ctx, out);
            made.push((id, None, timer));
        }
        for (index, panel) in cfg.panels.iter().enumerate() {
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

fn is_image(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "svg")
    )
}

/// Картинка для вывода: файл, либо очередная из каталога (слайд-шоу).
fn pick(path: &str, slot: u64) -> Option<PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    let p = syndesktop_common::paths::expand_tilde(path.trim());
    if p.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&p).ok()?.flatten().map(|e| e.path()).filter(|p| is_image(p)).collect();
        files.sort();
        if files.is_empty() {
            return None;
        }
        let i = (slot as usize) % files.len();
        return files.get(i).cloned();
    }
    p.exists().then_some(p)
}

fn wallpaper(ctx: ShellCtx, out: &OutputInfo) -> (SurfaceId, Option<u64>) {
    let cfg = ctx.cfg();
    let w = cfg.wallpaper.clone();
    let path = w.per_output.get(&out.name).cloned().unwrap_or_else(|| w.path.clone());
    let slide = use_signal(0u64);
    let mut timer = None;
    if w.slideshow_minutes > 0 {
        timer = Some(syngui_layer::add_timer(Duration::from_secs(w.slideshow_minutes as u64 * 60), move || {
            slide.set(slide.get_untracked() + 1);
            Some(Duration::from_secs(w.slideshow_minutes as u64 * 60))
        }));
    }
    let fit = match cfg.wallpaper.mode.as_str() {
        "fit" => ImageFit::Contain,
        "stretch" => ImageFit::Fill,
        "center" | "tile" => ImageFit::None,
        _ => ImageFit::Cover,
    };
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
            let mut stack = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("wallpaper-color"));
            let path = path.clone();
            stack = stack.child(crate::ui::rx(move || {
                let slot = slide.get();
                match pick(&path, slot) {
                    Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(fit).placeholder(false).class("wallpaper-image")),
                    None => Box::new(DecoratedBox::new()),
                }
            }));
            if icons {
                stack = stack.child(desktop_icons());
            }
            // Клик по рабочему столу закрывает открытые окна оболочки.
            Box::new(InputArea::new(stack).on_press(|_, _, _| ShellCtx::get().close_popup()))
        },
    );
    (id, timer)
}

// ─── Значки рабочего стола ───────────────────────────────────────────────────

fn desktop_dir() -> PathBuf {
    // XDG_DESKTOP_DIR из user-dirs.dirs, иначе ~/Desktop.
    let home = syndesktop_common::paths::expand_tilde("~");
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
