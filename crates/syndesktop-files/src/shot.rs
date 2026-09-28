//! Снимок интерфейса без окна: дерево syngui раскладывается и рисуется в
//! текстуру wgpu, текстура читается в PNG — проверка вёрстки без
//! графического сеанса (`--screenshot out.png [--size 1280x800] [путь]`).
//! `--script` — шаги перед снимком (клавиши, щелчки) для проверки поведения.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use syngui::gpu::{GpuShared, Renderer};
use syngui::mss::{cascade, parse_stylesheet_str, StyleEngine};
use syngui::prelude::*;
use syngui::IntoWidget;
use syngui::render::DisplayList;
use syngui::widget::context::TextMeasure;

use crate::state;

pub fn screenshot(
    out: &str,
    size: (u32, u32),
    scale: f64,
    init: impl FnOnce() -> state::Ctx,
    script: &[String],
) -> anyhow::Result<()> {
    let init = move || {
        let ctx = init();
        provide_context(ctx);
        ctx.theme
    };
    screenshot_with(out, size, scale, init, crate::ui::app::root, state::busy, script)
}

/// Снимок любого корня: `init` создаёт состояние и отдаёт сигнал темы,
/// `busy` — идёт ли ещё загрузка (шаги сценария ждут её конца).
pub fn screenshot_with<R: IntoWidget<M>, M>(
    out: &str,
    size: (u32, u32),
    scale: f64,
    init: impl FnOnce() -> RwSignal<String>,
    root_fn: impl FnOnce() -> R,
    busy: fn() -> bool,
    script: &[String],
) -> anyhow::Result<()> {
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();
    let theme = init();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .context("нет GPU-адаптера")?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("files screenshot"),
        ..Default::default()
    }))
    .context("не создать устройство")?;
    let gpu = GpuShared { instance, adapter, device, queue };

    let (lw, lh) = size;
    let pw = (lw as f64 * scale).round() as u32;
    let ph = (lh as f64 * scale).round() as u32;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&gpu, format, pw, ph, lw, lh, None);
    {
        let mut atlas = renderer.font_atlas.lock().unwrap_or_else(|e| e.into_inner());
        atlas.set_icon_font_data(syngui::text::icon_fonts::material::FONT_DATA.to_vec());
        atlas.set_scale_factor(scale as f32);
    }

    let mut tree = ElementTree::new();
    tree.text_measure = Some(renderer.font_atlas.clone() as Arc<dyn TextMeasure>);
    tree.image_store = Some(renderer.image_store.clone());

    let sheet = parse_stylesheet_str(&theme.get_untracked()).map_err(|e| anyhow!("MSS: {e:?}"))?;
    let engine = StyleEngine::new(sheet);

    let widget: Box<dyn Widget> = root_fn().into_widget();
    let element = widget.create_element();
    let root = tree.insert_with_type_id(element, None, widget.as_any().type_id());
    widget.mount(&mut tree, root);
    tree.set_root(root);

    let logical = Size::new(lw as f32, lh as f32);
    let mut list = DisplayList::new();
    // Несколько кадров: эффекты, пересборки, загрузка картинок.
    let mut script: std::collections::VecDeque<String> = script.iter().cloned().collect();
    let mut idle = 0;
    for frame in 0..400 {
        // Фоновые декодеры картинок отдают результат через poll_bg —
        // в приложении его зовёт рендер каждого кадра.
        if let Some(store) = tree.image_store.as_ref() {
            store.lock().unwrap_or_else(|e| e.into_inner()).poll_bg();
        }
        for _ in 0..8 {
            if !tree.rebuild_if_needed(root) {
                break;
            }
            cascade::apply_styles_dirty(&mut tree, &engine);
        }
        cascade::apply_styles_dirty(&mut tree, &engine);
        syngui::signal::drain_and_run_effects();
        syngui::async_runtime::drain_main_thread_callbacks();
        tree.viewport_size = logical;
        tree.set_pixel_snap_scale(0.0);
        let constraints = Constraints::new(0.0, lw as f32, 0.0, lh as f32);
        tree.layout(root, constraints);
        if tree.rebuild_if_needed(root) {
            cascade::apply_styles_dirty(&mut tree, &engine);
            tree.layout(root, constraints);
        }
        tree.animate(root, Duration::from_millis(400));
        list = DisplayList::new();
        list.set_surface_size(logical);
        list.set_scale_factor(scale as f32);
        tree.build_display_list(root, &mut list, Rect::new(Point::zero(), logical));
        // Шаг сценария — после того, как папка загрузилась.
        let busy = busy();
        if frame >= 2 && !busy {
            if let Some(step) = script.pop_front() {
                run_step(&mut tree, root, &step);
                idle = 0;
                std::thread::sleep(Duration::from_millis(60));
                continue;
            }
        }
        if frame >= 3 && script.is_empty() {
            let loading = tree
                .image_store
                .as_ref()
                .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).has_loading())
                .unwrap_or(false);
            if !loading && !busy {
                idle += 1;
                if idle >= 3 {
                    break;
                }
            } else {
                idle = 0;
            }
        }
        std::thread::sleep(Duration::from_millis(60));
    }

    let bg = root_background(&tree, root).unwrap_or(Color::from_srgb(0x16, 0x18, 0x1d, 1.0));
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    // Первый проход грузит картинки в GPU-кэш, второй рисует их.
    // Рендер загружает в GPU ограниченное число картинок за кадр — рисуем,
    // пока очередь загрузки не опустеет, иначе часть значков выйдет пустой.
    for _ in 0..40 {
        renderer.render_to_view(&gpu, &view, (pw, ph), &list, bg);
        let pending = tree
            .image_store
            .as_ref()
            .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).has_pending_uploads())
            .unwrap_or(false);
        if !pending {
            break;
        }
    }
    renderer.render_to_view(&gpu, &view, (pw, ph), &list, bg);

    let row = (pw * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (row * ph) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(ph) },
        },
        wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((pw * ph * 4) as usize);
    for y in 0..ph {
        let start = (y * row) as usize;
        rgba.extend_from_slice(&data[start..start + (pw * 4) as usize]);
    }
    drop(data);
    buf.unmap();

    let file = std::fs::File::create(out).with_context(|| out.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    println!("{out}: {pw}x{ph}");
    Ok(())
}

