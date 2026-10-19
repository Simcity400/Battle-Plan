//! Keep hash-backed state reproducible as preparation families are added.
use std::{fs, path::Path};

// Tokenize enough Rust syntax to inspect derives/field attributes. Comments and
// string contents cannot introduce fake types; strings remain single tokens so
// serializer paths are checked explicitly. This test adds no build dependency.
fn tokens(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            continue;
        }
        let start = at;
        // Raw strings (including byte raw strings) may contain Rust fragments.
        let raw_start = if bytes[at..].starts_with(b"br") {
            Some(at + 2)
        } else if bytes[at] == b'r' {
            Some(at + 1)
        } else {
            None
        };
        if let Some(mut quote) = raw_start {
            let hash_start = quote;
            while quote < bytes.len() && bytes[quote] == b'#' {
                quote += 1;
            }
            if quote < bytes.len() && bytes[quote] == b'"' {
                let hashes = quote - hash_start;
                at = quote + 1;
                while at < bytes.len() {
                    if bytes[at] == b'"'
                        && bytes
                            .get(at + 1..at + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|&byte| byte == b'#'))
                    {
                        at += 1 + hashes;
                        break;
                    }
                    at += 1;
                }
                out.push(&source[start..at]);
                continue;
            }
        }
        if bytes[at] == b'"' || bytes[at..].starts_with(b"b\"") {
            if bytes[at] == b'b' {
                at += 1;
            }
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at = (at + 2).min(bytes.len());
                } else if bytes[at] == b'"' {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
        } else if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
        } else if bytes[at] == b'\'' {
            let mut end = at + 1;
            if bytes.get(end) == Some(&b'\\') {
                end += 1;
                if bytes.get(end) == Some(&b'u') && bytes.get(end + 1) == Some(&b'{') {
                    end += 2;
                    while end < bytes.len() && bytes[end] != b'}' {
                        end += 1;
                    }
                    end += usize::from(end < bytes.len());
                } else if bytes.get(end) == Some(&b'x') {
                    end += 3;
                } else {
                    end += 1;
                }
            } else if end < bytes.len() {
                end += source[end..].chars().next().unwrap().len_utf8();
            }
            if bytes.get(end) == Some(&b'\'') {
                at = end + 1;
            } else {
                at += 1;
            } // A lifetime, not a character literal.
        } else {
            at += source[at..].chars().next().unwrap().len_utf8();
        }
        out.push(&source[start..at]);
    }
    out
}

fn matching(tokens: &[&str], start: usize, open: &str, close: &str) -> usize {
    let mut depth = 0;
    for (at, token) in tokens.iter().enumerate().skip(start) {
        if *token == open {
            depth += 1;
        }
        if *token == close {
            depth -= 1;
            if depth == 0 {
                return at;
            }
        }
    }
    panic!("unclosed {open} in serialization source guard");
}

fn ordered(field: &[&str]) -> bool {
    let mut at = 0;
    while at + 1 < field.len() && field[at..].starts_with(&["#", "["]) {
        let end = matching(field, at + 1, "[", "]");
        let attribute = &field[at + 2..end];
        if attribute.first() == Some(&"serde") {
            if attribute
                .iter()
                .any(|&token| token == "skip" || token == "skip_serializing")
            {
                return true;
            }
            for clause in attribute.windows(3) {
                if clause[1] != "=" {
                    continue;
                }
                let path = clause[2].trim_matches('"');
                if clause[0] == "with"
                    && matches!(
                        path,
                        "map_pairs"
                            | "crate::prepared_production::map_pairs"
                            | "crate::prepared_production::map_pairs::shared"
                            | "dashboard_core::map_pairs"
                            | "dashboard_core::map_pairs::shared"
                            | "versioned_pairs"
                    )
                {
                    return true;
                }
                if clause[0] == "serialize_with"
                    && [
                        "ordered_map",
                        "ordered_pairs",
                        "ordered_set",
                        "ordered_sets",
                        "ordered_optional_map",
                        "ordered_shared_optional_map",
                    ]
                    .iter()
                    .any(|method| {
                        path == format!("crate::prepared_production::map_pairs::{method}")
                            || path == format!("dashboard_core::map_pairs::{method}")
                    })
                {
                    return true;
                }
            }
        }
        at = end + 1;
    }
    false
}

fn inspect_field(field: &[&str], name: &str, failures: &mut Vec<String>) {
    if !field
        .iter()
        .any(|&token| token == "HashMap" || token == "HashSet")
    {
        return;
    }
    if ordered(field) {
        return;
    }
    let mut at = 0;
    while at + 1 < field.len() && field[at..].starts_with(&["#", "["]) {
        at = matching(field, at + 1, "[", "]") + 1;
    }
    let field_name = field[at..]
        .windows(2)
        .find(|pair| pair[1] == ":")
        .map_or("(tuple or nested variant)", |pair| pair[0]);
    failures.push(format!("{name}.{field_name}"));
}

