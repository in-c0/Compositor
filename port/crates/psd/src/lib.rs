//! Writes Adobe Photoshop documents: PSD (version 1) and PSB (version 2, the large document
//! format), 8-bit RGB, with a layer tree of pixel layers, groups, layer masks and Levels, Curves
//! and Hue/Saturation adjustment layers, plus a flattened composite in the image data section.
//!
//! The layout follows Adobe's *Photoshop File Formats Specification* (File Header, Color Mode
//! Data, Image Resources, Layer and Mask Information, Image Data). Layers are listed bottom to
//! top, as the file stores them. A group is written the way Photoshop writes it: a bounding
//! section divider (`lsct` type 3), then the group's children, then the folder record itself
//! (`lsct` type 1 when open, 2 when closed), which carries the name, blend mode and opacity.
//!
//! [`write_flat`] writes layerless files in other color modes and depths, for checking how
//! readers treat files they can't import.

mod packbits;
#[cfg(test)]
mod tests;
mod zlib;

pub use packbits::{pack_bits, unpack_bits};

use anyhow::{Result, bail, ensure};
use image::{GrayImage, RgbaImage};

/// File format: PSD (version 1, sides up to 30,000 px) or PSB (version 2, sides up to 300,000 px,
/// 8-byte section and channel lengths, 4-byte RLE row counts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Psd,
    Psb,
}

impl Version {
    fn number(self) -> u16 {
        match self {
            Version::Psd => 1,
            Version::Psb => 2,
        }
    }

    fn max_side(self) -> u32 {
        match self {
            Version::Psd => 30_000,
            Version::Psb => 300_000,
        }
    }

    fn is_psb(self) -> bool {
        self == Version::Psb
    }
}

/// How layer channels are stored. The composite in the image data section is always RLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// 0: uncompressed rows.
    Raw,
    /// 1: PackBits, with a table of per-row byte counts first.
    Rle,
    /// 2: ZIP without prediction. Written as stored (uncompressed) deflate blocks: a valid zlib
    /// stream, meant for checking how readers treat this compression.
    Zip,
}

impl Compression {
    fn code(self) -> u16 {
        match self {
            Compression::Raw => 0,
            Compression::Rle => 1,
            Compression::Zip => 2,
        }
    }
}

/// Photoshop blend modes, by their four-character keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    /// `pass`: groups only.
    PassThrough,
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// Any other key, written as is.
    Other([u8; 4]),
}

impl Blend {
    /// Every mode a layer can have, in Photoshop's menu order.
    pub const LAYER_MODES: [Blend; 27] = [
        Blend::Normal,
        Blend::Dissolve,
        Blend::Darken,
        Blend::Multiply,
        Blend::ColorBurn,
        Blend::LinearBurn,
        Blend::DarkerColor,
        Blend::Lighten,
        Blend::Screen,
        Blend::ColorDodge,
        Blend::LinearDodge,
        Blend::LighterColor,
        Blend::Overlay,
        Blend::SoftLight,
        Blend::HardLight,
        Blend::VividLight,
        Blend::LinearLight,
        Blend::PinLight,
        Blend::HardMix,
        Blend::Difference,
        Blend::Exclusion,
        Blend::Subtract,
        Blend::Divide,
        Blend::Hue,
        Blend::Saturation,
        Blend::Color,
        Blend::Luminosity,
    ];

    pub fn key(self) -> [u8; 4] {
        match self {
            Blend::PassThrough => *b"pass",
            Blend::Normal => *b"norm",
            Blend::Dissolve => *b"diss",
            Blend::Darken => *b"dark",
            Blend::Multiply => *b"mul ",
            Blend::ColorBurn => *b"idiv",
            Blend::LinearBurn => *b"lbrn",
            Blend::DarkerColor => *b"dkCl",
            Blend::Lighten => *b"lite",
            Blend::Screen => *b"scrn",
            Blend::ColorDodge => *b"div ",
            Blend::LinearDodge => *b"lddg",
            Blend::LighterColor => *b"lgCl",
            Blend::Overlay => *b"over",
            Blend::SoftLight => *b"sLit",
            Blend::HardLight => *b"hLit",
            Blend::VividLight => *b"vLit",
            Blend::LinearLight => *b"lLit",
            Blend::PinLight => *b"pLit",
            Blend::HardMix => *b"hMix",
            Blend::Difference => *b"diff",
            Blend::Exclusion => *b"smud",
            Blend::Subtract => *b"fsub",
            Blend::Divide => *b"fdiv",
            Blend::Hue => *b"hue ",
            Blend::Saturation => *b"sat ",
            Blend::Color => *b"colr",
            Blend::Luminosity => *b"lum ",
            Blend::Other(key) => key,
        }
    }

