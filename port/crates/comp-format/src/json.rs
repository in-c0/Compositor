use serde_json::Value;
use std::fmt::Write;

/// Formats JSON the way Swift's `JSONEncoder` does with `[.prettyPrinted, .sortedKeys]`:
/// two-space indents, `"key" : value`, keys in order, `/` escaped, and whole doubles
/// written without a fraction. Matching it keeps a Mac save and a port save diffable.
pub fn to_swift_json(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[\n\n");
                indent(out, depth);
                out.push(']');
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write_value(out, item, depth + 1);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{\n\n");
                indent(out, depth);
                out.push('}');
                return;
            }
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (i, key) in keys.iter().enumerate() {
                indent(out, depth + 1);
                write_string(out, key);
                out.push_str(" : ");
                write_value(out, &map[*key], depth + 1);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push('}');
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_number(out: &mut String, n: &serde_json::Number) {
    match n.as_f64().filter(|_| n.is_f64()) {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => {
            let _ = write!(out, "{}", f as i64);
        }
        Some(f) => {
            let _ = write!(out, "{f}");
        }
        None => {
            let _ = write!(out, "{n}");
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_swift_pretty_sorted_output() {
        let v = json!({"b": 1.0, "a": [0.5, "Hue/Saturation"], "c": {}});
        assert_eq!(
            to_swift_json(&v),
            "{\n  \"a\" : [\n    0.5,\n    \"Hue\\/Saturation\"\n  ],\n  \"b\" : 1,\n  \"c\" : {\n\n  }\n}"
        );
    }
}
