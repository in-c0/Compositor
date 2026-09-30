//! Reads Photoshop files the way Compositor's Mac app does: a port of `PSDReader.swift` and
//! `PSDChannelCoder.swift`. Only what the Mac keeps is read: 8-bit RGB, the resolution resource,
//! layer records with their transparency, color and user-mask channels (raw or PackBits), and the
//! additional layer information that decides a layer's kind, name, fill and section. Everything the
//! Mac refuses is refused with its message, and its quirks are kept (see `adjustments`).

use crate::adjustments;
use crate::text;
use crate::vector;
use comp_format::Adjustment;
use std::collections::HashMap;

/// `DocumentLimits.maxSide`.
pub const MAX_SIDE: i64 = 30_000;
/// `DocumentLimits.maxSurfacePixels`.
pub const MAX_SURFACE_PIXELS: i64 = 200_000_000;
/// `DocumentLimits.documentPixelBudget` on a Mac with 16 GB or more. Smaller Macs get less (a
/// sixteenth of their memory, at least `MAX_SURFACE_PIXELS`); only enormous files notice.
pub const DOCUMENT_PIXEL_BUDGET: i64 = 800_000_000;

/// Why a file can't be imported, with the Mac's messages (`PSDError`, `ImageImportError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Truncated,
    UnsupportedVersion,
    UnsupportedColorMode,
    UnsupportedDepth,
    UnsupportedCompression,
    Unreadable,
    TooLarge,
    /// The Mac imports this, but the port can't reproduce it yet; the text says what.
    NotPorted(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            ImportError::Truncated => "The Photoshop file could not be read. It may be damaged or incomplete.",
            ImportError::UnsupportedVersion => "This Photoshop file uses a format version Compositor can\u{2019}t read.",
            ImportError::UnsupportedColorMode | ImportError::UnsupportedDepth => "Only 8-bit RGB Photoshop files can be imported.",
            ImportError::UnsupportedCompression => "This Photoshop file uses a layer compression method that isn\u{2019}t supported.",
            ImportError::Unreadable => "The image could not be read. It may be damaged or unavailable.",
            ImportError::TooLarge => "This import exceeds the document budget or side limit.",
            ImportError::NotPorted(what) => return write!(f, "not supported yet: {what}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for ImportError {}

pub type Result<T> = std::result::Result<T, ImportError>;

/// A rectangle in document pixels, as `CGRect` with integer corners.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Premultiplied RGBA8, as the Mac's `CGImage`s from `PSDChannelCoder.rgbaImage`.
#[derive(Clone, Debug, PartialEq)]
pub struct Premultiplied {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An 8-bit grayscale plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raster,
    Group,
    Adjustment,
    Text,
    SmartObject,
    Effects,
    Vector,
}

/// `PSDRecord`: one layer or folder, bottom to top. `id` and `parent` index into the records'
/// own numbering (folder IDs are drawn when their section divider is met, as the Mac's UUIDs are).
#[derive(Clone, Debug)]
pub struct Record {
    pub id: usize,
    pub parent: Option<usize>,
    pub name: String,
    pub is_group: bool,
    pub is_visible: bool,
    pub opacity: f64,
    pub blend_key: String,
    pub clipping: bool,
    pub cropped_to_canvas: bool,
    pub bounds: Rect,
    pub image: Option<Premultiplied>,
    pub mask: Option<Gray>,
    pub mask_bounds: Rect,
    pub mask_default: u8,
    pub mask_enabled: bool,
    pub mask_linked: bool,
    pub adjustment: Option<Adjustment>,
    pub kind: Kind,
    /// A live shape layer (`PSDVector.live`) was made of this layer.
    pub is_shape: bool,
    /// The pixels are the Mac's own drawing of a vector (`PSDVector.live` or `.raster`), not the
    /// file's; the port doesn't draw those yet.
    pub drawn_by_mac: Option<&'static str>,
    /// `PSDText.parse` read editable type from this layer.
    pub text: bool,
}

/// `PSDDocument`.
#[derive(Clone, Debug)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub resolution: f64,
    pub layers: Vec<Record>,
    /// The merged image from the image data section, read only for files with no layer records
    /// (`ImageImporter.decode(flattenedPhotoshop:)`), as straight RGBA8.
    pub composite: Option<CompositeResult>,
}

#[derive(Clone, Debug)]
pub enum CompositeResult {
    Pixels(image::RgbaImage),
    Failed(ImportError),
}

pub(crate) struct Cursor<'a> {
    pub data: &'a [u8],
    pub offset: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn need(&self, count: usize) -> Result<()> {
        match self.offset.checked_add(count) {
            Some(end) if end <= self.data.len() => Ok(()),
            _ => Err(ImportError::Truncated),
        }
    }

    pub fn skip(&mut self, count: usize) -> Result<()> {
        self.need(count)?;
        self.offset += count;
        Ok(())
    }

    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        self.need(count)?;
        let s = &self.data[self.offset..self.offset + count];
        self.offset += count;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    /// `String(bytes:encoding: .ascii) ?? ""`.
    pub fn string(&mut self, count: usize) -> Result<String> {
        Ok(ascii(self.bytes(count)?))
    }
}