    /// The name Photoshop shows in its menu.
    pub fn name(self) -> &'static str {
        match self {
            Blend::PassThrough => "Pass Through",
            Blend::Normal => "Normal",
            Blend::Dissolve => "Dissolve",
            Blend::Darken => "Darken",
            Blend::Multiply => "Multiply",
            Blend::ColorBurn => "Color Burn",
            Blend::LinearBurn => "Linear Burn",
            Blend::DarkerColor => "Darker Color",
            Blend::Lighten => "Lighten",
            Blend::Screen => "Screen",
            Blend::ColorDodge => "Color Dodge",
            Blend::LinearDodge => "Linear Dodge (Add)",
            Blend::LighterColor => "Lighter Color",
            Blend::Overlay => "Overlay",
            Blend::SoftLight => "Soft Light",
            Blend::HardLight => "Hard Light",
            Blend::VividLight => "Vivid Light",
            Blend::LinearLight => "Linear Light",
            Blend::PinLight => "Pin Light",
            Blend::HardMix => "Hard Mix",
            Blend::Difference => "Difference",
            Blend::Exclusion => "Exclusion",
            Blend::Subtract => "Subtract",
            Blend::Divide => "Divide",
            Blend::Hue => "Hue",
            Blend::Saturation => "Saturation",
            Blend::Color => "Color",
            Blend::Luminosity => "Luminosity",
            Blend::Other(_) => "Other",
        }
    }
}

/// A layer mask (user mask, channel -2). `left` and `top` place it on the document, not on the
/// layer; everywhere outside it the mask is `default_color`.
#[derive(Clone, Debug)]
pub struct Mask {
    pub left: i32,
    pub top: i32,
    pub pixels: GrayImage,
    pub default_color: u8,
    pub disabled: bool,
    /// Flag bit 0, "position relative to layer". Photoshop sets it on masks that are unlinked
    /// from their layer.
    pub relative_to_layer: bool,
}

impl Mask {
    /// A mask at (`left`, `top`) on the document whose outside is white (revealed).
    pub fn new(left: i32, top: i32, pixels: GrayImage) -> Self {
        Self { left, top, pixels, default_color: 255, disabled: false, relative_to_layer: false }
    }

    pub fn default_color(mut self, value: u8) -> Self {
        self.default_color = value;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn relative_to_layer(mut self) -> Self {
        self.relative_to_layer = true;
        self
    }

    fn value_at(&self, x: i32, y: i32) -> u8 {
        let (mx, my) = (x - self.left, y - self.top);
        if mx >= 0 && my >= 0 && (mx as u32) < self.pixels.width() && (my as u32) < self.pixels.height() {
            self.pixels.get_pixel(mx as u32, my as u32)[0]
        } else {
            self.default_color
        }
    }
}

/// One Levels record, as Photoshop stores it: input black and white points, output black and
/// white points (0-255), and gamma (0.10-9.99, stored in hundredths).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelsRecord {
    pub input_black: u16,
    pub input_white: u16,
    pub output_black: u16,
    pub output_white: u16,
    pub gamma: f64,
}

impl LevelsRecord {
    pub const IDENTITY: Self = Self { input_black: 0, input_white: 255, output_black: 0, output_white: 255, gamma: 1.0 };
}

impl Default for LevelsRecord {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Levels: records for the composite (RGB), red, green and blue.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    pub records: [LevelsRecord; 4],
}

/// Curves: for the composite (RGB), red, green and blue, an optional list of (input, output)
/// points. Channels left as `None` are not written, as Photoshop omits untouched curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curves {
    pub channels: [Option<Vec<(u8, u8)>>; 4],
}

/// One Hue/Saturation range: its band in degrees (where it fades in, starts, ends and fades
/// out) and its hue, saturation and lightness shifts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueRange {
    pub band: [i16; 4],
    pub hue: i16,
    pub saturation: i16,
    pub lightness: i16,
}

