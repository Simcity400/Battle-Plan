//! The fold's document tree converted straight into the canonical value model
//! the replay cache encoder consumes, with no JSON text in between.
//!
//! The text the encoder hashes is exactly what `serde_json` would have written
//! for the same tree: strings and keys are escaped by the same table, numbers
//! are spelled by the same formatter, and members come in the same map order.
//! So the digest [`CanonValue::json_sha256_hex`] takes over the converted tree
//! equals the digest of the serialized document, which is what every recorded
//! `world_json_sha256` was and stays.
//!
//! Every subtree equal to its counterpart in the previous hour is SHARED with
//! it (an `Arc` bump, no rebuild), so the encoder's diff recognises it by
//! pointer and the text memo the previous hour filled carries over. The walk
//! is single-pass: a node is compared with its counterpart while its children
//! are converted, and nothing is allocated for a subtree that turns out equal.

use std::cmp::Ordering;
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

use crate::canon_value::{key_cmp_decoded, CanonValue};

pub fn from_value_shared(value: &Value, previous: Option<&CanonValue>) -> CanonValue {
    match convert(value, previous) {
        Converted::Shared => previous
            .expect("a subtree is only shared with a counterpart that exists")
            .clone(),
        Converted::Fresh(canon) => canon,
    }
}

enum Converted {
    /// Equal to its counterpart: the caller shares the counterpart.
    Shared,
    /// Built anew, sharing whatever children were equal.
    Fresh(CanonValue),
}

fn convert(value: &Value, previous: Option<&CanonValue>) -> Converted {
    match value {
        Value::Null => match previous {
            Some(CanonValue::Null) => Converted::Shared,
            _ => Converted::Fresh(CanonValue::Null),
        },
        Value::Bool(flag) => match previous {
            Some(CanonValue::Bool(stored)) if stored == flag => Converted::Shared,
            _ => Converted::Fresh(CanonValue::Bool(*flag)),
        },
        Value::Number(number) => match previous {
            Some(CanonValue::Number(token)) if number_matches(number, token) => Converted::Shared,
            _ => Converted::Fresh(CanonValue::Number(number_token(number))),
        },
        Value::String(text) => match previous {
            Some(CanonValue::String(body)) if escaped_matches(text, body) => Converted::Shared,
            _ => Converted::Fresh(CanonValue::String(escape_string_body(text))),
        },
        Value::Array(items) => convert_array(items, previous),
        Value::Object(map) => convert_object(map, previous),
    }
}

fn convert_array(items: &[Value], previous: Option<&CanonValue>) -> Converted {
    let base: Option<&[CanonValue]> = match previous {
        Some(CanonValue::Array(node)) => Some(node),
        _ => None,
    };
    // Materialised only once a child differs; until then every child was
    // shared, so the prefix is cloned from the base at that moment.
    let mut fresh: Option<Vec<CanonValue>> = None;
    for (index, item) in items.iter().enumerate() {
        let counterpart = base.and_then(|base| base.get(index));
        match convert(item, counterpart) {
            Converted::Shared => {
                if let Some(out) = &mut fresh {
                    out.push(counterpart.expect("shared with a counterpart").clone());
                }
            }
            Converted::Fresh(canon) => {
                let out = fresh.get_or_insert_with(|| {
                    let mut out = Vec::with_capacity(items.len());
                    if index > 0 {
                        out.extend(
                            base.expect("a shared prefix has a base")[..index]
                                .iter()
                                .cloned(),
                        );
                    }
                    out
                });
                out.push(canon);
            }
        }
    }
    match fresh {
        None if base.is_some_and(|base| base.len() == items.len()) => Converted::Shared,
        None => Converted::Fresh(CanonValue::array(match base {
            Some(base) => base[..items.len()].to_vec(),
            None => Vec::new(),
        })),
        Some(out) => Converted::Fresh(CanonValue::array(out)),
    }
}

