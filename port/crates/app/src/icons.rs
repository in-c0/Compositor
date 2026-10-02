//! Icons. The Mac uses SF Symbols, which can't ship on Windows, so each symbol maps to the
//! nearest Phosphor icon (MIT; docs/port/icons.md lists every substitution). The Mac's own
//! custom-drawn icons are ported path for path.

use crate::theme;
use eframe::egui::{self, Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use egui_phosphor::regular as ph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// An SF Symbol name.
    Symbol(&'static str),
    GradientTool,
    CloneStampTool,
    PolygonalLassoTool,
    ObjectSelectionTool,
}

/// SF Symbol name → Phosphor glyph and whether it's the filled variant.
pub fn phosphor(symbol: &str) -> Option<(&'static str, bool)> {
    Some(match symbol {
        "arrow.up.left.and.arrow.down.right" => (ph::ARROWS_OUT_SIMPLE, false),
        "rectangle.dashed" => (ph::SELECTION, false),
        "circle.dashed" => (ph::CIRCLE_DASHED, false),
        "lasso" => (ph::LASSO, false),
        "wand.and.stars" => (ph::MAGIC_WAND, false),
        "crop" => (ph::CROP, false),
        "paintbrush.pointed" => (ph::PAINT_BRUSH, false),
        "eraser" => (ph::ERASER, false),
        "bandage" => (ph::BANDAIDS, false),
        "seal" => (ph::STAMP, false),
        "drop" => (ph::DROP, false),
        "square.bottomhalf.filled" => (ph::GRADIENT, false),
        "square.on.circle" => (ph::SHAPES, false),
        "textformat" => (ph::TEXT_AA, false),
        "eyedropper" => (ph::EYEDROPPER, false),
        "hand.draw" => (ph::HAND, false),
        "magnifyingglass" => (ph::MAGNIFYING_GLASS, false),
        "plus" => (ph::PLUS, false),
        "minus" => (ph::MINUS, false),
        "plus.magnifyingglass" => (ph::MAGNIFYING_GLASS_PLUS, false),
        "minus.magnifyingglass" => (ph::MAGNIFYING_GLASS_MINUS, false),
        "xmark" => (ph::X, false),
        "chevron.down" => (ph::CARET_DOWN, false),
        "chevron.right" => (ph::CARET_RIGHT, false),
        "chevron.up.chevron.down" => (ph::CARET_UP_DOWN, false),
        "arrow.left.and.right" => (ph::ARROWS_LEFT_RIGHT, false),
        "arrow.counterclockwise" => (ph::ARROW_COUNTER_CLOCKWISE, false),
        "square.3.layers.3d" => (ph::STACK, false),
        "plus.square" => (ph::PLUS_SQUARE, false),
        "folder.badge.plus" => (ph::FOLDER_PLUS, false),
        "rectangle.inset.filled" => (ph::SQUARE_HALF, false),
        "sparkles" => (ph::SPARKLE, false),
        "circle.lefthalf.filled" => (ph::CIRCLE_HALF, true),
        "circle.righthalf.filled" => (ph::CIRCLE_HALF_TILT, true),
        "trash" => (ph::TRASH, false),
        "eye" => (ph::EYE, false),
        "eye.slash" => (ph::EYE_SLASH, false),
        "eye.fill" => (ph::EYE, true),
        "link" => (ph::LINK_SIMPLE, false),
        "folder" => (ph::FOLDER_SIMPLE, false),
        "scissors" => (ph::SCISSORS, false),
        "text.alignleft" => (ph::TEXT_ALIGN_LEFT, false),
        "text.aligncenter" => (ph::TEXT_ALIGN_CENTER, false),
        "text.alignright" => (ph::TEXT_ALIGN_RIGHT, false),
        "triangle.fill" => (ph::TRIANGLE, true),
        "plus.circle.fill" => (ph::PLUS_CIRCLE, true),
        "minus.circle.fill" => (ph::MINUS_CIRCLE, true),
        "hand.point.up.left" => (ph::HAND_POINTING, false),
        "scope" => (ph::CROSSHAIR, false),
        "line.diagonal" => (ph::LINE_SEGMENT, false),
        "multiply" => (ph::X, false),
        "circle" => (ph::CIRCLE, false),
        "circle.fill" => (ph::CIRCLE, true),
        "slider.horizontal.3" => (ph::SLIDERS_HORIZONTAL, false),
        "point.topleft.down.to.point.bottomright.curvepath" => (ph::BEZIER_CURVE, false),
        "plusminus.circle" => (ph::PLUS_MINUS, false),
        "paintpalette" => (ph::PALETTE, false),
        "circle.grid.3x3" => (ph::DOTS_NINE, false),
        "drop.fill" => (ph::DROP, true),
        "wind" => (ph::WIND, false),
        "circle.dotted" => (ph::CIRCLE_DASHED, false),
        "circle.filled.pattern.diagonalline.rectangle" => (ph::CHECKERBOARD, false),
        "scale.3d" => (ph::SCALES, false),
        "arrow.turn.down.right" => (ph::ARROW_BEND_DOWN_RIGHT, false),
        "rectangle.badge.plus" => (ph::SELECTION_PLUS, false),
        "rectangle.badge.minus" => (ph::SELECTION_SLASH, false),
        "arrow.triangle.2.circlepath" => (ph::ARROWS_CLOCKWISE, false),
        _ => return None,
    })
}

/// Draws `icon` centered on `center`. `size` is the SF Symbol point size the Mac uses; Phosphor
/// glyphs fill more of their em square, so they are drawn a little smaller to match optically.
pub fn paint(painter: &Painter, icon: Icon, center: Pos2, size: f32, color: Color32) {
    match icon {
        Icon::Symbol("rectangle.dashed") => rectangle_dashed(painter, center, size, color),
        Icon::Symbol("square.on.circle") => square_on_circle(painter, center, size, color),
        Icon::Symbol(name) => {
            let Some((glyph, fill)) = phosphor(name) else { return };
            let family = FontFamily::Name(if fill { theme::ICONS_FILL } else { theme::ICONS }.into());
            painter.text(center, egui::Align2::CENTER_CENTER, glyph, FontId::new(size * optical_scale(name), family), color);
        }
        Icon::GradientTool => gradient_tool(painter, Rect::from_center_size(center, vec2(18.0, 18.0)), color),
        Icon::CloneStampTool => clone_stamp_tool(painter, Rect::from_center_size(center, vec2(18.0, 18.0)), color),
        Icon::PolygonalLassoTool => polygonal_lasso_tool(painter, Rect::from_center_size(center, vec2(18.0, 18.0)), color),
        Icon::ObjectSelectionTool => object_selection_tool(painter, Rect::from_center_size(center, vec2(18.0, 18.0)), color),
    }
}

/// The points of a rounded rectangle's outline, clockwise from the top edge's start.
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let mut points = Vec::new();
    let corners = [
        (pos2(rect.max.x - radius, rect.min.y + radius), -90.0f32),
        (pos2(rect.max.x - radius, rect.max.y - radius), 0.0),
        (pos2(rect.min.x + radius, rect.max.y - radius), 90.0),
        (pos2(rect.min.x + radius, rect.min.y + radius), 180.0),
    ];
    points.push(pos2(rect.min.x + radius, rect.min.y));
    for (c, start) in corners {
        for k in 0..=6 {
            let a = (start + 15.0 * k as f32).to_radians();
            points.push(c + vec2(a.cos(), a.sin()) * radius);
        }
    }
    points.push(pos2(rect.min.x + radius, rect.min.y));
    points
}

