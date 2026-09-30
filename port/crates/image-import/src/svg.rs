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
    let approximation = (!pixel_aligned(tree.root(), transform)).then(|| {
        "SVG with antialiased edges, strokes or gradients: resvg rasterizes them, not Core Graphics".to_string()
    });
    Ok(Layer { pixels, approximation, notes: Vec::new() })
}

/// True when everything in the group is drawn exactly by any rasterizer: solid fills of
/// rectangles whose edges fall on whole pixels, with no strokes, gradients, patterns, images,
/// clipping, masks or filters. That is what the Mac and resvg are known to agree on byte for byte.
fn pixel_aligned(group: &usvg::Group, root: tiny_skia::Transform) -> bool {
    if group.clip_path().is_some() || group.mask().is_some() || !group.filters().is_empty() {
        return false;
    }
    group.children().iter().all(|node| match node {
        usvg::Node::Group(g) => pixel_aligned(g, root),
        usvg::Node::Path(path) => {
            if !path.is_visible() {
                return true;
            }
            if path.stroke().is_some() || !path.fill().is_some_and(|f| matches!(f.paint(), usvg::Paint::Color(_))) {
                return false;
            }
            let t = root.pre_concat(path.abs_transform());
            if t.kx != 0.0 || t.ky != 0.0 {
                return false;
            }
            let whole = |p: tiny_skia::Point| {
                let (x, y) = (p.x * t.sx + t.tx, p.y * t.sy + t.ty);
                x == x.round() && y == y.round()
            };
            let mut last: Option<tiny_skia::Point> = None;
            path.data().segments().all(|segment| match segment {
                tiny_skia::PathSegment::MoveTo(p) => {
                    last = Some(p);
                    whole(p)
                }
                tiny_skia::PathSegment::LineTo(p) => {
                    let straight = last.is_some_and(|l| l.x == p.x || l.y == p.y);
                    last = Some(p);
                    straight && whole(p)
                }
                tiny_skia::PathSegment::Close => true,
                _ => false,
            })
        }
        _ => false,
    })
}