/// `String(bytes:encoding: .ascii) ?? ""`: empty when any byte isn't ASCII.
pub(crate) fn ascii(bytes: &[u8]) -> String {
    if bytes.is_ascii() { bytes.iter().map(|&b| b as char).collect() } else { String::new() }
}

fn checked_length(value: u64) -> Result<usize> {
    if value > i64::MAX as u64 { Err(ImportError::TooLarge) } else { usize::try_from(value).map_err(|_| ImportError::TooLarge) }
}

fn be32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Crop {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Default)]
struct RawLayer {
    name: String,
    top: i64,
    left: i64,
    bottom: i64,
    right: i64,
    source_top: i64,
    source_left: i64,
    source_bottom: i64,
    source_right: i64,
    opacity: u8,
    fill: u8,
    clipping: bool,
    hidden: bool,
    blend_key: String,
    channels: Vec<(i64, usize)>,
    extra: HashMap<String, Vec<u8>>,
    mask_top: i64,
    mask_left: i64,
    mask_bottom: i64,
    mask_right: i64,
    source_mask_top: i64,
    source_mask_left: i64,
    source_mask_bottom: i64,
    source_mask_right: i64,
    mask_default: u8,
    mask_disabled: bool,
    mask_linked: bool,
    mask_from_render: bool,
    has_mask: bool,
    section: Option<u32>,
    image: Option<Premultiplied>,
    mask_image: Option<Gray>,
    image_crop: Option<Crop>,
    mask_crop: Option<Crop>,
    cropped: bool,
}

/// Keys whose length is 8 bytes in a PSB.
const PSB_LARGE_KEYS: [&str; 13] = ["LMsk", "Lr16", "Lr32", "Layr", "Mt16", "Mt32", "Mtrn", "Alph", "FMsk", "lnk2", "FEid", "FXid", "PxSD"];

/// `PSDReader.adjustmentKeys`.
pub const ADJUSTMENT_KEYS: [&str; 16] =
    ["levl", "curv", "hue2", "hue ", "expA", "grdm", "brit", "blnc", "nvrt", "thrs", "post", "mixr", "selc", "blwh", "phfl", "vibA"];

