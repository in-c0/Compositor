//! AppKit's controls in the dark appearance, drawn with egui: checkboxes, segmented pickers,
//! sliders, rounded-border fields, capsule buttons and pop-ups, color swatches, scrubbable labels.

use crate::icons::{self, Icon};
use crate::theme::{self, black_alpha, color, metric, white_alpha};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::hash::Hash;
use std::ops::RangeInclusive;

pub fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

/// The font controls use here: 12 points in the tool header (`ToolHeaderStyle.controlFont`),
/// the 13-point system font elsewhere. Set through the `Button` text style.
pub fn control_font(ui: &Ui) -> FontId {
    ui.style().text_styles.get(&egui::TextStyle::Button).cloned().unwrap_or_else(|| theme::regular(13.0))
}

/// The width left for a control that fills a row, given what follows it: `trailing` points of
/// fixed-width items and `gaps` of them (egui leaves the spacing after each item, so every item
/// after the filling one costs one spacing).
pub fn fill_width(ui: &Ui, trailing: f32, gaps: usize) -> f32 {
    (ui.available_width() - trailing - ui.spacing().item_spacing.x * gaps as f32).max(20.0)
}

/// The height SwiftUI gives a line of the system font at `size` points: SF Pro's ascent and
/// descent (0.952 and 0.241 of the size), rounded up, so 16 points for the 13-point body. The
/// 10-point caption measures 13 on the Mac.
pub fn line_height(size: f32) -> f32 {
    if size <= 10.0 { 13.0 } else { (size * 1.193).ceil() }
}

/// Where a line's baseline sits below its top: SF Pro's ascent.
pub fn ascent(size: f32) -> f32 {
    size * 0.952
}

/// The first row's baseline in `galley`, from the galley's top.
pub fn galley_baseline(galley: &egui::Galley) -> f32 {
    galley.rows.first().and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(galley.size().y * 0.78)
}

/// Paints `galley` as a line of `size`-point text whose line box starts at `top_left`, with the
/// baseline where SF Pro's would be.
pub fn paint_line(p: &egui::Painter, galley: std::sync::Arc<egui::Galley>, top_left: egui::Pos2, size: f32, color: Color32) {
    let y = top_left.y + ascent(size) - galley_baseline(&galley);
    p.galley_with_override_text_color(pos2(top_left.x, y), galley, color);
}

/// Paints a laid-out line with its line box centered on `center_y`, the baseline where SF Pro's
/// would be.
pub fn center_line(p: &egui::Painter, galley: std::sync::Arc<egui::Galley>, x: f32, center_y: f32, color: Color32) {
    let size = galley.job.sections.first().map_or(13.0, |s| theme::nominal(s.format.font_id.size));
    paint_line(p, galley, pos2(x, center_y - line_height(size) / 2.0), size, color);
}

/// Paints a line of text vertically centered on `center_y` as SwiftUI centers a line's box.
pub fn paint_centered(p: &egui::Painter, s: &str, font: FontId, color: Color32, x: f32, center_y: f32) -> Rect {
    let size = theme::nominal(font.size);
    let galley = p.layout_no_wrap(s.to_string(), font, color);
    let top = center_y - line_height(size) / 2.0;
    let rect = Rect::from_min_size(pos2(x, top), vec2(galley.size().x, line_height(size)));
    paint_line(p, galley, rect.min, size, color);
    rect
}

/// A non-interactive label, one line of the Mac's height.
pub fn text(ui: &mut Ui, s: impl Into<String>, font: FontId, color: Color32) -> Response {
    let size = theme::nominal(font.size);
    let galley = ui.painter().layout_no_wrap(s.into(), font, color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, line_height(size)), Sense::hover());
    paint_line(ui.painter(), galley, rect.min, size, color);
    response
}

/// One line of text cut to the available width with an ellipsis, as a Text that can't wrap.
pub fn truncated(ui: &mut Ui, s: &str, font: FontId, color: Color32) -> Response {
    let size = theme::nominal(font.size);
    let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, line_height(size)), Sense::hover());
    paint_line(ui.painter(), galley, rect.min, size, color);
    response
}

