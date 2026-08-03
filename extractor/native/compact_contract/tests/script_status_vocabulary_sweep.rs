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

// --- Parsers -----------------------------------------------------------------
//
// These read the tables out of Python and PowerShell source, so every one of
// them is FAIL-CLOSED by construction: anything it cannot account for is a
// panic, never a skip. A parser that silently ignores what it does not
// understand would let this sweep pass while the table it is guarding is
// wrong -- e.g. `23 + 0: "badge"` is a valid Python key that the runtime turns
// into a stripped bit 23, and a key parser that "only collects integer keys"
// would never see it. Same class of hole as the drift being guarded against,
// one level up.

/// Replaces every `#` line comment outside a string with spaces.
///
/// Both languages here use `#` to end of line. Nothing downstream is
/// comment-aware, and it must not have to be: a `# }` inside a table would
/// otherwise end a block early and hide every entry after it, and the runtime
/// would still read those entries.
fn strip_line_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut in_comment = false;
    for ch in text.chars() {
        if ch == '\n' {
            in_comment = false;
            quote = None;
            escaped = false;
            out.push(ch);
            continue;
        }
        if in_comment {
            out.push(' ');
            continue;
        }
        if let Some(q) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push(ch);
            }
            '#' => {
                in_comment = true;
                out.push(' ');
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Byte offset of `marker`, requiring it to occur EXACTLY once.
///
/// A marker that matches twice would silently bind the parser to whichever
/// copy came first, which is how a "checked" table quietly stops being the
/// table the runtime reads.
fn unique_marker(text: &str, marker: &str) -> usize {
    let count = text.matches(marker).count();
    assert_eq!(
        count, 1,
        "marker {marker:?} must occur exactly once in the script, found {count}"
    );
    text.find(marker).expect("counted above")
}

/// The delimited value that follows `marker`, exclusive of the outermost pair.
/// `text` must already be comment-stripped.
fn delimited_after(text: &str, marker: &str, open_ch: char, close_ch: char) -> String {
    let start = unique_marker(text, marker);
    let rest = &text[start + marker.len()..];
    let open = rest.find(open_ch).unwrap_or_else(|| {
        panic!("no {open_ch:?}-delimited value opens after {marker:?}")
    });
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut out = String::new();
    for ch in rest[open..].chars() {
        if let Some(q) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            out.push(ch);
            continue;
        }
        if ch == open_ch {
            depth += 1;
            if depth > 1 {
                out.push(ch);
            }
            continue;
        }
        if ch == close_ch {
            depth -= 1;
            if depth == 0 {
                return out;
            }
            out.push(ch);
            continue;
        }
        out.push(ch);
    }
    panic!("unterminated {open_ch:?}-delimited value after {marker:?}");
}

fn block_after(text: &str, marker: &str) -> String {
    delimited_after(text, marker, '{', '}')
}

fn paren_expr_after(text: &str, marker: &str) -> String {
    delimited_after(text, marker, '(', ')')
}

/// Collapses every run of whitespace to a single space, so a source check is
/// not a line-ending check.
fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits a literal's body at its TOP-LEVEL commas, quote- and nesting-aware.
/// Empty trailing elements (a trailing comma) are dropped; everything else is
/// returned verbatim for the caller to account for.
fn top_level_elements(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in block.chars() {
        if let Some(q) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            '{' | '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '}' | ']' | ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    out.push(current);
    assert!(quote.is_none(), "unterminated string literal in {block:?}");
    out.into_iter()
        .filter(|element| !element.trim().is_empty())
        .collect()
}

/// Top-level `<key>: { <inner> }` entries of a Python dict literal.
///
/// FAIL-CLOSED: every top-level element must have this exact shape. An element
/// whose value is not a brace literal, or which has no `:` at all, panics
/// rather than being skipped.
fn nested_entries(block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for element in top_level_elements(block) {
        let colon = top_level_colon(&element).unwrap_or_else(|| {
            panic!("dict element has no top-level `:`: {element:?}")
        });
        let (key, value) = element.split_at(colon);
        let value = value[1..].trim();
        assert!(
            value.starts_with('{') && value.ends_with('}'),
            "dict element value is not a brace literal (this sweep only \
             understands `<key>: {{...}}`, and refuses to skip what it cannot \
             read): {element:?}"
        );
        out.push((
            key.trim().to_string(),
            value[1..value.len() - 1].to_string(),
        ));
    }
    out
}

