//! The tool header bar (options bar): one header per tool, 42 points tall, 12-point controls,
//! 18 points of padding at each end (docs/port/ui-inventory.md §2.3).

use crate::app::App;
use crate::icons::Icon;
use crate::theme::{self, color, white_alpha};
use crate::tools::*;
use crate::widgets::{self as w, ButtonStyle, SwatchStyle};
use eframe::egui::{self, Align, CornerRadius, Layout, Rect, Sense, Ui, vec2};

pub fn tool_header(app: &mut App, ui: &mut Ui, rect: Rect) {
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    ui.style_mut().text_styles.insert(egui::TextStyle::Button, theme::regular(12.0));
    let ui = &mut ui;
    let pad = theme::metric::HEADER_PADDING;
    let inner = rect.shrink2(vec2(pad, 0.0));
    match app.tool {
        Tool::Move => transform(app, ui, rect),
        Tool::Brush | Tool::SpotHealing | Tool::CloneStamp | Tool::Blur => w::row(ui, inner, 12.0, |ui| brush(app, ui)),
        Tool::Marquee | Tool::Lasso | Tool::Wand => w::row(ui, inner, 12.0, |ui| selection(app, ui)),
        Tool::Gradient => w::row(ui, inner, 12.0, |ui| gradient(app, ui)),
        Tool::Type => w::row(ui, inner, 12.0, |ui| type_tool(app, ui)),
        Tool::Shape => w::row(ui, inner, 12.0, |ui| shape(app, ui)),
        Tool::Crop => w::row(ui, inner, 14.0, |ui| crop(app, ui)),
        Tool::Hand | Tool::Zoom => w::row(ui, inner, 12.0, |ui| navigation(app, ui)),
        Tool::Eyedropper => w::row(ui, inner, 16.0, |ui| {
            w::title(ui, "Eyedropper");
            w::checkbox(ui, &mut app.settings.sample_ring, "Sample Ring");
        }),
        Tool::Idle => w::row(ui, inner, 16.0, |ui| {
            w::title(ui, "Select a tool");
        }),
    }
}

/// A caption-sized, secondary scrub label, as the Transform fields use.
fn caption_scrub(ui: &mut Ui, s: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, enabled: bool) -> egui::Response {
    w::scrub_label(ui, s, theme::regular(10.0), color::secondary(), value, 1.0, range, enabled)
}

/// A 12-point scrub label in the label color.
fn scrub(ui: &mut Ui, s: &str, value: &mut f64, sensitivity: f64, range: std::ops::RangeInclusive<f64>) -> egui::Response {
    w::scrub_label(ui, s, theme::regular(12.0), color::label(), value, sensitivity, range, true)
}

/// Label, slider and a percent field for a 0…1 value.
fn percent_row(ui: &mut Ui, id: &str, title: &str, value: &mut f64, min: f64) {
    let mut pct = *value * 100.0;
    scrub(ui, title, &mut pct, 1.0, min * 100.0..=100.0);
    w::slider(ui, &mut pct, min * 100.0..=100.0, 100.0, true);
    w::number_field(ui, id, &mut pct, min * 100.0..=100.0, w::fmt_int, 42.0, false, true);
    w::unit(ui, "%");
    *value = pct / 100.0;
}

