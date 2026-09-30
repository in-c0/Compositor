//! HEIC. heic-rs decodes the HEVC picture; HEVC decoding is exact by specification, so its YCbCr
//! planes are the ones Apple's decoder makes. The conversion to RGB is the Mac's: chroma
//! upsampled bilinearly from sample centers, the matrix from the file's `nclx`, in float.

use crate::color::Space;
use crate::develop::{Samples, Source};
use crate::{ImportError, Result};
use heic_rs::context::Context;
use heic_rs::hevc::{ChromaFormat, Frame};
use heic_rs::props::{ItemProps, Transform};
use heic_rs::{MatrixCoefficients, Mirror, Range, Rotation};

/// An ISO base media file whose brand is an HEIF image brand.
pub(crate) fn matches(data: &[u8]) -> bool {
    data.len() >= 12
        && &data[4..8] == b"ftyp"
        && matches!(&data[8..12], b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"hevc" | b"hevx")
}

/// The item's picture, with a grid's tiles put together.
fn frame_of(ctx: &Context<'_>, item: u32, props: &ItemProps<'_>) -> Result<Frame> {
    let decode = |item: u32, props: &ItemProps<'_>| -> Result<Frame> {
        let hvcc = props.hvcc.as_ref().ok_or(ImportError::Unreadable)?;
        let data = ctx.item_data(item).map_err(|_| ImportError::Unreadable)?;
        let nals = hvcc.split_nals(&data).map_err(|_| ImportError::Unreadable)?;
        heic_rs::hevc::decode_still(&hvcc.parameter_sets(), &nals)
            .map_err(|e| ImportError::NotPorted(format!("an HEVC stream heic-rs doesn't decode ({e:?})")))
    };
    match ctx.grid(item).map_err(|_| ImportError::Unreadable)? {
        Some((grid, tiles)) => {
            let frames = tiles
                .iter()
                .map(|t| decode(*t, &ctx.props(*t).map_err(|_| ImportError::Unreadable)?))
                .collect::<Result<Vec<_>>>()?;
            heic_rs::grid::compose(&grid, &frames, crate::DOCUMENT_PIXEL_BUDGET).map_err(|_| ImportError::Unreadable)
        }
        None => decode(item, props),
    }
}

/// A plane's top-left `width` x `height` samples as 0-1.
fn plane(samples: &[u16], stride: usize, width: usize, height: usize, depth: u8) -> Vec<f32> {
    let max = ((1u32 << depth) - 1) as f32;
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        out.extend(samples[y * stride..y * stride + width].iter().map(|&v| v as f32 / max));
    }
    out
}

/// Chroma at every luma sample: bilinear between chroma sample centers with the edges clamped,
/// or (`nearest`) the sample each luma sample falls in.
fn upsample(chroma: &[f32], cw: usize, ch: usize, fx: usize, fy: usize, width: usize, height: usize, nearest: bool) -> Vec<f32> {
    let axis = |i: usize, f: usize, n: usize| -> (usize, usize, f32) {
        if nearest {
            return ((i / f).min(n - 1), (i / f).min(n - 1), 0.0);
        }
        let pos = (i as f32 + 0.5) / f as f32 - 0.5;
        if pos <= 0.0 {
            return (0, 0, 0.0);
        }
        let i0 = pos.floor() as usize;
        if i0 + 1 >= n {
            return (n - 1, n - 1, 0.0);
        }
        (i0, i0 + 1, pos - i0 as f32)
    };
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let (y0, y1, wy) = axis(y, fy, ch);
        for x in 0..width {
            let (x0, x1, wx) = axis(x, fx, cw);
            let top = chroma[y0 * cw + x0] * (1.0 - wx) + chroma[y0 * cw + x1] * wx;
            let bottom = chroma[y1 * cw + x0] * (1.0 - wx) + chroma[y1 * cw + x1] * wx;
            out.push(top * (1.0 - wy) + bottom * wy);
        }
    }
    out
}