/// `PSDReader.read`.
pub fn read(data: &[u8], remaining_pixels: i64) -> Result<Document> {
    let mut cursor = Cursor::new(data);
    if cursor.string(4)? != "8BPS" {
        return Err(ImportError::Unreadable);
    }
    let version = cursor.u16()?;
    if version != 1 && version != 2 {
        return Err(ImportError::UnsupportedVersion);
    }
    let is_psb = version == 2;
    cursor.skip(6)?;
    let channel_count = cursor.u16()?;
    let canvas_height = cursor.u32()? as i64;
    let canvas_width = cursor.u32()? as i64;
    let depth = cursor.u16()?;
    let mode = cursor.u16()?;
    if !(1..=MAX_SIDE).contains(&canvas_width) || !(1..=MAX_SIDE).contains(&canvas_height) || canvas_width * canvas_height > MAX_SURFACE_PIXELS {
        return Err(ImportError::TooLarge);
    }
    if depth != 8 {
        return Err(ImportError::UnsupportedDepth);
    }
    if mode != 3 {
        return Err(ImportError::UnsupportedColorMode);
    }
    let color_data = cursor.u32()? as usize;
    cursor.skip(color_data)?;
    let resources_length = cursor.u32()? as usize;
    let resources_end = cursor.offset + resources_length;
    let mut resolution = 72.0;
    while cursor.offset + 12 <= resources_end {
        if cursor.string(4)? != "8BIM" {
            break;
        }
        let id = cursor.u16()?;
        let name_length = cursor.u8()? as usize;
        cursor.skip(name_length)?;
        if (name_length + 1) % 2 == 1 {
            cursor.skip(1)?;
        }
        let length = cursor.u32()? as usize;
        let data_start = cursor.offset;
        if id == 1005 && length >= 4 {
            resolution = cursor.u32()? as f64 / 65536.0;
            if !resolution.is_finite() || resolution < 1.0 {
                resolution = 72.0;
            }
            resolution = resolution.clamp(1.0, 9600.0);
        }
        cursor.offset = data_start + length;
        if length % 2 == 1 {
            cursor.skip(1)?;
        }
    }
    cursor.offset = resources_end;
    let layer_section = checked_length(if is_psb { cursor.u64()? } else { cursor.u32()? as u64 })?;
    let layer_section_end = cursor.offset.saturating_add(layer_section);
    let (width, height) = (canvas_width as u32, canvas_height as u32);
    if layer_section < 4 {
        cursor.offset = layer_section_end;
        let composite = Some(match read_composite(&mut cursor, channel_count, width, height, is_psb) {
            Ok(pixels) => CompositeResult::Pixels(pixels),
            Err(e) => CompositeResult::Failed(e),
        });
        return Ok(Document { width, height, resolution, layers: Vec::new(), composite });
    }
    let _layer_info_length = checked_length(if is_psb { cursor.u64()? } else { cursor.u32()? as u64 })?;
    let raw_count = cursor.i16()?;
    let count = (raw_count as i32).unsigned_abs() as usize;
    if count > 10_000 {
        return Err(ImportError::TooLarge);
    }
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        raw.push(read_record(&mut cursor, is_psb)?);
    }
    if !fits_budget_all(&raw, remaining_pixels) {
        for layer in &mut raw {
            crop_to_canvas(layer, canvas_width, canvas_height);
        }
        if !fits_budget_all(&raw, remaining_pixels) {
            return Err(ImportError::TooLarge);
        }
    }
    let mut used_pixels = 0i64;
    for layer in &mut raw {
        decode_channels(&mut cursor, layer, remaining_pixels - used_pixels, is_psb)?;
        if let Some(image) = &layer.image {
            used_pixels += image.width as i64 * image.height as i64;
        }
    }
    cursor.offset = layer_section_end;
    let layers = assemble(raw, (canvas_width as f64, canvas_height as f64), remaining_pixels - used_pixels)?;
    let composite = if layers.is_empty() {
        Some(match read_composite(&mut cursor, channel_count, width, height, is_psb) {
            Ok(pixels) => CompositeResult::Pixels(pixels),
            Err(e) => CompositeResult::Failed(e),
        })
    } else {
        None
    };
    Ok(Document { width, height, resolution, layers, composite })
}

