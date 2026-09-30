//! Image files opened as the Mac app opens them: `EditorSession.importImages` into an empty session,
//! which makes a document the image's size with the image as its one layer.
//!
//! The Mac decodes JPEG, PNG, HEIC and TIFF with ImageIO and turns the result upright and into
//! 8-bit premultiplied sRGB through Core Image (`ImageImporter.decode`); draws SVG with AppKit
//! (`ImageImporter.decodeSVG`); and develops camera RAW files with `CIRAWFilter` after the develop
//! sheet (`RawImporter`). [`develop`] reproduces the Core Image step, and each format has a decoder
//! module. Where the port can't make exactly what the Mac makes, the import still succeeds but says
//! so in [`Import::approximation`], so a caller that needs parity can refuse it.

mod color;
mod develop;
mod dng;
mod exif;
mod heic;
mod jpeg;
mod jpeg_coefficients;
mod jpeg_markers;
mod png_file;
mod raw;
mod svg;
mod tiff_file;

pub use raw::RawSettings;

use comp_format::{Asset, BlendMode, LayerRecord, Manifest, Project, RgbaImage, Transform};
use std::collections::HashMap;
use std::path::Path;

/// `DocumentLimits.maxSide`.
pub const MAX_SIDE: u64 = 30_000;
/// `DocumentLimits.documentPixelBudget` on a Mac with 16 GB or more.
pub const DOCUMENT_PIXEL_BUDGET: u64 = 800_000_000;

/// Why an import failed. The texts are `ImageImportError`'s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Unreadable,
    Unsupported,
    TooLarge,
    /// The Mac imports this, but the port can't reproduce it yet; the text says what.
    NotPorted(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Unreadable => f.write_str("The image could not be read. It may be damaged or unavailable."),
            ImportError::Unsupported => f.write_str("Choose a JPEG, PNG, HEIC, TIFF, or Photoshop (PSD) file."),
            ImportError::TooLarge => write!(
                f,
                "This import exceeds the current {}-megapixel document budget or 30,000-pixel side limit.",
                DOCUMENT_PIXEL_BUDGET / 1_000_000
            ),
            ImportError::NotPorted(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for ImportError {}

pub type Result<T> = std::result::Result<T, ImportError>;

/// A finished import.
pub struct Import {
    pub project: Project,
    /// What the import did that is worth knowing, such as the RAW develop settings it used.
    pub notes: Vec<String>,
    /// Set when the pixels only approximate the Mac's, saying why (Apple's RAW engine, a CMYK
    /// profile the port doesn't have, and so on).
    pub approximation: Option<String>,
}

/// The pixels an import produces: the layer, straight RGBA8 as the project stores it.
pub(crate) struct Layer {
    pub pixels: RgbaImage,
    pub approximation: Option<String>,
    pub notes: Vec<String>,
}

/// Opens `path` as a new document. `raw` changes the develop sheet's settings for a camera RAW
/// file; the rest keep the camera's own, as the sheet opens with them.
pub fn import_file(path: &Path, raw: Option<&RawSettings>) -> Result<Import> {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let data = std::fs::read(path).map_err(|_| ImportError::Unreadable)?;
    import(&data, &extension, &name, raw)
}

/// Opens a file's bytes as a new document. `extension` (lowercase, without the dot) picks the
/// camera RAW and SVG paths, as the Mac's `UTType(filenameExtension:)` does; everything else is
/// told apart by its contents, as ImageIO does. `name` is the file name without its extension.
pub fn import(data: &[u8], extension: &str, name: &str, raw: Option<&RawSettings>) -> Result<Import> {
    let layer = if raw::matches(extension) {
        raw::develop(data, raw.cloned().unwrap_or_default())?
    } else if raw.is_some() {
        return Err(ImportError::NotPorted("RAW develop settings for a file that isn't camera RAW".into()));
    } else if extension == "svg" {
        svg::decode(data)?
    } else if data.starts_with(b"8BPS") {
        return Err(ImportError::NotPorted("Photoshop files are imported by the psd crate".into()));
    } else {
        decode(data)?
    };
    Ok(document(layer, name))
}

/// `ImageImporter.decode`: ImageIO reads JPEG, PNG, HEIC and TIFF, and nothing else.
fn decode(data: &[u8]) -> Result<Layer> {
    let source = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_file::decode(data)?
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        jpeg::decode(data)?
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        tiff_file::decode(data)?
    } else if heic::matches(data) {
        heic::decode(data)?
    } else if is_known_other(data) {
        return Err(ImportError::Unsupported);
    } else {
        return Err(ImportError::Unreadable);
    };
    check_size(source.width as u64, source.height as u64)?;
    develop::develop(source)
}

/// Formats ImageIO recognizes but the app refuses (`ImageImportError.unsupported`).
fn is_known_other(data: &[u8]) -> bool {
    data.starts_with(b"GIF87a")
        || data.starts_with(b"GIF89a")
        || data.starts_with(b"BM")
        || (data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP")
        || data.starts_with(&[0, 0, 1, 0])
        || data.starts_with(b"\0\0\0\x0cjP  ")
}

pub(crate) fn check_size(width: u64, height: u64) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(ImportError::Unreadable);
    }
    if width > MAX_SIDE || height > MAX_SIDE || width * height > DOCUMENT_PIXEL_BUDGET {
        return Err(ImportError::TooLarge);
    }
    Ok(())
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// `EditorSession.insert` into an empty session: a document the image's size at the default
/// resolution, with the image as its one layer, named after the file, centered (so at the origin).
fn document(layer: Layer, name: &str) -> Import {
    let (w, h) = layer.pixels.dimensions();
    let id = new_id();
    let record = LayerRecord {
        id: id.clone(),
        name: name.into(),
        is_visible: true,
        transform: Transform::at(0.0, 0.0, w as f64, h as f64),
        image_file: Some(format!("{id}.png")),
        parent_id: None,
        is_group: Some(false),
        opacity: Some(1.0),
        blend_mode: Some(BlendMode::Normal),
        mask_file: None,
        mask_enabled: None,
        mask_source_id: None,
        adjustment: None,
        mask_placement: None,
        mask_linked: None,
        shape: None,
        effects: None,
        text: None,
    };
    let manifest = Manifest {
        format: comp_format::FORMAT.into(),
        version: comp_format::CURRENT_VERSION,
        color_space: "sRGB".into(),
        resolution: Some(72.0),
        document_id: new_id(),
        width: w as i64,
        height: h as i64,
        active_layer_id: Some(id.clone()),
        layers: vec![record],
        guides: None,
    };
    let mut images = HashMap::new();
    images.insert(id, Asset::new(layer.pixels));
    Import { project: Project { manifest, images, masks: HashMap::new() }, notes: layer.notes, approximation: layer.approximation }
}