/// SF Symbols' `rectangle.dashed`, drawn: a wide dashed rounded rectangle, as the Marquee tool
/// shows it (20 × 15 at 17 points with its stroke, measured). Phosphor's selection glyph is square.
fn rectangle_dashed(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let k = size / 17.0;
    let rect = Rect::from_center_size(center, vec2(17.5, 12.5) * k);
    painter.extend(Shape::dashed_line(&rounded_outline(rect, 3.0 * k), Stroke::new(1.5 * k, color), 3.4 * k, 1.6 * k));
}

/// SF Symbols' `square.on.circle`, drawn: a circle behind a rounded square at its lower right,
/// as the Shape tool shows it. Phosphor's shapes glyph is a different picture.
fn square_on_circle(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let k = size / 17.0;
    let stroke = Stroke::new(1.5 * k, color);
    let square = Rect::from_center_size(center + vec2(2.0, 2.5) * k, vec2(11.5, 11.0) * k);
    painter.add(Shape::closed_line(rounded_outline(square, 3.0 * k), stroke));
    // The circle's arc, left out where the square covers it.
    let (c, r) = (center + vec2(-2.5, -1.5) * k, 6.0 * k);
    let hidden = square.expand(0.75 * k);
    let mut run: Vec<Pos2> = Vec::new();
    for i in 0..=96 {
        let a = i as f32 / 96.0 * std::f32::consts::TAU;
        let q = c + vec2(a.cos(), a.sin()) * r;
        if hidden.contains(q) {
            if run.len() > 1 {
                painter.add(Shape::line(std::mem::take(&mut run), stroke));
            }
            run.clear();
        } else {
            run.push(q);
        }
    }
    if run.len() > 1 {
        painter.add(Shape::line(run, stroke));
    }
}