pub(crate) fn decode(data: &[u8]) -> Result<Source> {
    let ctx = Context::open(data).map_err(|_| ImportError::Unreadable)?;
    let item = ctx.meta.primary;
    let props = ctx.props(item).map_err(|_| ImportError::Unreadable)?;
    let (width, height) = ctx.coded_size(item, &props).map_err(|_| ImportError::Unreadable)?;
    crate::check_size(width as u64, height as u64)?;
    if props.icc.is_some() {
        return Err(ImportError::NotPorted("HEIC with an ICC profile".into()));
    }
    let nclx = props.nclx.unwrap_or_default();
    if nclx.primaries != 1 || nclx.transfer != 13 {
        return Err(ImportError::NotPorted(format!("HEIC with nclx primaries {} and transfer {}", nclx.primaries, nclx.transfer)));
    }
    let (kr, kb) = match nclx.matrix {
        MatrixCoefficients::Bt601 => (0.299f32, 0.114f32),
        MatrixCoefficients::Bt709 | MatrixCoefficients::Unspecified => (0.2126, 0.0722),
        other => return Err(ImportError::NotPorted(format!("HEIC matrix {other:?}"))),
    };
    // A clean aperture comes first in the files encoders write (libheif crops the coded size to
    // the picture's own); rotation and mirroring after it become the orientation.
    let (crop, rest) = match props.transforms.split_first() {
        Some((Transform::Crop(clap), rest)) => (Some(*clap), rest),
        _ => (None, &props.transforms[..]),
    };
    let orientation = orientation(rest)?;
    let frame = frame_of(&ctx, item, &props)?;
    let (w, h) = (width as usize, height as usize);
    if (frame.width as usize) < w || (frame.height as usize) < h {
        return Err(ImportError::Unreadable);
    }
    let (fx, fy) = match frame.chroma {
        ChromaFormat::Yuv420 => (2, 2),
        ChromaFormat::Yuv422 => (2, 1),
        ChromaFormat::Yuv444 => (1, 1),
        ChromaFormat::Monochrome => return Err(ImportError::NotPorted("monochrome HEIC".into())),
    };
    let depth = frame.bit_depth;
    let luma = plane(&frame.y, frame.y_stride as usize, w, h, depth);
    let (cw, ch) = (w.div_ceil(fx), h.div_ceil(fy));
    let alpha_item = ctx.alpha_item(item).map_err(|_| ImportError::Unreadable)?;
    // Apple's decoder takes a different path for a picture with an alpha plane, which repeats
    // each chroma sample instead of interpolating and rounds the color to 8 bits before
    // premultiplying (measured on import/heic-rgba).
    let nearest = alpha_item.is_some();
    let cb = upsample(&plane(&frame.cb, frame.c_stride as usize, cw, ch, depth), cw, ch, fx, fy, w, h, nearest);
    let cr = upsample(&plane(&frame.cr, frame.c_stride as usize, cw, ch, depth), cw, ch, fx, fy, w, h, nearest);
    let alpha = match alpha_item {
        Some(aux) => {
            let aux_props = ctx.props(aux).map_err(|_| ImportError::Unreadable)?;
            let a = frame_of(&ctx, aux, &aux_props)?;
            Some(plane(&a.y, a.y_stride as usize, w, h, a.bit_depth))
        }
        None => None,
    };
    let max = ((1u32 << depth) - 1) as f32;
    let half = (1u32 << (depth - 1)) as f32 / max;
    // Apple divides chroma's distance from its midpoint by 254 where luma is divided by 255
    // (fitted exactly on import/heic-noise-420: every pixel of both 4:2:0 cases matches).
    let chroma_scale = max / (max - 1.0);
    let limited = nclx.range == Range::Limited;
    let kg = 1.0 - kr - kb;
    let mut samples = Vec::with_capacity(w * h * (3 + alpha.is_some() as usize));
    for i in 0..w * h {
        let (mut y, mut u, mut v) = (luma[i], (cb[i] - half) * chroma_scale, (cr[i] - half) * chroma_scale);
        if limited {
            y = (y - 16.0 / 255.0) * 255.0 / 219.0;
            u *= 255.0 / 224.0;
            v *= 255.0 / 224.0;
        }
        let r = y + 2.0 * (1.0 - kr) * v;
        let b = y + 2.0 * (1.0 - kb) * u;
        let g = (y - kr * r - kb * b) / kg;
        if alpha.is_some() {
            // The alpha path hands over 8-bit color, rounded before it is premultiplied.
            samples.extend([r, g, b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() / 255.0));
        } else {
            samples.extend([r, g, b]);
        }
        if let Some(a) = &alpha {
            samples.push(a[i]);
        }
    }
    let channels = 3 + alpha.is_some() as usize;
    let (samples, width, height) = match crop {
        None => (samples, width, height),
        Some(clap) => {
            let (x, y, cw, ch) = heic_rs::transform::crop_rect(width, height, clap).map_err(|_| ImportError::Unreadable)?;
            let mut cropped = Vec::with_capacity(cw as usize * ch as usize * channels);
            for row in y as usize..(y + ch) as usize {
                let start = (row * w + x as usize) * channels;
                cropped.extend_from_slice(&samples[start..start + cw as usize * channels]);
            }
            (cropped, cw, ch)
        }
    };
    Ok(Source {
        width,
        height,
        colors: 3,
        alpha: alpha.is_some(),
        premultiplied: false,
        samples: Samples::F32(samples),
        space: Space::Srgb,
        orientation,
        approximation: None,
    })
}

/// `irot` and `imir` as the EXIF orientation ImageIO reports for them.
fn orientation(transforms: &[Transform]) -> Result<u16> {
    let mut result = 1;
    for t in transforms {
        let o = match t {
            Transform::Crop(_) => return Err(ImportError::NotPorted("HEIC clean aperture (clap)".into())),
            Transform::Rotate(Rotation::None) => continue,
            Transform::Rotate(Rotation::Ccw90) => 8,
            Transform::Rotate(Rotation::Ccw180) => 3,
            Transform::Rotate(_) => 6,
            Transform::Mirror(Mirror::LeftRight) => 2,
            Transform::Mirror(Mirror::TopBottom) => 4,
        };
        if result != 1 {
            return Err(ImportError::NotPorted("HEIC with both a rotation and a mirror".into()));
        }
        result = o;
    }
    Ok(result)
}
