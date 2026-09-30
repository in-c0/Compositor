//! The Type tool's text layers. Every case types its text through the app's own Type tool (the
//! `text` op) or opens an existing text layer and changes it (`editText`), so the Mac rasterizes
//! it with Core Text and the port has to lay it out and draw it the same way.
//!
//! ArialMT, Georgia, Verdana and the other faces in `BOTH` are installed on the Mac and on Windows
//! alike, so those cases compare the port's rasterizer with Core Text on the same outlines.
//! Helvetica and the system font aren't on Windows: those cases measure the substitutes.

use super::builder::{CaseWriter, Doc, LayerSpec};
use super::images;
use anyhow::Result;
use comp_format::*;
use serde_json::{Value, json};

/// A transparent canvas, so the export keeps the glyphs' own coverage in its alpha.
fn blank(w: &CaseWriter, case: &str, width: u32, height: u32) -> Doc {
    let mut d = w.doc("type", case, width, height);
    d.image("Background", images::solid(width, height, [0, 0, 0, 0]), LayerSpec::default());
    d
}

fn boxed(rect: [f64; 4], style: Value) -> Value {
    json!({ "op": "text", "rect": rect, "style": style })
}

/// A case whose text sits in a box filling a transparent canvas, one pixel in from its corner.
fn in_box(w: &mut CaseWriter, case: &str, label: &str, size: [u32; 2], style: Value) -> Result<()> {
    let d = blank(w, case, size[0] + 2, size[1] + 2);
    w.write("type", case, label, d, vec![boxed([1.0, 1.0, size[0] as f64, size[1] as f64], style)])
}

/// Faces installed with both macOS and Windows.
const BOTH: [(&str, &str); 8] = [
    ("ArialMT", "arial"),
    ("Georgia", "georgia"),
    ("Verdana", "verdana"),
    ("TimesNewRomanPSMT", "times-new-roman"),
    ("CourierNewPSMT", "courier-new"),
    ("TrebuchetMS", "trebuchet"),
    ("Tahoma", "tahoma"),
    ("Impact", "impact"),
];

const SAMPLE: &str = "Hamburgefonstiv";