/// Hue/Saturation (`hue2`, version 2). With `colorize` on, `colorize_values` (hue 0-360,
/// saturation 0-100, lightness -100-100) apply; otherwise `master` and the six ranges (reds,
/// yellows, greens, cyans, blues, magentas).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueSaturation {
    pub colorize: bool,
    pub colorize_values: [i16; 3],
    pub master: [i16; 3],
    pub ranges: [HueRange; 6],
}

impl Default for HueSaturation {
    /// Photoshop's defaults: no change, and its standard bands.
    fn default() -> Self {
        let band = |center: i16| {
            let wrap = |v: i16| v.rem_euclid(360);
            HueRange { band: [wrap(center - 45), wrap(center - 15), wrap(center + 15), wrap(center + 45)], hue: 0, saturation: 0, lightness: 0 }
        };
        Self {
            colorize: false,
            colorize_values: [0, 25, 0],
            master: [0, 0, 0],
            ranges: [band(0), band(60), band(120), band(180), band(240), band(300)],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Adjustment {
    Levels(Levels),
    Curves(Curves),
    HueSaturation(HueSaturation),
}

impl Adjustment {
    fn key(&self) -> [u8; 4] {
        match self {
            Adjustment::Levels(_) => *b"levl",
            Adjustment::Curves(_) => *b"curv",
            Adjustment::HueSaturation(_) => *b"hue2",
        }
    }

    /// The additional layer information payload for this adjustment.
    pub fn payload(&self) -> Vec<u8> {
        let mut b = Buf::default();
        match self {
            Adjustment::Levels(levels) => {
                // Version 2, then 29 records; Photoshop uses the first four for RGB documents.
                b.u16(2);
                for i in 0..29 {
                    let r = levels.records.get(i).copied().unwrap_or(LevelsRecord::IDENTITY);
                    b.u16(r.input_black);
                    b.u16(r.input_white);
                    b.u16(r.output_black);
                    b.u16(r.output_white);
                    b.u16((r.gamma * 100.0).round().clamp(10.0, 999.0) as u16);
                }
            }
            Adjustment::Curves(curves) => {
                // A filler byte, version 1, a bit mask of the channels that follow, then each
                // curve as a point count and (output, input) pairs.
                b.u8(0);
                b.u16(1);
                let mask = curves.channels.iter().enumerate().filter(|(_, c)| c.is_some()).fold(0u32, |m, (i, _)| m | 1 << i);
                b.u32(mask);
                for points in curves.channels.iter().flatten() {
                    b.u16(points.len() as u16);
                    for &(input, output) in points {
                        b.u16(output as u16);
                        b.u16(input as u16);
                    }
                }
            }
            Adjustment::HueSaturation(h) => {
                b.u16(2);
                b.u8(u8::from(h.colorize));
                b.u8(0);
                for v in h.colorize_values.iter().chain(&h.master) {
                    b.i16(*v);
                }
                for range in &h.ranges {
                    for v in range.band {
                        b.i16(v);
                    }
                    b.i16(range.hue);
                    b.i16(range.saturation);
                    b.i16(range.lightness);
                }
            }
        }
        b.0
    }
}

/// A layer: pixels (straight-alpha RGBA placed at `left`, `top`, possibly partly or wholly off
/// the canvas), an adjustment, or nothing.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub left: i32,
    pub top: i32,
    pub pixels: Option<RgbaImage>,
    pub visible: bool,
    pub opacity: u8,
    /// Fill opacity (`iOpa`), written only when it isn't 255.
    pub fill: u8,
    pub blend: Blend,
    pub clipping: bool,
    pub mask: Option<Mask>,
    pub adjustment: Option<Adjustment>,
    /// Additional layer information written verbatim after the standard blocks.
    pub extra: Vec<([u8; 4], Vec<u8>)>,
}

impl Layer {
    /// A layer with no pixels and no adjustment.
    pub fn empty(name: &str) -> Self {
        Self {
            name: name.into(),
            left: 0,
            top: 0,
            pixels: None,
            visible: true,
            opacity: 255,
            fill: 255,
            blend: Blend::Normal,
            clipping: false,
            mask: None,
            adjustment: None,
            extra: Vec::new(),
        }
    }

    pub fn pixels(name: &str, left: i32, top: i32, pixels: RgbaImage) -> Self {
        Self { left, top, pixels: Some(pixels), ..Self::empty(name) }
    }

    pub fn adjustment(name: &str, adjustment: Adjustment) -> Self {
        Self { adjustment: Some(adjustment), ..Self::empty(name) }
    }

    pub fn opacity(mut self, value: u8) -> Self {
        self.opacity = value;
        self
    }

    pub fn fill(mut self, value: u8) -> Self {
        self.fill = value;
        self
    }

    pub fn blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    pub fn clipped(mut self) -> Self {
        self.clipping = true;
        self
    }

    pub fn mask(mut self, mask: Mask) -> Self {
        self.mask = Some(mask);
        self
    }

    pub fn extra(mut self, key: [u8; 4], payload: Vec<u8>) -> Self {
        self.extra.push((key, payload));
        self
    }
}

/// A group (layer folder). Children are bottom to top.
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    /// Written both as the folder record's blend key and in its `lsct` block.
    pub blend: Blend,
    pub closed: bool,
    pub clipping: bool,
    pub mask: Option<Mask>,
    pub children: Vec<Node>,
}

impl Group {
    pub fn new(name: &str, children: Vec<Node>) -> Self {
        Self { name: name.into(), visible: true, opacity: 255, blend: Blend::PassThrough, closed: false, clipping: false, mask: None, children }
    }