/// Text wrapped to the available width, each line the Mac's height.
pub fn wrapped(ui: &mut Ui, s: &str, font: FontId, color: Color32) -> Response {
    let size = theme::nominal(font.size);
    let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat { font_id: font, color, line_height: Some(line_height(size)), ..Default::default() });
    job.wrap.max_width = ui.available_width();
    let galley = ui.painter().layout_job(job);
    let lines = galley.rows.len().max(1) as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, lines * line_height(size)), Sense::hover());
    paint_line(ui.painter(), galley, rect.min, size, color);
    response
}

/// A header's title: 13 pt semibold.
pub fn title(ui: &mut Ui, s: &str) -> Response {
    text(ui, s, theme::semibold(13.0), color::label())
}

/// A control label in the control font.
pub fn label(ui: &mut Ui, s: &str) -> Response {
    text(ui, s, control_font(ui), color::label())
}

pub fn secondary(ui: &mut Ui, s: &str, size: f32) -> Response {
    text(ui, s, theme::regular(size), color::secondary())
}

/// A label that changes `value` when dragged sideways (`scrubbable`), `sensitivity` per point.
pub fn scrub_label(ui: &mut Ui, s: &str, font: FontId, color: Color32, value: &mut f64, sensitivity: f64, range: RangeInclusive<f64>, enabled: bool) -> Response {
    let size = theme::nominal(font.size);
    let galley = ui.painter().layout_no_wrap(s.to_string(), font, color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, line_height(size)), if enabled { Sense::drag() } else { Sense::hover() });
    // A disabled SwiftUI container leaves plain text as it is.
    paint_line(ui.painter(), galley, rect.min, size, color);
    if enabled {
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            let dx = response.drag_delta().x as f64;
            *value = (*value + dx * sensitivity).clamp(*range.start(), *range.end());
        }
        return response;
    }
    response
}

/// The Mac's checkbox (macOS 26): a 16-point rounded box, a light well when off and the accent
/// with a white check when on, then the title 6 points on.
pub fn checkbox(ui: &mut Ui, value: &mut bool, title: &str) -> Response {
    let font = control_font(ui);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, color::label());
    // A checkbox row in a VStack takes 16.5 points, the box half a point below its top.
    let size = vec2(16.0 + 6.0 + galley.size().x, 16.5);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let p = ui.painter();
    let bx = Rect::from_min_size(pos2(rect.min.x, (rect.center().y - 7.75).round()), vec2(16.0, 16.0));
    if *value {
        p.rect_filled(bx, 4.5, color::CONTROL_ACCENT);
        let s = Stroke::new(1.7, Color32::WHITE);
        p.line(vec![pos2(bx.min.x + 4.0, bx.center().y + 0.3), pos2(bx.min.x + 6.8, bx.max.y - 4.3), pos2(bx.max.x - 3.8, bx.min.y + 4.0)], s);
    } else {
        p.rect_filled(bx, 4.5, color::control_well());
    }
    center_line(p, galley, bx.max.x + 6.0, bx.center().y, color::label());
    response
}

/// A segmented picker (`.pickerStyle(.segmented)`) in macOS 26: a 24-point capsule track with
/// equal segments, the selected one filled with the accent and titled in white.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, all: &[T], title: impl Fn(T) -> &'static str) -> Response {
    let font = control_font(ui);
    let galleys: Vec<_> = all.iter().map(|v| ui.painter().layout_no_wrap(title(*v).to_string(), font.clone(), color::label())).collect();
    let segment = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max).ceil() + 23.0;
    let total = segment * all.len() as f32;
    let (rect, mut response) = ui.allocate_exact_size(vec2(total, metric::CONTROL_HEIGHT), Sense::click());
    let p = ui.painter();
    let radius = CornerRadius::same((metric::CONTROL_HEIGHT / 2.0) as u8);
    p.rect_filled(rect, radius, color::segment_track());
    let selected = all.iter().position(|v| *v == *value);
    for (i, g) in galleys.into_iter().enumerate() {
        let seg = Rect::from_min_size(pos2(rect.min.x + i as f32 * segment, rect.min.y), vec2(segment, rect.height()));
        let text = if Some(i) == selected {
            p.rect_filled(seg, radius, color::CONTROL_ACCENT);
            Color32::WHITE
        } else {
            color::label()
        };
        // A 14-point divider between two unselected segments.
        if i > 0 && Some(i) != selected && Some(i - 1) != selected {
            let x = seg.min.x.round();
            p.rect_filled(Rect::from_min_max(pos2(x, rect.center().y - 7.0), pos2(x + 1.0, rect.center().y + 7.0)), 0.0, white_alpha(0.1));
        }
        let x = seg.center().x - g.size().x / 2.0;
        center_line(p, g, x, seg.center().y, text);
        if response.clicked() && response.interact_pointer_pos().is_some_and(|pos| seg.contains(pos)) && Some(i) != selected {
            *value = all[i];
            response.mark_changed();
        }
    }
    response
}