fn convert_object(map: &Map<String, Value>, previous: Option<&CanonValue>) -> Converted {
    let base: Option<&[(String, CanonValue)]> = match previous {
        Some(CanonValue::Object(node)) => Some(node),
        _ => None,
    };
    // Both sides are sorted by decoded key, so one cursor finds every
    // counterpart without a search per member.
    let mut cursor = 0usize;
    let mut fresh: Option<Vec<(String, CanonValue)>> = None;
    for (index, (key, member)) in map.iter().enumerate() {
        let counterpart = base.and_then(|members| loop {
            let (stored, value) = members.get(cursor)?;
            match key_cmp_decoded(stored, key) {
                Ordering::Less => cursor += 1,
                Ordering::Equal => break Some((stored, value)),
                Ordering::Greater => break None,
            }
        });
        let converted = convert(member, counterpart.map(|(_, value)| value));
        match (converted, counterpart) {
            (Converted::Shared, Some((stored, value))) => {
                if let Some(out) = &mut fresh {
                    out.push((stored.clone(), value.clone()));
                }
            }
            (Converted::Shared, None) => {
                unreachable!("a member is only shared with a counterpart")
            }
            (Converted::Fresh(canon), _) => {
                let out = fresh.get_or_insert_with(|| {
                    let mut out = Vec::with_capacity(map.len());
                    if index > 0 {
                        shared_prefix(
                            base.expect("a shared prefix has a base"),
                            map,
                            index,
                            &mut out,
                        );
                    }
                    out
                });
                out.push((escape_string_body(key), canon));
            }
        }
    }
    match fresh {
        None if base.is_some_and(|base| base.len() == map.len()) => Converted::Shared,
        None => {
            let mut out = Vec::with_capacity(map.len());
            if let Some(base) = base {
                shared_prefix(base, map, map.len(), &mut out);
            }
            Converted::Fresh(CanonValue::object(out))
        }
        Some(out) => Converted::Fresh(CanonValue::object(out)),
    }
}

/// The first `count` members of `map`, every one of which was found equal to
/// its counterpart in `base`, cloned from `base` in map order.
fn shared_prefix(
    base: &[(String, CanonValue)],
    map: &Map<String, Value>,
    count: usize,
    out: &mut Vec<(String, CanonValue)>,
) {
    let mut cursor = 0usize;
    for (key, _) in map.iter().take(count) {
        loop {
            let (stored, value) = &base[cursor];
            cursor += 1;
            if key_cmp_decoded(stored, key) == Ordering::Equal {
                out.push((stored.clone(), value.clone()));
                break;
            }
        }
    }
}

/// Whether `canon` spells exactly the document `value` serializes to. No
/// allocation: strings are compared through the escape table and numbers
/// through a stack formatter.
pub fn unchanged(value: &Value, canon: &CanonValue) -> bool {
    match (value, canon) {
        (Value::Null, CanonValue::Null) => true,
        (Value::Bool(left), CanonValue::Bool(right)) => left == right,
        (Value::Number(number), CanonValue::Number(token)) => number_matches(number, token),
        (Value::String(text), CanonValue::String(body)) => escaped_matches(text, body),
        (Value::Array(items), CanonValue::Array(node)) => {
            items.len() == node.len()
                && items
                    .iter()
                    .zip(node.iter())
                    .all(|(item, canon)| unchanged(item, canon))
        }
        (Value::Object(map), CanonValue::Object(node)) => {
            map.len() == node.len()
                && map
                    .iter()
                    .zip(node.iter())
                    .all(|((key, member), (body, canon))| {
                        escaped_matches(key, body) && unchanged(member, canon)
                    })
        }
        _ => false,
    }
}

/// The number token `serde_json` writes: `itoa` for integers and its float
/// formatter for finite floats, which is also what its `Display` spells.
pub fn number_token(number: &Number) -> String {
    number.to_string()
}

const BB: u8 = b'b';
const TT: u8 = b't';
const NN: u8 = b'n';
const FF: u8 = b'f';
const RR: u8 = b'r';
const QU: u8 = b'"';
const BS: u8 = b'\\';
const UU: u8 = b'u';

/// The escape table `serde_json` serializes strings with: control bytes below
/// 0x20 as `\u00xx` except the five short escapes, the quote and the backslash
/// escaped, everything else (0x7f and all non-ASCII included) written raw.
const fn escape_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut byte = 0usize;
    while byte < 0x20 {
        table[byte] = UU;
        byte += 1;
    }
    table[0x08] = BB;
    table[0x09] = TT;
    table[0x0a] = NN;
    table[0x0c] = FF;
    table[0x0d] = RR;
    table[b'"' as usize] = QU;
    table[b'\\' as usize] = BS;
    table
}

