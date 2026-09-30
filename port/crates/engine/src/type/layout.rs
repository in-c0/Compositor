//! Where each glyph goes: `NSLayoutManager` laying `EditorSession.attributedText(style)` into a
//! text container (TextKit 1), each line `lineHeight` tall plus its fonts' line gap.
//!
//! What the Mac does, read off the harness's `textLayout` notes:
//! - Each run of one face is shaped on its own, with its standard ligatures. Kerning applies only
//!   when tracking isn't 0: the `.kern` attribute set to exactly 0 turns the font's kerning off.
//! - Tracking is added after every glyph, the last on a line included (a ligature gets it once).
//! - A line fragment is `lineHeight` plus the largest line gap (hhea lineGap) of its fonts; the
//!   baseline sits the largest descent, rounded to a whole pixel, above `lineHeight`.
//! - Lines break after spaces and hyphens, trailing spaces hanging past the edge; a word wider
//!   than the line breaks between letters. A line that doesn't fit in the container isn't laid
//!   out, except the first.
//! - Alignment and the measured width leave out a line's trailing whitespace.

use super::fonts::{self, Face};
use comp_format::{TextAlignment, TextStyle};
use harfrust::{Feature, ShapeOptions, ShaperData, ShaperInstance, Tag, UnicodeBuffer, Variation};

pub struct Glyph {
    pub face: &'static Face,
    pub id: u32,
    /// The glyph's origin on its baseline, from the text container's top left.
    pub x: f64,
    pub y: f64,
    pub color: [f64; 3],
    /// Spaces and line breaks: laid out, but nothing to draw.
    pub blank: bool,
}

pub struct Layout {
    pub glyphs: Vec<Glyph>,
    /// The widest line's used width and the lines' total height, as `boundingRect` measures them.
    pub width: f64,
    pub height: f64,
}

/// One shaped glyph, before it is placed on a line.
struct Item {
    face: &'static Face,
    glyph: u32,
    /// Its advance with tracking and kerning, in pixels.
    advance: f64,
    offset: (f64, f64),
    color: [f64; 3],
    /// The first character it draws.
    ch: char,
}

impl Item {
    fn is_space(&self) -> bool {
        self.ch.is_whitespace()
    }
}

fn is_break(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn line_height(style: &TextStyle) -> f64 {
    if style.leading > 0.0 { style.leading } else { style.font_size * 1.2 }
}

fn run_at<'a, T>(runs: &'a Option<Vec<T>>, unit: usize, span: impl Fn(&T) -> (i64, i64)) -> Option<&'a T> {
    runs.iter().flatten().find(|r| {
        let (location, length) = span(r);
        location as usize <= unit && unit < (location + length) as usize
    })
}

/// The paragraph's characters shaped run by run, one run per face.
fn shape(style: &TextStyle, chars: &[(char, usize)]) -> Vec<Item> {
    let size = style.font_size;
    let mut items = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let face_name = |i: usize| {
            run_at(&style.font_runs, chars[i].1, |r| (r.location, r.length)).map(|r| r.font_name.as_str()).unwrap_or(&style.font_name)
        };
        let name = face_name(start);
        let end = (start..chars.len()).find(|&i| face_name(i) != name).unwrap_or(chars.len());
        let face = fonts::face(name);
        let font = face.font();
        let data = ShaperData::new(&font);
        let variations = face.variations(size);
        let instance = ShaperInstance::from_variations(&font, variations.iter().map(|&(tag, value)| Variation { tag, value }));
        let shaper = data.shaper(&font).instance((!variations.is_empty()).then_some(&instance)).build();
        let mut buffer = UnicodeBuffer::new();
        for (i, &(ch, _)) in chars[start..end].iter().enumerate() {
            buffer.add(ch, i as u32);
        }
        buffer.guess_segment_properties();
        let features = [Feature::new(Tag::new(b"kern"), (style.tracking != 0.0) as u32, ..)];
        let shaped = shaper.shape(buffer, ShapeOptions::new().features(&features).point_size(face.tracks.then_some(size as f32)));
        let scale = size / face.units_per_em;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let (ch, unit) = chars[start + info.cluster as usize];
            let color = run_at(&style.color_runs, unit, |r| (r.location, r.length)).map(|r| [r.red, r.green, r.blue]).unwrap_or([style.red, style.green, style.blue]);
            items.push(Item {
                face,
                glyph: info.glyph_id,
                advance: pos.x_advance as f64 * scale + style.tracking,
                offset: (pos.x_offset as f64 * scale, -pos.y_offset as f64 * scale),
                color,
                ch,
            });
        }
        start = end;
    }
    items
}

/// Lays `style` out in a container `width` by `height`.
pub fn layout(style: &TextStyle, width: f64, height: f64) -> Layout {
    let mut chars = Vec::new();
    let mut unit = 0;
    for ch in style.content.chars() {
        chars.push((ch, unit));
        unit += ch.len_utf16();
    }
    let line = line_height(style);
    let main = fonts::face(&style.font_name);
    let mut out = Layout { glyphs: Vec::new(), width: 0.0, height: 0.0 };
    let mut lines = 0;
    let mut start = 0;
    loop {
        let end = chars[start..].iter().position(|c| is_break(c.0)).map(|i| start + i).unwrap_or(chars.len());
        let items = shape(style, &chars[start..end]);
        let mut from = 0;
        loop {
            let to = from + break_line(&items[from..], width);
            let used = &items[from..to];
            let faces: Vec<&Face> = if used.is_empty() { vec![main] } else { used.iter().map(|i| i.face).collect() };
            let gap = faces.iter().map(|f| f.line_gap_at(style.font_size)).fold(0.0, f64::max);
            let descent = faces.iter().map(|f| -f.descender_at(style.font_size)).fold(0.0, f64::max);
            let fragment = line + gap;
            if out.height + fragment > height && lines > 0 {
                return out;
            }
            let full: f64 = used.iter().map(|i| i.advance).sum();
            let visible = full - used.iter().rev().take_while(|i| i.is_space()).map(|i| i.advance).sum::<f64>();
            out.width = out.width.max(visible);
            let baseline = out.height + line - descent.round();
            let mut x = match style.alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => (width - visible) / 2.0,
                TextAlignment::Right => width - visible,
            };
            for item in used {
                out.glyphs.push(Glyph {
                    face: item.face,
                    id: item.glyph,
                    x: x + item.offset.0,
                    y: baseline + item.offset.1,
                    color: item.color,
                    blank: item.is_space(),
                });
                x += item.advance;
            }
            out.height += fragment;
            lines += 1;
            from = to;
            if from >= items.len() {
                break;
            }
        }
        if end >= chars.len() {
            break;
        }
        start = end + 1;
    }
    out
}

/// How many of `items` fit on a line `width` wide: whole words where they can, trailing spaces
/// hanging past the edge; a word wider than the line breaks between letters.
fn break_line(items: &[Item], width: f64) -> usize {
    let mut x = 0.0;
    let mut last_break = None;
    for (i, item) in items.iter().enumerate() {
        if item.is_space() {
            x += item.advance;
            last_break = Some(i + 1);
            continue;
        }
        if x + item.advance > width && i > 0 {
            return last_break.unwrap_or(i);
        }
        x += item.advance;
        if item.ch == '-' {
            last_break = Some(i + 1);
        }
    }
    items.len()
}