    pub fn opacity(mut self, value: u8) -> Self {
        self.opacity = value;
        self
    }

    pub fn blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    pub fn closed(mut self) -> Self {
        self.closed = true;
        self
    }

    pub fn clipped(mut self) -> Self {
        self.clipping = true;
        self
    }

    pub fn mask(mut self, mask: Mask) -> Self {
        self.mask = Some(mask);
        self
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Layer(Layer),
    Group(Group),
}

impl From<Layer> for Node {
    fn from(layer: Layer) -> Self {
        Node::Layer(layer)
    }
}

impl From<Group> for Node {
    fn from(group: Group) -> Self {
        Node::Group(group)
    }
}

/// An 8-bit RGB document. `layers` are bottom to top.
#[derive(Clone, Debug)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    /// Pixels per inch, written as the ResolutionInfo resource (1005).
    pub resolution: f64,
    pub layers: Vec<Node>,
}

impl Document {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, resolution: 72.0, layers: Vec::new() }
    }

    pub fn push(&mut self, node: impl Into<Node>) -> &mut Self {
        self.layers.push(node.into());
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WriteOptions {
    pub version: Version,
    pub compression: Compression,
    /// Write each name as a `luni` block as well as the legacy Pascal string.
    pub unicode_names: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self { version: Version::Psd, compression: Compression::Rle, unicode_names: true }
    }
}

impl WriteOptions {
    pub fn psd() -> Self {
        Self::default()
    }

    pub fn psb() -> Self {
        Self { version: Version::Psb, ..Self::default() }
    }

    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    pub fn without_unicode_names(mut self) -> Self {
        self.unicode_names = false;
        self
    }

    /// The file extension Photoshop uses for this version.
    pub fn extension(&self) -> &'static str {
        match self.version {
            Version::Psd => "psd",
            Version::Psb => "psb",
        }
    }
}

/// Writes `doc` as a layered, 8-bit RGB Photoshop file.
pub fn write(doc: &Document, options: &WriteOptions) -> Result<Vec<u8>> {
    let version = options.version;
    let psb = version.is_psb();
    check_size(doc.width, doc.height, version)?;

    let mut file = Buf::default();
    // Four channels: the composite carries its transparency, signalled by a negative layer count.
    header(&mut file, version, 4, doc.width, doc.height, 8, 3);
    file.u32(0); // Color mode data: none for RGB.
    resources(&mut file, doc.resolution);

    let mut records = Vec::new();
    flatten(&doc.layers, options, &mut records)?;
    ensure!(records.len() <= i16::MAX as usize, "too many layer records ({})", records.len());

    let mut info = Buf::default();
    info.i16(-(records.len() as i16));
    let mut channel_data = Buf::default();
    for record in &records {
        let encoded: Vec<Vec<u8>> = record.channels.iter().map(|c| encode_channel(c, options.compression, psb)).collect::<Result<_>>()?;
        write_record(&mut info, record, &encoded, psb)?;
        for e in &encoded {
            channel_data.bytes(e);
        }
    }
    info.bytes(&channel_data.0);
    if info.0.len() % 2 == 1 {
        info.u8(0);
    }
    let mut section = Buf::default();
    section.length(psb, info.0.len())?;
    section.bytes(&info.0);
    section.u32(0); // Global layer mask info: none.
    file.length(psb, section.0.len())?;
    file.bytes(&section.0);

    let planes = merged_planes(doc);
    let (w, h) = (doc.width as usize, doc.height as usize);
    image_data(&mut file, &planes, w, h, Compression::Rle, psb)?;
    Ok(file.0)
}

