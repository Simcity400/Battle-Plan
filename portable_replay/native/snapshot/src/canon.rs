//! Canonical JSON serialization for the manifest digest (RFC-8785-style).
//!
//! The `manifest_digest` is SHA-256 over the canonical form of the manifest with
//! `manifest_digest` and `signature` removed. Canonical = object keys sorted
//! lexicographically by UTF-16 code unit (RFC 8785), minimal separators, numbers
//! in their shortest serde_json form. Using the PARSED value (not string
//! concatenation) means a `/` or newline inside a path can never create a
//! delimiter ambiguity (Codex b).
//!
//! Duplicate object keys: the container reader STRICT-parses every trusted JSON
//! body (rejecting duplicate object members at every depth via
//! `crate::strict_json`) before any canonical hashing runs, so the parsed value
//! hashed by [`manifest_canonical_bytes`] cannot differ from the raw body's key
//! set — a duplicated member is a typed refusal, never a last-wins collapse.

use serde_json::{Map, Value};

/// Append the canonical serialization of `v` to `out`.
pub fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_json_string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(e, out);
            }
            out.push(']');
        }
        Value::Object(m) => write_object(m, out),
    }
}

fn write_object(m: &Map<String, Value>, out: &mut String) {
    // sort keys by UTF-16 code units (RFC 8785). For BMP-only ASCII keys this is
    // identical to byte order; we sort on the UTF-16 sequence to be correct for
    // any future non-ASCII manifest key.
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    out.push('{');
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(k, out);
        out.push(':');
        write_canonical(&m[*k], out);
    }
    out.push('}');
}

/// JSON string with RFC 8785 minimal escaping.
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Canonical bytes of a manifest value with the integrity fields stripped, i.e.
/// exactly what `manifest_digest` is computed over.
pub fn manifest_canonical_bytes(manifest: &Value) -> Vec<u8> {
    let mut v = manifest.clone();
    if let Value::Object(m) = &mut v {
        m.remove("manifest_digest");
        m.remove("signature");
    }
    let mut s = String::new();
    write_canonical(&v, &mut s);
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_sorted_and_minimal() {
        let v = json!({ "b": 1, "a": [3, 2], "c": { "y": true, "x": null } });
        let mut s = String::new();
        write_canonical(&v, &mut s);
        assert_eq!(s, r#"{"a":[3,2],"b":1,"c":{"x":null,"y":true}}"#);
    }

    #[test]
    fn strips_integrity_fields_deterministically() {
        let a = json!({ "x": 1, "manifest_digest": "AAA", "signature": "S" });
        let b = json!({ "x": 1, "manifest_digest": "BBB", "signature": null });
        assert_eq!(manifest_canonical_bytes(&a), manifest_canonical_bytes(&b));
    }

    #[test]
    fn string_escaping_is_unambiguous() {
        // a slash and a newline inside a value must not change structure
        let v = json!({ "path": "payload/g/x\n.json" });
        let mut s = String::new();
        write_canonical(&v, &mut s);
        assert_eq!(s, r#"{"path":"payload/g/x\n.json"}"#);
    }
}