/// A horizontal slider, `width` wide (macOS 26): a 6-point track filled with the accent up to a
/// 20 × 16 capsule knob, which travels within the frame.
pub fn slider(ui: &mut Ui, value: &mut f64, range: RangeInclusive<f64>, width: f32, enabled: bool) -> Response {
    slider_with_ticks(ui, value, range, width, enabled, None)
}

/// A `slider` with `ticks` + 1 tick marks under it, as a SwiftUI Slider with a `step` draws:
/// 2-point dots below the track, which then sits near the top of a 16-point frame.
pub fn slider_with_ticks(ui: &mut Ui, value: &mut f64, range: RangeInclusive<f64>, width: f32, enabled: bool, ticks: Option<usize>) -> Response {
    let height = if ticks.is_some() { 16.0 } else { metric::CONTROL_HEIGHT };
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let (lo, hi) = (*range.start(), *range.end());
    let (knob_w, knob_h) = (20.0, 16.0);
    let cy = if ticks.is_some() { rect.min.y + 7.5 } else { rect.center().y.round() };
    let (start, end) = (rect.min.x + knob_w / 2.0, rect.max.x - knob_w / 2.0);
    if enabled && (response.dragged() || response.clicked()) {
        if let Some(pos) = response.interact_pointer_pos() {
            // A track click jumps the knob there (`SliderSnap`), as does a drag.
            let t = ((pos.x - start) / (end - start)).clamp(0.0, 1.0) as f64;
            let v = lo + t * (hi - lo);
            if v != *value {
                *value = v;
                response.mark_changed();
            }
        }
    }
    let t = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let p = ui.painter();
    let track = Rect::from_min_max(pos2(rect.min.x, cy - 3.0), pos2(rect.max.x, cy + 3.0));
    p.rect_filled(track, 3.0, color::control_well());
    if let Some(n) = ticks.filter(|n| *n > 0) {
        for k in 0..=n {
            let x = (start + (end - start) * k as f32 / n as f32).round();
            p.rect_filled(Rect::from_min_size(pos2(x - 1.0, cy + 6.5), vec2(2.0, 2.0)), 0.0, white_alpha(0.187));
        }
    }
    let knob_x = start + t * (end - start);
    // Disabled, the filled part turns a light gray and the knob stays as it is.
    if t > 0.0 {
        p.rect_filled(Rect::from_min_max(track.min, pos2(knob_x, track.max.y)), 3.0, if enabled { color::CONTROL_ACCENT } else { white_alpha(0.12) });
    }
    let knob = Rect::from_center_size(pos2(knob_x, cy), vec2(knob_w, knob_h));
    p.rect_filled(knob.translate(vec2(0.0, 0.5)).expand(0.5), 8.5, black_alpha(0.06));
    p.rect_filled(knob, 8.0, color::KNOB);
    response
}

/// Whole numbers.
pub fn fmt_int(v: f64) -> String {
    let r = v.round();
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

/// Up to `n` decimals, trailing zeros trimmed.
fn trimmed(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}

pub fn fmt_trim1(v: f64) -> String {
    trimmed(v, 1)
}

pub fn fmt_trim2(v: f64) -> String {
    trimmed(v, 2)
}

/// The Transform bar's fields: an integer when whole, else two decimals.
pub fn fmt_whole_or_2(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { fmt_int(v) } else { format!("{v:.2}") }
}

/// A rounded-border field holding a number, clamped to `range`, shown through `fmt`.
pub fn number_field(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut f64, range: RangeInclusive<f64>, fmt: fn(f64) -> String, width: f32, right: bool, enabled: bool) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let buf_id = id.with("text");
    let focused = ui.memory(|m| m.has_focus(id));
    let font = control_font(ui);
    let mut buf = if focused { ui.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| fmt(*value)) } else { fmt(*value) };
    let response = field_frame(ui, width, enabled, &font.clone(), |ui| {
        let edit = egui::TextEdit::singleline(&mut buf)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(font)
            .horizontal_align(if right { Align::Max } else { Align::Min })
            .vertical_align(Align::Min)
            .margin(egui::Margin::symmetric(0, 0));
        ui.add_enabled_ui(enabled, |ui| ui.add_sized(ui.available_size(), edit)).inner
    });
    let mut response = response;
    if response.changed() {
        if let Ok(v) = buf.trim().trim_end_matches(['%', '°']).trim().parse::<f64>() {
            *value = v.clamp(*range.start(), *range.end());
        }
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, buf));
    } else {
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    if response.lost_focus() {
        response.mark_changed();
    }
    response
}

