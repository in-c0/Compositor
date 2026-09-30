//! Faces by PostScript name, as `NSFont(name:size:)` finds them: an installed face when there is
//! one, otherwise an open substitute shipped with the port for the Apple faces Windows lacks, and
//! the system font for any other name.

use skrifa::MetadataProvider;
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::StringId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// One face, with the metrics Core Text reads from it, in font units.
pub struct Face {
    pub data: &'static [u8],
    pub index: u32,
    /// The PostScript name of the face actually used.
    pub name: String,
    /// Whether this is the face asked for, rather than a substitute or the system font.
    pub exact: bool,
    pub units_per_em: f64,
    pub ascender: f64,
    /// Negative, below the baseline.
    pub descender: f64,
    pub line_gap: f64,
    /// Has a `trak` table (the system font): Core Text tracks it by point size.
    pub tracks: bool,
    /// Has an optical size axis, which Core Text sets to the point size.
    pub optical: bool,
}

impl Face {
    pub fn font(&self) -> FontRef<'static> {
        FontRef::from_index(self.data, self.index).expect("a face that parsed once")
    }

    fn load(data: &'static [u8], index: u32, exact: bool) -> Option<Face> {
        let font = FontRef::from_index(data, index).ok()?;
        let head = font.head().ok()?;
        let hhea = font.hhea().ok()?;
        Some(Face {
            data,
            index,
            name: postscript_name(&font).unwrap_or_default(),
            exact,
            units_per_em: head.units_per_em() as f64,
            ascender: hhea.ascender().to_i16() as f64,
            descender: hhea.descender().to_i16() as f64,
            line_gap: hhea.line_gap().to_i16() as f64,
            tracks: font.trak().is_ok(),
            optical: font.axes().iter().any(|a| a.tag() == skrifa::Tag::new(b"opsz")),
        })
    }

    /// The variation settings Core Text uses at `size`: optical size set to it.
    pub fn variations(&self, size: f64) -> Vec<(skrifa::Tag, f32)> {
        if self.optical { vec![(skrifa::Tag::new(b"opsz"), size as f32)] } else { Vec::new() }
    }

    /// `NSFont.leading` at `size`: the gap between lines.
    pub fn line_gap_at(&self, size: f64) -> f64 {
        self.line_gap * size / self.units_per_em
    }

    /// `NSFont.descender` at `size`: negative.
    pub fn descender_at(&self, size: f64) -> f64 {
        self.descender * size / self.units_per_em
    }
}

fn postscript_name(font: &FontRef) -> Option<String> {
    let name = font.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first()?;
    Some(name.chars().collect())
}

fn style_of(name: &str) -> (bool, bool) {
    let lower = name.to_ascii_lowercase();
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
    let italic = lower.contains("oblique") || lower.contains("italic");
    (bold, italic)
}

/// What stands in for Helvetica (or Arial) when it isn't installed: installed Arial, which has
/// Helvetica's widths and measures closer to Helvetica's pixels than the open alternatives tried.
/// The port ships no fonts of its own; every Windows has Arial.
fn stand_in(name: &str) -> Option<Face> {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with("helvetica") && !lower.starts_with("arial") {
        return None;
    }
    let arial = match style_of(name) {
        (false, false) => "ArialMT",
        (true, false) => "Arial-BoldMT",
        (false, true) => "Arial-ItalicMT",
        (true, true) => "Arial-BoldItalicMT",
    };
    installed(arial).and_then(|(data, index)| Face::load(data, index, false))
}

/// A sans-serif face that is installed, for when neither SF nor Arial is: Segoe UI on Windows,
/// DejaVu Sans or Liberation Sans on other systems.
fn any_sans() -> Option<Face> {
    ["SegoeUI", "DejaVuSans", "LiberationSans"]
        .into_iter()
        .find_map(|name| installed(name).and_then(|(data, index)| Face::load(data, index, false)))
}

/// A substitute takes the vertical metrics of the face it stands in for, so lines and baselines
/// fall where the Mac puts them (Helvetica.ttc's hhea, as the Mac's `NSFont` reports it).
fn with_apple_metrics(mut face: Face, name: &str) -> Face {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("helvetica") && !lower.starts_with("helveticaneue") {
        let scale = face.units_per_em / 2048.0;
        face.ascender = 1577.0 * scale;
        face.descender = -471.0 * scale;
        face.line_gap = 0.0;
    }
    face
}

/// The system font, `NSFont.systemFont(ofSize:)`: SF Pro where it is installed.
fn system_font() -> &'static Face {
    static SYSTEM: OnceLock<Face> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        for name in [".SFNS-Regular", "SFNS-Regular", "SFPro-Regular", ".SFNSText-Regular"] {
            if let Some((data, index)) = installed(name) {
                if let Some(face) = Face::load(data, index, false) {
                    return face;
                }
            }
        }
        // The system font's file on the Mac, whatever its faces are named.
        if cfg!(target_os = "macos") {
            if let Ok(data) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
                if let Some(face) = Face::load(Box::leak(data.into_boxed_slice()), 0, false) {
                    return face;
                }
            }
        }
        // Elsewhere Arial, whose widths are the closest to SF Pro's at text sizes of the faces at
        // hand, with SF's vertical metrics (SFNS.ttf's hhea: 1980, -432, no gap).
        let mut face = stand_in("Helvetica").or_else(any_sans).expect("a sans-serif font is installed (Arial on Windows)");
        let scale = face.units_per_em / 2048.0;
        face.ascender = 1980.0 * scale;
        face.descender = -432.0 * scale;
        face.line_gap = 0.0;
        face
    })
}

/// The face `NSFont(name:size:) ?? NSFont.systemFont(ofSize:)` gives for `name`.
pub fn face(name: &str) -> &'static Face {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static Face>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(face) = cache.lock().unwrap().get(name) {
        return face;
    }
    let found = installed(name)
        .and_then(|(data, index)| Face::load(data, index, true))
        .or_else(|| stand_in(name).map(|f| with_apple_metrics(f, name)))
        .map(|f| &*Box::leak(Box::new(f)))
        .unwrap_or_else(system_font);
    cache.lock().unwrap().insert(name.to_string(), found);
    found
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Fonts"));
        }
    } else if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        dirs.push(windir.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
    }
    dirs
}

/// Every installed face by PostScript name: the file and the face's index in it.
fn installed_index() -> &'static HashMap<String, (PathBuf, u32)> {
    static INDEX: OnceLock<HashMap<String, (PathBuf, u32)>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        let mut pending = font_dirs();
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            files.sort();
            for path in files {
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                    continue;
                }
                let Ok(data) = std::fs::read(&path) else { continue };
                let Ok(file) = FileRef::new(&data) else { continue };
                for (i, font) in file.fonts().enumerate() {
                    if let Some(name) = font.ok().as_ref().and_then(postscript_name) {
                        index.entry(name).or_insert((path.clone(), i as u32));
                    }
                }
            }
        }
        index
    })
}

/// The bytes of the installed face named `name`, loaded once.
fn installed(name: &str) -> Option<(&'static [u8], u32)> {
    static FILES: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let (path, index) = installed_index().get(name)?;
    let files = FILES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut files = files.lock().unwrap();
    if let Some(data) = files.get(path) {
        return Some((data, *index));
    }
    let data: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
    files.insert(path.clone(), data);
    Some((data, *index))
}