fn read_record(cursor: &mut Cursor, is_psb: bool) -> Result<RawLayer> {
    let mut layer = RawLayer { opacity: 255, fill: 255, mask_default: 255, mask_linked: true, blend_key: "norm".into(), ..Default::default() };
    layer.top = cursor.i32()? as i64;
    layer.left = cursor.i32()? as i64;
    layer.bottom = cursor.i32()? as i64;
    layer.right = cursor.i32()? as i64;
    layer.source_top = layer.top;
    layer.source_left = layer.left;
    layer.source_bottom = layer.bottom;
    layer.source_right = layer.right;
    let channel_count = cursor.u16()? as usize;
    if channel_count > 56 {
        return Err(ImportError::TooLarge);
    }
    for _ in 0..channel_count {
        let id = cursor.i16()? as i64;
        let length = checked_length(if is_psb { cursor.u64()? } else { cursor.u32()? as u64 })?;
        layer.channels.push((id, length));
    }
    if cursor.string(4)? != "8BIM" {
        return Err(ImportError::Truncated);
    }
    layer.blend_key = cursor.string(4)?;
    layer.opacity = cursor.u8()?;
    layer.clipping = cursor.u8()? != 0;
    let flags = cursor.u8()?;
    layer.hidden = flags & 2 != 0;
    cursor.skip(1)?;
    let extra_length = cursor.u32()? as usize;
    let extra_end = cursor.offset + extra_length;
    let mask_length = cursor.u32()? as usize;
    let mask_end = cursor.offset + mask_length;
    if mask_length >= 20 {
        layer.has_mask = true;
        layer.mask_top = cursor.i32()? as i64;
        layer.mask_left = cursor.i32()? as i64;
        layer.mask_bottom = cursor.i32()? as i64;
        layer.mask_right = cursor.i32()? as i64;
        layer.source_mask_top = layer.mask_top;
        layer.source_mask_left = layer.mask_left;
        layer.source_mask_bottom = layer.mask_bottom;
        layer.source_mask_right = layer.mask_right;
        layer.mask_default = cursor.u8()?;
        let mask_flags = cursor.u8()?;
        layer.mask_disabled = mask_flags & 2 != 0;
        // Bit 0 is Photoshop's "position relative to layer"; the Mac reads it as unlinked.
        layer.mask_linked = mask_flags & 1 == 0;
        layer.mask_from_render = mask_flags & 8 != 0;
    }
    cursor.offset = mask_end;
    let ranges = cursor.u32()? as usize;
    cursor.skip(ranges)?;
    let name_count = cursor.u8()? as usize;
    let name_bytes = cursor.bytes(name_count)?;
    layer.name = mac_roman(name_bytes);
    let name_pad = (4 - ((name_count + 1) % 4)) % 4;
    cursor.skip(name_pad)?;
    while cursor.offset + 12 <= extra_end {
        let signature = cursor.string(4)?;
        if signature != "8BIM" && signature != "8B64" {
            break;
        }
        let key = cursor.string(4)?;
        let length = if signature == "8B64" || (is_psb && PSB_LARGE_KEYS.contains(&key.as_str())) {
            if cursor.offset + 8 > extra_end {
                break;
            }
            checked_length(cursor.u64()?)?
        } else {
            cursor.u32()? as usize
        };
        let payload = cursor.bytes(length)?.to_vec();
        if length % 2 == 1 {
            cursor.skip(1)?;
        }
        if key == "luni" {
            if let Some(unicode) = unicode_name(&payload) {
                layer.name = unicode;
            }
        }
        if key == "iOpa" {
            if let Some(&fill) = payload.first() {
                layer.fill = fill;
            }
        }
        if (key == "lsct" || key == "lsdk") && payload.len() >= 4 {
            layer.section = Some(be32(&payload, 0));
        }
        layer.extra.insert(key, payload);
    }
    cursor.offset = extra_end;
    Ok(layer)
}

fn unicode_name(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    let count = be32(data, 0) as usize;
    if count == 0 || data.len() < 4 + count * 2 {
        return None;
    }
    let units: Vec<u16> = (0..count).map(|i| u16::from_be_bytes([data[4 + i * 2], data[5 + i * 2]])).collect();
    Some(String::from_utf16_lossy(&units).trim_matches('\0').to_string())
}

/// The Mac OS Roman characters for bytes 0x80 to 0xFF.
const MAC_ROMAN_HIGH: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
    '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ', //
];

/// `String(bytes:encoding: .macOSRoman)`.
fn mac_roman(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { MAC_ROMAN_HIGH[(b - 0x80) as usize] }).collect()
}

/// Transparency, R, G, B and the user mask. Other channels are skipped undecoded.
const UNPACKED_CHANNEL_IDS: [i64; 5] = [-1, 0, 1, 2, -2];

fn fits_budget_all(layers: &[RawLayer], remaining_pixels: i64) -> bool {
    let mut used = 0i64;
    for layer in layers {
        let width = (layer.right - layer.left).max(0);
        let height = (layer.bottom - layer.top).max(0);
        let mask_width = (layer.mask_right - layer.mask_left).max(0);
        let mask_height = (layer.mask_bottom - layer.mask_top).max(0);
        if !fits_budget(width, height, mask_width, mask_height, layer.has_mask, remaining_pixels - used) {
            return false;
        }
        if width > 0 && height > 0 {
            used += width * height;
        }
    }
    true
}