/// A rounded-border text field for free text, with an optional placeholder.
pub fn text_field(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut String, width: f32, placeholder: &str, font: FontId) -> Response {
    let id = ui.make_persistent_id(id_salt);
    field_frame(ui, width, true, &font.clone(), |ui| {
        let edit = egui::TextEdit::singleline(value)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(font)
            .hint_text(placeholder)
            .vertical_align(Align::Min)
            .margin(egui::Margin::symmetric(0, 0));
        ui.add_sized(ui.available_size(), edit)
    })
}

/// A rounded-border field (macOS 26): `textBackgroundColor` inside a faint bezel drawn one point
/// outside the control's frame, text inset 7 points.
fn field_frame(ui: &mut Ui, width: f32, enabled: bool, font: &FontId, content: impl FnOnce(&mut Ui) -> Response) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, metric::CONTROL_HEIGHT), Sense::hover());
    paint_field(ui.painter(), rect);
    // The text sits in a child so the row's cursor stays where the frame left it, placed so its
    // baseline falls where SF Pro's would in a line centered in the field.
    let size = theme::nominal(font.size);
    let baseline = rect.center().y - line_height(size) / 2.0 + ascent(size);
    let sample = ui.painter().layout_no_wrap("0".into(), font.clone(), color::label());
    let top = baseline - galley_baseline(&sample);
    let text = Rect::from_min_max(pos2(rect.min.x + 7.0, top), pos2(rect.max.x - 7.0, rect.max.y));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::left_to_right(Align::Min)));
    if !enabled {
        child.disable();
    }
    content(&mut child)
}

/// A text field's bezel around a control frame `rect`.
pub fn paint_field(p: &egui::Painter, rect: Rect) {
    // The bezel's ring lies over what is behind the field, the fill inside it.
    p.rect_filled(rect.expand(1.0), 8.0, color::field_border());
    p.rect_filled(rect, 7.0, color::FIELD);
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    Bordered,
    Prominent,
}

/// A bordered push button with the capsule shape `.roundedControls()` gives every button: 24
/// points tall, the title 13 points in from each end.
pub fn button(ui: &mut Ui, title: &str, size: f32, style: ButtonStyle, enabled: bool) -> Response {
    let font = theme::regular(size);
    let text_color = match style {
        ButtonStyle::Prominent => Color32::WHITE,
        ButtonStyle::Bordered => color::label(),
    };
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, text_color);
    let height = metric::CONTROL_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x.ceil() + 26.0, height), if enabled { Sense::click() } else { Sense::hover() });
    let pressed = response.is_pointer_button_down_on();
    let fill = match style {
        ButtonStyle::Prominent => if pressed { color::ACCENT.gamma_multiply(0.8) } else { color::ACCENT },
        ButtonStyle::Bordered => if pressed { color::control_pressed() } else { color::control() },
    };
    let dim = if enabled { 1.0 } else { 0.4 };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((height / 2.0) as u8), fill.gamma_multiply(dim));
    center_line(p, galley.clone(), rect.center().x - galley.size().x / 2.0, rect.center().y, text_color.gamma_multiply(dim));
    response
}

