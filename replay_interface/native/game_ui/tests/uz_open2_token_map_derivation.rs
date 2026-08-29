//! UZ-OPEN-2's answer, re-derived from the raw live-read bytes.
//!
//! Four authored keys -- `AND`, `OR`, `NOT`, `OWNER` -- are interned by the
//! the game lexer itself, with no registrar call site, so their ids exist only in
//! the running process. They were read on 2026-08-29 out of the engine's own
//! static-token hash map. This test re-derives them from those raw bytes
//! instead of trusting the number typed into `AUTHORED_KEY_BINDINGS`: it
//! decodes the map header, recomputes each hash, reproduces the bucket address
//! the read actually used, walks the bucket, and reads the id back.
//!
//! The bytes come from `portable_engine_evidence`, compiled in rather than
//! opened: a replay-side test must not reach for a path under the acquisition
//! tree, which `replay_dependency_boundary.rs` enforces.
//!
//! The seven controls are why the answer is trustworthy. A mis-addressed
//! bucket returns a clean-looking miss that is indistinguishable from "key
//! absent" -- the runner's first attempt did exactly that, from a hand-computed
//! offset. Keys whose ids are already known statically catch it.

use game_ui::ability_visibility::{authored_key_binding, AuthoredKeyBinding};

fn read_json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("the harness writes well-formed json")
}