/// How much larger than the SF Symbol's point size a Phosphor glyph is drawn to look the same
/// size: 1.12 in general; SF Symbols' eye is much wider than Phosphor's (18 against 14 points
/// wide at 13 points, measured in the Layers panel), so the eyes are drawn larger.
fn optical_scale(symbol: &str) -> f32 {
    match symbol {
        "eye" | "eye.slash" | "eye.fill" => 1.12 * 1.3,
        _ => 1.12,
    }
}

/// Like `paint`, rotated by `angle` radians about its center (the swap arrows, the mask link).
pub fn paint_rotated(painter: &Painter, symbol: &'static str, center: Pos2, size: f32, color: Color32, angle: f32) {
    let Some((glyph, fill)) = phosphor(symbol) else { return };
    let family = FontFamily::Name(if fill { theme::ICONS_FILL } else { theme::ICONS }.into());
    let galley = painter.layout_no_wrap(glyph.to_string(), FontId::new(size * 1.12, family), color);
    let half = galley.size() / 2.0;
    // TextShape rotates about its top-left corner; place that corner so the center lands on `center`.
    let (s, c) = angle.sin_cos();
    let offset = vec2(half.x * c - half.y * s, half.x * s + half.y * c);
    let text = egui::epaint::TextShape::new(center - offset, galley, color).with_angle(angle);
    painter.add(text);
}

/// `GradientToolIcon`: a 16×16 Floyd–Steinberg dither of a left-to-right ramp in a rounded frame.
fn gradient_tool(painter: &Painter, rect: Rect, color: Color32) {
    const N: usize = 16;
    let mut ramp = [[0f32; N]; N];
    for row in &mut ramp {
        for (x, v) in row.iter_mut().enumerate() {
            *v = x as f32 / (N - 1) as f32;
        }
    }
    let frame = rect.shrink(1.0);
    let cell = frame.width() / N as f32;
    let mut mesh = egui::Mesh::default();
    for y in 0..N {
        for x in 0..N {
            let on = ramp[y][x] >= 0.5;
            let error = ramp[y][x] - if on { 1.0 } else { 0.0 };
            if x + 1 < N {
                ramp[y][x + 1] += error * 7.0 / 16.0;
            }
            if y + 1 < N {
                if x > 0 {
                    ramp[y + 1][x - 1] += error * 3.0 / 16.0;
                }
                ramp[y + 1][x] += error * 5.0 / 16.0;
                if x + 1 < N {
                    ramp[y + 1][x + 1] += error / 16.0;
                }
            }
            if on {
                let min = pos2(frame.min.x + x as f32 * cell, frame.min.y + y as f32 * cell);
                mesh.add_colored_rect(Rect::from_min_size(min, vec2(cell, cell)), color);
            }
        }
    }
    // The dots are clipped to the rounded frame; at this size only the four corner cells differ.
    painter.with_clip_rect(frame).add(Shape::mesh(mesh));
    painter.rect_stroke(frame, 3.5, Stroke::new(1.4, color), egui::StrokeKind::Middle);
}