/// `TransformInspector`: Auto Select, Show Controls, then X, Y, W, H, lock, Scale, angle,
/// Sampling and the flips for the active layer. X and Y move the layer; the rest wait on the
/// engine's transforms.
fn transform(app: &mut App, ui: &mut Ui, rect: Rect) {
    let inner = Rect::from_min_max(rect.min + vec2(18.0, 0.0), rect.max - vec2(18.0, 0.0));
    let doc_layer = app.doc().and_then(|d| {
        let l = d.active_layer()?;
        if l.is_group() || l.adjustment.is_some() {
            return None;
        }
        let pixels = d.project.images.get(&l.id).map(|a| a.pixels.width() as f64);
        Some((l.id.clone(), l.transform, pixels))
    });
    let enabled = doc_layer.is_some();
    let (mut x, mut y, mut wd, mut ht, mut scale, mut angle) = match &doc_layer {
        Some((_, t, pixels)) => (t.origin[0], t.origin[1], t.size[0], t.size[1], pixels.map_or(100.0, |p| t.size[0] / p * 100.0), t.rotation),
        None => (0.0, 0.0, 0.0, 0.0, 100.0, 0.0),
    };
    let (x0, y0) = (x, y);
    w::row(ui, inner, 12.0, |ui| {
        w::title(ui, "Transform");
        w::checkbox(ui, &mut app.settings.auto_select, "Auto Select");
        w::checkbox(ui, &mut app.settings.show_controls, "Show Controls");
        ui.add_space(18.0);
        for (label, value, range) in [("X", &mut x, -30000.0..=30000.0), ("Y", &mut y, -30000.0..=30000.0)] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                caption_scrub(ui, label, value, range.clone(), enabled);
                w::number_field(ui, ("transform", label), value, range, w::fmt_whole_or_2, 85.0, false, enabled);
            });
        }
        for (label, value) in [("W", &mut wd), ("H", &mut ht)] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                caption_scrub(ui, label, value, 1.0..=30000.0, false);
                w::number_field(ui, ("transform", label), value, 1.0..=30000.0, w::fmt_whole_or_2, 85.0, false, false);
            });
        }
        lock_toggle(ui, &mut app.settings.lock_aspect, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            caption_scrub(ui, "Scale", &mut scale, 0.1..=30000.0, false);
            w::number_field(ui, ("transform", "scale"), &mut scale, 0.1..=30000.0, w::fmt_whole_or_2, 110.0, false, false);
            w::unit(ui, "%");
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            caption_scrub(ui, "°", &mut angle, -360.0..=360.0, false);
            w::number_field(ui, ("transform", "angle"), &mut angle, -360.0..=360.0, w::fmt_whole_or_2, 75.0, false, false);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            w::labeled_popup(ui, "Sampling", &mut app.settings.transform_sampling, &[Sampling::ALL], Sampling::title, 170.0, false);
        });
        w::button(ui, "Flip H", 12.0, ButtonStyle::Bordered, false);
        w::button(ui, "Flip V", 12.0, ButtonStyle::Bordered, false);
    });
    if let Some((id, _, _)) = doc_layer {
        if x != x0 || y != y0 {
            let (nx, ny) = (x.round(), y.round());
            if let Some(d) = app.doc_mut() {
                d.edit("Move Layer", true, |m| {
                    if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                        l.transform.origin = [nx, ny];
                    }
                });
            }
        } else if let Some(d) = app.doc_mut() {
            if !ui.ctx().egui_is_using_pointer() && !ui.ctx().egui_wants_keyboard_input() {
                d.end_coalescing();
            }
        }
    }
}

/// The lock-aspect toggle: a `.button`-style toggle showing the `link` symbol.
fn lock_toggle(ui: &mut Ui, on: &mut bool, enabled: bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, theme::metric::CONTROL_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    if response.clicked() {
        *on = !*on;
    }
    let dim = if enabled { 1.0 } else { 0.4 };
    let fill = if *on { white_alpha(0.28) } else { color::control() };
    ui.painter().rect_filled(rect, CornerRadius::same(11), fill.gamma_multiply(dim));
    crate::icons::paint(ui.painter(), Icon::Symbol("link"), rect.center(), 12.0, color::label().gamma_multiply(dim));
}

fn tip_mut(app: &mut App) -> &mut Tip {
    match app.tool {
        Tool::CloneStamp => &mut app.settings.clone,
        Tool::Blur => &mut app.settings.smear,
        _ => &mut app.settings.brush,
    }
}