fn unordered(source: &str) -> Vec<String> {
    let tokens = tokens(source);
    let mut failures = Vec::new();
    let mut at = 0;
    while at + 2 < tokens.len() {
        if !tokens[at..].starts_with(&["#", "[", "derive"]) {
            at += 1;
            continue;
        }
        let end = matching(&tokens, at + 1, "[", "]");
        if !tokens[at..=end].contains(&"Serialize") {
            at = end + 1;
            continue;
        }
        let mut header = end + 1;
        while header + 1 < tokens.len() && tokens[header..].starts_with(&["#", "["]) {
            header = matching(&tokens, header + 1, "[", "]") + 1;
        }
        while header < tokens.len() && !matches!(tokens[header], "struct" | "enum" | ";" | "{") {
            header += 1;
        }
        if header == tokens.len() || !matches!(tokens[header], "struct" | "enum") {
            at = end + 1;
            continue;
        }
        let name = tokens[header + 1];
        header += 2;
        while header < tokens.len() && !matches!(tokens[header], "{" | "(" | ";") {
            header += 1;
        }
        if header == tokens.len() || tokens[header] == ";" {
            at = header + 1;
            continue;
        }
        let open = tokens[header];
        let close = if open == "{" { "}" } else { ")" };
        let end = matching(&tokens, header, open, close);
        let mut field = header + 1;
        let mut depth = 0;
        for next in header + 1..end {
            match tokens[next] {
                "<" | "(" | "[" | "{" => depth += 1,
                ">" | ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => {
                    inspect_field(&tokens[field..next], name, &mut failures);
                    field = next + 1;
                }
                _ => {}
            }
        }
        inspect_field(&tokens[field..end], name, &mut failures);
        at = end + 1;
    }
    failures
}

fn inspect_tree(root: &Path, failures: &mut Vec<String>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().unwrap() != "tests" {
                inspect_tree(&path, failures);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().unwrap() != "tests.rs"
        {
            let source = fs::read_to_string(&path).unwrap();
            failures.extend(
                unordered(&source)
                    .into_iter()
                    .map(|field| format!("{}: {field}", path.display())),
            );
        }
    }
}

#[test]
fn serialized_dashboard_hash_fields_are_ordered() {
    let mut failures = Vec::new();
    // Every dashboard crate (core, text, base and the product crates) persists the same way, so
    // their fields are held to it too.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for entry in fs::read_dir(&crates).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("dashboard"))
        {
            inspect_tree(&path.join("src"), &mut failures);
        }
    }
    failures.sort();
    // No exceptions: even CountryIndex is saved through occupation scan/trend.
    assert!(
        failures.is_empty(),
        "Unordered persisted hash fields:\n{}",
        failures.join("\n")
    );
}

#[test]
fn serialization_source_guard_rejects_unordered_fields_and_unreviewed_serializers() {
    for source in [
        "#[derive(Serialize)] struct New { map: HashMap<String, usize> }",
        "#[derive(Serialize)] struct New { nested: BTreeMap<String, HashSet<u32>> }",
        "#[derive(Serialize)] struct New { #[serde(with=\"unchecked\")] map: HashMap<String, usize> }",
        "#[derive(Serialize)] struct New(HashSet<u32>);",
    ] {
        assert_eq!(unordered(source).len(), 1, "{source}");
    }
    for source in [
        "#[derive(Serialize)] struct New { #[serde(skip)] map: HashMap<String, usize> }",
        "#[derive(Serialize)] struct New { #[serde(serialize_with=\"crate::prepared_production::map_pairs::ordered_map\")] map: HashMap<String, usize> }",
        "#[derive(Serialize)] struct New { #[serde(with=\"crate::prepared_production::map_pairs\")] map: HashMap<(u32,u32), usize> }",
        "#[derive(Serialize)] struct New { #[serde(with=\"crate::prepared_production::map_pairs::shared\")] map: Arc<HashMap<(u32,u32), usize>> }",
    ] {
        assert!(unordered(source).is_empty(), "{source}");
    }
}

#[test]
fn ordered_saved_hash_collections_keep_json_shapes_and_round_trip() {
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, HashMap, HashSet};
    #[derive(Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
    struct Key {
        id: u32,
        token: Vec<u8>,
    } // Existing design keys need not implement Ord.
    #[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
    struct Saved {
        #[serde(with = "crate::prepared_production::map_pairs")]
        pairs: HashMap<Key, usize>,
        #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
        object: HashMap<u32, String>,
        #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_sets")]
        holders: BTreeMap<String, HashSet<u32>>,
        #[serde(with = "crate::prepared_production::map_pairs::shared")]
        shared: std::sync::Arc<HashMap<(String, String), i32>>,
    }
    let make = |reverse: bool| {
        let keys: Vec<_> = if reverse {
            (0..32).rev().collect()
        } else {
            (0..32).collect()
        };
        Saved {
            pairs: keys
                .iter()
                .map(|&id| {
                    (
                        Key {
                            id,
                            token: vec![id as u8],
                        },
                        id as usize,
                    )
                })
                .collect(),
            object: keys.iter().map(|&id| (id, id.to_string())).collect(),
            shared: std::sync::Arc::new(
                keys.iter()
                    .map(|&id| ((id.to_string(), "subject".to_owned()), id as i32))
                    .collect(),
            ),
            holders: [("holder".to_owned(), keys.into_iter().collect())].into(),
        }
    };
    let first = make(false);
    let bytes = serde_json::to_vec(&first).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&make(true)).unwrap());
    assert_eq!(first, serde_json::from_slice::<Saved>(&bytes).unwrap());
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["object"].is_object());
    assert!(value["pairs"].is_array());
    assert!(value["pairs"][0][0].is_object());
    assert_eq!(value["shared"][0], serde_json::json!([["0", "subject"], 0]));
    assert_eq!(
        value["holders"]["holder"],
        serde_json::json!((0..32).collect::<Vec<_>>())
    );
}
