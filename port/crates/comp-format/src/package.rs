use crate::{Manifest, json::to_swift_json, validate};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub type RgbaImage = image::RgbaImage;
pub type GrayImage = image::GrayImage;

/// A loaded project: the manifest plus decoded layer and mask pixels, keyed by layer ID.
///
/// Layer pixels are straight (unpremultiplied) RGBA8 as stored in the PNG. The PNG bytes they were
/// decoded from are kept, so an unedited asset is written back byte for byte.
#[derive(Clone, Debug)]
pub struct Project {
    pub manifest: Manifest,
    pub images: HashMap<String, Asset<RgbaImage>>,
    pub masks: HashMap<String, Asset<GrayImage>>,
}

#[derive(Clone, Debug)]
pub struct Asset<I> {
    pub pixels: I,
    /// The encoded file this asset came from; `None` once the pixels have been replaced.
    pub png: Option<Vec<u8>>,
}

impl<I> Asset<I> {
    pub fn new(pixels: I) -> Self {
        Self { pixels, png: None }
    }
}

/// Mirrors `ProjectStore.readPackage`: header check, full decode, validation, then every asset.
pub fn load(path: &Path) -> Result<Project> {
    if !path.is_dir() {
        bail!("{} is not a project package", path.display());
    }
    let bytes = fs::read(path.join("manifest.json")).context("reading manifest.json")?;
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("manifest.json is larger than 4 MiB");
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).context("decoding manifest.json")?;
    validate(&manifest)?;
    let images_dir = path.join("images");
    let mut images = HashMap::new();
    let mut masks = HashMap::new();
    for layer in &manifest.layers {
        if let Some(file) = &layer.image_file {
            let png = fs::read(images_dir.join(file)).with_context(|| format!("reading {file}"))?;
            let decoded = decode_png(&png).with_context(|| format!("decoding {file}"))?;
            images.insert(layer.id.clone(), Asset { pixels: decoded.to_rgba8(), png: Some(png) });
        }
        if let Some(file) = &layer.mask_file {
            let png = fs::read(images_dir.join(file)).with_context(|| format!("reading {file}"))?;
            let decoded = decode_png(&png).with_context(|| format!("decoding {file}"))?;
            // `LayerMask.isValid`: 8-bit grayscale without alpha.
            let image::DynamicImage::ImageLuma8(gray) = decoded else {
                bail!("{file} is not an 8-bit grayscale mask without alpha");
            };
            masks.insert(layer.id.clone(), Asset { pixels: gray, png: Some(png) });
        }
    }
    Ok(Project { manifest, images, masks })
}

fn decode_png(bytes: &[u8]) -> Result<image::DynamicImage> {
    let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))?;
    use image::ImageDecoder;
    let color = decoder.color_type();
    if color.bytes_per_pixel() as u16 / color.channel_count() as u16 > 1 {
        bail!("images deeper than 8 bits per channel are not supported");
    }
    Ok(image::DynamicImage::from_decoder(decoder)?)
}

/// Writes the package the way `ProjectStore.save` lays it out: `manifest.json` plus `images/`.
/// The package is staged beside the destination and swapped in, so a reader never sees half of it.
pub fn save(project: &Project, path: &Path) -> Result<()> {
    validate(&project.manifest)?;
    let staging = path.with_extension("comp-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let images_dir = staging.join("images");
    fs::create_dir_all(&images_dir)?;
    for layer in &project.manifest.layers {
        if let Some(file) = &layer.image_file {
            let asset = project.images.get(&layer.id).with_context(|| format!("missing pixels for {file}"))?;
            let bytes = match &asset.png {
                Some(png) => png.clone(),
                None => encode_png(asset.pixels.as_raw(), asset.pixels.width(), asset.pixels.height(), image::ExtendedColorType::Rgba8)?,
            };
            fs::write(images_dir.join(file), bytes)?;
        }
        if let Some(file) = &layer.mask_file {
            let asset = project.masks.get(&layer.id).with_context(|| format!("missing mask for {file}"))?;
            let bytes = match &asset.png {
                Some(png) => png.clone(),
                None => encode_png(asset.pixels.as_raw(), asset.pixels.width(), asset.pixels.height(), image::ExtendedColorType::L8)?,
            };
            fs::write(images_dir.join(file), bytes)?;
        }
    }
    let value = serde_json::to_value(&project.manifest)?;
    fs::write(staging.join("manifest.json"), to_swift_json(&value))?;
    if path.exists() {
        let old = path.with_extension("comp-old");
        if old.exists() {
            fs::remove_dir_all(&old)?;
        }
        fs::rename(path, &old)?;
        fs::rename(&staging, path)?;
        fs::remove_dir_all(&old)?;
    } else {
        fs::rename(&staging, path)?;
    }
    Ok(())
}

pub fn encode_png(raw: &[u8], width: u32, height: u32, color: image::ExtendedColorType) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(raw, width, height, color)?;
    Ok(out)
}
