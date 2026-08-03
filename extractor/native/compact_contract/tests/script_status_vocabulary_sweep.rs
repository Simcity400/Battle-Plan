//! Anti-drift sweep for the SCRIPT-side copies of the compact status
//! vocabulary.
//!
//! `tests/lane_status_bit_sweep.rs` pins the producer half: every lane-local
//! status-bit declaration in `extractor/native/lanes/` must be registered in
//! `compact_contract::lane_local_status_mask`. This file pins the consumer
//! half. Four gate instruments outside the Rust workspace read the same status
//! word and each carries its own hand-written copy of that registry:
//!
//! * `scripts/report_raw_capture.py` — the raw-capture domain report
//! * `scripts/c0_domain_census.py` — "the exit code IS the C0 gate"
//! * `scripts/c0_pass2_spans.py` — per-hour status spans and reconciliation
//! * `scripts/ProductionCapture.Promotion.ps1` — the promotion check
//!
//! On 2026-08-03 all four had drifted from the contract at once, and every one
//! of them failed in the SAME direction: a bit the contract defines and names
//! was reported as undefined, and the capture was blocked for it. The t15
//! capture published `d53=0x530300` and `d60=0x30300`; the report called
//! `0x530000`/`0x30000` `UNKNOWN_BITS`, the C0 census called d60 bit 17
//! "contract drift", the span reconciler classified it as an unexplained
//! failure incidence, and the promotion gate threw "lacks real-source
//! completeness". None of those bits was undefined — they are
//! `DOMAIN53_STATUS_*` and `DOMAIN60_STATUS_*`, registered in
//! `lane_local_status_mask` since the batch-3a classification fix.
//!
//! A copy of a table is only safe if something reads both. That is this file.
//! It is deliberately a PARSE of the scripts rather than a generated header
//! they import: these instruments are audited, standalone, and must stay
//! readable and runnable with no build step, so the duplication stays and the
//! disagreement is what gets made impossible.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::{
    assigned_descriptor_by_name, lane_local_status_mask, COMPACT_STATUS_FAILURE_MASK,
    COMPACT_STATUS_V1_KNOWN_MASK,
};

fn scripts_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts")
}

fn read_script(name: &str) -> String {
    let path = scripts_root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Every domain the contract registers a lane-local vocabulary for. Derived,
/// never listed: a new registration is picked up here without editing this
/// file, and every script table is then required to carry it.
fn registered_domains() -> BTreeMap<u16, u32> {
    (0..=u16::MAX)
        .filter_map(|domain| {
            let mask = lane_local_status_mask(domain);
            (mask != 0).then_some((domain, mask))
        })
        .collect()
}

/// The text of the brace-delimited value that follows `marker`, exclusive of
/// the outermost braces. Handles `{`, `@{` and quoted strings (the badge names
/// in these tables contain `:` and `{`-free but quoted text).
fn block_after(text: &str, marker: &str) -> String {
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("marker {marker:?} not found"));
    let rest = &text[start + marker.len()..];
    let open = rest
        .find('{')
        .unwrap_or_else(|| panic!("no block opens after {marker:?}"));
    let bytes: Vec<char> = rest[open..].chars().collect();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut out = String::new();
    for (index, ch) in bytes.iter().copied().enumerate() {
        if let Some(q) = quote {
            out.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push(ch);
            }
            '{' => {
                depth += 1;
                if depth > 1 {
                    out.push(ch);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return out;
                }
                out.push(ch);
            }
            _ => out.push(ch),
        }
        let _ = index;
    }
    panic!("unterminated block after {marker:?}");
}