pub fn type_cases(w: &mut CaseWriter) -> Result<()> {
    // Where Core Text puts glyphs: eight lines of eight l's, each glyph an eighth of a pixel further
    // right than the one before it and each line an eighth of a pixel further down.
    let grid = vec!["llllllll"; 8].join("\n");
    in_box(w, "probe-subpixel-arial", "ArialMT l's stepping by an eighth of a pixel across and down", [80, 200], json!({
        "content": grid, "fontName": "ArialMT", "fontSize": 20, "tracking": 6.125 - 455.0 / 2048.0 * 20.0, "leading": 21.125,
    }))?;
    in_box(w, "probe-subpixel-helvetica", "Helvetica l's stepping by an eighth of a pixel across and down", [96, 200], json!({
        "content": grid, "fontName": "Helvetica", "fontSize": 20, "tracking": 1.0, "leading": 21.125,
    }))?;

    // Finer: sixteen l's a sixteenth of a pixel apart across, and sixteen lines a sixteenth apart down.
    in_box(w, "probe-x16-arial", "ArialMT l's stepping by a sixteenth of a pixel across", [124, 40], json!({
        "content": "l".repeat(16), "fontName": "ArialMT", "fontSize": 20, "tracking": 6.0625 - 455.0 / 2048.0 * 20.0,
    }))?;
    let gap = 67.0 / 2048.0 * 20.0;
    in_box(w, "probe-y16-arial", "ArialMT lines stepping by a sixteenth of a pixel down", [40, 364], json!({
        "content": vec!["l"; 16].join("
"), "fontName": "ArialMT", "fontSize": 20, "leading": 21.0625 - gap,
    }))?;
    // The same at other sizes: how finely Core Graphics places glyphs depends on their size.
    for size in [8.0f64, 10.0, 12.0, 14.0, 16.0, 18.0, 22.0, 28.0, 32.0, 36.0, 40.0, 56.0, 72.0, 100.0] {
        let advance = 455.0 / 2048.0 * size;
        let step = (advance + 1.0).ceil() + 1.0 / 16.0;
        let width = (24.0 + 16.0 * step + 4.0).ceil() as u32;
        let height = (size * 1.2 + 30.0).ceil() as u32;
        in_box(w, &format!("probe-x16-arial-{size}"), &format!("ArialMT l's at {size} px stepping by a sixteenth of a pixel across"), [width, height], json!({
            "content": "l".repeat(16), "fontName": "ArialMT", "fontSize": size, "tracking": step - advance,
        }))?;
    }
    for size in [10.0f64, 48.0] {
        let gap = 67.0 / 2048.0 * size;
        let line = (size * 1.2).ceil() + 1.0 / 16.0;
        let height = (24.0 + 16.0 * (line + gap) + 4.0).ceil() as u32;
        in_box(w, &format!("probe-y16-arial-{size}"), &format!("ArialMT lines at {size} px stepping by a sixteenth of a pixel down"), [(size + 30.0) as u32, height], json!({
            "content": vec!["l"; 16].join("
"), "fontName": "ArialMT", "fontSize": size, "leading": line,
        }))?;
    }

    // Core Graphics grows glyph edges by an amount that also depends on the text's color: grays,
    // primaries and white at several sizes, each as a row of stems.
    let stems = |w: &mut CaseWriter, case: &str, label: &str, size: f64, rgb: [f64; 3]| -> Result<()> {
        let advance = 455.0 / 2048.0 * size;
        let step = (advance + 2.0).ceil();
        let width = (24.0 + 4.0 * step + 4.0).ceil() as u32;
        let height = (size * 1.2 + 30.0).ceil() as u32;
        in_box(w, case, label, [width, height], json!({
            "content": "llll", "fontName": "ArialMT", "fontSize": size, "tracking": step - advance,
            "red": rgb[0], "green": rgb[1], "blue": rgb[2],
        }))
    };
    for gray in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0f64] {
        stems(w, &format!("probe-gray-{gray}"), &format!("ArialMT stems at 40 px in gray {gray}"), 40.0, [gray; 3])?;
    }
    for (name, rgb) in [("red", [1.0, 0.0, 0.0]), ("green", [0.0, 1.0, 0.0]), ("blue", [0.0, 0.0, 1.0]), ("yellow", [1.0, 1.0, 0.0]), ("cyan", [0.0, 1.0, 1.0]), ("magenta", [1.0, 0.0, 1.0])] {
        stems(w, &format!("probe-color-{name}"), &format!("ArialMT stems at 40 px in {name}"), 40.0, rgb)?;
    }
    for size in [10.0f64, 20.0, 56.0, 100.0] {
        stems(w, &format!("probe-white-{size}"), &format!("White ArialMT stems at {size} px"), size, [1.0; 3])?;
    }
    // At 10 px the growth stays under its 0.3 px cap for every color, so it can be read off exactly.
    for step in 0..=20 {
        let gray = step as f64 / 20.0;
        stems(w, &format!("probe-gray10-{step:02}"), &format!("ArialMT stems at 10 px in gray {gray}"), 10.0, [gray; 3])?;
    }
    for (name, rgb) in [("red", [1.0, 0.0, 0.0]), ("green", [0.0, 1.0, 0.0]), ("blue", [0.0, 0.0, 1.0]), ("yellow", [1.0, 1.0, 0.0]), ("cyan", [0.0, 1.0, 1.0]), ("magenta", [1.0, 0.0, 1.0]), ("orange", [0.8, 0.3, 0.1]), ("navy", [0.2, 0.3, 0.7]), ("half-red", [0.5, 0.0, 0.0]), ("half-green", [0.0, 0.5, 0.0])] {
        stems(w, &format!("probe-color10-{name}"), &format!("ArialMT stems at 10 px in {name}"), 10.0, rgb)?;
    }

    // How far glyph edges grow with size: a stem, bars above and below the baseline, a bowl and a
    // diagonal, in one face on both machines.
    for size in [10.0f64, 14.0, 20.0, 28.0, 40.0, 56.0, 72.0, 100.0, 140.0, 200.0] {
        let width = (size * 3.2 + 30.0).ceil() as u32;
        let height = (size * 1.4 + 30.0).ceil() as u32;
        in_box(w, &format!("probe-bars-arial-{size}"), &format!("ArialMT stem, bars, bowl and diagonal at {size} px"), [width, height], json!({
            "content": "I_-o/", "fontName": "ArialMT", "fontSize": size, "tracking": size * 0.1,
        }))?;
    }

    // Sizes, in a face on both machines and in the default face.
    for (face, key) in [("ArialMT", "arial"), ("Helvetica", "helvetica")] {
        for (size, bw, bh) in [(8.0, 110, 36), (12.0, 140, 40), (24.0, 240, 56), (48.0, 440, 90)] {
            let case = format!("size-{key}-{size}");
            in_box(w, &case, &format!("{face} at {size} px"), [bw, bh], json!({ "content": SAMPLE, "fontName": face, "fontSize": size }))?;
        }
        let case = format!("size-{key}-200");
        in_box(w, &case, &format!("{face} at 200 px"), [300, 280], json!({ "content": "Ag", "fontName": face, "fontSize": 200 }))?;
    }
    for (face, key) in BOTH.iter().skip(1) {
        in_box(w, &format!("face-{key}"), &format!("{face} at 20 px"), [260, 48], json!({ "content": "Hamburgefonstiv 0123", "fontName": face, "fontSize": 20 }))?;
    }
    in_box(w, "face-missing", "A face that isn't installed falls back to the system font", [260, 48], json!({ "content": "Hamburgefonstiv 0123", "fontName": "NoSuchFont-Regular", "fontSize": 20 }))?;
    in_box(w, "face-helvetica-bold", "Helvetica-Bold at 24 px", [260, 56], json!({ "content": SAMPLE, "fontName": "Helvetica-Bold", "fontSize": 24 }))?;

    // Colors, and colored text over a backdrop.
    in_box(w, "color-arial", "Colored ArialMT on a transparent canvas", [240, 56], json!({ "content": SAMPLE, "fontName": "ArialMT", "fontSize": 24, "red": 0.8, "green": 0.3, "blue": 0.1 }))?;
    let mut d = w.doc("type", "over-photo", 160, 64);
    d.image("Photo", images::photo(160, 64), LayerSpec::default());
    w.write("type", "over-photo", "Colored text over an opaque backdrop", d, vec![boxed([2.0, 2.0, 156.0, 60.0], json!({ "content": "Over it", "fontName": "ArialMT", "fontSize": 30, "red": 0.95, "green": 0.9, "blue": 0.2 }))])?;

    // Kerning and ligatures: whether they apply with tracking at 0 and with a little tracking.
    for (face, key) in [("ArialMT", "arial"), ("Helvetica", "helvetica")] {
        for (tracking, tkey) in [(0.0, "0"), (0.5, "0.5")] {
            let case = format!("kern-{key}-{tkey}");
            in_box(w, &case, &format!("{face} kerning pairs and ligatures, tracking {tracking}"), [300, 56], json!({
                "content": "AVATAR To. Wa fi fl", "fontName": face, "fontSize": 24, "tracking": tracking,
            }))?;
        }
    }

    // Tracking and leading at their extremes.
    for (tracking, key) in [(-3.0, "minus-3"), (25.0, "25")] {
        in_box(w, &format!("tracking-{key}"), &format!("Tracking {tracking}"), [300, 56], json!({ "content": "Tracked", "fontName": "ArialMT", "fontSize": 24, "tracking": tracking }))?;
    }
    for (leading, key) in [(10.0, "10"), (48.0, "48")] {
        in_box(w, &format!("leading-{key}"), &format!("Three lines at leading {leading}"), [120, 180], json!({ "content": "One\nTwo\nThree", "fontName": "ArialMT", "fontSize": 20, "leading": leading }))?;
    }

    // Alignment and wrapping inside a box.
    let paragraph = "The quick brown fox jumps over the lazy dog, twice over.";
    for align in ["Left", "Center", "Right"] {
        let case = format!("wrap-{}", align.to_lowercase());
        in_box(w, &case, &format!("A wrapping paragraph, aligned {align}"), [150, 130], json!({ "content": paragraph, "fontName": "ArialMT", "fontSize": 16, "alignment": align }))?;
    }
    in_box(w, "wrap-helvetica", "A wrapping paragraph in Helvetica", [150, 130], json!({ "content": paragraph, "fontName": "Helvetica", "fontSize": 16 }))?;
    in_box(w, "wrap-long-word", "A word longer than its box breaks across lines", [90, 110], json!({ "content": "Supercalifragilistic word", "fontName": "ArialMT", "fontSize": 18 }))?;
    in_box(w, "wrap-overflow", "More lines than fit in the box", [100, 40], json!({ "content": "One\nTwo\nThree\nFour", "fontName": "ArialMT", "fontSize": 18 }))?;

    // Runs: some letters in another color, some in another face.
    in_box(w, "color-runs", "Letters colored by runs", [260, 56], json!({
        "content": "Red, green and blue", "fontName": "ArialMT", "fontSize": 24,
        "colorRuns": [
            { "location": 0, "length": 3, "red": 0.9, "green": 0.1, "blue": 0.1 },
            { "location": 5, "length": 5, "red": 0.1, "green": 0.7, "blue": 0.2 },
            { "location": 15, "length": 4, "red": 0.1, "green": 0.2, "blue": 0.9 },
        ],
    }))?;
    in_box(w, "font-runs", "Letters set in other faces by runs", [300, 56], json!({
        "content": "Arial Georgia Verdana", "fontName": "ArialMT", "fontSize": 22,
        "fontRuns": [
            { "location": 6, "length": 7, "fontName": "Georgia" },
            { "location": 14, "length": 7, "fontName": "Verdana" },
        ],
    }))?;
    in_box(w, "font-runs-helvetica", "Helvetica with a Helvetica-Bold run", [260, 56], json!({
        "content": "Regular Bold", "fontName": "Helvetica", "fontSize": 24,
        "fontRuns": [{ "location": 8, "length": 4, "fontName": "Helvetica-Bold" }],
    }))?;

    // Point text: a click, so the layer is as big as what's typed and its baseline is on the click.
    let d = blank(w, "point-arial", 200, 80);
    w.write("type", "point-arial", "Point text in ArialMT, its baseline on the click", d, vec![json!({
        "op": "text", "point": [20, 50], "style": { "content": "Point text", "fontName": "ArialMT", "fontSize": 24 },
    })])?;
    let d = blank(w, "point-default", 260, 120);
    w.write("type", "point-default", "Point text in the Type tool's defaults: Helvetica at 72 px", d, vec![json!({
        "op": "text", "point": [10, 90], "style": { "content": "Text" },
    })])?;

    // Editing a text layer that is already there, upright and transformed.
    for (case, label, transform) in [
        ("edit", "An existing text layer retyped and resized", Transform::at(4.0, 4.0, 100.0, 40.0)),
        ("edit-rotated", "A rotated, scaled text layer retyped", Transform { rotation: 20.0, ..Transform::at(20.0, 10.0, 150.0, 60.0) }),
    ] {
        let mut d = blank(w, case, 180, 100);
        let id = d.image("Old text", images::solid(100, 40, [90, 90, 90, 255]), LayerSpec { transform: Some(transform), ..LayerSpec::default() });
        d.layer(&id).text = Some(TextStyle {
            content: "Old text".into(),
            font_name: "ArialMT".into(),
            font_size: 20.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alignment: TextAlignment::Left,
            tracking: 0.0,
            leading: 0.0,
            box_size: None,
            color_runs: None,
            font_runs: None,
        });
        w.write("type", case, label, d, vec![json!({
            "op": "editText", "layer": id, "style": { "content": "New words", "fontSize": 26, "red": 0.2, "green": 0.3, "blue": 0.7 },
        })])?;
    }
    Ok(())
}