/// Color modes, by their header codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Bitmap,
    Grayscale,
    Indexed,
    Rgb,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
}

impl ColorMode {
    fn code(self) -> u16 {
        match self {
            ColorMode::Bitmap => 0,
            ColorMode::Grayscale => 1,
            ColorMode::Indexed => 2,
            ColorMode::Rgb => 3,
            ColorMode::Cmyk => 4,
            ColorMode::Multichannel => 7,
            ColorMode::Duotone => 8,
            ColorMode::Lab => 9,
        }
    }
}

/// A layerless image: only the image data section. `planes` hold one sample per pixel, row by
/// row, in the file's channel order (CMYK samples are stored inverted: 255 is no ink).
#[derive(Clone, Debug)]
pub struct FlatImage {
    pub width: u32,
    pub height: u32,
    pub mode: ColorMode,
    /// 8 or 16 bits per sample.
    pub depth: u16,
    pub planes: Vec<Vec<u16>>,
    pub resolution: f64,
}

/// Writes a file with no layers, only the flattened image, in any grayscale, RGB, CMYK,
/// multichannel or Lab mode at 8 or 16 bits. Raw or RLE only.
pub fn write_flat(image: &FlatImage, options: &WriteOptions) -> Result<Vec<u8>> {
    let psb = options.version.is_psb();
    check_size(image.width, image.height, options.version)?;
    ensure!(image.depth == 8 || image.depth == 16, "depth must be 8 or 16, not {}", image.depth);
    ensure!(
        !matches!(image.mode, ColorMode::Bitmap | ColorMode::Indexed | ColorMode::Duotone),
        "{:?} needs color mode data or 1-bit samples, which this writer doesn't produce",
        image.mode
    );
    ensure!((1..=56).contains(&image.planes.len()), "a file has 1 to 56 channels");
    ensure!(options.compression != Compression::Zip, "flat files are written raw or RLE");
    let (w, h) = (image.width as usize, image.height as usize);
    let bytes_per_sample = image.depth as usize / 8;
    let mut planes = Vec::new();
    for plane in &image.planes {
        ensure!(plane.len() == w * h, "a plane has {} samples, not {}", plane.len(), w * h);
        let mut bytes = Vec::with_capacity(plane.len() * bytes_per_sample);
        for &v in plane {
            if image.depth == 8 {
                ensure!(v <= 255, "an 8-bit sample is {v}");
                bytes.push(v as u8);
            } else {
                bytes.extend_from_slice(&v.to_be_bytes());
            }
        }
        planes.push(bytes);
    }
    let mut file = Buf::default();
    header(&mut file, options.version, image.planes.len() as u16, image.width, image.height, image.depth, image.mode.code());
    file.u32(0);
    resources(&mut file, image.resolution);
    file.length(psb, 0)?; // No layer and mask information.
    image_data(&mut file, &planes, w * bytes_per_sample, h, options.compression, psb)?;
    Ok(file.0)
}

fn check_size(width: u32, height: u32, version: Version) -> Result<()> {
    let max = version.max_side();
    ensure!((1..=max).contains(&width) && (1..=max).contains(&height), "a {version:?} is 1 to {max} pixels a side, not {width} x {height}");
    Ok(())
}

fn header(file: &mut Buf, version: Version, channels: u16, width: u32, height: u32, depth: u16, mode: u16) {
    file.bytes(b"8BPS");
    file.u16(version.number());
    file.bytes(&[0; 6]);
    file.u16(channels);
    file.u32(height);
    file.u32(width);
    file.u16(depth);
    file.u16(mode);
}

/// Image resources: ResolutionInfo (1005), horizontal and vertical resolution in 16.16 fixed
/// point pixels per inch, each followed by its display units (1, inches).
fn resources(file: &mut Buf, resolution: f64) {
    let fixed = (resolution.clamp(1.0, 9600.0) * 65536.0).round() as u32;
    let mut r = Buf::default();
    r.bytes(b"8BIM");
    r.u16(1005);
    r.bytes(&[0, 0]); // Empty Pascal name, padded to even.
    r.u32(16);
    for _ in 0..2 {
        r.u32(fixed);
        r.u16(1);
        r.u16(1);
    }
    file.u32(r.0.len() as u32);
    file.bytes(&r.0);
}

