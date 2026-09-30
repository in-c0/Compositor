//! Photoshop import cases. Every input is a `.psd` or `.psb` written by the `psd` crate, and the
//! harness records what Compositor's reader makes of it: which layers, groups, masks, clipping
//! and adjustments survive, and what it converts, drops or rejects. Canvases are 64 x 64.

use super::builder::CaseWriter;
use super::images::{self, Alpha};
use ::psd::{
    Adjustment, Blend, ColorMode, Compression, Curves, Document, FlatImage, Group, HueRange, HueSaturation, Layer, Levels,
    LevelsRecord, Mask, Node, WriteOptions,
};
use anyhow::Result;
use image::GrayImage;

const N: u32 = 64;

pub fn psd(w: &mut CaseWriter) -> Result<()> {
    blend_modes(w)?;
    opacity(w)?;
    visibility_and_bounds(w)?;
    names(w)?;
    masks(w)?;
    groups(w)?;
    clipping(w)?;
    adjustments(w)?;
    encoding(w)?;
    rejected(w)?;
    Ok(())
}

fn put(w: &mut CaseWriter, case: &str, label: &str, doc: &Document, options: WriteOptions) -> Result<()> {
    let bytes = ::psd::write(doc, &options)?;
    w.write_file("psd", case, label, &format!("input.{}", options.extension()), &bytes, vec![])
}

fn put_flat(w: &mut CaseWriter, case: &str, label: &str, image: &FlatImage) -> Result<()> {
    let bytes = ::psd::write_flat(image, &WriteOptions::psd())?;
    w.write_file("psd", case, label, "input.psd", &bytes, vec![])
}

fn doc(layers: Vec<Node>) -> Document {
    let mut d = Document::new(N, N);
    d.layers = layers;
    d
}

fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    s.trim_matches('-').to_string()
}

fn key(blend: Blend) -> String {
    String::from_utf8_lossy(&blend.key()).trim_end().to_string()
}

fn backdrop() -> Node {
    Layer::pixels("Backdrop", 0, 0, images::photo(N, N)).into()
}

fn noise(seed: u32) -> Layer {
    Layer::pixels("Noise", 0, 0, images::noise(N, N, seed, Alpha::Varied))
}

fn disc(name: &str, color: [u8; 3]) -> Layer {
    Layer::pixels(name, 8, 8, images::disc(48, 48, color))
}

fn blend_modes(w: &mut CaseWriter) -> Result<()> {
    for mode in Blend::LAYER_MODES {
        let mapped = !matches!(mode, Blend::Dissolve | Blend::DarkerColor | Blend::LighterColor);
        let case = format!("blend-{}", slug(mode.name()));
        let d = doc(vec![backdrop(), noise(2).blend(mode).opacity(153).into()]);
        let label = if mapped {
            format!("{} ('{}') at 60% over a photo", mode.name(), key(mode))
        } else {
            format!("{} ('{}') at 60% over a photo; the Mac reader has no such mode and applies Normal", mode.name(), key(mode))
        };
        put(w, &case, &label, &d, WriteOptions::psd())?;
    }
    Ok(())
}

fn opacity(w: &mut CaseWriter) -> Result<()> {
    let cases: [(&str, &str, u8, u8, Blend); 5] = [
        ("opacity-half", "Opacity 50%, fill 100%", 128, 255, Blend::Normal),
        ("fill-half", "Fill 50% ('iOpa'), opacity 100%", 255, 128, Blend::Normal),
        ("opacity-and-fill", "Opacity 75% and fill 40%, which the Mac reader multiplies", 191, 102, Blend::Normal),
        ("fill-hard-mix", "Hard Mix at fill 50%, which Photoshop renders differently from opacity 50%", 255, 128, Blend::HardMix),
        ("opacity-zero", "Opacity 0% with fill 100%", 0, 255, Blend::Normal),
    ];
    for (case, label, opacity, fill, blend) in cases {
        let d = doc(vec![backdrop(), noise(3).opacity(opacity).fill(fill).blend(blend).into()]);
        put(w, case, label, &d, WriteOptions::psd())?;
    }
    Ok(())
}

