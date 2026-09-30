//! Whether the Mac keeps a Photoshop type layer as editable text (`PSDText.parse`). The type tool
//! object is read as the Mac reads it: a 2 x 3 transform, the text descriptor (and, when the text
//! isn't in it, the engine data's `/Text`), and the paragraph frame. Text the model can't hold
//! (vertical, sheared or unevenly scaled, empty, or a frame it can't store) stays pixels. The text
//! itself is drawn by Core Text, which the port doesn't reproduce, so the import reports editable
//! text as not supported.

use std::collections::HashMap;

/// `LayerTextStyle.padding`.
const PADDING: f64 = 12.0;

pub fn parses(extra: &HashMap<String, Vec<u8>>) -> bool {
    parse(extra).is_some()
}

fn parse(extra: &HashMap<String, Vec<u8>>) -> Option<()> {
    let data = extra.get("TySh").or_else(|| extra.get("tySh"))?;
    if data.len() > 8_000_000 {
        return None;
    }
    let mut reader = Reader { data, offset: 0 };
    if reader.u16()? != 1 {
        return None;
    }
    let m: Vec<f64> = (0..6).map(|_| reader.f64()).collect::<Option<_>>()?;
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    if reader.u16()? != 50 {
        return None;
    }
    let text = reader.descriptor(true)?;
    if let Some(Value::Enumeration(o)) = text.get("Ornt")
        && o == "Vrtc"
    {
        return None;
    }
    let pixel_scale = placement(m[0], m[1], m[2], m[3])?;
    let engine = match text.get("EngineData") {
        Some(Value::Data(d)) => engine_value(d),
        _ => None,
    };
    let string = |key: &str| match text.get(key) {
        Some(Value::Text(t)) => Some(t),
        _ => None,
    };
    let from_descriptor = string("Txt ").or_else(|| string("Txt")).map(|t| cleaned(t));
    let content = from_descriptor.or_else(|| match walk(engine.as_ref(), &["EngineDict", "Editor", "Text"]) {
        Some(Engine::String(s)) => Some(cleaned(s)),
        _ => None,
    })?;
    if content.is_empty() || content.encode_utf16().count() > 100_000 {
        return None;
    }
    // Font size, color, tracking and leading are clamped into range as they're read, so only the
    // paragraph frame can still make the style invalid.
    if let (Some(bounds), Some(glyphs)) = (rect(&text, "bounds"), rect(&text, "boundingBox"))
        && bounds.2 > glyphs.2 + 4.0
        && bounds.3 > glyphs.3 + 4.0
        && bounds.2 > 1.0
        && bounds.3 > 1.0
    {
        let (w, h) = (bounds.2 * pixel_scale + PADDING * 2.0, bounds.3 * pixel_scale + PADDING * 2.0);
        let valid = w.is_finite() && h.is_finite() && (16.0..=30_000.0).contains(&w) && (16.0..=30_000.0).contains(&h) && w * h <= 200_000_000.0;
        if !valid {
            return None;
        }
    }
    Some(())
}

/// Uniform scale, rotation and an optional vertical flip; shear and uneven scale are refused.
/// Returns the pixel scale.
fn placement(xx: f64, xy: f64, yx: f64, yy: f64) -> Option<f64> {
    let scale_x = xx.hypot(yx);
    if scale_x <= 1e-6 {
        return None;
    }
    let (cos, sin) = (xx / scale_x, yx / scale_x);
    let local_x = cos * xy + sin * yy;
    let local_y = -sin * xy + cos * yy;
    let scale_y = local_y.abs();
    if scale_y <= 1e-6 {
        return None;
    }
    let largest = scale_x.max(scale_y);
    if local_x.abs() > 0.02 * largest || (scale_x - scale_y).abs() > 0.02 * largest {
        return None;
    }
    (scale_x.is_finite() && scale_x > 0.0).then_some(scale_x)
}