/// `BrushControls`: Brush, Eraser, Spot Healing, Clone Stamp and Smear.
fn brush(app: &mut App, ui: &mut Ui) {
    let tool = app.tool;
    let s = &mut app.settings;
    let title = match tool {
        Tool::SpotHealing => "Spot Healing",
        Tool::CloneStamp => "Clone Stamp",
        Tool::Blur => "Smear",
        _ if s.brush_mode == BrushMode::Erase => "Eraser",
        _ => "Brush",
    };
    w::title(ui, title);
    match tool {
        Tool::Brush => {
            w::segmented(ui, &mut s.brush_mode, BrushMode::ALL, BrushMode::title);
        }
        Tool::Blur => {
            w::segmented(ui, &mut s.smear_mode, SmearMode::ALL, SmearMode::title);
        }
        Tool::SpotHealing => {
            w::segmented(ui, &mut s.healing, HealingType::ALL, HealingType::title);
        }
        Tool::CloneStamp => {
            w::checkbox(ui, &mut s.clone_aligned, "Aligned");
            w::segmented(ui, &mut s.clone_sample, SampleLayers::ALL, SampleLayers::title);
        }
        _ => {}
    }
    let smear_mode = s.smear_mode;
    let brush_mode = s.brush_mode;
    let foreground = s.foreground;
    let tip = tip_mut(app);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        scrub(ui, "Size", &mut tip.size, 1.0, 1.0..=2000.0);
        w::number_field(ui, "brush-size", &mut tip.size, 1.0..=2000.0, w::fmt_int, 48.0, false, true);
        w::unit(ui, "px");
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        percent_row(ui, "brush-hardness", "Hardness", &mut tip.hardness, 0.0);
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        percent_row(ui, "brush-opacity", if tool == Tool::Blur { "Strength" } else { "Opacity" }, &mut tip.opacity, 0.01);
    });
    let s = &mut app.settings;
    if tool == Tool::Blur && smear_mode == SmearMode::Blur {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Radius", &mut s.smear_radius, 0.1, 0.5..=50.0);
            let mut slider = s.smear_radius.min(20.0);
            if w::slider(ui, &mut slider, 0.5..=20.0, 100.0, true).changed() {
                s.smear_radius = slider;
            }
            w::number_field(ui, "smear-radius", &mut s.smear_radius, 0.5..=50.0, w::fmt_trim1, 42.0, false, true);
            w::unit(ui, "px");
        });
    }
    if tool == Tool::Brush {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Smoothing", &mut s.smoothing, 1.0, 0.0..=100.0);
            w::slider(ui, &mut s.smoothing, 0.0..=100.0, 100.0, true);
            w::number_field(ui, "smoothing", &mut s.smoothing, 0.0..=100.0, w::fmt_int, 42.0, false, true);
        });
    }
    if matches!(tool, Tool::Brush | Tool::SpotHealing) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            w::label(ui, "Color");
            w::swatch(ui, w::rgb(foreground), vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 });
        });
    }
    let _ = brush_mode;
    if tool == Tool::CloneStamp {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            w::secondary(ui, "Option-click to set the source", 12.0);
        });
    }
}

/// `LassoControls`: Marquee, Lasso and Magic.
fn selection(app: &mut App, ui: &mut Ui) {
    let tool = app.tool;
    let s = &mut app.settings;
    w::title(ui, match tool {
        Tool::Marquee => "Marquee",
        Tool::Lasso => "Lasso",
        _ => "Magic",
    });
    match tool {
        Tool::Marquee => {
            w::segmented(ui, &mut s.marquee, MarqueeKind::ALL, MarqueeKind::title);
        }
        Tool::Wand => {
            w::segmented(ui, &mut s.wand, WandMode::ALL, WandMode::title);
        }
        _ => {
            w::segmented(ui, &mut s.lasso, LassoKind::ALL, LassoKind::title);
        }
    }
    w::segmented(ui, &mut s.selection_mode, SelectionMode::ALL, SelectionMode::title);
    if tool == Tool::Wand && s.wand == WandMode::Wand {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Tolerance", &mut s.tolerance, 1.0, 0.0..=255.0);
            w::number_field(ui, "tolerance", &mut s.tolerance, 0.0..=255.0, w::fmt_int, 44.0, true, true);
        });
        w::popup(ui, "sample-size", &mut s.sample_size, &[SampleSize::ALL], SampleSize::title, None, true);
        w::segmented(ui, &mut s.wand_layers, SampleLayers::ALL, SampleLayers::title);
        w::checkbox(ui, &mut s.contiguous, "Contiguous");
    }
    if tool == Tool::Wand && s.wand == WandMode::Object {
        w::segmented(ui, &mut s.object_layers, SampleLayers::ALL, SampleLayers::title);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Edge", &mut s.object_edge, 1.0, -10.0..=10.0);
            w::number_field(ui, "edge", &mut s.object_edge, -10.0..=10.0, w::fmt_int, 40.0, false, true);
            w::unit(ui, "px");
        });
    }
    if tool == Tool::Lasso || tool == Tool::Wand || s.marquee == MarqueeKind::Ellipse {
        w::checkbox(ui, &mut s.anti_alias, "Anti-alias");
    }
    w::vdivider(ui, 18.0);
    // No selection exists until the selection tools land, so these stay disabled as on the Mac.
    for (title, value, max, width) in [("Expand", &mut s.expand, 500.0, 40.0), ("Contract", &mut s.contract, 500.0, 40.0), ("Feather", &mut s.feather, 250.0, 48.0)] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = if title == "Feather" { 5.0 } else { 8.0 };
            w::button(ui, title, 12.0, ButtonStyle::Bordered, false);
            w::number_field(ui, ("amount", title), value, 1.0..=max, w::fmt_int, width, false, false);
            w::unit(ui, "px");
        });
    }
}