/// Byte offset of the first `:` at nesting depth 0 and outside a string.
fn top_level_colon(element: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in element.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ':' if depth == 0 => return Some(offset),
            _ => {}
        }
    }
    None
}

/// Integer keys of a Python dict literal.
///
/// FAIL-CLOSED: every top-level element must be `<integer literal>: <value>`.
/// A computed key (`23 + 0:`), a `**spread`, or a non-integer key panics --
/// the runtime would honour them and this sweep must never claim to have
/// checked a table it only partly read.
fn int_keys(block: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for element in top_level_elements(block) {
        let colon = top_level_colon(&element).unwrap_or_else(|| {
            panic!("dict element has no top-level `:`: {element:?}")
        });
        let key = element[..colon].trim();
        let bit: u32 = key.parse().unwrap_or_else(|_| {
            panic!(
                "status-bit key {key:?} is not a plain integer literal; this \
                 sweep refuses to guess what the runtime would compute"
            )
        });
        assert!(bit < 32, "status-bit key {bit} is outside a u32 status word");
        assert!(out.insert(bit), "status-bit key {bit} is declared twice");
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
    let text = strip_line_comments(&read_script("report_raw_capture.py"));
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
    let stripped = strip_line_comments(&text);
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
    let declared: BTreeSet<u32> = paren_expr_after(&stripped, "FAILURE_MASK = ")
        .split('|')
        .map(|term| term.trim().trim_matches(['(', ')']).trim().to_string())
        .map(|term| {
            // Fail closed: a term this sweep cannot read is a term it cannot
            // vouch for, and skipping it would let an extra bit ride along.
            let shift = term
                .strip_prefix("1 <<")
                .unwrap_or_else(|| panic!("FAILURE_MASK term {term:?} is not `1 << <bit>`"));
            shift
                .trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("FAILURE_MASK term {term:?} has a non-literal shift"))
        })
        .collect();
    assert_eq!(
        declared, failure_bits,
        "report_raw_capture.py FAILURE_MASK is not compact_contract::COMPACT_STATUS_FAILURE_MASK \
         ({COMPACT_STATUS_FAILURE_MASK:#x})"
    );
}

#[test]
fn c0_domain_census_lane_local_table_matches_the_contract() {
    let text = strip_line_comments(&read_script("c0_domain_census.py"));
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
    let raw = read_script("c0_pass2_spans.py");
    let text = strip_line_comments(&raw);
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
        squeeze(&raw).contains(
            "LANE_LOCAL_MASKS = { domain: sum(1 << bit for bit in bits) \
             for domain, bits in DOMAIN_STATUS_BITS.items() }"
        ),
        "c0_pass2_spans.py LANE_LOCAL_MASKS is not derived from DOMAIN_STATUS_BITS"
    );
}