/// Writes the image data section: compression, then every channel's rows. RLE puts every
/// row's byte count (all channels) before the packed rows.
fn image_data(file: &mut Buf, planes: &[Vec<u8>], row_bytes: usize, rows: usize, compression: Compression, psb: bool) -> Result<()> {
    file.u16(compression.code());
    match compression {
        Compression::Raw => {
            for p in planes {
                file.bytes(p);
            }
        }
        Compression::Rle => {
            let packed: Vec<Vec<u8>> = planes.iter().flat_map(|p| p.chunks(row_bytes.max(1)).take(rows).map(pack_bits)).collect();
            for row in &packed {
                row_count(file, row.len(), psb)?;
            }
            for row in &packed {
                file.bytes(row);
            }
        }
        Compression::Zip => bail!("the image data section is written raw or RLE"),
    }
    Ok(())
}

fn row_count(buf: &mut Buf, len: usize, psb: bool) -> Result<()> {
    if psb {
        buf.u32(u32::try_from(len)?);
    } else {
        buf.u16(u16::try_from(len)?);
    }
    Ok(())
}

/// One channel of a layer record, before encoding.
struct Channel {
    id: i16,
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Channel {
    fn empty(id: i16) -> Self {
        Self { id, width: 0, height: 0, data: Vec::new() }
    }
}

struct Record<'a> {
    /// Top, left, bottom, right.
    rect: [i32; 4],
    channels: Vec<Channel>,
    blend: [u8; 4],
    opacity: u8,
    clipping: bool,
    flags: u8,
    mask: Option<&'a Mask>,
    name: &'a str,
    extras: Vec<([u8; 4], Vec<u8>)>,
}

/// Layer flags: bit 1 hidden; bit 3 says bit 4 is meaningful; bit 4 says the pixel data doesn't
/// affect the document's appearance (set on folders and dividers, as Photoshop does).
const FLAG_HIDDEN: u8 = 0x02;
const FLAG_PS5: u8 = 0x08;
const FLAG_NO_PIXELS: u8 = 0x10;

fn flatten<'a>(nodes: &'a [Node], options: &WriteOptions, out: &mut Vec<Record<'a>>) -> Result<()> {
    for node in nodes {
        match node {
            Node::Layer(layer) => out.push(layer_record(layer, options)?),
            Node::Group(group) => {
                out.push(Record {
                    rect: [0; 4],
                    channels: empty_channels(),
                    blend: *b"norm",
                    opacity: 255,
                    clipping: false,
                    flags: FLAG_PS5 | FLAG_NO_PIXELS,
                    mask: None,
                    name: "</Layer group>",
                    extras: section_extras("</Layer group>", 3, None, options),
                });
                flatten(&group.children, options, out)?;
                let mut channels = empty_channels();
                if let Some(mask) = &group.mask {
                    channels.push(mask_channel(mask));
                }
                let kind = if group.closed { 2 } else { 1 };
                out.push(Record {
                    rect: [0; 4],
                    channels,
                    blend: group.blend.key(),
                    opacity: group.opacity,
                    clipping: group.clipping,
                    flags: FLAG_PS5 | FLAG_NO_PIXELS | if group.visible { 0 } else { FLAG_HIDDEN },
                    mask: group.mask.as_ref(),
                    name: &group.name,
                    extras: section_extras(&group.name, kind, Some(group.blend.key()), options),
                });
            }
        }
    }
    Ok(())
}

fn empty_channels() -> Vec<Channel> {
    [-1, 0, 1, 2].into_iter().map(Channel::empty).collect()
}

fn section_extras(name: &str, kind: u32, blend: Option<[u8; 4]>, options: &WriteOptions) -> Vec<([u8; 4], Vec<u8>)> {
    let mut extras = Vec::new();
    if options.unicode_names {
        extras.push((*b"luni", unicode_name(name)));
    }
    let mut lsct = Buf::default();
    lsct.u32(kind);
    if let Some(key) = blend {
        lsct.bytes(b"8BIM");
        lsct.bytes(&key);
    }
    extras.push((*b"lsct", lsct.0));
    extras
}