/// `GradientControls`.
fn gradient(app: &mut App, ui: &mut Ui) {
    let s = &mut app.settings;
    w::title(ui, "Gradient");
    w::segmented(ui, &mut s.gradient, GradientKind::ALL, GradientKind::title);
    // Preview: a 4-point checkerboard (gray 45% over white) under the gradient of the current colors.
    let (rect, _) = ui.allocate_exact_size(vec2(56.0, 18.0), Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 3.0, egui::Color32::WHITE);
    for row in 0..5 {
        for col in 0..14 {
            if (row + col) % 2 == 0 {
                let r = Rect::from_min_size(rect.min + vec2(col as f32 * 4.0, row as f32 * 4.0), vec2(4.0, 4.0));
                p.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 115));
            }
        }
    }
    let fg = w::rgb(s.foreground);
    let end = match s.gradient_colors {
        GradientColors::ToBackground => w::rgb(s.background),
        GradientColors::ToTransparent => egui::Color32::TRANSPARENT,
    };
    let (a, b) = if s.gradient_reverse { (end, fg) } else { (fg, end) };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), a);
    mesh.colored_vertex(rect.right_top(), b);
    mesh.colored_vertex(rect.right_bottom(), b);
    mesh.colored_vertex(rect.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::black_alpha(0.5)), egui::StrokeKind::Inside);
    w::popup(ui, "gradient-colors", &mut s.gradient_colors, &[GradientColors::ALL], GradientColors::title, None, true);
    w::checkbox(ui, &mut s.gradient_reverse, "Reverse");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        percent_row(ui, "gradient-opacity", "Opacity", &mut s.gradient_opacity, 0.0);
    });
}

/// `TypeControls`.
fn type_tool(app: &mut App, ui: &mut Ui) {
    let editable = app.doc().and_then(|d| d.active_layer()).is_some_and(|l| l.text.is_some());
    let s = &mut app.settings;
    w::title(ui, "Type");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // The font list is the system's; Helvetica stands in until text rendering exists.
        let mut font = 0usize;
        w::popup(ui, "font", &mut font, &[&[0usize]], |_| "Helvetica", Some(210.0), true);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            w::number_field(ui, "font-size", &mut s.font_size, 1.0..=2000.0, w::fmt_int, 52.0, false, true);
            w::unit(ui, "px");
        });
        w::swatch(ui, w::rgb(s.text_color), vec2(36.0, 18.0), SwatchStyle { radius: 3.0, inner_white: 0.0, outer_black: 0.5 });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (align, symbol) in [(Alignment::Left, "text.alignleft"), (Alignment::Center, "text.aligncenter"), (Alignment::Right, "text.alignright")] {
                let (rect, response) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
                if s.alignment == align {
                    ui.painter().rect_filled(rect, 4.0, white_alpha(0.14));
                }
                crate::icons::paint(ui.painter(), Icon::Symbol(symbol), rect.center(), 13.0, color::label());
                if response.clicked() {
                    s.alignment = align;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            scrub(ui, "Tracking", &mut s.tracking, 1.0, -100.0..=1000.0);
            w::number_field(ui, "tracking", &mut s.tracking, -100.0..=1000.0, w::fmt_int, 45.0, false, true);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            scrub(ui, "Leading", &mut s.leading, 1.0, 0.0..=5000.0);
            if s.leading == 0.0 {
                let mut text = String::new();
                w::text_field(ui, "leading-auto", &mut text, 52.0, "Auto", theme::regular(12.0));
                if let Ok(v) = text.trim().parse::<f64>() {
                    s.leading = v.clamp(0.0, 5000.0);
                }
            } else {
                w::number_field(ui, "leading", &mut s.leading, 0.0..=5000.0, w::fmt_int, 52.0, false, true);
            }
        });
    });
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        w::button(ui, "Edit Text", 12.0, ButtonStyle::Bordered, editable);
    });
}