fn root_background(tree: &ElementTree, root: ElementId) -> Option<Color> {
    tree.get(root)?.mss()?.background_color
}

/// Шаг сценария: `key:ctrl+a`, `click:x,y`, `rclick:x,y`, `dclick:x,y`,
/// `move:x,y`, `drag:x1,y1,x2,y2`, `type:текст`, `wait:мс`.
fn run_step(tree: &mut ElementTree, root: ElementId, step: &str) {
    use syngui::input::{Event, Modifiers, MouseButton};
    let (cmd, arg) = step.split_once(':').unwrap_or((step, ""));
    let pt = |s: &str| -> Vec<f32> { s.split(',').filter_map(|v| v.trim().parse().ok()).collect() };
    fn send(tree: &mut ElementTree, root: ElementId, e: Event) {
        tree.handle_event(root, &e);
        syngui::signal::drain_and_run_effects();
        syngui::async_runtime::drain_main_thread_callbacks();
    }
    match cmd {
        "key" => {
            let mut m = Modifiers::default();
            let mut key = None;
            for part in arg.split('+') {
                match part.to_lowercase().as_str() {
                    "ctrl" => m.ctrl = true,
                    "shift" => m.shift = true,
                    "alt" => m.alt = true,
                    k => key = parse_key(k),
                }
            }
            tree.modifiers = m;
            if let Some(k) = key {
                send(tree, root, Event::KeyDown(k));
                send(tree, root, Event::KeyUp(k));
            }
            tree.modifiers = Modifiers::default();
        }
        "click" | "rclick" | "dclick" | "mclick" => {
            let v = pt(arg);
            if v.len() < 2 {
                return;
            }
            let p = Point::new(v[0], v[1]);
            let b = match cmd {
                "rclick" => MouseButton::Right,
                "mclick" => MouseButton::Middle,
                _ => MouseButton::Left,
            };
            send(tree, root, Event::MouseMove(p));
            send(tree, root, Event::MouseDown { button: b, position: p });
            send(tree, root, Event::MouseUp { button: b, position: p });
            if cmd == "dclick" {
                send(tree, root, Event::DoubleClick { button: b, position: p });
                send(tree, root, Event::MouseUp { button: b, position: p });
            }
        }
        "move" => {
            let v = pt(arg);
            if v.len() >= 2 {
                send(tree, root, Event::MouseMove(Point::new(v[0], v[1])));
            }
        }
        "drag" => {
            let v = pt(arg);
            if v.len() < 4 {
                return;
            }
            let (a, b) = (Point::new(v[0], v[1]), Point::new(v[2], v[3]));
            send(tree, root, Event::MouseMove(a));
            send(tree, root, Event::MouseDown { button: MouseButton::Left, position: a });
            for i in 1..=8 {
                let t = i as f32 / 8.0;
                send(tree, root, Event::MouseMove(Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)));
            }
        }
        "release" => {
            let v = pt(arg);
            if v.len() >= 2 {
                send(tree, root, Event::MouseUp { button: MouseButton::Left, position: Point::new(v[0], v[1]) });
            }
        }
        "type" => {
            for c in arg.chars() {
                send(tree, root, Event::CharInput(c));
            }
        }
        "wait" => std::thread::sleep(Duration::from_millis(arg.parse().unwrap_or(200))),
        _ => eprintln!("неизвестный шаг сценария: {step}"),
    }
}

fn parse_key(k: &str) -> Option<syngui::input::Key> {
    use syngui::input::Key;
    Some(match k {
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "menu" => Key::ContextMenu,
        s if s.len() >= 2 && s.starts_with('f') && s[1..].parse::<u8>().is_ok() => match s[1..].parse::<u8>().unwrap() {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            9 => Key::F9,
            11 => Key::F11,
            _ => return None,
        },
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            match c {
                'a'..='z' => [
                    Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M,
                    Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
                ][(c as u8 - b'a') as usize],
                '0'..='9' => [
                    Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9,
                ][(c as u8 - b'0') as usize],
                _ => return None,
            }
        }
        _ => return None,
    })
}