fn visibility_and_bounds(w: &mut CaseWriter) -> Result<()> {
    let d = doc(vec![backdrop(), noise(4).hidden().into(), disc("Disc", [40, 90, 220]).into()]);
    put(w, "hidden-layer", "A hidden layer between two visible ones", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Group::new("Hidden folder", vec![disc("Disc", [220, 60, 40]).into()]).hidden().into()]);
    put(w, "hidden-group", "A hidden folder whose child is visible", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Layer::pixels("Overhang", -16, 36, images::noise(48, 40, 5, Alpha::Varied)).into()]);
    put(w, "offcanvas-partial", "A 48 x 40 layer at (-16, 36), hanging off the left and bottom edges", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Layer::pixels("Oversized", -16, -24, images::checker(96, 112, 8)).opacity(200).into()]);
    put(w, "offcanvas-all-sides", "A 96 x 112 layer at (-16, -24), larger than the canvas on every side", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Layer::pixels("Outside", 80, 10, images::noise(24, 24, 6, Alpha::Opaque)).into()]);
    put(w, "offcanvas-entirely", "A layer placed wholly to the right of the canvas", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Layer::empty("Empty").into(), disc("Disc", [250, 200, 30]).opacity(200).into()]);
    put(w, "empty-layer", "A pixel layer with no pixels (an empty rectangle)", &d, WriteOptions::psd())?;

    let mut d = doc(vec![backdrop(), disc("Disc", [30, 200, 120]).into()]);
    d.resolution = 300.0;
    put(w, "resolution-300", "A 300 ppi document", &d, WriteOptions::psd())?;
    Ok(())
}

fn names(w: &mut CaseWriter) -> Result<()> {
    let d = doc(vec![
        Layer::pixels("Ebene 1 — Überlagerung", 0, 0, images::photo(N, N)).into(),
        Group::new("レイヤーグループ", vec![disc("🎨 Farbe", [200, 40, 160]).opacity(220).into()]).into(),
        Layer::pixels("A long layer name that runs past the thirty-one characters old Photoshop kept", 0, 0, images::noise(N, N, 7, Alpha::Varied)).opacity(90).into(),
    ]);
    put(w, "unicode-names", "Layer and folder names with accents, CJK, an emoji (a UTF-16 surrogate pair) and a long name, in 'luni' blocks", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), disc("Legacy name only", [90, 90, 240]).into()]);
    put(w, "legacy-names", "Names only in the Pascal string, with no 'luni' block", &d, WriteOptions::psd().without_unicode_names())?;
    Ok(())
}

fn masks(w: &mut CaseWriter) -> Result<()> {
    let masked = |mask: Mask| doc(vec![backdrop(), Layer::pixels("Masked", 0, 0, images::noise(N, N, 8, Alpha::Opaque)).mask(mask).into()]);
    let full = || Mask::new(0, 0, images::mask_mixed(N, N));
    let small = || Mask::new(16, 20, images::gray_ramp(24, 16, true));

    put(w, "mask-enabled", "A full-size mask: black, a ramp, then white", &masked(full()), WriteOptions::psd())?;
    put(w, "mask-disabled", "The same mask, disabled (flag bit 1)", &masked(full().disabled()), WriteOptions::psd())?;
    put(w, "mask-small-default-white", "A 24 x 16 ramp mask at (16, 20) on a full-size layer; white outside it", &masked(small()), WriteOptions::psd())?;
    put(w, "mask-small-default-black", "A 24 x 16 ramp mask at (16, 20) on a full-size layer; black outside it", &masked(small().default_color(0)), WriteOptions::psd())?;
    put(w, "mask-relative-flag", "A full-size mask with flag bit 0 (position relative to layer), which the Mac reader reads as unlinked", &masked(full().relative_to_layer()), WriteOptions::psd())?;
    put(w, "mask-empty-black", "A mask with an empty rectangle and a black default: Photoshop's Hide All", &masked(Mask::new(0, 0, GrayImage::new(0, 0)).default_color(0)), WriteOptions::psd())?;

    let d = doc(vec![backdrop(), disc("Disc", [240, 120, 20]).mask(Mask::new(30, -6, images::gray_ramp(40, 30, false)).default_color(0)).into()]);
    put(w, "mask-offset", "A 48 x 48 layer at (8, 8) with a 40 x 30 mask at (30, -6), partly outside both the layer and the canvas", &d, WriteOptions::psd())?;

    let d = doc(vec![
        backdrop(),
        Group::new("Masked folder", vec![noise(9).into(), disc("Disc", [20, 20, 20]).opacity(180).into()]).mask(Mask::new(0, 0, images::gray_ramp(N, N, true))).into(),
    ]);
    put(w, "mask-on-group", "A folder with a horizontal ramp mask", &d, WriteOptions::psd())?;
    Ok(())
}

