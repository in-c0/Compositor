//! The port's renderer. Everything that produces pixels runs on the GPU through wgpu, so the
//! same WGSL renders on Metal (Mac) and DX12 (Windows).

use comp_format::Project;
use image::RgbaImage;

pub mod blend;
pub mod gpu;
pub mod mask;
pub mod order;

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
        let m = &project.manifest;
        let (width, height) = (m.width as u32, m.height as u32);
        let by_id: std::collections::HashMap<&str, &comp_format::LayerRecord> = m.layers.iter().map(|l| (l.id.as_str(), l)).collect();
        let visible = order::visible_layers(&m.layers);
        let gpu = &self.gpu;
        let mut canvas = gpu.image(width, height);
        for layer in visible {
            let unsupported = |what: &str| Err(RenderError::Unsupported(what.to_string()));
            if layer.adjustment.is_some() {
                return unsupported("adjustment layers");
            }

            if layer.mask_source_id.is_some() {
                return unsupported("clipping masks");
            }
            if layer.effects.is_some() {
                return unsupported("layer effects");
            }
            let Some(asset) = project.images.get(&layer.id) else {
                continue;
            };
            let t = &layer.transform;
            let (w, h) = asset.pixels.dimensions();
            let upright = t.rotation == 0.0 && !t.flip_x && !t.flip_y && t.size == [w as f64, h as f64]
                && t.origin[0].fract() == 0.0 && t.origin[1].fract() == 0.0;
            if !upright {
                return unsupported("transformed layers");
            }
            let coverage = match mask::coverage(project, layer, &by_id, (w, h)) {
                Ok(c) => c,
                Err(what) => return unsupported(&what),
            };
            let coverage = coverage.map(|c| gpu.bytes(bytemuck::cast_slice(&c)));
            let pixels = gpu.upload(w, h, asset.pixels.as_raw());
            let opacity = order::effective_opacity(layer, &by_id);
            canvas = blend::draw_upright(gpu, &canvas, &pixels, (t.origin[0] as i32, t.origin[1] as i32), layer.blend_mode(), opacity, coverage.as_ref());
        }
        let straight = blend::unpremultiply(gpu, &canvas);
        let bytes = gpu.download(&straight)?;
        Ok(RgbaImage::from_raw(width, height, bytes).expect("canvas size"))
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