fn layer_record<'a>(layer: &'a Layer, options: &WriteOptions) -> Result<Record<'a>> {
    let mut channels;
    let mut rect = [0; 4];
    match &layer.pixels {
        Some(img) if img.width() > 0 && img.height() > 0 => {
            let (w, h) = (img.width() as usize, img.height() as usize);
            let right = layer.left.checked_add(i32::try_from(w)?);
            let bottom = layer.top.checked_add(i32::try_from(h)?);
            let (Some(right), Some(bottom)) = (right, bottom) else { bail!("layer '{}' reaches past the coordinate range", layer.name) };
            rect = [layer.top, layer.left, bottom, right];
            let raw = img.as_raw();
            let plane = |c: usize| raw.as_chunks::<4>().0.iter().map(|p| p[c]).collect::<Vec<u8>>();
            channels = vec![
                Channel { id: -1, width: w, height: h, data: plane(3) },
                Channel { id: 0, width: w, height: h, data: plane(0) },
                Channel { id: 1, width: w, height: h, data: plane(1) },
                Channel { id: 2, width: w, height: h, data: plane(2) },
            ];
        }
        _ => channels = empty_channels(),
    }
    if let Some(mask) = &layer.mask {
        channels.push(mask_channel(mask));
    }
    let mut extras = Vec::new();
    if let Some(adjustment) = &layer.adjustment {
        extras.push((adjustment.key(), adjustment.payload()));
    }
    if options.unicode_names {
        extras.push((*b"luni", unicode_name(&layer.name)));
    }
    if layer.fill != 255 {
        extras.push((*b"iOpa", vec![layer.fill, 0, 0, 0]));
    }
    extras.extend(layer.extra.iter().cloned());
    Ok(Record {
        rect,
        channels,
        blend: layer.blend.key(),
        opacity: layer.opacity,
        clipping: layer.clipping,
        flags: FLAG_PS5 | if layer.visible { 0 } else { FLAG_HIDDEN },
        mask: layer.mask.as_ref(),
        name: &layer.name,
        extras,
    })
}

fn mask_channel(mask: &Mask) -> Channel {
    Channel { id: -2, width: mask.pixels.width() as usize, height: mask.pixels.height() as usize, data: mask.pixels.as_raw().clone() }
}

/// Compression, then the channel's data. Channels with no pixels are just a raw compression code.
fn encode_channel(channel: &Channel, compression: Compression, psb: bool) -> Result<Vec<u8>> {
    let mut out = Buf::default();
    if channel.width == 0 || channel.height == 0 {
        out.u16(0);
        return Ok(out.0);
    }
    match compression {
        Compression::Raw | Compression::Rle => image_data(&mut out, std::slice::from_ref(&channel.data), channel.width, channel.height, compression, psb)?,
        Compression::Zip => {
            out.u16(compression.code());
            out.bytes(&zlib::stored(&channel.data));
        }
    }
    Ok(out.0)
}

fn write_record(buf: &mut Buf, record: &Record, encoded: &[Vec<u8>], psb: bool) -> Result<()> {
    for v in record.rect {
        buf.i32(v);
    }
    buf.u16(record.channels.len() as u16);
    for (channel, data) in record.channels.iter().zip(encoded) {
        buf.i16(channel.id);
        buf.length(psb, data.len())?;
    }
    buf.bytes(b"8BIM");
    buf.bytes(&record.blend);
    buf.u8(record.opacity);
    buf.u8(u8::from(record.clipping));
    buf.u8(record.flags);
    buf.u8(0);

    let mut extra = Buf::default();
    match record.mask {
        Some(mask) => {
            // Rectangle, default color, flags, and two bytes of padding.
            extra.u32(20);
            let (w, h) = (i32::try_from(mask.pixels.width())?, i32::try_from(mask.pixels.height())?);
            extra.i32(mask.top);
            extra.i32(mask.left);
            extra.i32(mask.top + h);
            extra.i32(mask.left + w);
            extra.u8(mask.default_color);
            extra.u8(u8::from(mask.relative_to_layer) | if mask.disabled { 2 } else { 0 });
            extra.u16(0);
        }
        None => extra.u32(0),
    }
    // Blending ranges: composite gray and each of the four channels, source and destination,
    // each the full 0-255 with no feathering.
    extra.u32(40);
    for _ in 0..10 {
        extra.bytes(&[0, 0, 255, 255]);
    }
    // Legacy name: a Pascal string padded to a multiple of four bytes, ASCII only here; the
    // `luni` block carries the real name.
    let legacy: Vec<u8> = record.name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'?' }).take(255).collect();
    extra.u8(legacy.len() as u8);
    extra.bytes(&legacy);
    extra.bytes(&[0; 3][..(4 - (legacy.len() + 1) % 4) % 4]);
    for (key, payload) in &record.extras {
        additional_info(&mut extra, *key, payload, psb)?;
    }
    buf.u32(u32::try_from(extra.0.len())?);
    buf.bytes(&extra.0);
    Ok(())
}