/// A pop-up button (`NSPopUpButton` / `.pickerStyle(.menu)`) in macOS 26: a 24-point capsule,
/// the title 13 points in, up-down chevrons centered 13.5 points from the right end. `width`
/// fixes the button's width; `None` fits the widest option, as AppKit sizes a pop-up.
/// Returns true when the value changed.
pub fn popup<T: Copy + PartialEq>(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut T, groups: &[&[T]], title: impl Fn(T) -> &'static str, width: Option<f32>, enabled: bool) -> bool {
    let font = control_font(ui);
    let galley = ui.painter().layout_no_wrap(title(*value).to_string(), font.clone(), color::label());
    let widest = groups
        .iter()
        .flat_map(|g| g.iter())
        .map(|o| ui.painter().layout_no_wrap(title(*o).to_string(), font.clone(), color::label()).size().x)
        .fold(galley.size().x, f32::max);
    let w = width.unwrap_or(widest.ceil() + POPUP_CHROME);
    let (rect, response) = ui.allocate_exact_size(vec2(w, metric::CONTROL_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(egui::CursorIcon::Default);
    let dim = if enabled { 1.0 } else { 0.4 };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((metric::CONTROL_HEIGHT / 2.0) as u8), color::control().gamma_multiply(dim));
    let text_clip = Rect::from_min_max(rect.min, pos2(rect.max.x - 24.0, rect.max.y));
    center_line(&p.with_clip_rect(text_clip), galley, rect.min.x + 13.0, rect.center().y, color::label().gamma_multiply(dim));
    icons::paint(p, Icon::Symbol("chevron.up.chevron.down"), pos2(rect.max.x - 13.5, rect.center().y), 10.0, color::label().gamma_multiply(dim));
    let mut changed = false;
    let _ = id_salt;
    egui::Popup::menu(&response).width(w.max(120.0)).show(|ui| {
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                ui.separator();
            }
            for option in *group {
                let checked = *option == *value;
                let text = egui::RichText::new(title(*option)).font(theme::regular(13.0));
                if ui.add(egui::Button::new(text).selected(checked).frame_when_inactive(false)).clicked() {
                    *value = *option;
                    changed = true;
                }
            }
        }
    });
    changed
}

/// What a fitted pop-up adds to its widest title: 13 points before it, and after it the gap and
/// the chevrons (measured on the Mac: Canvas Size's units make a 122-point pop-up).
pub const POPUP_CHROME: f32 = 54.0;

/// A labeled menu picker: "Label" then the pop-up, `width` wide in all.
pub fn labeled_popup<T: Copy + PartialEq>(ui: &mut Ui, label_text: &str, value: &mut T, groups: &[&[T]], title: impl Fn(T) -> &'static str, width: f32, enabled: bool) -> bool {
    let before = ui.cursor().min.x;
    label(ui, label_text);
    let used = ui.cursor().min.x - before;
    popup(ui, label_text, value, groups, title, Some((width - used).max(60.0)), enabled)
}

#[derive(Clone, Copy)]
pub struct SwatchStyle {
    pub radius: f32,
    /// A white inner ring inset by one point, this wide (0 for none).
    pub inner_white: f32,
    /// The outer border's black alpha.
    pub outer_black: f32,
}

/// Color swatch buttons: `RoundedRectangle` fill, optional white inner ring, black outer border.
pub fn swatch(ui: &mut Ui, fill: Color32, size: egui::Vec2, style: SwatchStyle) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    paint_swatch(ui.painter(), rect, fill, style);
    response
}

pub fn paint_swatch(p: &egui::Painter, rect: Rect, fill: Color32, style: SwatchStyle) {
    p.rect_filled(rect, style.radius, fill);
    if style.inner_white > 0.0 {
        p.rect_stroke(rect.shrink(1.0), (style.radius - 1.0).max(0.0), Stroke::new(style.inner_white, Color32::WHITE), StrokeKind::Inside);
    }
    p.rect_stroke(rect, style.radius, Stroke::new(1.0, black_alpha(style.outer_black)), StrokeKind::Inside);
}

/// `Divider().frame(height:)` inside a row.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::separator());
}

/// A field followed by its unit, two points apart (`unitSuffix`).
pub fn unit(ui: &mut Ui, s: &str) {
    // The row already left its spacing after the field; the unit draws back over all but 2 points.
    let spacing = ui.spacing().item_spacing.x;
    let galley = ui.painter().layout_no_wrap(s.to_string(), control_font(ui), color::label());
    let size = vec2((galley.size().x + 2.0 - spacing).max(0.0), galley.size().y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().galley(pos2(rect.min.x - spacing + 2.0, rect.min.y), galley, color::label());
}

/// The width `unit` takes in a row.
pub fn unit_width(ui: &Ui, s: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(s.to_string(), control_font(ui), color::label());
    (galley.size().x + 2.0 - ui.spacing().item_spacing.x).max(0.0)
}

/// Lays `content` out left to right in `rect`, centered vertically, `spacing` apart.
pub fn row<R>(ui: &mut Ui, rect: Rect, spacing: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing = vec2(spacing, 0.0);
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    content(&mut child)
}