/// `ShapeControls`.
fn shape(app: &mut App, ui: &mut Ui) {
    let s = &mut app.settings;
    w::title(ui, "Shape");
    w::segmented(ui, &mut s.shape, ShapeKind::ALL, ShapeKind::title);
    match s.shape {
        ShapeKind::Line => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                scrub(ui, "Width", &mut s.line_width, 1.0, 1.0..=5000.0);
                let mut slider = s.line_width.min(100.0);
                if w::slider(ui, &mut slider, 1.0..=100.0, 100.0, true).changed() {
                    s.line_width = slider.round();
                }
                w::number_field(ui, "line-width", &mut s.line_width, 1.0..=5000.0, w::fmt_int, 48.0, true, true);
                w::unit(ui, "px");
            });
        }
        ShapeKind::Rectangle => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                scrub(ui, "Radius", &mut s.corner_radius, 1.0, 0.0..=5000.0);
                let mut slider = s.corner_radius.min(200.0);
                if w::slider(ui, &mut slider, 0.0..=200.0, 100.0, true).changed() {
                    s.corner_radius = slider.round();
                }
                w::number_field(ui, "corner-radius", &mut s.corner_radius, 0.0..=5000.0, w::fmt_int, 48.0, true, true);
                w::unit(ui, "px");
            });
        }
        ShapeKind::Ellipse => {}
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        w::label(ui, "Fill");
        w::swatch(ui, w::rgb(s.foreground), vec2(36.0, 18.0), SwatchStyle { radius: 3.0, inner_white: 0.0, outer_black: 0.5 });
    });
}

/// `CropControls`.
fn crop(app: &mut App, ui: &mut Ui) {
    let size = app.doc().map(|d| (d.project.manifest.width, d.project.manifest.height));
    w::title(ui, "Crop");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        w::labeled_popup(ui, "Ratio", &mut app.settings.crop_ratio, &[CropRatio::ALL], CropRatio::title, 170.0, true);
    });
    if let Some((wd, ht)) = size {
        w::label(ui, &format!("{wd} × {ht} px"));
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        w::button(ui, "Apply Crop", 12.0, ButtonStyle::Bordered, false);
        w::button(ui, "Cancel", 12.0, ButtonStyle::Bordered, false);
    });
}

/// `NavigationToolHeader`: Pan, or Zoom with the zoom field.
fn navigation(app: &mut App, ui: &mut Ui) {
    if app.tool == Tool::Hand {
        w::title(ui, "Pan");
        return;
    }
    w::title(ui, "Zoom");
    let zoom = app.doc().map(|d| d.view.zoom as f64 * 100.0);
    let mut value = zoom.unwrap_or(100.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let response = w::number_field(ui, "zoom-field", &mut value, 0.1..=3200.0, w::fmt_trim2, 72.0, true, zoom.is_some());
        w::unit(ui, "%");
        if response.lost_focus() && zoom.is_some_and(|z| z != value) {
            app.zoom_to((value / 100.0) as f32);
        }
    });
}