fn groups(w: &mut CaseWriter) -> Result<()> {
    let d = doc(vec![
        backdrop(),
        Group::new(
            "Outer",
            vec![
                noise(10).opacity(200).into(),
                Group::new("Inner", vec![disc("Multiply disc", [60, 160, 230]).blend(Blend::Multiply).opacity(179).into()]).into(),
            ],
        )
        .into(),
    ]);
    put(w, "group-pass-through-nested", "Two nested pass-through folders, a Multiply layer at 70% in the inner one", &d, WriteOptions::psd())?;

    let children = || -> Vec<Node> { vec![noise(11).blend(Blend::Screen).opacity(200).into(), disc("Disc", [230, 40, 90]).blend(Blend::Overlay).into()] };
    let d = doc(vec![backdrop(), Group::new("Normal folder", children()).blend(Blend::Normal).opacity(153).into()]);
    put(w, "group-normal-mode", "A folder set to Normal ('norm', isolated in Photoshop) at 60%, holding Screen and Overlay layers", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Group::new("Multiply folder", children()).blend(Blend::Multiply).opacity(204).into()]);
    put(w, "group-multiply-mode", "A folder set to Multiply at 80%; the Mac reader makes it pass-through with a note", &d, WriteOptions::psd())?;

    let d = doc(vec![
        backdrop(),
        Group::new(
            "Half",
            vec![
                noise(12).opacity(230).into(),
                Group::new("Three quarters", vec![disc("Disc", [250, 250, 250]).into()]).opacity(191).into(),
            ],
        )
        .opacity(128)
        .into(),
    ]);
    put(w, "group-opacity-nested", "Folder opacity 50% around a layer and a 75% folder", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Group::new("Closed", vec![disc("Disc", [120, 60, 200]).into()]).closed().into()]);
    put(w, "group-closed", "A closed folder ('lsct' type 2)", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Group::new("Empty folder", vec![]).into(), disc("Disc", [200, 200, 40]).into()]);
    put(w, "group-empty", "An empty folder under a layer", &d, WriteOptions::psd())?;
    Ok(())
}

fn clipping(w: &mut CaseWriter) -> Result<()> {
    let base = || disc("Base", [240, 240, 240]);
    let d = doc(vec![backdrop(), base().into(), noise(13).clipped().blend(Blend::Multiply).opacity(204).into()]);
    put(w, "clip-basic", "A Multiply noise layer at 80% clipped to a disc", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), base().into(), noise(14).clipped().into(), Layer::pixels("Checker", 0, 0, images::checker(N, N, 8)).clipped().opacity(128).into()]);
    put(w, "clip-chain", "Two layers clipped to the same base", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), base().hidden().into(), noise(15).clipped().into()]);
    put(w, "clip-hidden-base", "A visible layer clipped to a hidden base", &d, WriteOptions::psd())?;

    let d = doc(vec![Group::new("Folder", vec![backdrop(), base().into(), noise(16).clipped().opacity(191).into()]).into()]);
    put(w, "clip-inside-group", "Base and clipped layer inside a folder", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Group::new("Folder base", vec![base().into()]).into(), noise(17).clipped().into()]);
    put(w, "clip-onto-group", "A layer clipped to a folder; the Mac reader skips this clipping with a note", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), base().into(), Layer::adjustment("Hue/Saturation", hue_master([120, 20, 0])).clipped().into()]);
    put(w, "clip-adjustment", "A Hue/Saturation adjustment clipped to a disc", &d, WriteOptions::psd())?;

    let d = doc(vec![backdrop(), Layer::adjustment("Levels", levels_contrast()).into(), noise(18).clipped().into()]);
    put(w, "clip-onto-adjustment", "A layer clipped to an adjustment layer; the Mac reader skips this clipping with a note", &d, WriteOptions::psd())?;
    Ok(())
}

fn levels_contrast() -> Adjustment {
    let composite = LevelsRecord { input_black: 20, input_white: 230, output_black: 0, output_white: 255, gamma: 1.4 };
    let red = LevelsRecord { output_white: 230, ..LevelsRecord::IDENTITY };
    Adjustment::Levels(Levels { records: [composite, red, LevelsRecord::IDENTITY, LevelsRecord::IDENTITY] })
}