fn fits_budget(width: i64, height: i64, mask_width: i64, mask_height: i64, has_mask: bool, remaining_pixels: i64) -> bool {
    let budget = remaining_pixels.max(0);
    if width > 0 && height > 0 && !(width <= MAX_SIDE && height <= MAX_SIDE && width * height <= budget) {
        return false;
    }
    if has_mask && mask_width > 0 && mask_height > 0 && !(mask_width <= MAX_SIDE && mask_height <= MAX_SIDE && mask_width * mask_height <= budget) {
        return false;
    }
    true
}

fn crop_to_canvas(layer: &mut RawLayer, width: i64, height: i64) {
    let image_crop = crop(layer.left, layer.top, layer.right, layer.bottom, width, height);
    if image_crop.x != 0 || image_crop.y != 0 || image_crop.width != layer.right - layer.left || image_crop.height != layer.bottom - layer.top {
        layer.left += image_crop.x;
        layer.top += image_crop.y;
        layer.right = layer.left + image_crop.width;
        layer.bottom = layer.top + image_crop.height;
        layer.image_crop = Some(image_crop);
        layer.cropped = true;
    }
    if !layer.has_mask {
        return;
    }
    let mask_crop = crop(layer.mask_left, layer.mask_top, layer.mask_right, layer.mask_bottom, width, height);
    if mask_crop.x != 0 || mask_crop.y != 0 || mask_crop.width != layer.mask_right - layer.mask_left || mask_crop.height != layer.mask_bottom - layer.mask_top {
        layer.mask_left += mask_crop.x;
        layer.mask_top += mask_crop.y;
        layer.mask_right = layer.mask_left + mask_crop.width;
        layer.mask_bottom = layer.mask_top + mask_crop.height;
        layer.mask_crop = Some(mask_crop);
        layer.cropped = true;
    }
}

fn crop(left: i64, top: i64, right: i64, bottom: i64, canvas_width: i64, canvas_height: i64) -> Crop {
    let cropped_left = left.max(0).min(canvas_width);
    let cropped_top = top.max(0).min(canvas_height);
    let cropped_right = right.min(canvas_width).max(cropped_left);
    let cropped_bottom = bottom.min(canvas_height).max(cropped_top);
    Crop { x: cropped_left - left, y: cropped_top - top, width: cropped_right - cropped_left, height: cropped_bottom - cropped_top }
}