static ESCAPE: [u8; 256] = escape_table();
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The JSON string body (between the quotes) `serde_json` writes for `text`.
pub fn escape_string_body(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut start = 0usize;
    for (index, &byte) in bytes.iter().enumerate() {
        let escape = ESCAPE[byte as usize];
        if escape == 0 {
            continue;
        }
        out.push_str(&text[start..index]);
        let mut sequence = [0u8; 6];
        let written = escape_sequence(byte, escape, &mut sequence);
        out.push_str(std::str::from_utf8(&sequence[..written]).expect("ASCII escape"));
        start = index + 1;
    }
    out.push_str(&text[start..]);
    out
}

fn escape_sequence(byte: u8, escape: u8, out: &mut [u8; 6]) -> usize {
    out[0] = b'\\';
    if escape == UU {
        out[1] = b'u';
        out[2] = b'0';
        out[3] = b'0';
        out[4] = HEX_DIGITS[(byte >> 4) as usize];
        out[5] = HEX_DIGITS[(byte & 0x0f) as usize];
        6
    } else {
        out[1] = escape;
        2
    }
}

/// Whether escaping `text` yields exactly `body`, without building it.
fn escaped_matches(text: &str, body: &str) -> bool {
    let body = body.as_bytes();
    let mut position = 0usize;
    for &byte in text.as_bytes() {
        let escape = ESCAPE[byte as usize];
        if escape == 0 {
            if body.get(position) != Some(&byte) {
                return false;
            }
            position += 1;
            continue;
        }
        let mut sequence = [0u8; 6];
        let written = escape_sequence(byte, escape, &mut sequence);
        if body.get(position..position + written) != Some(&sequence[..written]) {
            return false;
        }
        position += written;
    }
    position == body.len()
}

struct StackText {
    bytes: [u8; 64],
    len: usize,
}

