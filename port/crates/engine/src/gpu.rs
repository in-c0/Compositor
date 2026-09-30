use anyhow::{Context, Result};

/// A headless wgpu device. The parity tool and the app share this setup.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .context("no GPU adapter")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("compositor"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .context("creating the GPU device")?;
        Ok(Self { instance, adapter, device, queue })
    }
}
