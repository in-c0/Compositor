//! The one GPU device the app has: the engine composites on it and egui draws with it.

use eframe::egui;
use eframe::egui::mutex::RwLock;
use eframe::egui_wgpu;
use engine::gpu::{Gpu, GpuImage};
use std::sync::Arc;

#[derive(Clone)]
pub struct Gfx {
    pub gpu: Arc<Gpu>,
    pub renderer: Arc<RwLock<egui_wgpu::Renderer>>,
    /// The engine's operations (the parity ops, import and export) on the same device.
    pub engine: Arc<engine::Renderer>,
}

impl Gfx {
    /// The engine on eframe's device.
    pub fn from_render_state(state: &egui_wgpu::RenderState) -> Self {
        let gpu = Gpu::from_parts(state.instance.clone(), state.adapter.clone(), state.device.clone(), state.queue.clone());
        Self::new(Arc::new(gpu), state.renderer.clone())
    }

    pub fn new(gpu: Arc<Gpu>, renderer: Arc<RwLock<egui_wgpu::Renderer>>) -> Self {
        let engine = engine::Renderer { gpu: Gpu::from_parts(gpu.instance.clone(), gpu.adapter.clone(), gpu.device.clone(), gpu.queue.clone()) };
        Self { gpu, renderer, engine: Arc::new(engine) }
    }
}

/// How eframe should create its device: the engine's backend (DX12 or Metal, where parity is
/// measured), its shader compiler, and the adapter's full limits so big canvases fit in one buffer.
pub fn wgpu_options() -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    if std::env::var_os("WGPU_BACKEND").is_none() {
        setup.instance_descriptor.backends = if cfg!(windows) {
            wgpu::Backends::DX12
        } else if cfg!(target_os = "macos") {
            wgpu::Backends::METAL
        } else {
            wgpu::Backends::PRIMARY
        };
    }
    setup.instance_descriptor.backend_options.dx12.shader_compiler = engine::gpu::dx12_compiler();
    setup.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
        label: Some("compositor"),
        required_limits: adapter.limits(),
        ..Default::default()
    });
    egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() }
}

/// A flattened canvas shown by egui. The engine's canvas is premultiplied RGBA8 in a storage
/// buffer; egui's textures are premultiplied RGBA8 too, in gamma space, so the bytes go across
/// unchanged. The texture is registered twice: sharp for 200% and up, smooth below.
pub struct CanvasTexture {
    pub width: u32,
    pub height: u32,
    texture: wgpu::Texture,
    pub nearest: egui::TextureId,
    pub linear: egui::TextureId,
}

impl CanvasTexture {
    pub fn new(gfx: &Gfx, width: u32, height: u32) -> Self {
        let device = &gfx.gpu.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas"),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut renderer = gfx.renderer.write();
        let nearest = renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
        let linear = renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        Self { width, height, texture, nearest, linear }
    }

    /// Copies `image` (premultiplied, the engine's layout) into the texture. Rows are padded to
    /// the 256-byte stride texture copies need by a small kernel first.
    pub fn upload(&self, gfx: &Gfx, image: &GpuImage) {
        let gpu = &gfx.gpu;
        let stride = image.width.div_ceil(64) * 64;
        let padded = gpu.image(stride, image.height);
        let pipeline = gpu.pipeline("app_pad_rows", include_str!("pad_rows.wgsl"));
        let params = [image.width, image.height, stride, 0].map(u32::to_le_bytes).concat();
        gpu.dispatch(&pipeline, &params, &[&image.buffer, &padded.buffer], image.width, image.height);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &padded.buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride * 4), rows_per_image: Some(image.height) },
            },
            self.texture.as_image_copy(),
            wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([encoder.finish()]);
    }

    pub fn free(&self, gfx: &Gfx) {
        let mut renderer = gfx.renderer.write();
        renderer.free_texture(&self.nearest);
        renderer.free_texture(&self.linear);
    }
}