/// The text of the parenthesised expression that follows `marker`, exclusive
/// of the outermost parentheses.
fn paren_expr_after(text: &str, marker: &str) -> String {
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("marker {marker:?} not found"));
    let rest = &text[start + marker.len()..];
    let open = rest
        .find('(')
        .unwrap_or_else(|| panic!("no parenthesised expression after {marker:?}"));
    let mut depth = 0i32;
    let mut out = String::new();
    for ch in rest[open..].chars() {
        match ch {
            '(' => {
                depth += 1;
                if depth > 1 {
                    out.push(ch);
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return out;
                }
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    panic!("unterminated parenthesised expression after {marker:?}");
}

/// Collapses every run of whitespace to a single space, so a source check is
/// not a line-ending check.
fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Top-level `<key>: { <inner> }` entries of a Python dict literal, as
/// `(key text, inner text)`. Quote-aware; nested braces inside the inner block
/// are kept.
fn nested_entries(block: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = block.chars().collect();
    let mut out = Vec::new();
    let mut key = String::new();
    let mut index = 0usize;
    let mut quote: Option<char> = None;
    while index < chars.len() {
        let ch = chars[index];
        if let Some(q) = quote {
            key.push(ch);
            if ch == q {
                quote = None;
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                key.push(ch);
                index += 1;
            }
            '#' => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            ':' => {
                // The value follows. If it opens a block, capture it.
                let mut cursor = index + 1;
                while cursor < chars.len() && chars[cursor].is_whitespace() {
                    cursor += 1;
                }
                if cursor < chars.len() && chars[cursor] == '{' {
                    let mut depth = 0i32;
                    let mut inner = String::new();
                    let mut inner_quote: Option<char> = None;
                    while cursor < chars.len() {
                        let c = chars[cursor];
                        if let Some(q) = inner_quote {
                            inner.push(c);
                            if c == q {
                                inner_quote = None;
                            }
                            cursor += 1;
                            continue;
                        }
                        match c {
                            '\'' | '"' => {
                                inner_quote = Some(c);
                                inner.push(c);
                            }
                            '{' => {
                                depth += 1;
                                if depth > 1 {
                                    inner.push(c);
                                }
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    cursor += 1;
                                    break;
                                }
                                inner.push(c);
                            }
                            _ => inner.push(c),
                        }
                        cursor += 1;
                    }
                    out.push((key.trim().to_string(), inner));
                    key.clear();
                    index = cursor;
                } else {
                    key.clear();
                    index += 1;
                }
            }
            ',' => {
                key.clear();
                index += 1;
            }
            _ => {
                key.push(ch);
                index += 1;
            }
        }
    }
    out
}

/// Integer keys of a Python dict literal: every `<int>:` at the top level of
/// `block`. Quote- and comment-aware, so a `:` inside a badge name or a
/// comment is not a key separator.
fn int_keys(block: &str) -> BTreeSet<u32> {
    let chars: Vec<char> = block.chars().collect();
    let mut out = BTreeSet::new();
    let mut token = String::new();
    let mut depth = 0i32;
    let mut index = 0usize;
    let mut quote: Option<char> = None;
    while index < chars.len() {
        let ch = chars[index];
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                token.clear();
            }
            '#' => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                token.clear();
                continue;
            }
            '(' | '[' | '{' => {
                depth += 1;
                token.clear();
            }
            ')' | ']' | '}' => {
                depth -= 1;
                token.clear();
            }
            ':' if depth == 0 => {
                if let Ok(bit) = token.trim().parse::<u32>() {
                    out.insert(bit);
                }
                token.clear();
            }
            ',' => token.clear(),
            _ => token.push(ch),
        }
        index += 1;
    }
    out
}

fn mask_of(bits: &BTreeSet<u32>) -> u32 {
    bits.iter().fold(0u32, |mask, bit| mask | (1u32 << bit))
}

/// One script table, reduced to `domain id -> mask`.
fn check_table(label: &str, observed: &BTreeMap<u16, u32>) {
    let expected = registered_domains();
    assert_eq!(
        observed, &expected,
        "{label} disagrees with compact_contract::lane_local_status_mask.\n  \
         script:   {observed:#x?}\n  contract: {expected:#x?}\n  \
         A bit missing here is not a labelling nit: every one of these \
         instruments treats an unregistered high bit as an undefined status \
         bit and BLOCKS the capture for it."
    );
}

#[test]
fn report_raw_capture_lane_local_table_matches_the_contract() {
    let text = read_script("report_raw_capture.py");
    let block = block_after(&text, "LANE_LOCAL_BITS = ");
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let domain: u16 = key
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {key:?}"));
        observed.insert(domain, mask_of(&int_keys(&inner)));
    }
    check_table("report_raw_capture.py LANE_LOCAL_BITS", &observed);
}

#[test]
fn report_raw_capture_global_masks_match_the_contract() {
    let text = read_script("report_raw_capture.py");
    // Production change that makes this fail: assigning status bit 15 in the
    // contract without widening the report's KNOWN_MASK, which would make the
    // report call the new assigned bit undefined and block every capture that
    // publishes it.
    let known_shift = COMPACT_STATUS_V1_KNOWN_MASK.count_ones();
    assert!(
        text.contains(&format!("KNOWN_MASK = (1 << {known_shift}) - 1")),
        "report_raw_capture.py KNOWN_MASK is not (1 << {known_shift}) - 1, the contract's \
         COMPACT_STATUS_V1_KNOWN_MASK ({COMPACT_STATUS_V1_KNOWN_MASK:#x})"
    );
    assert_eq!(
        COMPACT_STATUS_V1_KNOWN_MASK,
        (1u32 << known_shift) - 1,
        "the assigned vocabulary is no longer a contiguous run from bit 0, so the \
         scripts' `(1 << N) - 1` idiom is wrong and this sweep is checking the wrong shape"
    );
    let failure_bits: BTreeSet<u32> = (0..32)
        .filter(|bit| COMPACT_STATUS_FAILURE_MASK & (1u32 << bit) != 0)
        .collect();
    let declared: BTreeSet<u32> = paren_expr_after(&text, "FAILURE_MASK = ")
        .split('|')
        .map(|term| term.trim().trim_matches(['(', ')']).trim())
        .filter_map(|term| term.strip_prefix("1 <<")?.trim().parse().ok())
        .collect();
    assert_eq!(
        declared, failure_bits,
        "report_raw_capture.py FAILURE_MASK is not compact_contract::COMPACT_STATUS_FAILURE_MASK \
         ({COMPACT_STATUS_FAILURE_MASK:#x})"
    );
}