fn hue_master(master: [i16; 3]) -> Adjustment {
    Adjustment::HueSaturation(HueSaturation { master, ..Default::default() })
}

fn adjustments(w: &mut CaseWriter) -> Result<()> {
    let over_photo = |layer: Layer| doc(vec![backdrop(), layer.into()]);

    put(w, "adj-levels", "Levels: RGB input 20-230 at gamma 1.40, red output white 230", &over_photo(Layer::adjustment("Levels", levels_contrast())), WriteOptions::psd())?;

    let channels = Levels {
        records: [
            LevelsRecord::IDENTITY,
            LevelsRecord { input_black: 40, gamma: 0.7, ..LevelsRecord::IDENTITY },
            LevelsRecord { output_black: 30, output_white: 200, ..LevelsRecord::IDENTITY },
            LevelsRecord { input_white: 180, gamma: 2.2, ..LevelsRecord::IDENTITY },
        ],
    };
    put(w, "adj-levels-channels", "Levels on the red, green and blue channels only", &over_photo(Layer::adjustment("Levels", Adjustment::Levels(channels))), WriteOptions::psd())?;

    let curves = Curves { channels: [Some(vec![(0, 0), (64, 44), (192, 212), (255, 255)]), None, None, Some(vec![(0, 30), (255, 225)])] };
    put(
        w,
        "adj-curves",
        "Curves as Photoshop writes them (4-byte channel mask): an S curve on RGB and a lifted, lowered blue",
        &over_photo(Layer::adjustment("Curves", Adjustment::Curves(curves))),
        WriteOptions::psd(),
    )?;

    // The Mac reader reads a 2-byte curve count where Photoshop stores a 4-byte channel mask. This
    // payload is laid out the way that reader expects (not valid for Photoshop), so the harness
    // also records its Curves conversion when the payload does get through.
    let mut payload = vec![0u8, 0, 1, 0, 2];
    for points in [&[(0u8, 0u8), (64, 44), (192, 212), (255, 255)][..], &[(0, 30), (255, 225)][..]] {
        payload.extend_from_slice(&(points.len() as u16).to_be_bytes());
        for &(input, output) in points {
            payload.extend_from_slice(&(output as u16).to_be_bytes());
            payload.extend_from_slice(&(input as u16).to_be_bytes());
        }
    }
    put(
        w,
        "adj-curves-reader-layout",
        "Curves with a 2-byte curve count instead of Photoshop's channel mask (the layout the Mac reader parses): an S curve on RGB, a lifted red",
        &over_photo(Layer::empty("Curves").extra(*b"curv", payload)),
        WriteOptions::psd(),
    )?;

    put(w, "adj-hue-sat-master", "Hue/Saturation, master: hue +30, saturation -20, lightness +10", &over_photo(Layer::adjustment("Hue/Saturation", hue_master([30, -20, 10]))), WriteOptions::psd())?;

    let mut ranges = HueSaturation::default();
    ranges.ranges[0] = HueRange { saturation: 40, ..ranges.ranges[0] };
    ranges.ranges[2] = HueRange { hue: -45, lightness: -20, ..ranges.ranges[2] };
    ranges.ranges[4] = HueRange { band: [180, 210, 260, 290], hue: 60, saturation: -50, lightness: 0 };
    put(w, "adj-hue-sat-ranges", "Hue/Saturation on the reds, greens and a widened blues band only", &over_photo(Layer::adjustment("Hue/Saturation", Adjustment::HueSaturation(ranges))), WriteOptions::psd())?;

    let colorize = HueSaturation { colorize: true, colorize_values: [200, 60, -10], master: [15, 15, 15], ..Default::default() };
    put(w, "adj-hue-sat-colorize", "Hue/Saturation with Colorize: hue 200, saturation 60, lightness -10", &over_photo(Layer::adjustment("Colorize", Adjustment::HueSaturation(colorize))), WriteOptions::psd())?;

    let masked = Layer::adjustment("Levels", levels_contrast()).opacity(128).mask(Mask::new(16, 8, images::gray_ramp(32, 48, true)).default_color(0));
    put(w, "adj-opacity-mask", "Levels at 50% through a 32 x 48 ramp mask, black outside it", &over_photo(masked), WriteOptions::psd())?;

    put(w, "adj-hidden", "A hidden Hue/Saturation layer", &over_photo(Layer::adjustment("Hue/Saturation", hue_master([180, 0, 0])).hidden()), WriteOptions::psd())?;

    put(w, "adj-unsupported-invert", "An Invert adjustment ('nvrt'), which the Mac reader skips with a note", &over_photo(Layer::empty("Invert").extra(*b"nvrt", vec![])), WriteOptions::psd())?;
    Ok(())
}