fn decode_channels(cursor: &mut Cursor, layer: &mut RawLayer, remaining_pixels: i64, is_psb: bool) -> Result<()> {
    let mut planes: HashMap<i64, Vec<u8>> = HashMap::new();
    let width = (layer.right - layer.left).max(0);
    let height = (layer.bottom - layer.top).max(0);
    let mask_width = (layer.mask_right - layer.mask_left).max(0);
    let mask_height = (layer.mask_bottom - layer.mask_top).max(0);
    if !fits_budget(width, height, mask_width, mask_height, layer.has_mask, remaining_pixels) {
        return Err(ImportError::TooLarge);
    }
    let source_width = (layer.source_right - layer.source_left).max(0);
    let source_height = (layer.source_bottom - layer.source_top).max(0);
    let source_mask_width = (layer.source_mask_right - layer.source_mask_left).max(0);
    let source_mask_height = (layer.source_mask_bottom - layer.source_mask_top).max(0);
    for &(id, length) in &layer.channels {
        let start = cursor.offset;
        let decoded = (|| -> Result<()> {
            if !UNPACKED_CHANNEL_IDS.contains(&id) || length < 2 {
                return Ok(());
            }
            let compression = cursor.u16()?;
            let payload = cursor.bytes(length - 2)?;
            let is_mask = id == -2;
            let (source_w, source_h) = if is_mask { (source_mask_width, source_mask_height) } else { (source_width, source_height) };
            let (target_w, target_h) = if is_mask { (mask_width, mask_height) } else { (width, height) };
            let crop = if is_mask { layer.mask_crop } else { layer.image_crop };
            if target_w > 0 && target_h > 0 {
                planes.insert(id, decode(compression, source_w as usize, source_h as usize, payload, is_psb, crop)?);
            }
            Ok(())
        })();
        cursor.offset = start + length;
        decoded?;
    }
    if layer.has_mask && mask_width > 0 && mask_height > 0 {
        if let Some(gray) = planes.get(&-2) {
            let count = (mask_width * mask_height) as usize;
            if gray.len() >= count {
                layer.mask_image = Some(Gray { width: mask_width as u32, height: mask_height as u32, pixels: gray[..count].to_vec() });
            }
        }
    }
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    let count = (width * height) as usize;
    let opaque = vec![255u8; count];
    let black = vec![0u8; count];
    let red = planes.get(&0).unwrap_or(&black);
    let green = planes.get(&1).unwrap_or(&black);
    let blue = planes.get(&2).unwrap_or(&black);
    let alpha = planes.get(&-1).unwrap_or(&opaque);
    if red.len() < count || green.len() < count || blue.len() < count || alpha.len() < count {
        return Err(ImportError::Truncated);
    }
    let mut rgba = vec![0u8; count * 4];
    for i in 0..count {
        let a = alpha[i] as u16;
        rgba[i * 4] = ((red[i] as u16 * a + 127) / 255) as u8;
        rgba[i * 4 + 1] = ((green[i] as u16 * a + 127) / 255) as u8;
        rgba[i * 4 + 2] = ((blue[i] as u16 * a + 127) / 255) as u8;
        rgba[i * 4 + 3] = a as u8;
    }
    layer.image = Some(Premultiplied { width: width as u32, height: height as u32, rgba });
    Ok(())
}

/// `PSDChannelCoder.decode`.
pub(crate) fn decode(compression: u16, width: usize, height: usize, data: &[u8], large: bool, crop: Option<Crop>) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }
    let Some(crop) = crop else {
        return match compression {
            0 => {
                let expected = width * height;
                if data.len() < expected {
                    return Err(ImportError::Truncated);
                }
                Ok(data[..expected].to_vec())
            }
            1 => unpack_rle(width, height, data, large, None),
            _ => Err(ImportError::UnsupportedCompression),
        };
    };
    if crop.x < 0 || crop.y < 0 || crop.width < 0 || crop.height < 0 || crop.x + crop.width > width as i64 || crop.y + crop.height > height as i64 {
        return Err(ImportError::Truncated);
    }
    if crop.width == 0 || crop.height == 0 {
        return Ok(Vec::new());
    }
    match compression {
        0 => {
            if data.len() < width * height {
                return Err(ImportError::Truncated);
            }
            let (cx, cy, cw, ch) = (crop.x as usize, crop.y as usize, crop.width as usize, crop.height as usize);
            let mut plane = vec![0u8; cw * ch];
            for row in 0..ch {
                let source = (cy + row) * width + cx;
                plane[row * cw..(row + 1) * cw].copy_from_slice(&data[source..source + cw]);
            }
            Ok(plane)
        }
        1 => unpack_rle(width, height, data, large, Some(crop)),
        _ => Err(ImportError::UnsupportedCompression),
    }
}