#[test]
fn c0_domain_census_lane_local_table_matches_the_contract() {
    let text = read_script("c0_domain_census.py");
    let block = block_after(&text, "LANE_LOCAL = ");
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let name = key.trim().trim_matches(['"', '\'']);
        let descriptor = assigned_descriptor_by_name(name)
            .unwrap_or_else(|| panic!("c0_domain_census.py names unknown domain {name:?}"));
        observed.insert(descriptor.id, mask_of(&int_keys(&inner)));
    }
    check_table("c0_domain_census.py LANE_LOCAL", &observed);
}

#[test]
fn c0_pass2_spans_lane_local_table_matches_the_contract() {
    let text = read_script("c0_pass2_spans.py");
    let block = block_after(&text, "DOMAIN_STATUS_BITS: Mapping[int, Mapping[int, StatusSpec]] = ");
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let domain: u16 = key
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {key:?}"));
        observed.insert(domain, mask_of(&int_keys(&inner)));
    }
    check_table("c0_pass2_spans.py DOMAIN_STATUS_BITS", &observed);
    // The file's second table must be DERIVED from the first, not a second
    // hand-written registry: two local copies in one file drifted apart from
    // each other before they drifted from the contract.
    assert!(
        squeeze(&text).contains(
            "LANE_LOCAL_MASKS = { domain: sum(1 << bit for bit in bits) \
             for domain, bits in DOMAIN_STATUS_BITS.items() }"
        ),
        "c0_pass2_spans.py LANE_LOCAL_MASKS is not derived from DOMAIN_STATUS_BITS"
    );
}

#[test]
fn production_promotion_lane_local_table_matches_the_contract() {
    let text = read_script("ProductionCapture.Promotion.ps1");
    let block = block_after(&text, "$script:ProductionPromotionLaneLocalStatusMask = ");
    let mut observed = BTreeMap::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((domain, value)) = line.split_once('=') else {
            continue;
        };
        let domain: u16 = domain
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {domain:?}"));
        let hex = value
            .trim()
            .strip_prefix("[uint32]0x")
            .unwrap_or_else(|| panic!("promotion mask for domain {domain} is not [uint32]0x..."));
        let mask = u32::from_str_radix(hex.trim(), 16)
            .unwrap_or_else(|_| panic!("promotion mask for domain {domain} is not hex"));
        observed.insert(domain, mask);
    }
    check_table(
        "ProductionCapture.Promotion.ps1 ProductionPromotionLaneLocalStatusMask",
        &observed,
    );
}

#[test]
fn production_promotion_unknown_status_mask_is_the_contract_complement() {
    let text = read_script("ProductionCapture.Promotion.ps1");
    let expected = format!("{:08X}", !COMPACT_STATUS_V1_KNOWN_MASK);
    assert!(
        text.contains(&format!("[uint32]::Parse('{expected}'")),
        "ProductionCapture.Promotion.ps1's unknown-status mask is not the complement of \
         COMPACT_STATUS_V1_KNOWN_MASK ({COMPACT_STATUS_V1_KNOWN_MASK:#x}); expected {expected}"
    );
    let expected_failure = format!("[uint32]0x{:X}", COMPACT_STATUS_FAILURE_MASK);
    assert!(
        text.contains(&format!(
            "$script:ProductionPromotionFailureMask = {expected_failure}"
        )),
        "ProductionCapture.Promotion.ps1's failure mask is not \
         COMPACT_STATUS_FAILURE_MASK; expected {expected_failure}"
    );
}

/// The promotion gate must test DEFINEDNESS on the globalized word. Testing it
/// on the raw word is what threw "lacks real-source completeness" for d53 and
/// d60, and it is a silent failure mode: the gate looks strict, and it is
/// strict about the wrong thing.
#[test]
fn production_promotion_tests_definedness_on_the_globalized_word() {
    let text = read_script("ProductionCapture.Promotion.ps1");
    assert!(
        !text.contains("($rawStatusFlags -band $script:ProductionPromotionUnknownStatusMask)"),
        "the promotion gate applies the unknown-status mask to the RAW per-domain status \
         word; registered lane-local badges must be stripped first"
    );
    assert!(
        text.contains("($globalStatusFlags -band $script:ProductionPromotionUnknownStatusMask)"),
        "the promotion gate no longer tests the unknown-status mask against the globalized word"
    );
    assert!(
        text.contains("function Get-ProductionPromotionGlobalStatusFlags"),
        "the promotion gate has no globalization helper"
    );
}