/// A document touching most features at once: blend modes, off-canvas pixels, a masked layer,
/// clipping, nested folders with opacity and a non-pass mode, fill, adjustments and a hidden layer.
fn multi_feature() -> Document {
    doc(vec![
        backdrop(),
        Layer::pixels("Off canvas", -12, 30, images::noise(40, 40, 19, Alpha::Varied)).blend(Blend::Screen).opacity(200).into(),
        Group::new(
            "Folder — Ü",
            vec![
                disc("Disc", [220, 80, 40]).mask(Mask::new(0, 0, images::mask_mixed(N, N))).into(),
                Layer::pixels("Clipped", 0, 0, images::checker(N, N, 8)).clipped().blend(Blend::Multiply).opacity(180).into(),
                Group::new("Inner", vec![Layer::pixels("Corners", 0, 0, images::corners(N, N, [30, 160, 90])).fill(128).into()]).blend(Blend::Multiply).opacity(200).into(),
            ],
        )
        .opacity(230)
        .into(),
        Layer::adjustment("Levels", levels_contrast()).opacity(191).into(),
        Layer::adjustment("Hue/Saturation", hue_master([-40, 25, 0])).mask(Mask::new(32, 0, images::gray_solid(32, N, 255)).default_color(0)).into(),
        noise(20).hidden().into(),
    ])
}

fn encoding(w: &mut CaseWriter) -> Result<()> {
    let d = multi_feature();
    put(w, "compression-rle", "The multi-feature document, channels RLE (PackBits)", &d, WriteOptions::psd())?;
    put(w, "compression-raw", "The multi-feature document, channels uncompressed", &d, WriteOptions::psd().compression(Compression::Raw))?;
    put(w, "psb-rle", "The multi-feature document as a PSB (8-byte lengths, 4-byte RLE row counts)", &d, WriteOptions::psb())?;
    put(w, "psb-raw", "The multi-feature document as a PSB, channels uncompressed", &d, WriteOptions::psb().compression(Compression::Raw))?;
    Ok(())
}

fn photo_planes() -> [Vec<u16>; 3] {
    let photo = images::photo(N, N);
    let plane = |c: usize| photo.pixels().map(|p| p[c] as u16).collect::<Vec<_>>();
    [plane(0), plane(1), plane(2)]
}

fn rejected(w: &mut CaseWriter) -> Result<()> {
    let d = doc(vec![backdrop(), disc("Disc", [40, 200, 200]).opacity(200).into()]);
    put(w, "reject-zip", "Layer channels ZIP-compressed (compression 2), which the Mac reader rejects", &d, WriteOptions::psd().compression(Compression::Zip))?;

    let [r, g, b] = photo_planes();
    let flat = |mode: ColorMode, depth: u16, planes: Vec<Vec<u16>>| FlatImage { width: N, height: N, mode, depth, planes, resolution: 72.0 };

    let wide = |p: &Vec<u16>| p.iter().map(|v| v * 257).collect::<Vec<_>>();
    put_flat(w, "reject-depth-16", "16-bit RGB, flattened (no layers); the Mac reader imports only 8-bit", &flat(ColorMode::Rgb, 16, vec![wide(&r), wide(&g), wide(&b)]))?;

    // CMYK samples are stored inverted (255 is no ink): a naive conversion with no black.
    put_flat(w, "reject-cmyk", "8-bit CMYK, flattened; the Mac reader imports only RGB", &flat(ColorMode::Cmyk, 8, vec![r.clone(), g.clone(), b.clone(), vec![255; (N * N) as usize]]))?;

    let gray: Vec<u16> = r.iter().zip(&g).zip(&b).map(|((&r, &g), &b)| ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114 + 500) / 1000) as u16).collect();
    put_flat(w, "reject-grayscale", "8-bit grayscale, flattened; the Mac reader imports only RGB", &flat(ColorMode::Grayscale, 8, vec![gray]))?;

    put_flat(w, "flat-rgb-no-layers", "8-bit RGB with only the flattened image and no layer records; the Mac reader ignores the flattened image", &flat(ColorMode::Rgb, 8, vec![r, g, b]))?;
    Ok(())
}
