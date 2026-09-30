//! The port's renderer. Everything that produces pixels runs on the GPU through wgpu, so the
//! same WGSL renders on Metal (Mac) and DX12 (Windows).

use comp_format::Project;
use image::RgbaImage;

pub mod adjust;
pub mod blend;
pub mod composite;
pub mod document;
pub mod effects;
pub mod export;
pub mod gpu;
pub mod mask;
pub mod order;
pub mod session;

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
    pub fn render(&self, project: &Project) -> Result<RgbaImage, RenderError> {
        let (width, height) = (project.manifest.width as u32, project.manifest.height as u32);
        let canvas = composite::Compositor::new(&self.gpu, project).render()?;
        let straight = blend::unpremultiply(&self.gpu, &canvas);
        let bytes = self.gpu.download(&straight)?;
        Ok(RgbaImage::from_raw(width, height, bytes).expect("canvas size"))
    }

    /// Applies one corpus operation (`filter`, `crop`, `canvasSize`, `imageSize`) to `project`.
    pub fn apply_op(&self, project: &mut Project, op: &serde_json::Value) -> Result<(), RenderError> {
        let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
        let num = |key: &str| op.get(key).and_then(|v| v.as_f64());
        let pair = |key: &str| op.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>());
        *project = match name {
            "crop" => {
                let r = pair("rect").filter(|r| r.len() == 4).ok_or_else(|| RenderError::Failed(anyhow::anyhow!("crop needs rect")))?;
                document::crop(project, [r[0], r[1], r[2], r[3]])?
            }
            "canvasSize" => {
                let options = document::CanvasSize {
                    width: num("width").unwrap_or(0.0) as i64,
                    height: num("height").unwrap_or(0.0) as i64,
                    anchor: num("anchor").unwrap_or(4.0) as i64,
                    fill: pair("fill").filter(|f| f.len() == 3).map(|f| [f[0], f[1], f[2]]),
                    content_offset: pair("contentOffset").filter(|o| o.len() == 2).map(|o| [o[0], o[1]]),
                };
                document::canvas_size(project, &options)?
            }
            other => return Err(RenderError::Unsupported(format!("operation `{other}`"))),
        };
        Ok(())
    }

    /// File > Export JPEG: the flattened image on the matte, encoded at `options.quality`.
    pub fn export_jpeg(&self, project: &Project, options: &serde_json::Value) -> Result<Vec<u8>, RenderError> {
        let quality = options.get("quality").and_then(|v| v.as_f64()).unwrap_or(0.85);
        let matte = options
            .get("matte")
            .and_then(|v| v.as_array())
            .and_then(|a| (a.len() == 3).then(|| [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(1.0))))
            .unwrap_or([1.0; 3]);
        let canvas = composite::Compositor::new(&self.gpu, project).render()?;
        let rgb = export::flatten_on_matte(&self.gpu, &canvas, matte)?;
        Ok(export::jpeg(&rgb, canvas.width, canvas.height, quality)?)
    }

    /// Imports a Photoshop file as the Mac app's File > Open does.
    pub fn import_psd(&self, _path: &std::path::Path) -> Result<Project, RenderError> {
        Err(RenderError::Unsupported("PSD import".into()))
    }
}
