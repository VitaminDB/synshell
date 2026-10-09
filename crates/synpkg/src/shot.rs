//! Снимок интерфейса без окна (`synpkg --screenshot out.png [--tab updates]
//! [--meta] [--groups открытая-группа] [--size 1240x820] [--scale 1] [--view cards|list] [--queue имя:действие,…]`): данные читаются
//! сразу (установленные, каталог, обновления), дерево syngui раскладывается и
//! рисуется в текстуру wgpu, текстура — в PNG. Для проверки вёрстки без
//! графического сеанса. Действие очереди: install, aur, remove, upgrade,
//! upgrade-aur; `--meta` — раздел «Метапакеты и группы» обзора или фильтр
//! «Метапакеты» установленных, `--groups` — группы этого раздела;
//! `SYNPKG_SHOT_MENU=имя` — открыть меню пакета (как правым щелчком).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use synshell_common::Config;
use synsystem::packages as pk;
use syngui::gpu::{GpuShared, Renderer};
use syngui::mss::{cascade, parse_stylesheet_str, StyleEngine};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::TextMeasure;

use crate::{arg_value, QAct, QItem, Tab};

pub fn screenshot(out: &str) -> anyhow::Result<()> {
    let tab = match arg_value("--tab").as_deref().unwrap_or("updates") {
        "explore" => Tab::Explore,
        "aur" => Tab::Aur,
        "installed" => Tab::Installed,
        "jobs" => Tab::Jobs,
        _ => Tab::Updates,
    };
    let (lw, lh) = arg_value("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1240u32, 820u32));
    let scale: f64 = arg_value("--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();
    let (cfg, _) = Config::load();
    let mss = crate::theme(&cfg);
    let aur = cfg.packages.aur;
    let st = crate::new_state(cfg, String::new(), use_signal(syngui::window::WindowState::default()));
    st.tab.set(tab);
    if let Some(v) = arg_value("--view") {
        st.cards.set(v != "list");
    }
    st.installed.set(pk::installed());
    crate::set_catalog(st, pk::catalog());
    st.metas.set(Some(pk::metapackages()));
    st.groups.set(pk::groups());
    if matches!(tab, Tab::Updates | Tab::Jobs) {
        st.updates.set(Some(pk::updates(aur)));
    }
    if tab == Tab::Aur {
        st.aur_updates.set(Some(pk::aur_updates()));
    }
    for spec in arg_value("--queue").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        let (name, act) = spec.split_once(':').unwrap_or((spec, "install"));
        let act = match act {
            "aur" => QAct::InstallAur,
            "remove" => QAct::Remove,
            "upgrade" => QAct::Upgrade,
            "upgrade-aur" => QAct::UpgradeAur,
            _ => QAct::Install,
        };
        crate::enqueue(st, QItem::new(name, act));
    }
    if std::env::args().any(|a| a == "--meta") {
        st.category.set(Some(crate::META_KEY));
        st.inst_filter.set(crate::InstFilter::Meta);
    }
    if let Some(g) = arg_value("--groups") {
        st.category.set(Some(crate::META_KEY));
        st.show_groups.set(true);
        st.group_open.set(Some(g).filter(|g| !g.is_empty()));
    }

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
        label: Some("synpkg screenshot"),
        ..Default::default()
    }))
    .context("не создать устройство")?;
    let gpu = GpuShared { instance, adapter, device, queue };

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

    let sheet = parse_stylesheet_str(&mss).map_err(|e| anyhow!("MSS: {e:?}"))?;
    let engine = StyleEngine::new(sheet);

    // Размер окна известен заранее — раскладка (телефон/рабочий стол) по нему.
    syngui::viewport::viewport_size().set(Size::new(lw as f32, lh as f32));
    let widget = crate::ui::root(st);
    let element = widget.create_element();
    let root = tree.insert_with_type_id(element, None, widget.as_any().type_id());
    widget.mount(&mut tree, root);
    tree.set_root(root);

    let logical = Size::new(lw as f32, lh as f32);
    let mut list = DisplayList::new();
    let menu = std::env::var("SYNPKG_SHOT_MENU").ok();
    // Несколько кадров: эффекты, пересборки, загрузка картинок.
    for frame in 0..20 {
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
        if frame == 1 {
            if let Some(name) = &menu {
                let p = st.updates.get_untracked().unwrap_or_default().into_iter().find(|u| &u.name == name).map(|u| pk::Pkg {
                    name: u.name,
                    version: u.new,
                    description: String::new(),
                    source: u.source,
                    installed: Some(u.old),
                    votes: None,
                    popularity: None,
                    out_of_date: false,
                    meta: false,
                });
                if let Some(p) = p {
                    crate::ui::pkg_menu(st, p, Point::new(lw as f32 / 2.0, 200.0), true);
                }
            }
        }
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
