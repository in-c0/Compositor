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
pub mod filters;
pub mod gpu;
pub mod mask;
pub mod ml;
pub mod order;
pub mod paint;
pub mod select;
pub mod session;
pub mod transform;

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

    /// Applies a case's operations to `project` in order. Painting ops share a `paint::Session`,
    /// as strokes in one Mac session share the brush settings and Clone Stamp's source, and every
    /// op sees the selection the ops before it made.
    pub fn apply_ops(&self, project: &mut Project, ops: &[serde_json::Value]) -> Result<(), RenderError> {
        let mut painting = paint::Session::default();
        let mut selection = None;
        for op in ops {
            match op.get("op").and_then(|v| v.as_str()) {
                Some("stroke") => paint::apply(&self.gpu, project, &mut painting, op)?,
                _ => self.apply_op_with_selection(project, &mut selection, op)?,
            }
        }
        Ok(())
    }

    /// Applies one corpus operation (`filter`, `crop`, `canvasSize`, `imageSize`, `stroke`) to `project`.
    /// The selection ops leave the project as it is (the selection isn't part of it) and are
    /// checked here; to keep the selection they make, use [`Renderer::apply_op_with_selection`].
    pub fn apply_op(&self, project: &mut Project, op: &serde_json::Value) -> Result<(), RenderError> {
        self.apply_op_with_selection(project, &mut None, op)
    }

    /// [`Renderer::apply_op`], with the session's selection alongside the project: the selection
    /// ops (`select::OPS`) make and change it.
    pub fn apply_op_with_selection(
        &self,
        project: &mut Project,
        selection: &mut Option<select::Selection>,
        op: &serde_json::Value,
    ) -> Result<(), RenderError> {
        let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
        if select::OPS.contains(&name) {
            return select::apply(&self.gpu, project, selection, op);
        }
        if name == "filter" {
            return filters::apply(&self.gpu, project, op);
        }
        if name == "stroke" {
            return paint::apply(&self.gpu, project, &mut paint::Session::default(), op);
        }
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

    /// The selection's coverage at document size, one byte per pixel, as the Mac's
    /// `DocumentSelection.coverage` draws it.
    pub fn selection_coverage(&self, project: &Project, selection: &select::Selection) -> Result<image::GrayImage, RenderError> {
        let (width, height) = (project.manifest.width as u32, project.manifest.height as u32);
        let bytes = select::coverage(&self.gpu, selection, width, height)?;
        Ok(image::GrayImage::from_raw(width, height, bytes).expect("canvas size"))
    }

    /// Imports a Photoshop file as the Mac app's File > Open does.
    pub fn import_psd(&self, path: &std::path::Path) -> Result<Project, RenderError> {
        match psd::import_file(path) {
            Ok(imported) => Ok(imported.project),
            Err(psd::ImportError::NotPorted(what)) => Err(RenderError::Unsupported(what)),
            Err(e) => Err(RenderError::Failed(anyhow::anyhow!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy()))),
        }
    }

    /// Imports a JPEG, PNG, HEIC, TIFF, SVG or camera RAW file as the Mac app's File > Open does.
    /// `raw` holds the develop sheet's settings to change (`exposure`, `temperature`, `tint`,
    /// `boost`). An import that only approximates the Mac's pixels reports as not supported.
    pub fn import_image(&self, path: &std::path::Path, raw: Option<&serde_json::Value>) -> Result<Project, RenderError> {
        let settings = raw.map(|r| {
            let get = |key: &str| r.get(key).and_then(|v| v.as_f64()).map(|v| v as f32);
            image_import::RawSettings { exposure: get("exposure"), temperature: get("temperature"), tint: get("tint"), boost: get("boost") }
        });
        match image_import::import_file(path, settings.as_ref()) {
            Ok(imported) => match imported.approximation {
                Some(why) => Err(RenderError::Unsupported(why)),
                None => Ok(imported.project),
            },
            Err(image_import::ImportError::NotPorted(what)) => Err(RenderError::Unsupported(what)),
            Err(e) => Err(RenderError::Failed(anyhow::anyhow!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy()))),
        }
    }
}