#[test]
fn production_promotion_lane_local_table_matches_the_contract() {
    let text = strip_line_comments(&read_script("ProductionCapture.Promotion.ps1"));
    let block = block_after(&text, "$script:ProductionPromotionLaneLocalStatusMask = ");
    let mut observed = BTreeMap::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Fail closed: every non-blank line in this hashtable must be an entry
        // this sweep can read. Skipping the rest is how a second, unchecked
        // entry would ride along unseen.
        let (domain, value) = line.split_once('=').unwrap_or_else(|| {
            panic!("promotion lane-local hashtable line is not `<domain> = <mask>`: {line:?}")
        });
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

// --- Parser regression tests -------------------------------------------------
//
// These pin the parsers against the ways a hand-written parser silently
// under-reads. Each one is a shape a real script edit could take that the
// RUNTIME would honour; if the parser skipped it instead of panicking, this
// sweep would keep reporting "table matches the contract" about a table it had
// only partly read.

#[test]
fn a_comment_cannot_truncate_a_block() {
    // `# }` inside the table: Python ignores it, and an unstripped scanner
    // would end the block there and never see bit 23.
    let source = "TABLE = {\n    53: {16: 'a'},  # }\n    60: {23: 'b'},\n}\n";
    let block = block_after(&strip_line_comments(source), "TABLE = ");
    let entries = nested_entries(&block);
    assert_eq!(
        entries.len(),
        2,
        "the comment truncated the block: {entries:?}"
    );
    assert_eq!(mask_of(&int_keys(&entries[1].1)), 1 << 23);
}

#[test]
fn a_comment_cannot_truncate_a_paren_expression() {
    let source = "MASK = (\n    (1 << 0)  # )\n    | (1 << 9)\n)\n";
    let expr = paren_expr_after(&strip_line_comments(source), "MASK = ");
    assert_eq!(expr.split('|').count(), 2, "the comment truncated: {expr:?}");
}

#[test]
#[should_panic(expected = "not a plain integer literal")]
fn a_computed_key_is_refused_not_skipped() {
    // Valid Python. The runtime derives bit 23 from it and strips that bit;
    // a parser that only collected integer keys would report mask 0x10000.
    int_keys("16: 'a', 23 + 0: 'b'");
}

#[test]
#[should_panic(expected = "declared twice")]
fn a_duplicated_bit_key_is_refused() {
    int_keys("16: 'a', 16: 'b'");
}

#[test]
#[should_panic(expected = "outside a u32 status word")]
fn a_bit_key_beyond_the_status_word_is_refused() {
    int_keys("32: 'a'");
}

#[test]
#[should_panic(expected = "not a brace literal")]
fn a_non_dict_table_value_is_refused_not_skipped() {
    // `dict(...)` and a bare alias are both valid Python that the runtime would
    // expand into real bits.
    nested_entries("53: {16: 'a'}, 60: dict(((17, 'b'),))");
}

#[test]
#[should_panic(expected = "must occur exactly once")]
fn an_ambiguous_marker_is_refused() {
    // Two candidate tables and a parser that silently takes the first is a
    // "checked" table that is no longer the one the runtime reads.
    block_after("TABLE = {53: {16: 'a'}}\nTABLE = {53: {}}\n", "TABLE = ");
}

#[test]
fn a_hash_inside_a_string_is_not_a_comment() {
    // The badge names in these tables are quoted text; stripping a `#` inside
    // one would corrupt the source the later scanners read.
    let source = "TABLE = {\n    53: {16: 'a#}b'},\n}\n";
    let block = block_after(&strip_line_comments(source), "TABLE = ");
    assert_eq!(mask_of(&int_keys(&nested_entries(&block)[0].1)), 1 << 16);
}

#[test]
fn a_colon_inside_a_badge_name_is_not_a_key_separator() {
    // Every real badge name in these tables contains a `:` (`d53:intel_map_...`,
    // `LANE:TORN_ROWS_PRESENT`), so this is the live case, not a hypothetical.
    assert_eq!(
        mask_of(&int_keys(
            "16: \"d53:operations_detail_resisted\", 17: 'LANE:X'"
        )),
        (1 << 16) | (1 << 17)
    );
}

// --- The harness verdict actually reaches the caller -------------------------

/// Extracts the body of `if (<condition>) {` ... matching `}` from a
/// PowerShell script, condition included, as executable text.
fn powershell_if_block(text: &str, condition: &str) -> String {
    let marker = format!("if ({condition}) {{");
    let start = unique_marker(text, &marker);
    let rest = &text[start..];
    let mut depth = 0i32;
    for (offset, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..=offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated if block for {condition:?}");
}

/// The exit-2 branch is the whole point of this lane's change: callers separate
/// "undefined status bits" from every ordinary capture failure by that code.
///
/// This does NOT retype the branch -- it lifts the exact lines out of the
/// harness and runs them, because the defect it pins is invisible to reading:
/// the harness sets `$ErrorActionPreference = 'Stop'`, under which `Write-Error`
/// is TERMINATING, so a `Write-Error` immediately before `exit 2` means `exit 2`
/// never executes and PowerShell 5.1 returns 1 instead. The branch looked
/// correct and silently produced the wrong code.
#[test]
#[cfg(windows)]
fn the_harness_undefined_status_branch_really_exits_two() {
    use std::io::Write;

    let harness = read_script("Invoke-HeadlessRawCapture.ps1");
    let branch = powershell_if_block(
        &harness,
        "-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)",
    );
    // Deliberately NOT a `branch.contains("exit 2")` guard: the branch's own
    // comment contains that text, so such a guard passes while the code exits
    // 1. Only running it decides. Same reason the defect survived review.

    // Same preamble the harness runs under. Anything else would prove nothing.
    let preference = "$ErrorActionPreference = 'Stop'";
    assert!(
        harness.contains(preference),
        "the harness no longer sets {preference}, so this proof no longer \
         reproduces its conditions"
    );
    let script = format!(
        "{preference}\n$undefinedStatusBitFailure = 'proof'\n{branch}\nexit 0\n"
    );

    let path = std::env::temp_dir().join(format!(
        "hoi4_lane_i_exit_probe_{}.ps1",
        std::process::id()
    ));
    let mut file = fs::File::create(&path).expect("create probe script");
    file.write_all(script.as_bytes()).expect("write probe script");
    drop(file);

    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run powershell");
    let _ = fs::remove_file(&path);

    assert_eq!(
        status.code(),
        Some(2),
        "the harness's undefined-status branch does not deliver exit code 2 \
         under its own $ErrorActionPreference; a terminating Write-Error before \
         `exit 2` collapses it to 1. Branch was:\n{branch}"
    );
}

/// A capture that finalized with no reporter present is an UNASKED question.
/// The two Test-Path conditions are checked apart precisely so this case cannot
/// hide inside a combined `-and` and fall through to a plain informational
/// message with the recorder's own exit code.
#[test]
fn a_missing_reporter_is_not_a_pass() {
    let harness = read_script("Invoke-HeadlessRawCapture.ps1");
    assert!(
        !harness.contains(
            "if ((Test-Path -LiteralPath $streamReport -PathType Leaf) -and\n    (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {"
        ) && !harness.contains(
            "if ((Test-Path -LiteralPath $streamReport -PathType Leaf) -and\r\n    (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {"
        ),
        "the reporter and manifest existence checks are combined again, which \
         makes a finalized capture with a missing reporter exit 0"
    );
    let branch = powershell_if_block(&harness, "$haveManifest -and -not $haveStreamReport");
    assert!(
        branch.contains("$undefinedStatusBitFailure ="),
        "a finalized capture with no reporter does not set the blocking verdict: {branch}"
    );
}

/// Every way the report can fail to answer must set the blocking verdict.
/// Invoke-BoundedNativeCapture THROWS on timeout, so the catch is on this path,
/// not off it.
#[test]
fn an_unanswered_report_is_not_a_pass() {
    let harness = squeeze(&read_script("Invoke-HeadlessRawCapture.ps1"));
    for required in [
        // the undefined verdict itself
        "if ($reportResult.ExitCode -eq 2) { $undefinedStatusBitFailure =",
        // an exit code outside the report's own vocabulary
        "} elseif ($reportResult.ExitCode -notin @(0, 1)) {",
        // timeout / launch failure, which arrives as a throw
        "} catch { Write-Host \"report failed: $($_.Exception.Message)\"",
    ] {
        assert!(
            harness.contains(required),
            "the harness no longer covers an unanswered report: missing {required:?}"
        );
    }
    assert_eq!(
        harness.matches("$undefinedStatusBitFailure =").count(),
        5,
        "expected the declaration plus exactly four assignments of the blocking \
         verdict (missing reporter, report exit 2, report exit outside its own \
         vocabulary, report threw); a removed one is a reopened fail-open, and \
         an added one is a path this test has not been told about"
    );
}