/// `CloneStampToolIcon`: handle knob, stem, block and base.
fn clone_stamp_tool(painter: &Painter, r: Rect, color: Color32) {
    let (w, h) = (r.width(), r.height());
    let at = |x: f32, y: f32, ww: f32, hh: f32| Rect::from_min_size(pos2(r.min.x + x * w, r.min.y + y * h), vec2(ww * w, hh * h));
    let knob = at(0.33, 0.02, 0.34, 0.30);
    painter.add(egui::epaint::EllipseShape::filled(knob.center(), knob.size() / 2.0, color));
    painter.rect_filled(at(0.43, 0.28, 0.14, 0.28), 0.0, color);
    painter.rect_filled(at(0.12, 0.54, 0.76, 0.22), w * 0.08, color);
    painter.rect_filled(at(0.06, 0.82, 0.88, 0.12), 0.0, color);
}

fn path(r: Rect, points: &[(f32, f32)]) -> Vec<Pos2> {
    let unit = r.width() / 18.0;
    points.iter().map(|(x, y)| pos2(r.min.x + x * unit, r.min.y + y * unit)).collect()
}

/// `PolygonalLassoToolIcon`: a wide loop, a knot below its right side, a short rope.
fn polygonal_lasso_tool(painter: &Painter, r: Rect, color: Color32) {
    let stroke = Stroke::new(1.4 * r.width() / 18.0, color);
    painter.add(Shape::closed_line(path(r, &[(1.2, 7.0), (4.0, 2.4), (11.8, 1.8), (16.8, 5.2), (15.6, 10.4), (7.0, 11.6)]), stroke));
    painter.add(Shape::closed_line(path(r, &[(8.9, 10.9), (13.3, 10.5), (11.6, 14.5)]), stroke));
    painter.add(Shape::line(path(r, &[(11.6, 14.5), (12.9, 17.3)]), stroke));
}

/// `ObjectSelectionToolIcon`: four frame corners around an arrow cursor.
fn object_selection_tool(painter: &Painter, r: Rect, color: Color32) {
    let stroke = Stroke::new(1.6 * r.width() / 18.0, color);
    for corner in [
        [(2.0, 6.0), (2.0, 2.0), (6.0, 2.0)],
        [(12.0, 2.0), (16.0, 2.0), (16.0, 6.0)],
        [(16.0, 12.0), (16.0, 16.0), (12.0, 16.0)],
        [(6.0, 16.0), (2.0, 16.0), (2.0, 12.0)],
    ] {
        painter.add(Shape::line(path(r, &corner), stroke));
    }
    let cursor = path(r, &[(7.0, 5.0), (7.0, 14.0), (9.6, 11.7), (11.3, 15.3), (13.2, 14.4), (11.5, 10.9), (14.5, 10.9)]);
    // Not convex: fill it as triangles from the tip.
    let mut mesh = egui::Mesh::default();
    for p in &cursor {
        mesh.colored_vertex(*p, color);
    }
    for (a, b, c) in [(0, 1, 2), (0, 2, 5), (0, 5, 6), (2, 3, 4), (2, 4, 5)] {
        mesh.add_triangle(a, b, c);
    }
    painter.add(Shape::mesh(mesh));
}

/// The symbol an adjustment layer's thumbnail shows (`AdjustmentKind.symbol`).
pub fn adjustment_symbol(kind: comp_format::AdjustmentKind) -> &'static str {
    use comp_format::AdjustmentKind::*;
    match kind {
        HueSaturation => "circle.lefthalf.filled",
        Levels => "slider.horizontal.3",
        Curves => "point.topleft.down.to.point.bottomright.curvepath",
        Exposure => "plusminus.circle",
        GradientMap => "paintpalette",
        Grain => "circle.grid.3x3",
        AddNoise => "circle.dotted",
        GaussianBlur => "drop.fill",
        MotionBlur => "wind",
        Invert => "circle.righthalf.filled",
        BlackWhite => "circle.filled.pattern.diagonalline.rectangle",
        ColorBalance => "scale.3d",
    }
}
