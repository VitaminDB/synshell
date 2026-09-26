//! Общий wgpu: одно устройство на все поверхности, wgpu-surface из
//! `wl_surface`, снимок кадра в PNG для отладки.

use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use std::ptr::NonNull;
use syngui::gpu::{GpuShared, WindowSurface};
use wayland_client::{protocol::wl_surface::WlSurface, Proxy};

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub shared: Option<GpuShared>,
}

impl Gpu {
    pub fn new() -> Self {
        // Как у winit-приложений syngui: Vulkan, иначе GL.
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            flags: syngui::gpu::instance_flags(),
            ..Default::default()
        });
        Self { instance, shared: None }
    }

    /// Устройство, совместимое с `surface` (или без поверхности — headless).
    pub fn ensure(&mut self, surface: Option<&wgpu::Surface<'static>>) -> anyhow::Result<&GpuShared> {
        if self.shared.is_none() {
            let adapter = pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: surface,
                force_fallback_adapter: false,
            }))
            .map_err(|e| anyhow::anyhow!("нет подходящего GPU-адаптера: {e}"))?;
            let info = adapter.get_info();
            log::info!("syngui-layer: адаптер «{}» ({:?})", info.name, info.backend);
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("syngui-layer"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))?;
            self.shared = Some(GpuShared { instance: self.instance.clone(), adapter, device, queue });
        }
        Ok(self.shared.as_ref().unwrap())
    }

    /// wgpu-поверхность поверх `wl_surface`. Уничтожать раньше `wl_surface`.
    pub fn create_surface(&self, wl: &WlSurface) -> anyhow::Result<wgpu::Surface<'static>> {
        let display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(wl.backend().upgrade().ok_or_else(|| anyhow::anyhow!("соединение закрыто"))?.display_ptr() as *mut _).ok_or_else(|| anyhow::anyhow!("нет wl_display"))?,
        ));
        let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(wl.id().as_ptr() as *mut _).ok_or_else(|| anyhow::anyhow!("нет wl_surface"))?,
        ));
        // SAFETY: wl_display живёт всё время работы цикла, wl_surface —
        // дольше wgpu-поверхности (порядок полей в `Surface`).
        let s = unsafe {
            self.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: display,
                raw_window_handle: window,
            })?
        };
        Ok(s)
    }
}

/// Настроить wgpu-поверхность: sRGB-формат, премультиплицированная альфа
/// (прозрачные углы плавающих панелей), без ожидания vsync — темп задают
/// frame callbacks Wayland.
pub fn configure_surface(gpu: &GpuShared, surface: wgpu::Surface<'static>, w: u32, h: u32) -> WindowSurface {
    let caps = surface.get_capabilities(&gpu.adapter);
    let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
    let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::PostMultiplied]
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m))
        .unwrap_or(caps.alpha_modes[0]);
    let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|m| caps.present_modes.contains(m))
        .unwrap_or(wgpu::PresentMode::Fifo);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: w.max(1),
        height: h.max(1),
        present_mode,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);
    WindowSurface { surface, surface_config: config }
}

/// Offscreen-цель: кадр рисуется в текстуру и читается в RGBA.
pub struct Offscreen {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    format: wgpu::TextureFormat,
}

impl Offscreen {
    pub fn new(gpu: &GpuShared, format: wgpu::TextureFormat, w: u32, h: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("syngui-layer offscreen"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view, size: (w.max(1), h.max(1)), format }
    }

    /// Прочитать пиксели и сохранить PNG.
    pub fn save_png(&self, gpu: &GpuShared, path: &std::path::Path) -> anyhow::Result<()> {
        let (w, h) = self.size;
        let row = w * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("syngui-layer readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()??;
        let data = slice.get_mapped_range();
        let bgra = matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((row * h) as usize);
        for y in 0..h {
            let line = &data[(y * padded) as usize..(y * padded + row) as usize];
            for px in line.chunks_exact(4) {
                if bgra {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                } else {
                    rgba.extend_from_slice(px);
                }
            }
        }
        drop(data);
        buf.unmap();
        image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8)?;
        Ok(())
    }
}
