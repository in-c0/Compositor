//! The port's renderer. Everything that produces pixels runs on the GPU through wgpu, so the
//! same WGSL renders on Metal (Mac) and DX12 (Windows).

use comp_format::Project;
use image::RgbaImage;

pub mod gpu;

#[derive(Debug)]
pub enum RenderError {
    /// The project uses something the port doesn't draw yet; the name says what.
    Unsupported(String),
    Failed(anyhow::Error),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            RenderError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for RenderError {
    fn from(e: anyhow::Error) -> Self {
        RenderError::Failed(e)
    }
}

pub struct Renderer {
    pub gpu: gpu::Gpu,
}

impl Renderer {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self { gpu: gpu::Gpu::new()? })
    }

    pub fn adapter_name(&self) -> String {
        let info = self.gpu.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    /// Flattens `project` as the Mac app's PNG export does, returning straight-alpha RGBA8.
    pub fn render(&self, _project: &Project) -> Result<RgbaImage, RenderError> {
        Err(RenderError::Unsupported("layer compositing".into()))
    }

    /// Applies one corpus operation (`filter`, `crop`, `canvasSize`, `imageSize`) to `project`.
    pub fn apply_op(&self, _project: &mut Project, op: &serde_json::Value) -> Result<(), RenderError> {
        let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
        Err(RenderError::Unsupported(format!("operation `{name}`")))
    }

    /// Imports a Photoshop file as the Mac app's File > Open does.
    pub fn import_psd(&self, _path: &std::path::Path) -> Result<Project, RenderError> {
        Err(RenderError::Unsupported("PSD import".into()))
    }
}
