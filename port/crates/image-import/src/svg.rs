//! SVG, which the Mac draws once into pixels with AppKit's SVG renderer (`NSImage`), at the size
//! the file declares when there is no canvas yet (`ImageImporter.decodeSVG`). The port draws it
//! with resvg.

use crate::develop::unpremultiply;
use crate::{ImportError, Layer, Result};
use comp_format::RgbaImage;
use resvg::{tiny_skia, usvg};

pub(crate) fn decode(data: &[u8]) -> Result<Layer> {
    let text = std::str::from_utf8(data).map_err(|_| ImportError::Unreadable)?;
    if text.contains("<text") {
        return Err(ImportError::NotPorted("SVG text (AppKit draws it with Core Text)".into()));
    }
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|_| ImportError::Unreadable)?;
    let size = tree.size();
    // `max(1, Int((svg.size.width * scale).rounded()))`, with scale 1 into an empty session.
    let width = (size.width() as f64).round().max(1.0) as u32;
    let height = (size.height() as f64).round().max(1.0) as u32;
    crate::check_size(width as u64, height as u64)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(ImportError::Unreadable)?;
    // `svg.draw(in: CGRect(x: 0, y: 0, width: width, height: height))`: the drawing is stretched
    // to the rounded size.
    let transform = tiny_skia::Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = RgbaImage::new(width, height);
    for (out, p) in pixels.pixels_mut().zip(pixmap.pixels()) {
        out.0 = unpremultiply([p.red(), p.green(), p.blue(), p.alpha()]);
    }
    Ok(Layer { pixels, approximation: None, notes: Vec::new() })
}
