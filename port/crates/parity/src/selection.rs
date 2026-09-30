//! The `#selection` check: a case that makes a selection is compared on the selection's coverage
//! as well as on its pixels. The harness writes the Mac's as `<case>.selection.png`, 8-bit gray at
//! document size; no file means the Mac ended with no selection.

use crate::cases::Case;
use crate::compare::{self, CaseResult, Status, Tolerances};
use image::{Rgba, RgbaImage};
use std::path::Path;

/// Whether the case runs any selection op.
pub fn applies(case: &Case) -> bool {
    case.spec.ops.iter().any(|op| op.get("op").and_then(|v| v.as_str()).is_some_and(|name| engine::select::OPS.contains(&name)))
}

pub fn check(renderer: &engine::Renderer, case: &Case, refs: &Path, out: &Path, tolerances: &Tolerances) -> CaseResult {
    let id = format!("{}#selection", case.id);
    let limit = tolerances.for_case(&id);
    let mut result = CaseResult {
        id: id.clone(),
        feature: case.spec.feature.clone(),
        label: format!("{} (selection)", case.spec.label),
        status: Status::Error,
        tolerance: limit.max_channel_diff,
        max_channel_diff: None,
        differing_pixels: None,
        total_pixels: None,
        message: None,
        heatmap: None,
    };
    let mac = match std::fs::exists(refs.join(format!("{}.selection.png", case.id))) {
        Ok(true) => match image::open(refs.join(format!("{}.selection.png", case.id))) {
            Ok(img) => Some(img.to_luma8()),
            Err(e) => {
                result.message = Some(format!("unreadable reference: {e}"));
                return result;
            }
        },
        _ => None,
    };
    let port = match crate::build_session(renderer, case) {
        Ok((project, selection)) => match selection {
            Some(selection) => match renderer.selection_coverage(&project, &selection) {
                Ok(coverage) => Some(coverage),
                Err(e) => return finish(result, e),
            },
            None => None,
        },
        Err(e) => return finish(result, e),
    };
    let (mac, port) = match (mac, port) {
        (None, None) => {
            result.status = Status::Pass;
            result.message = Some("neither ends with a selection".into());
            return result;
        }
        (Some(_), None) => {
            result.status = Status::Fail;
            result.message = Some("the Mac ends with a selection and the port with none".into());
            return result;
        }
        (None, Some(_)) => {
            result.status = Status::Fail;
            result.message = Some("the port ends with a selection and the Mac with none".into());
            return result;
        }
        (Some(mac), Some(port)) => (gray_to_rgba(&mac), gray_to_rgba(&port)),
    };
    let render_path = out.join("renders").join(format!("{}.selection.png", case.id));
    let _ = std::fs::create_dir_all(render_path.parent().unwrap());
    let _ = port.save(&render_path);
    match compare::diff(&mac, &port, limit.max_channel_diff) {
        Ok(d) => {
            result.max_channel_diff = Some(d.max_channel_diff);
            result.differing_pixels = Some(d.differing_pixels);
            result.total_pixels = Some(d.total_pixels);
            if d.differing_pixels <= limit.max_pixels_over {
                result.status = Status::Pass;
            } else {
                result.status = Status::Fail;
                let name = format!("{}.selection.png", case.id.replace('/', "__"));
                let _ = compare::heatmap(&mac, &port, &d, limit.max_channel_diff).save(out.join("heatmaps").join(&name));
                result.heatmap = Some(format!("heatmaps/{name}"));
            }
        }
        Err(e) => result.message = Some(e.to_string()),
    }
    result
}

fn finish(mut result: CaseResult, e: engine::RenderError) -> CaseResult {
    if let engine::RenderError::Unsupported(what) = &e {
        result.status = Status::Pending;
        result.message = Some(format!("not supported yet: {what}"));
    } else {
        result.message = Some(e.to_string());
    }
    result
}

/// Coverage as opaque gray, so every byte is compared.
fn gray_to_rgba(gray: &image::GrayImage) -> RgbaImage {
    RgbaImage::from_fn(gray.width(), gray.height(), |x, y| {
        let v = gray.get_pixel(x, y)[0];
        Rgba([v, v, v, 255])
    })
}