fn bytes_of(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "a byte string has even length");
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn u32_at(buffer: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buffer[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(buffer: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buffer[at..at + 8].try_into().expect("eight bytes"))
}

/// FNV-1a, 32 bit. The engine hashes the LOWERCASED key text.
fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

/// The map header, decoded from the stage A read at rva `0x30AECC0`.
struct MapHeader {
    bucket_base: u64,
    mask: u32,
}

fn stage_a() -> MapHeader {
    let result = read_json(portable_engine_evidence::artifacts::TOKEN_MAP_HEADER);
    let observation = &result["observations"][0];
    assert_eq!(observation["path_id"], "uz-tokenmap-header2");
    let buffer = bytes_of(observation["value"].as_str().expect("a value was read"));

    let bucket_base = u64_at(&buffer, 0x10);
    let elements = u32_at(&buffer, 0x18);
    let mask = u32_at(&buffer, 0x1C);
    let load_factor = f32::from_bits(u32_at(&buffer, 0x24));

    // A bucket array still inside the module image would mean the map was not
    // yet seeded and every stage B read was of uninitialised memory. This one
    // is on the heap, far below the module.
    let module_base = result["module_base"]
        .as_str()
        .and_then(|text| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok())
        .expect("the harness reports where the module landed");
    assert!(
        !(module_base..module_base + 0x800_0000).contains(&bucket_base),
        "the bucket array is off-image, so the map was seeded: {bucket_base:#X}"
    );
    assert_eq!(mask, 0x1_FFFF, "131072 buckets");
    assert_eq!(elements, 61686);
    let buckets = f64::from(mask) + 1.0;
    assert!(
        f64::from(elements) / buckets < f64::from(load_factor),
        "a map over its own load factor would have been resized out from under the read"
    );

    MapHeader { bucket_base, mask }
}

/// One decoded hash-map entry: the stored key text and the id interned for it.
fn entry_in(bucket: &[u8], key: &str) -> Option<(String, u32)> {
    const STRIDE: usize = 0x30;
    (0..bucket.len() / STRIDE).find_map(|index| {
        let entry = &bucket[index * STRIDE..(index + 1) * STRIDE];
        let occupancy = u32_at(entry, 0x04);
        let length = u64_at(entry, 0x18) as usize;
        let capacity = u64_at(entry, 0x20);
        // Occupancy 0 is a free slot; a capacity over 0xF stores the text out
        // of line, which none of these short keys does.
        if occupancy == 0 || capacity > 0xF || length > 0xF {
            return None;
        }
        let text = String::from_utf8(entry[0x08..0x08 + length].to_vec()).ok()?;
        text.eq_ignore_ascii_case(key)
            .then(|| (text, u32_at(entry, 0x28)))
    })
}

/// Every key the stage B read covered, with the id already known statically for
/// the seven controls. `None` is one of the four the read was taken to answer.
const READ: &[(&str, &str, Option<u32>)] = &[
    ("uz-b-and", "and", None),
    ("uz-b-or", "or", None),
    ("uz-b-not", "not", None),
    ("uz-b-owner", "owner", None),
    // The controls. `if` and `limit` are the sharpest two: they were derived
    // from the TRIGGER registry, while this map is the TOKEN registrar, so
    // agreement here is two independent sources on one id space.
    ("uz-b-if", "if", Some(0x27C3)),
    ("uz-b-limit", "limit", Some(0x2A0A)),
    ("uz-b-has-dlc", "has_dlc", Some(0x2D1B)),
    ("uz-b-is-border-war", "is_border_war", Some(0x399A)),
    ("uz-b-allowed", "allowed", Some(0x2FE7)),
    ("uz-b-else", "else", Some(0x36D3)),
    ("uz-b-else-if", "else_if", Some(0x38ED)),
];

#[test]
fn the_interned_ids_re_derive_from_the_raw_live_read() {
    let header = stage_a();
    let result = read_json(portable_engine_evidence::artifacts::TOKEN_MAP_BUCKETS);
    let observations = result["observations"]
        .as_array()
        .expect("stage B read every path in one job");
    assert_eq!(observations.len(), READ.len());

    let mut controls = 0;
    for (path_id, key, known) in READ {
        let observation = observations
            .iter()
            .find(|candidate| candidate["path_id"] == *path_id)
            .unwrap_or_else(|| panic!("{path_id} was read"));

        // The harness dereferenced a heap pointer rather than resolving a
        // module rva, which is the only way a heap-allocated map is reachable.
        assert!(
            observation["resolved_module_rva"].is_null(),
            "{key} was reached by pointer deref"
        );
        let read_at = observation["resolved_address"]
            .as_str()
            .and_then(|text| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok())
            .expect("an absolute address");

        // The address the read actually used must be the one this key's hash
        // selects. This is what a hand-computed offset fails.
        let bucket = u64::from(fnv1a32(key) & header.mask);
        assert_eq!(
            read_at,
            header.bucket_base + bucket * 0x30,
            "{key} was read from the bucket its own hash selects"
        );
        // ... and only for the lowercase spelling. The authored files write
        // these keys uppercase; the engine interns the lowercased form, so an
        // exact-bytes rule would have missed all four while every control passed.
        if key.chars().any(char::is_alphabetic) {
            let upper = key.to_ascii_uppercase();
            if upper != *key {
                assert_ne!(
                    fnv1a32(&upper) & header.mask,
                    fnv1a32(key) & header.mask,
                    "{key} and {upper} land in different buckets"
                );
            }
        }

        let window = bytes_of(observation["value"].as_str().expect("a value was read"));
        let (stored, id) =
            entry_in(&window, key).unwrap_or_else(|| panic!("{key} is in its own bucket window"));
        assert_eq!(stored, *key, "the map stores the lowercase spelling");

        if let Some(expected) = known {
            assert_eq!(id, *expected, "control {key} came back exact");
            controls += 1;
        }

        // The id in the shipped table is the id the engine actually interned.
        // `allowed` is the ability schema's own key, read here as a control
        // only: it names the block, so it is never a node inside one.
        // `authored_key_binding` matches case-insensitively, so the lowercase
        // spelling read out of the map finds the uppercase authored key.
        match authored_key_binding(key) {
            Some(declared) => assert_eq!(
                declared.interned_id(),
                id,
                "{key}'s shipped id matches the live read"
            ),
            None => assert_eq!(*key, "allowed", "{key} is a key this module parses"),
        }
    }
    assert_eq!(controls, 7, "the whole control gate ran");
}

/// The four the read was taken for, and the classification they carry.
#[test]
fn the_four_lexer_interned_keys_carry_their_live_read_ids() {
    for (key, stored, id) in [
        ("AND", "and", 0x2968_u32),
        ("OR", "or", 0x2969),
        ("NOT", "not", 0x296A),
        ("OWNER", "owner", 0x283E),
    ] {
        let binding = authored_key_binding(key).expect("a parsed key");
        let AuthoredKeyBinding::LexerTable {
            id: declared,
            stored: spelling,
            ..
        } = binding
        else {
            panic!("{key} is interned by the lexer, not by either registrar");
        };
        assert_eq!(declared, id);
        assert_eq!(spelling, stored);
    }
}