/// PackBits rows after a table of per-row byte counts (2 bytes each, 4 in a PSB), cropped when asked.
fn unpack_rle(width: usize, height: usize, data: &[u8], large: bool, crop: Option<Crop>) -> Result<Vec<u8>> {
    let mut offset = 0usize;
    let mut counts = vec![0usize; height];
    let size = if large { 4 } else { 2 };
    for count in counts.iter_mut() {
        if offset + size > data.len() {
            return Err(ImportError::Truncated);
        }
        *count = if large { be32(data, offset) as usize } else { u16::from_be_bytes([data[offset], data[offset + 1]]) as usize };
        offset += size;
    }
    let (cx, cy, cw, ch) = match crop {
        Some(c) => (c.x as usize, c.y as usize, c.width as usize, c.height as usize),
        None => (0, 0, width, height),
    };
    let mut plane = vec![0u8; cw * ch];
    let mut row_buffer = vec![0u8; width];
    for (row, &count) in counts.iter().enumerate() {
        let end = offset.checked_add(count).ok_or(ImportError::Truncated)?;
        if end > data.len() {
            return Err(ImportError::Truncated);
        }
        if row < cy || row >= cy + ch {
            offset = end;
            continue;
        }
        let mut written = 0;
        while written < width {
            if offset >= end {
                return Err(ImportError::Truncated);
            }
            let n = data[offset] as i8;
            offset += 1;
            if n >= 0 {
                let run = n as usize + 1;
                if written + run > width || offset + run > end {
                    return Err(ImportError::Truncated);
                }
                row_buffer[written..written + run].copy_from_slice(&data[offset..offset + run]);
                offset += run;
                written += run;
            } else if n != -128 {
                let run = (1 - n as i32) as usize;
                if written + run > width || offset >= end {
                    return Err(ImportError::Truncated);
                }
                let value = data[offset];
                offset += 1;
                row_buffer[written..written + run].fill(value);
                written += run;
            }
        }
        let target = (row - cy) * cw;
        plane[target..target + cw].copy_from_slice(&row_buffer[cx..cx + cw]);
        offset = end;
    }
    Ok(plane)
}

fn assemble(raw: Vec<RawLayer>, canvas: (f64, f64), remaining_pixels: i64) -> Result<Vec<Record>> {
    let mut result = Vec::new();
    let mut groups: Vec<usize> = Vec::new();
    let mut next_id = 0usize;
    let mut new_id = || {
        next_id += 1;
        next_id - 1
    };
    let mut remaining = remaining_pixels.max(0);
    for layer in raw {
        // Photoshop stores a folder bottom to top: a divider (type 3), the children, then the folder (type 1 or 2).
        if layer.section == Some(3) {
            groups.push(new_id());
            continue;
        }
        let is_group = layer.section == Some(1) || layer.section == Some(2);
        let id = if is_group { groups.pop().unwrap_or_else(&mut new_id) } else { new_id() };
        let kind = kind_of(&layer, is_group);
        let has_effects = kind == Kind::Effects || ["lfx2", "lrFX", "lmfx"].iter().any(|k| layer.extra.contains_key(*k));
        let opacity =
            if has_effects && layer.fill != 255 { layer.opacity as f64 / 255.0 } else { (layer.opacity as f64 / 255.0) * (layer.fill as f64 / 255.0) };
        let bounds = if is_group {
            Rect { x: 0.0, y: 0.0, width: canvas.0, height: canvas.1 }
        } else {
            Rect {
                x: layer.left as f64,
                y: layer.top as f64,
                width: (layer.right - layer.left).max(0) as f64,
                height: (layer.bottom - layer.top).max(0) as f64,
            }
        };
        let mut record = Record {
            id,
            parent: groups.last().copied(),
            name: if layer.name.is_empty() { "Layer".into() } else { layer.name.clone() },
            is_group,
            is_visible: !layer.hidden,
            opacity,
            blend_key: if is_group && (layer.blend_key == "pass" || layer.blend_key == "norm") { "pass".into() } else { layer.blend_key.clone() },
            clipping: layer.clipping,
            cropped_to_canvas: layer.cropped,
            bounds,
            image: if is_group { None } else { layer.image.clone() },
            mask: None,
            mask_bounds: Rect::default(),
            mask_default: 255,
            mask_enabled: true,
            mask_linked: true,
            adjustment: None,
            kind,
            is_shape: false,
            drawn_by_mac: None,
            text: false,
        };
        let text = if record.kind == Kind::Text { text::parses(&layer.extra) } else { false };
        if text {
            record.text = true;
        } else if !is_group && let Some(live) = vector::live(&layer.extra, canvas, remaining)? {
            record.image = None;
            record.bounds = live.bounds;
            record.is_shape = true;
            record.drawn_by_mac = Some("Photoshop shape layers");
            record.kind = Kind::Vector;
            remaining = (remaining - live.pixels).max(0);
        } else if record.image.is_none() && !is_group && let Some(raster) = vector::raster(&layer.extra, canvas, remaining)? {
            record.bounds = raster.bounds;
            record.drawn_by_mac = Some("Photoshop vector masks drawn as pixels");
            record.kind = Kind::Vector;
            remaining = (remaining - raster.pixels).max(0);
        }
        record.mask = if layer.mask_from_render { None } else { layer.mask_image.clone() };
        record.mask_bounds = Rect {
            x: layer.mask_left as f64,
            y: layer.mask_top as f64,
            width: (layer.mask_right - layer.mask_left) as f64,
            height: (layer.mask_bottom - layer.mask_top) as f64,
        };
        record.mask_default = layer.mask_default;
        record.mask_enabled = !layer.mask_disabled;
        record.mask_linked = layer.mask_linked;
        if !is_group {
            record.adjustment = adjustments::parse(&layer.extra);
        }
        if record.adjustment.is_some() {
            record.kind = Kind::Adjustment;
        }
        result.push(record);
    }
    if !groups.is_empty() {
        return Err(ImportError::Truncated);
    }
    Ok(result)
}