impl std::fmt::Write for StackText {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn number_matches(number: &Number, token: &str) -> bool {
    let mut stack = StackText {
        bytes: [0u8; 64],
        len: 0,
    };
    match write!(stack, "{number}") {
        Ok(()) => &stack.bytes[..stack.len] == token.as_bytes(),
        Err(_) => number.to_string() == token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::Sha256;
    use serde_json::json;
    use std::sync::Arc;

    fn sample() -> Value {
        json!({
            "a": [0, -1, 18446744073709551615u64, -9223372036854775808i64, 0.1, 1.0, -0.0, 1e300, 5e-324, 123456789.125, 2.5e-7],
            "b": {"quote": "say \"hi\"", "backslash": "a\\b", "newline": "x\ny", "tab": "\t", "nul": "\u{0}", "unit": "\u{1f}", "del": "\u{7f}", "umlaut": "ü", "emoji": "🙂", "slash": "a/b"},
            "c": [[], {}, null, true, false, ""],
            "d\u{1}key": {"z": 1, "y": [1, 2, 3]},
            "e": "plain"
        })
    }

    fn round_trip(value: &Value, previous: Option<&CanonValue>) -> CanonValue {
        let text = serde_json::to_vec(value).expect("serializes");
        let canon = from_value_shared(value, previous);
        assert_eq!(canon.to_json_bytes(), text);
        assert_eq!(canon, CanonValue::parse(&text).expect("parses"));
        let (hex, len) = canon.json_sha256_hex();
        assert_eq!(hex, Sha256::hex(&text));
        assert_eq!(len, text.len() as u64);
        assert!(unchanged(value, &canon));
        canon
    }

    #[test]
    fn spelling_matches_serde_json() {
        round_trip(&sample(), None);
    }

    #[test]
    fn every_escapable_byte_spells_like_serde_json() {
        for byte in 0u8..=0x7f {
            let text = String::from_utf8(vec![b'x', byte, b'y']).expect("ASCII");
            let expected = serde_json::to_string(&text).expect("serializes");
            let body = escape_string_body(&text);
            assert_eq!(format!("\"{body}\""), expected, "byte {byte:#x}");
            assert!(escaped_matches(&text, &body));
            assert!(!escaped_matches(&text, &format!("{body}z")));
        }
    }

    #[test]
    fn shares_unchanged_subtrees_and_rebuilds_changed_ones() {
        let first = sample();
        let previous = round_trip(&first, None);
        let mut second = first.clone();
        second["d\u{1}key"]["z"] = json!(2);
        let converted = round_trip(&second, Some(&previous));
        assert!(converted
            .member("a")
            .expect("a")
            .shares_storage_with(previous.member("a").expect("a")));
        assert!(converted
            .member("b")
            .expect("b")
            .shares_storage_with(previous.member("b").expect("b")));
        let changed = converted.member("d\u{1}key").expect("d");
        assert!(!changed.shares_storage_with(previous.member("d\u{1}key").expect("d")));
        assert!(changed.member("y").expect("y").shares_storage_with(
            previous
                .member("d\u{1}key")
                .expect("d")
                .member("y")
                .expect("y")
        ));
        assert!(!unchanged(&second, &previous));
        let again = from_value_shared(&second, Some(&converted));
        assert!(again.shares_storage_with(&converted));
    }

    #[test]
    fn shape_changes_against_the_base_rebuild_correctly() {
        let base_value = json!({"k": [1, 2, 3], "m": {"a": 1, "b": 2, "c": 3}, "n": 5});
        let base = round_trip(&base_value, None);
        let cases = [
            json!({"k": [1, 2], "m": {"a": 1, "c": 3}, "n": 5}),
            json!({"k": [1, 2, 3, 4], "m": {"a": 1, "b": 2, "c": 3, "d": 4}, "n": "5"}),
            json!({"k": {"x": 1}, "m": [1], "n": null}),
            json!({"k": [], "m": {}, "n": 5, "o": {"p": [1, {"q": 2}]}}),
            json!({"j": 0, "k": [1, 2, 3], "m": {"a": 1, "b": 2, "c": 3}}),
            json!({"m": {"b": 2}}),
            json!({"k": [1, 2, 3], "m": {"a": 1, "b": 2, "c": 3}, "n": 5, "z": {"a": 1, "c": 3}}),
            json!([1, 2]),
            json!("scalar"),
        ];
        for case in &cases {
            round_trip(case, Some(&base));
        }
        let canons: Vec<CanonValue> = cases.iter().map(|case| round_trip(case, None)).collect();
        for case in &cases {
            for canon in &canons {
                round_trip(case, Some(canon));
            }
        }
    }

    #[test]
    fn text_memo_carries_over_sharing_and_clears_on_mutation() {
        let rows: Vec<Value> = (0..200)
            .map(|i| json!({"id": i, "name": format!("row {i}")}))
            .collect();
        let first =
            json!({"table": {"rows": rows.clone(), "meta": {"count": 200}}, "small": {"k": [1]}});
        let previous = from_value_shared(&first, None);
        let text = serde_json::to_vec(&first).expect("serializes");
        assert_eq!(previous.json_sha256_hex().0, Sha256::hex(&text));
        assert!(
            previous.text_memo_bytes() > 0,
            "the row block is large enough to memoise"
        );
        let mut second = first.clone();
        second["table"]["meta"]["count"] = json!(201);
        let converted = from_value_shared(&second, Some(&previous));
        let text = serde_json::to_vec(&second).expect("serializes");
        assert_eq!(converted.json_sha256_hex().0, Sha256::hex(&text));
        let mut mutated = converted.clone();
        if let CanonValue::Object(root) = &mut mutated {
            let members = Arc::make_mut(root).contents_mut();
            if let CanonValue::Object(table) = &mut members[1].1 {
                let members = Arc::make_mut(table).contents_mut();
                if let CanonValue::Array(rows) = &mut members[1].1 {
                    Arc::make_mut(rows).contents_mut().pop();
                }
            }
        }
        assert_eq!(
            mutated.json_sha256_hex().0,
            Sha256::hex(&mutated.to_json_bytes()),
            "a mutated node re-spells instead of serving its old text"
        );
        assert_ne!(mutated.json_sha256_hex().0, converted.json_sha256_hex().0);
    }
}
