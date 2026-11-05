//! Снимок интерфейса без окна: дерево syngui раскладывается и рисуется в
//! текстуру wgpu, текстура читается в PNG. Нужен для проверки вёрстки в CI
//! и без графического сеанса (`--screenshot out.png --page panels`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use syngui::gpu::{GpuShared, Renderer};
use syngui::mss::{cascade, parse_stylesheet_str, StyleEngine};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::TextMeasure;

use crate::{app, state};

pub fn screenshot(out: &str, page: &str, size: (u32, u32), scale: f64) -> anyhow::Result<()> {
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();
    let ctx = state::init(page);

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
        label: Some("settings screenshot"),
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

    let sheet = parse_stylesheet_str(&ctx.theme.get_untracked()).map_err(|e| anyhow!("MSS: {e:?}"))?;
    let engine = StyleEngine::new(sheet);

    provide_context(ctx);
    // Размер окна известен заранее — раскладка (узкая/широкая) по нему.
    syngui::viewport::viewport_size().set(Size::new(lw as f32, lh as f32));
    if std::env::var_os("SYNSETTINGS_SHOT_OPEN").is_some() {
        ctx.page_open.set(true);
    }
    if let Some(p) = std::env::var_os("SYNSETTINGS_SHOT_WALL_EDIT") {
        // SYNSETTINGS_SHOT_WALL_DESK=N — участок стола N (с 1) в панораме.
        let desk = std::env::var("SYNSETTINGS_SHOT_WALL_DESK").ok().and_then(|v| v.parse::<u32>().ok()).and_then(|v| v.checked_sub(1));
        crate::pages::open_wallpaper_editor(p.into(), desk);
    }
    let widget = app::root(ctx);
    let element = widget.create_element();
    let root = tree.insert_with_type_id(element, None, widget.as_any().type_id());
    widget.mount(&mut tree, root);
    tree.set_root(root);

    let logical = Size::new(lw as f32, lh as f32);
    let mut list = DisplayList::new();
    // Несколько кадров: эффекты, пересборки, загрузка картинок.
    for frame in 0..20 {
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
        if frame >= 3 {
            let loading = tree
                .image_store
                .as_ref()
                .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).has_loading())
                .unwrap_or(false);
            if !loading {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(120));
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
    renderer.render_to_view(&gpu, &view, (pw, ph), &list, bg);
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
