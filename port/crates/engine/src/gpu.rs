use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A headless wgpu device. The parity tool and the app share this setup.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipelines: Mutex<HashMap<&'static str, Arc<wgpu::ComputePipeline>>>,
}

/// An image on the GPU: one `u32` per pixel, RGBA8 packed with R in the low byte.
///
/// The Mac app works on 8-bit premultiplied canvases and rounds after every step, so the port
/// keeps pixels as integers in storage buffers and rounds in WGSL exactly where Core Graphics and
/// Core Image do, rather than trusting each GPU's float-to-unorm conversion.
pub struct GpuImage {
    pub buffer: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        // DX12 on Windows and Metal on the Mac are the backends parity is measured on.
        // WGPU_BACKEND still overrides this, for experiments.
        if std::env::var_os("WGPU_BACKEND").is_none() {
            descriptor.backends = if cfg!(windows) {
                wgpu::Backends::DX12
            } else if cfg!(target_os = "macos") {
                wgpu::Backends::METAL
            } else {
                wgpu::Backends::all()
            };
        }
        descriptor.backend_options.dx12.shader_compiler = dx12_compiler();
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .context(if cfg!(windows) {
            "no GPU adapter (on Windows, DX12 needs dxcompiler.dll and dxil.dll next to the executable: run port/tools/fetch-dxc.ps1)"
        } else {
            "no GPU adapter"
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("compositor"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .context("creating the GPU device")?;
        Ok(Self { instance, adapter, device, queue, pipelines: Mutex::new(HashMap::new()) })
    }

    pub fn image(&self, width: u32, height: u32) -> GpuImage {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image"),
            size: (width as u64 * height as u64 * 4).max(4),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        GpuImage { buffer, width, height }
    }

    /// Uploads RGBA8 bytes as they are, without converting anything.
    pub fn upload(&self, width: u32, height: u32, rgba: &[u8]) -> GpuImage {
        let img = self.image(width, height);
        self.queue.write_buffer(&img.buffer, 0, rgba);
        img
    }

    /// A read-only storage buffer holding `data`.
    pub fn bytes(&self, data: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let mut padded = data.to_vec();
        padded.resize(padded.len().div_ceil(4).max(1) * 4, 0);
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("data"),
            contents: &padded,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        })
    }

    /// A copy of `img`.
    pub fn copy(&self, img: &GpuImage) -> GpuImage {
        let out = self.image(img.width, img.height);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&img.buffer, 0, &out.buffer, 0, img.buffer.size());
        self.queue.submit([encoder.finish()]);
        out
    }

    /// A copy of a storage buffer.
    pub fn copy_buffer(&self, buffer: &wgpu::Buffer) -> wgpu::Buffer {
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("copy"),
            size: buffer.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &out, 0, buffer.size());
        self.queue.submit([encoder.finish()]);
        out
    }

    /// Reads RGBA8 bytes back as they are.
    pub fn download(&self, img: &GpuImage) -> Result<Vec<u8>> {
        let size = (img.width as u64 * img.height as u64 * 4).max(4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&img.buffer, 0, &staging, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let data = slice.get_mapped_range()?.to_vec();
        staging.unmap();
        Ok(data[..(img.width as usize * img.height as usize * 4)].to_vec())
    }

    /// A compute pipeline for `source`, compiled once per `name`. Every kernel is prefixed with
    /// the shared helpers in `common.wgsl`.
    pub fn pipeline(&self, name: &'static str, source: &str) -> Arc<wgpu::ComputePipeline> {
        let mut cache = self.pipelines.lock().unwrap();
        cache
            .entry(name)
            .or_insert_with(|| {
                let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", include_str!("shaders/common.wgsl"), source).into()),
                });
                Arc::new(self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                }))
            })
            .clone()
    }

    /// Runs `pipeline` once per pixel of a `width` x `height` grid. `params` is bound at
    /// binding 0 as a uniform buffer, then `buffers` at 1, 2, … in order.
    pub fn dispatch(&self, pipeline: &wgpu::ComputePipeline, params: &[u8], buffers: &[&wgpu::Buffer], width: u32, height: u32) {
        use wgpu::util::DeviceExt;
        let mut padded = params.to_vec();
        padded.resize(padded.len().div_ceil(16).max(1) * 16, 0);
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: &padded,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        for (i, b) in buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: i as u32 + 1, resource: b.as_entire_binding() });
        }
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        self.queue.submit([encoder.finish()]);
    }
}

/// DX12 compiles WGSL through Microsoft's DXC, loaded from `dxcompiler.dll` next to the executable
/// (`port/tools/fetch-dxc.ps1` puts it there) or on the PATH. The older FXC can't compile the
/// engine's shaders.
fn dx12_compiler() -> wgpu::Dx12Compiler {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("dxcompiler.dll")));
    match beside_exe {
        Some(path) if path.exists() => wgpu::Dx12Compiler::DynamicDxc { dxc_path: path.to_string_lossy().into_owned() },
        _ => wgpu::Dx12Compiler::default_dynamic_dxc(),
    }
}