fn kind_of(layer: &RawLayer, is_group: bool) -> Kind {
    let has = |keys: &[&str]| keys.iter().any(|k| layer.extra.contains_key(*k));
    if is_group {
        Kind::Group
    } else if has(&["TySh", "tySh", "txt2"]) {
        Kind::Text
    } else if has(&["vmsk", "vsms", "vogk"]) {
        Kind::Vector
    } else if has(&["SoLd", "SoLE"]) {
        Kind::SmartObject
    } else if has(&["lfx2", "lrFX", "lmfx"]) {
        Kind::Effects
    } else if has(&ADJUSTMENT_KEYS) {
        Kind::Adjustment
    } else {
        Kind::Raster
    }
}

/// The merged image in the image data section, as ImageIO reads a layerless PSD: compression,
/// then each channel's plane (PackBits row counts for every channel come first). Only three
/// channel (opaque RGB) files are read; others aren't ported yet.
fn read_composite(cursor: &mut Cursor, channels: u16, width: u32, height: u32, is_psb: bool) -> Result<image::RgbaImage> {
    if channels != 3 {
        return Err(ImportError::NotPorted(format!("a layerless Photoshop file with {channels} channels")));
    }
    let compression = cursor.u16()?;
    let (w, h) = (width as usize, height as usize);
    let rest = &cursor.data[cursor.offset..];
    let planes: Vec<Vec<u8>> = match compression {
        0 => {
            if rest.len() < w * h * 3 {
                return Err(ImportError::Unreadable);
            }
            (0..3).map(|c| rest[c * w * h..(c + 1) * w * h].to_vec()).collect()
        }
        1 => {
            let size = if is_psb { 4 } else { 2 };
            let table = h * 3 * size;
            if rest.len() < table {
                return Err(ImportError::Unreadable);
            }
            let counts: Vec<usize> = (0..h * 3)
                .map(|i| if is_psb { be32(rest, i * 4) as usize } else { u16::from_be_bytes([rest[i * 2], rest[i * 2 + 1]]) as usize })
                .collect();
            let mut offset = table;
            let mut planes = Vec::new();
            for c in 0..3 {
                // Each plane as its own block: its row counts, then its rows.
                let mut block = Vec::new();
                let mut rows = Vec::new();
                for r in 0..h {
                    let n = counts[c * h + r];
                    if is_psb { block.extend_from_slice(&(n as u32).to_be_bytes()) } else { block.extend_from_slice(&(n as u16).to_be_bytes()) }
                    let end = offset.checked_add(n).filter(|&e| e <= rest.len()).ok_or(ImportError::Unreadable)?;
                    rows.extend_from_slice(&rest[offset..end]);
                    offset = end;
                }
                block.extend_from_slice(&rows);
                planes.push(unpack_rle(w, h, &block, is_psb, None).map_err(|_| ImportError::Unreadable)?);
            }
            planes
        }
        _ => return Err(ImportError::NotPorted(format!("a layerless Photoshop file with compression {compression}"))),
    };
    let mut out = image::RgbaImage::new(width, height);
    for (i, px) in out.pixels_mut().enumerate() {
        px.0 = [planes[0][i], planes[1][i], planes[2][i], 255];
    }
    Ok(out)
}