/// Keys whose length is 8 bytes in a PSB.
const PSB_LONG_KEYS: [&[u8; 4]; 13] = [b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD"];

/// Additional layer information: signature, key, length, payload. The payload is padded to an
/// even length and the length counts the padding.
fn additional_info(buf: &mut Buf, key: [u8; 4], payload: &[u8], psb: bool) -> Result<()> {
    let padded = payload.len() + payload.len() % 2;
    buf.bytes(b"8BIM");
    buf.bytes(&key);
    if psb && PSB_LONG_KEYS.contains(&&key) {
        buf.u64(padded as u64);
    } else {
        buf.u32(u32::try_from(padded)?);
    }
    buf.bytes(payload);
    if payload.len() % 2 == 1 {
        buf.u8(0);
    }
    Ok(())
}

/// `luni`: a count of UTF-16 code units, the units big-endian, then zero padding to a multiple
/// of four bytes (outside the count).
fn unicode_name(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut b = Buf::default();
    b.u32(units.len() as u32);
    for u in units {
        b.u16(u);
    }
    while b.0.len() % 4 != 0 {
        b.u8(0);
    }
    b.0
}

/// The flattened image for the image data section: visible pixel layers, Normal over Normal,
/// scaled by opacity, fill, enabled masks and enclosing group opacity. Blend modes, clipping,
/// group masks and adjustments are ignored. Color is matted over white, as Photoshop stores the
/// composite of a document with transparency, and alpha is the fourth plane.
fn merged_planes(doc: &Document) -> Vec<Vec<u8>> {
    let (w, h) = (doc.width as usize, doc.height as usize);
    let mut canvas = vec![[0f32; 4]; w * h];
    draw(&doc.layers, &mut canvas, w, h, 1.0);
    let mut planes: Vec<Vec<u8>> = (0..4).map(|_| Vec::with_capacity(w * h)).collect();
    for px in &canvas {
        let a = px[3].clamp(0.0, 1.0);
        for c in 0..3 {
            planes[c].push(((px[c] + (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        planes[3].push((a * 255.0).round() as u8);
    }
    planes
}

fn draw(nodes: &[Node], canvas: &mut [[f32; 4]], w: usize, h: usize, factor: f32) {
    for node in nodes {
        match node {
            Node::Group(g) if g.visible => draw(&g.children, canvas, w, h, factor * g.opacity as f32 / 255.0),
            Node::Layer(layer) if layer.visible && layer.adjustment.is_none() => {
                let Some(img) = &layer.pixels else { continue };
                let f = factor * layer.opacity as f32 / 255.0 * layer.fill as f32 / 255.0;
                let mask = layer.mask.as_ref().filter(|m| !m.disabled);
                for (x, y, p) in img.enumerate_pixels() {
                    let (cx, cy) = (layer.left as i64 + x as i64, layer.top as i64 + y as i64);
                    if cx < 0 || cy < 0 || cx >= w as i64 || cy >= h as i64 {
                        continue;
                    }
                    let mut a = p[3] as f32 / 255.0 * f;
                    if let Some(m) = mask {
                        a *= m.value_at(cx as i32, cy as i32) as f32 / 255.0;
                    }
                    if a <= 0.0 {
                        continue;
                    }
                    let d = &mut canvas[cy as usize * w + cx as usize];
                    for c in 0..3 {
                        d[c] = p[c] as f32 / 255.0 * a + d[c] * (1.0 - a);
                    }
                    d[3] = a + d[3] * (1.0 - a);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Buf(Vec<u8>);

impl Buf {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    /// A section or channel length: 4 bytes in a PSD, 8 in a PSB.
    fn length(&mut self, psb: bool, len: usize) -> Result<()> {
        if psb {
            self.u64(len as u64);
        } else {
            self.u32(u32::try_from(len)?);
        }
        Ok(())
    }
}