fn cleaned(text: &str) -> String {
    let text = text.trim_start_matches(['\u{feff}', '\0']).trim_end_matches('\0');
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `left, top, width, height` of a `Left`/`Top `/`Rght`/`Btom` descriptor.
fn rect(items: &HashMap<String, Value>, key: &str) -> Option<(f64, f64, f64, f64)> {
    let Some(Value::Descriptor(d)) = items.get(key) else { return None };
    let side = |name: &str| match d.get(name).or_else(|| d.get(name.trim())) {
        Some(Value::Number(v)) => Some(*v),
        _ => None,
    };
    let (l, t, r, b) = (side("Left")?, side("Top ")?, side("Rght")?, side("Btom")?);
    [l, t, r, b].iter().all(|v| v.is_finite()).then_some((l, t, r - l, b - t))
}

enum Value {
    Text(String),
    Number(f64),
    Enumeration(String),
    Data(Vec<u8>),
    Descriptor(HashMap<String, Value>),
    List,
}

/// The descriptor walker (Photoshop File Formats: class and keys length-prefixed, or 4 bytes when
/// the length is 0).
struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(count)?;
        if end > self.data.len() {
            return None;
        }
        let s = &self.data[self.offset..end];
        self.offset = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(self.u32()? as i32)
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(u64::from_be_bytes(self.bytes(8)?.try_into().ok()?)))
    }
    fn four_cc(&mut self) -> Option<String> {
        let raw = self.bytes(4)?;
        raw.is_ascii().then(|| raw.iter().map(|&b| b as char).collect())
    }
    fn unicode(&mut self) -> Option<String> {
        let count = self.u32()?;
        if count > 1_000_000 {
            return None;
        }
        let raw = self.bytes(count as usize * 2)?;
        let units: Vec<u16> = raw.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).ok()
    }
    fn identifier(&mut self) -> Option<String> {
        let length = self.u32()?;
        if length == 0 {
            return self.four_cc();
        }
        if length > 10_000 {
            return None;
        }
        let raw = self.bytes(length as usize)?;
        raw.is_ascii().then(|| raw.iter().map(|&b| b as char).collect())
    }

    fn descriptor(&mut self, versioned: bool) -> Option<HashMap<String, Value>> {
        if versioned && self.u32()? != 16 {
            return None;
        }
        self.unicode()?;
        self.identifier()?;
        let count = self.u32()?;
        if count > 10_000 {
            return None;
        }
        let mut items = HashMap::new();
        for _ in 0..count {
            let key = self.identifier()?;
            let kind = self.four_cc()?;
            let value = self.value(&kind)?;
            items.insert(key, value);
        }
        Some(items)
    }

    fn value(&mut self, kind: &str) -> Option<Value> {
        Some(match kind {
            "doub" => Value::Number(self.f64()?),
            "UntF" => {
                self.four_cc()?;
                Value::Number(self.f64()?)
            }
            "long" => Value::Number(self.i32()? as f64),
            "comp" => Value::Number(i64::from_be_bytes(self.bytes(8)?.try_into().ok()?) as f64),
            // The Mac reads booleans as the number 0.
            "bool" => {
                self.u8()?;
                Value::Number(0.0)
            }
            "TEXT" => Value::Text(self.unicode()?),
            "enum" => {
                self.identifier()?;
                Value::Enumeration(self.identifier()?)
            }
            "tdta" => {
                let length = self.u32()?;
                if length > 8_000_000 {
                    return None;
                }
                Value::Data(self.bytes(length as usize)?.to_vec())
            }
            "Objc" | "GlbO" => Value::Descriptor(self.descriptor(false)?),
            "VlLs" => {
                let count = self.u32()?;
                if count > 10_000 {
                    return None;
                }
                for _ in 0..count {
                    let kind = self.four_cc()?;
                    self.value(&kind)?;
                }
                Value::List
            }
            "alis" => {
                let length = self.u32()?;
                if length > 8_000_000 {
                    return None;
                }
                self.bytes(length as usize)?;
                Value::Number(0.0)
            }
            "obj " => {
                if !self.reference() {
                    return None;
                }
                Value::Number(0.0)
            }
            "type" | "GlbC" => {
                self.unicode()?;
                self.identifier()?;
                Value::Number(0.0)
            }
            _ => return None,
        })
    }

    fn reference(&mut self) -> bool {
        let Some(count) = self.u32() else { return false };
        if count > 10_000 {
            return false;
        }
        for _ in 0..count {
            let Some(form) = self.four_cc() else { return false };
            let ok = match form.as_str() {
                "prop" => self.unicode().is_some() && self.identifier().is_some() && self.identifier().is_some(),
                "Clss" => self.unicode().is_some() && self.identifier().is_some(),
                "Enmr" => self.unicode().is_some() && self.identifier().is_some() && self.identifier().is_some() && self.identifier().is_some(),
                "rele" => self.unicode().is_some() && self.identifier().is_some() && self.i32().is_some(),
                "Idnt" | "indx" => self.i32().is_some(),
                "name" => self.unicode().is_some(),
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

/// Photoshop's text-engine dictionary, a small PostScript-like language.
enum Engine {
    Number,
    Bool,
    String(String),
    Dict(HashMap<String, Engine>),
    Array,
}

fn walk<'a>(value: Option<&'a Engine>, keys: &[&str]) -> Option<&'a Engine> {
    let mut current = value?;
    for key in keys {
        let Engine::Dict(items) = current else { return None };
        current = items.get(*key)?;
    }
    Some(current)
}

fn engine_value(data: &[u8]) -> Option<Engine> {
    if let Some(dict) = dictionary(data, 0) {
        return Some(dict);
    }
    let start = data.windows(2).position(|w| w == b"<<")?;
    if start == 0 {
        return None;
    }
    dictionary(data, start)
}

fn dictionary(data: &[u8], start: usize) -> Option<Engine> {
    let mut cursor = EngineCursor { bytes: data, index: start };
    match cursor.value()? {
        Engine::Dict(items) => Some(Engine::Dict(items)),
        _ => None,
    }
}

struct EngineCursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl EngineCursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn value(&mut self) -> Option<Engine> {
        self.skip_whitespace();
        let byte = self.peek()?;
        match byte {
            b'<' if self.bytes.get(self.index + 1) == Some(&b'<') => self.dictionary(),
            b'<' => self.hex(),
            b'[' => self.array(),
            b'(' => self.string(),
            b'/' => {
                self.index += 1;
                Some(Engine::String(self.token()))
            }
            b'-' | b'+' | b'.' | b'0'..=b'9' => self.number().map(|_| Engine::Number),
            _ => {
                if self.take_word("true") || self.take_word("false") {
                    Some(Engine::Bool)
                } else if self.take_word("null") {
                    Some(Engine::String(String::new()))
                } else {
                    None
                }
            }
        }
    }

    fn dictionary(&mut self) -> Option<Engine> {
        if !self.take(b"<<") {
            return None;
        }
        let mut items = HashMap::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None | Some(b'>') => break,
                Some(b'/') => {}
                _ => return None,
            }
            self.index += 1;
            let key = self.token();
            let value = self.value()?;
            items.insert(key, value);
        }
        self.take(b">>").then_some(Engine::Dict(items))
    }

    fn array(&mut self) -> Option<Engine> {
        if !self.take(b"[") {
            return None;
        }
        loop {
            self.skip_whitespace();
            if matches!(self.peek(), None | Some(b']')) {
                break;
            }
            self.value()?;
        }
        self.take(b"]").then_some(Engine::Array)
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.index;
        let digits = |c: &mut Self| {
            while matches!(c.peek(), Some(b'0'..=b'9')) {
                c.index += 1;
            }
        };
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.index += 1;
        }
        digits(self);
        if self.peek() == Some(b'.') {
            self.index += 1;
            digits(self);
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            digits(self);
        }
        if self.index <= start {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.index]).ok()?.parse().ok()
    }

    fn string(&mut self) -> Option<Engine> {
        if !self.take(b"(") {
            return None;
        }
        let mut raw = Vec::new();
        while let Some(byte) = self.peek() {
            self.index += 1;
            if byte == b')' {
                break;
            }
            if byte == b'\\' {
                let escaped = self.peek()?;
                self.index += 1;
                match escaped {
                    b'n' => raw.push(0x0A),
                    b'r' => raw.push(0x0D),
                    b't' => raw.push(0x09),
                    b'0'..=b'7' => {
                        let mut value = (escaped - b'0') as u32;
                        for _ in 0..2 {
                            match self.peek() {
                                Some(d @ b'0'..=b'7') => {
                                    self.index += 1;
                                    value = value * 8 + (d - b'0') as u32;
                                }
                                _ => break,
                            }
                        }
                        raw.push((value & 0xFF) as u8);
                    }
                    b'\n' | b'\r' => {}
                    other => raw.push(other),
                }
            } else {
                raw.push(byte);
            }
        }
        Some(Engine::String(decode_engine(&raw)))
    }

    fn hex(&mut self) -> Option<Engine> {
        if !self.take(b"<") {
            return None;
        }
        let mut nibbles = Vec::new();
        while let Some(byte) = self.peek() {
            if byte == b'>' {
                break;
            }
            self.index += 1;
            if let Some(n) = (byte as char).to_digit(16) {
                nibbles.push(n as u8);
            }
        }
        if !self.take(b">") {
            return None;
        }
        let raw: Vec<u8> = nibbles.chunks_exact(2).map(|p| p[0] << 4 | p[1]).collect();
        Some(Engine::String(decode_engine(&raw)))
    }

    fn token(&mut self) -> String {
        let start = self.index;
        while let Some(byte) = self.peek() {
            if is_delimiter(byte) {
                break;
            }
            self.index += 1;
        }
        let raw = &self.bytes[start..self.index];
        if raw.is_ascii() { raw.iter().map(|&b| b as char).collect() } else { String::new() }
    }

    fn take_word(&mut self, word: &str) -> bool {
        let w = word.as_bytes();
        if !self.bytes[self.index..].starts_with(w) {
            return false;
        }
        let after = self.index + w.len();
        if after < self.bytes.len() && !is_delimiter(self.bytes[after]) {
            return false;
        }
        self.index = after;
        true
    }

    fn take(&mut self, token: &[u8]) -> bool {
        if self.bytes[self.index.min(self.bytes.len())..].starts_with(token) {
            self.index += token.len();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if byte == b'%' {
                while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                    self.index += 1;
                }
            } else if byte <= 0x20 {
                self.index += 1;
            } else {
                break;
            }
        }
    }
}

fn is_delimiter(byte: u8) -> bool {
    byte <= 0x20 || matches!(byte, b'/' | b'<' | b'>' | b'[' | b']' | b'(' | b')')
}

fn decode_engine(raw: &[u8]) -> String {
    if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
        let body = &raw[2..];
        if body.len() % 2 != 0 {
            return String::new();
        }
        let units: Vec<u16> = body.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16(&units).unwrap_or_default();
    }
    raw.iter().map(|&b| b as char).collect()
}
