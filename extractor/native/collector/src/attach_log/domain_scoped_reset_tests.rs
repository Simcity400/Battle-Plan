//! Gates for the DOMAIN-SCOPED rejected-bracket reset.
//!
//! `reset_trackers_after_rejected_bracket` used to wipe every domain's delta
//! pin whenever ANY domain crossed an hour boundary. At speed five that is
//! essentially every frame. Measured on
//! `runs/live_unlock/t26/collector_stream.bin` (2,594,412,495 bytes), keyframes
//! are 82.0% of all bytes; of those, **273.15 MB in 2,554 records** were
//! emitted by a domain that had NO gap of its own and lost its pin only because
//! somebody else crossed an hour boundary. Those are the records this narrowing
//! converts back into deltas. The larger 1,126.91 MB row in
//! `DELTA_WIRE_DESIGN.md` §2.3 is a different bucket — keyframes following the
//! domain's OWN gap — and this change deliberately does not touch it.
//!
//! [`for_each_domain_scoped_tracker`] narrows that to the domains actually
//! rejected. This module holds the questions the narrowing has to answer, and
//! every test is written so that a production change turns it red:
//!
//! 1. **Is the table right?**
//!    `domain_scoped_tracker_allowlist_matches_the_borrow_sites` re-derives the
//!    (field, tag) pairs from this crate's own source text and compares them to
//!    the table. Point a table entry at a different domain and it fails.
//! 2. **Does a rejected domain still lose its pin?**
//!    `a_rejected_domain_loses_its_pin` rejects one domain each probe's tag
//!    covers and asserts the marked field came back `Default`.
//! 3. **Does an unrejected domain keep its pin?**
//!    `an_unrejected_domain_keeps_its_pin` is the control that stops every
//!    other test here from passing trivially under a blanket reset.

use super::*;

use std::collections::BTreeMap;

// ---------------------------------------------------------------------
// The table, and the source text it must agree with
// ---------------------------------------------------------------------

/// Fields of `CompactPerfGateTrackers` that are deliberately NOT domain-scoped,
/// each with the reason. `every_perf_gate_tracker_field_is_classified` holds
/// this list and [`DOMAIN_SCOPED_TRACKER_RETENTION`] to cover the struct, so a
/// field added later cannot quietly land on one side or the other.
const BLANKET_RESET_TRACKER_FIELDS: &[(&str, &str)] = &[
    (
        "hook_reservations",
        "frame custody, carried across the reset unconditionally by the caller",
    ),
    (
        "domain_gap_ordinals",
        "H4DG ordinal bookkeeping, carried across unconditionally",
    ),
    (
        "domain_outcomes",
        "this frame's outcome ledger; rewritten after the reset on every exit",
    ),
    (
        "d45_terminal_read",
        "already domain-scoped by its own carve-out on d45",
    ),
    (
        "d57_terminal_frame",
        "already domain-scoped by its own carve-out on d57",
    ),
    (
        "division_roots",
        "root cache shared by five readers and rebuilt on the main thread; not a pin",
    ),
    (
        "characters_leaders_cabinet_live",
        "d47 prewarm root cache, written outside any reader bracket; d47 emits no deltas",
    ),
    (
        "acceptance_weather_fault_injected",
        "session-local one-shot, not a per-domain read product",
    ),
    (
        "focus_research_decisions_depth_live",
        "d45's pin is now TRANSACTIONAL -- staged inside the bracket, committed only where the \
         late custody status is final -- so it can no longer run ahead of the fold. Joining the \
         allowlist is a separate question with its own fold-equivalence proof to write, and it \
         stays on the blanket reset until that lands",
    ),
    (
        "tech_country_delta",
        "d13's wire baseline is advanced at EMISSION, after every reader has returned, so no \
         reader bracket can decide whether it survived; the blanket reset is what keeps the pin \
         and the fold's mirror in step",
    ),
    (
        "land_combat_detail_events_live",
        "d55 took the same repair as d45: the snapshot baseline is staged and committed against \
         the same shared contract predicate the fold applies. Also awaiting its own \
         fold-equivalence proof before it can join the allowlist",
    ),
];

/// This crate's own `attach_log.rs`, with line endings normalised to LF.
///
/// Every structural match below is written against `\n`, and this repo checks
/// `attach_log.rs` out with CRLF. Un-normalised, the two-line rustfmt shape of
/// a binding (`let x =` / `&mut trackers.field;`) stops being recognised, so
/// those fields are reported as having no borrow site at all — which is how
/// this module first went red, and is why the normalisation is load-bearing
/// rather than tidy. The shared `collector_source_lf` in the sibling `tests`
/// module is the one place that idiom lives.
fn attach_log_source() -> String {
    super::tests::collector_source_lf()
}

/// The body of `build_compact_extraction_gate_records`, which is the only
/// function that hands a per-domain tracker to a reader.
fn frame_builder_body(source: &str) -> &str {
    const OPEN: &str = "\nfn build_compact_extraction_gate_records(";
    let start = source
        .find(OPEN)
        .expect("the frame builder is declared at column zero");
    let rest = &source[start + 1..];
    let end = rest
        .match_indices("\nfn ")
        .map(|(at, _)| at)
        .chain(rest.match_indices("\npub fn ").map(|(at, _)| at))
        .min()
        .expect("another item follows the frame builder");
    &source[start..start + 1 + end]
}

/// One `spawn_gated(..)` / `lane_time_main(..)` call: its byte span inside the
/// frame builder's body, and the bracket tag it opens.
struct ReaderCallSpan {
    start: usize,
    end: usize,
    tag: String,
}

/// Every reader call in `body`, paren-matched so the span is exact.
///
/// A nearest-preceding-`spawn_gated` heuristic is not good enough: several
/// tracker locals are rebound (`let frd_depth_tracker = frd_depth_tracker;`)
/// or projected (`let d47_roots = &..._tracker.roots;`) in the statement that
/// PRECEDES a call, and a heuristic reads those as uses inside the previous
/// reader's call — which is exactly the misattribution this table must not
/// make.
fn reader_call_spans(body: &str) -> Vec<ReaderCallSpan> {
    let bytes = body.as_bytes();
    let mut spans = Vec::new();
    for (name, tag_arg) in [("spawn_gated(", 3usize), ("lane_time_main(", 1usize)] {
        let mut from = 0usize;
        while let Some(at) = body[from..].find(name) {
            let call_start = from + at;
            let open = call_start + name.len() - 1;
            let mut depth = 0i32;
            let mut i = open;
            while i < bytes.len() {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            assert!(i < bytes.len(), "a {name} call is closed");
            let args = split_top_level_args(&body[open + 1..i]);
            let tag = args
                .get(tag_arg)
                .unwrap_or_else(|| panic!("a {name} call carries a bracket tag argument"))
                .clone();
            spans.push(ReaderCallSpan {
                start: call_start,
                end: i,
                tag,
            });
            from = open + 1;
        }
    }
    spans
}

/// Split a call's argument list on top-level commas, dropping comment lines.
fn split_top_level_args(call: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for ch in call.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            args.push(strip_comment_lines(&current));
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.trim().is_empty() {
        args.push(strip_comment_lines(&current));
    }
    args.retain(|arg| !arg.is_empty());
    args
}

fn strip_comment_lines(arg: &str) -> String {
    arg.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Is `at` a use of the identifier rather than a substring of a longer one?
fn is_word_boundary(body: &str, at: usize, len: usize) -> bool {
    let bytes = body.as_bytes();
    let before_ok = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
    let after = at + len;
    let after_ok =
        after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
    before_ok && after_ok
}

fn line_bounds(body: &str, at: usize) -> (usize, usize) {
    let start = body[..at].rfind('\n').map(|nl| nl + 1).unwrap_or(0);
    let end = body[at..].find('\n').map(|nl| at + nl).unwrap_or(body.len());
    (start, end)
}

fn line_containing(body: &str, at: usize) -> &str {
    let (start, end) = line_bounds(body, at);
    &body[start..end]
}

/// Re-derive `field -> {bracket tag}` from the source, exactly as a reader of
/// the code would: find the `&mut trackers.<field>` binding, then find every
/// use of the local it bound and the reader call that encloses it.
fn derive_field_tags_from_source(body: &str) -> BTreeMap<String, Vec<String>> {
    let spans = reader_call_spans(body);
    assert!(
        spans.len() > 30,
        "the frame builder should hold dozens of reader calls, found {}",
        spans.len()
    );

    let mut derived: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // The struct-literal form: `CompactCountryGateTrackers { fuel: &mut
    // trackers.fuel, .. }`. The whole struct moves into ONE reader call, so
    // every field in it takes that call's tag.
    let country_literal_tag = spans
        .iter()
        .find(|span| span.tag == "LW_TAG_COUNTRY_BUNDLE")
        .map(|span| span.tag.clone())
        .expect("the country bundle reader call is present");
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_suffix(',') {
            if let Some((lhs, rhs)) = rest.split_once(": &mut trackers.") {
                if !lhs.contains(' ') && !rhs.contains(' ') {
                    derived
                        .entry(rhs.to_string())
                        .or_default()
                        .push(country_literal_tag.clone());
                }
            }
        }
    }

    // The `let <local> = &mut trackers.<field>;` form, including the two-line
    // rustfmt shape where the borrow lands on its own line.
    let flat = body.replace("=\n", "= ");
    let mut bindings: Vec<(String, String)> = Vec::new();
    for line in flat.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("let ") else {
            continue;
        };
        let Some((local, tail)) = rest.split_once(" = ") else {
            continue;
        };
        let tail = tail.trim();
        let Some(field) = tail
            .strip_prefix("&mut trackers.")
            .or_else(|| tail.strip_prefix("&trackers."))
            .and_then(|field| field.strip_suffix(';'))
        else {
            continue;
        };
        if local.contains(' ') || field.contains(' ') || field.contains('.') {
            continue;
        }
        bindings.push((local.to_string(), field.to_string()));
    }
    assert!(
        bindings.len() > 25,
        "the frame builder binds dozens of tracker locals, parsed {}",
        bindings.len()
    );

    for (local, field) in &bindings {
        let entry = derived.entry(field.clone()).or_default();
        let mut search = 0usize;
        while let Some(at) = body[search..].find(local.as_str()) {
            let use_at = search + at;
            search = use_at + local.len();
            if !is_word_boundary(body, use_at, local.len()) {
                continue;
            }
            let trimmed = line_containing(body, use_at).trim().to_string();
            // The binding statement itself, in either rustfmt shape.
            if trimmed.starts_with(&format!("let {local} = &mut trackers."))
                || trimmed.starts_with(&format!("let {local} = &trackers."))
                || trimmed == format!("let {local} =")
                || trimmed == format!("&mut trackers.{field};")
                || trimmed == format!("&trackers.{field};")
            {
                continue;
            }
            // A rebind that exists only to name the capture for a move closure.
            if trimmed == format!("let {local} = {local};") {
                continue;
            }
            let innermost = spans
                .iter()
                .filter(|span| span.start < use_at && use_at < span.end)
                .max_by_key(|span| span.start);
            match innermost {
                Some(span) => entry.push(span.tag.clone()),
                // A use outside every reader call: recorded under a name no
                // table entry can carry, so the comparison reports it rather
                // than silently accepting it.
                None => entry.push(format!(
                    "OUTSIDE-EVERY-READER-CALL(body line {})",
                    body[..use_at].matches('\n').count() + 1
                )),
            }
        }
    }

    for tags in derived.values_mut() {
        tags.sort();
        tags.dedup();
    }
    derived
}

/// The table of table entries, flattened out of the macro.
fn table() -> BTreeMap<&'static str, &'static str> {
    let mut table = BTreeMap::new();
    macro_rules! collect_table {
        ($($field:ident => $tag:expr,)*) => {
            $( table.insert(stringify!($field), stringify!($tag)); )*
        };
    }
    for_each_domain_scoped_tracker!(collect_table);
    table
}

/// The allowlist must say what the code says.
///
/// This is what turns a hand-written table into a checked one. The retention
/// predicate is "the reader that writes this field was not rejected", and the
/// only thing that makes a (field, tag) pair TRUE is that the field's `&mut`
/// borrow is consumed by a reader running under that tag. So the test
/// re-derives the pairs from the source and demands equality.
///
/// What production change makes this fail: point a table entry at a different
/// domain constant; move a reader to a different bracket tag; give a listed
/// field a second writer, or any writer outside a reader's bracket.
#[test]
fn domain_scoped_tracker_allowlist_matches_the_borrow_sites() {
    let source = attach_log_source();
    let body = frame_builder_body(&source);
    let derived = derive_field_tags_from_source(body);
    let table = table();

    assert_eq!(
        table.len(),
        DOMAIN_SCOPED_TRACKER_RETENTION.len(),
        "the table macro and the flattened constant must describe the same list"
    );

    let mut disagreements = Vec::new();
    for (field, expected_tag) in &table {
        match derived.get(*field) {
            Some(tags) if tags.as_slice() == [expected_tag.to_string()] => {}
            Some(tags) => disagreements.push(format!(
                "{field}: table says [{expected_tag}], source says {tags:?}"
            )),
            None => disagreements.push(format!(
                "{field}: table says [{expected_tag}], source has no `&mut trackers.{field}` \
                 binding inside the frame builder"
            )),
        }
    }
    assert!(
        disagreements.is_empty(),
        "the domain-scoped retention table disagrees with the borrow sites it claims to \
         describe:\n  {}",
        disagreements.join("\n  ")
    );

    // Following the BINDING's uses cannot see a direct write that never touches
    // the local — `trackers.weather = ...` further down the frame builder would
    // be a second writer the derivation above is blind to. So each allowlisted
    // field is also required to appear exactly ONCE as `trackers.<field>` in the
    // whole frame builder: its `&mut` borrow, and nothing else.
    let mut second_writers = Vec::new();
    for field in table.keys() {
        let needle = format!("trackers.{field}");
        let mut occurrences = 0usize;
        let mut search = 0usize;
        while let Some(at) = body[search..].find(&needle) {
            let at = search + at;
            search = at + needle.len();
            if !is_word_boundary(body, at + "trackers.".len(), field.len()) {
                continue;
            }
            occurrences += 1;
        }
        if occurrences != 1 {
            second_writers.push(format!(
                "{field}: `trackers.{field}` appears {occurrences} times in the frame builder; \
                 exactly one (its `&mut` borrow) is what makes its bracket tag the whole answer"
            ));
        }
    }
    assert!(
        second_writers.is_empty(),
        "an allowlisted tracker field is reachable other than through its single borrow:\n  {}",
        second_writers.join("\n  ")
    );

    // The converse: every field the source shows being handed to a reader must
    // be classified one way or the other before it ships.
    let blanket: BTreeMap<&str, &str> = BLANKET_RESET_TRACKER_FIELDS.iter().copied().collect();
    let mut unclassified = Vec::new();
    for (field, tags) in &derived {
        if table.contains_key(field.as_str()) || blanket.contains_key(field.as_str()) {
            continue;
        }
        unclassified.push(format!("{field} (reader tags {tags:?})"));
    }
    assert!(
        unclassified.is_empty(),
        "these tracker fields are in neither the domain-scoped allowlist nor \
         BLANKET_RESET_TRACKER_FIELDS: {unclassified:?}"
    );
}

/// Both exclusions must remain excluded for the stated reason: each is touched
/// somewhere no single reader bracket covers, so no one tag can decide whether
/// it survived.
///
/// What production change makes this fail: moving d47's prewarm inside its
/// worker, or making the division root cache a per-reader product — either of
/// which would make the field eligible and this exclusion stale.
#[test]
fn the_two_excluded_root_caches_are_touched_outside_every_reader_bracket() {
    let source = attach_log_source();
    let body = frame_builder_body(&source);
    let derived = derive_field_tags_from_source(body);

    for field in ["characters_leaders_cabinet_live", "division_roots"] {
        let tags = derived
            .get(field)
            .unwrap_or_else(|| panic!("{field} is bound in the frame builder"));
        assert!(
            tags.iter()
                .any(|tag| tag.starts_with("OUTSIDE-EVERY-READER-CALL")),
            "{field} is excluded from the domain-scoped allowlist because it is written \
             outside every reader bracket; the source now says its uses are {tags:?}, so the \
             exclusion needs re-deciding rather than keeping"
        );
    }
}

/// d45 and d55 are held OUT of the allowlist deliberately, and re-adding either
/// must be a decision rather than an edit.
///
/// Both survive their clock bracket and can still acquire a failure-class status
/// afterwards, from hook-reservation custody settled on the main thread. The
/// consumer discards a failure-class record whole, so their pin would advance on
/// a record the fold never applied. Bracket survival — which is the entire
/// retention predicate — does not decide that question for these two, and the
/// blanket reset is what currently hides it.
///
/// What production change makes this fail: putting either field back in the
/// allowlist. If that is intended, the tracker advance has to become
/// transactional with the late status decision first, and this test is where
/// that argument gets recorded.
#[test]
fn the_two_late_failure_domains_are_not_domain_scoped() {
    for field in [
        "focus_research_decisions_depth_live",
        "land_combat_detail_events_live",
    ] {
        assert!(
            !DOMAIN_SCOPED_TRACKER_RETENTION
                .iter()
                .any(|(name, _)| *name == field),
            "`{field}` must stay on the blanket reset until its tracker advance is transactional \
             with the hook-custody status that can badge its record failure-class after the \
             reader's bracket has closed"
        );
        assert!(
            BLANKET_RESET_TRACKER_FIELDS
                .iter()
                .any(|(name, _)| *name == field),
            "`{field}` must carry its exclusion reason in BLANKET_RESET_TRACKER_FIELDS"
        );
    }
}

/// Every field of the tracker struct is classified, one way or the other.
///
/// What production change makes this fail: adding a field to
/// `CompactPerfGateTrackers` without deciding whether a rejected bracket may
/// retain it.
#[test]
fn every_perf_gate_tracker_field_is_classified() {
    let source = attach_log_source();
    let start = source
        .find("struct CompactPerfGateTrackers {")
        .expect("the tracker struct is declared");
    let end = start
        + source[start..]
            .find("\n}\n")
            .expect("the tracker struct is closed");
    let declaration = &source[start..end];

    let mut fields = Vec::new();
    let flat = declaration.replace(":\n", ": ");
    for line in flat.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }
        let Some((name, rest)) = trimmed.split_once(':') else {
            continue;
        };
        if name.contains(' ') || name.is_empty() || rest.is_empty() {
            continue;
        }
        if !name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        {
            continue;
        }
        fields.push(name.to_string());
    }
    assert!(
        fields.len() > 40,
        "the tracker struct should hold dozens of fields, parsed {}: {fields:?}",
        fields.len()
    );

    let scoped: Vec<&str> = DOMAIN_SCOPED_TRACKER_RETENTION
        .iter()
        .map(|(field, _)| *field)
        .collect();
    let blanket: Vec<&str> = BLANKET_RESET_TRACKER_FIELDS
        .iter()
        .map(|(field, _)| *field)
        .collect();
    let unclassified: Vec<&String> = fields
        .iter()
        .filter(|field| !scoped.contains(&field.as_str()) && !blanket.contains(&field.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "these `CompactPerfGateTrackers` fields are in neither the domain-scoped allowlist \
         nor BLANKET_RESET_TRACKER_FIELDS: {unclassified:?}"
    );

    for field in scoped.iter().chain(blanket.iter()) {
        assert!(
            fields.iter().any(|declared| declared == field),
            "`{field}` is classified but is not a field of CompactPerfGateTrackers"
        );
    }
}

// ---------------------------------------------------------------------
// The retention predicate itself
// ---------------------------------------------------------------------

/// Every domain a bracket tag covers, through the production expansion.
fn covered_domains(tag: u16) -> Vec<u16> {
    let mut domains = Vec::new();
    for_each_bracket_covered_domain(tag, |domain| domains.push(domain));
    domains
}

/// A domain id nothing in the table covers, used as the "unrelated rejection"
/// in the control tests.
const UNRELATED_DOMAIN: u16 = 0xBEEF;

/// A tag that covers more than one domain must lose its fields when ANY of
/// them is rejected, not only when all of them are.
///
/// What production change makes this fail: testing `rejected_domains` against
/// the raw tag instead of expanding it through
/// `for_each_bracket_covered_domain` — which would let the country bundle keep
/// eight pins when domain 14 tore, and let d49 keep its pin when d50 tore.
#[test]
fn a_multi_domain_tag_survives_only_when_every_covered_domain_did() {
    let multi: Vec<u16> = DOMAIN_SCOPED_TRACKER_RETENTION
        .iter()
        .map(|(_, tag)| *tag)
        .filter(|tag| covered_domains(*tag).len() > 1)
        .collect();
    assert!(
        !multi.is_empty(),
        "at least the country bundle and d49 cover several domains each"
    );
    for tag in multi {
        let domains = covered_domains(tag);
        assert!(
            domain_scoped_tracker_tag_survived(tag, &BTreeSet::new()),
            "tag {tag} survives an empty rejected set"
        );
        for domain in &domains {
            let rejected: BTreeSet<u16> = [*domain].into_iter().collect();
            assert!(
                !domain_scoped_tracker_tag_survived(tag, &rejected),
                "tag {tag} covers domain {domain}; rejecting it must not leave the tag's \
                 trackers retained (covered domains {domains:?})"
            );
        }
    }
}

/// Every tag in the table loses its fields when its own domain is rejected,
/// and keeps them when an unrelated domain is.
#[test]
fn the_retention_predicate_answers_per_tag() {
    for (field, tag) in DOMAIN_SCOPED_TRACKER_RETENTION {
        let covered = covered_domains(*tag);
        assert!(
            !covered.is_empty(),
            "`{field}`'s tag {tag} must cover at least one domain"
        );
        assert!(
            !covered.contains(&UNRELATED_DOMAIN),
            "the synthetic unrelated domain must not be covered by `{field}`'s tag {tag}"
        );
        let unrelated: BTreeSet<u16> = [UNRELATED_DOMAIN].into_iter().collect();
        assert!(
            domain_scoped_tracker_tag_survived(*tag, &unrelated),
            "`{field}` must survive a rejection of a domain its tag does not cover"
        );
        for domain in covered {
            let rejected: BTreeSet<u16> = [domain].into_iter().collect();
            assert!(
                !domain_scoped_tracker_tag_survived(*tag, &rejected),
                "`{field}` must not survive the rejection of covered domain {domain}"
            );
        }
    }
}

// ---------------------------------------------------------------------
// The reset, end to end over the real struct
// ---------------------------------------------------------------------

/// One observable per-domain tracker field: how to put a non-default value in
/// it, and how to ask whether it is still there.
///
/// The fields differ in type, so there is no generic "is this default"; each
/// probe names a concrete leaf whose default is unambiguous (an empty map, a
/// `None`). Three probes cover a multi-domain tag and two single-domain ones.
struct TrackerProbe {
    field: &'static str,
    mark: fn(&mut CompactPerfGateTrackers),
    is_marked: fn(&CompactPerfGateTrackers) -> bool,
}

const TRACKER_PROBES: &[TrackerProbe] = &[
    // The country bundle: one tag, eight fields, domains 10..=17.
    TrackerProbe {
        field: "fuel",
        mark: |trackers| {
            trackers
                .fuel
                .previous_rows
                .insert("probe".to_string(), "value".to_string());
        },
        is_marked: |trackers| !trackers.fuel.previous_rows.is_empty(),
    },
    // d44, whose tracker ALSO carries the two install booleans the reset
    // carves out unconditionally — so this probe is the one that would notice
    // the carry-over and the carve-out fighting each other.
    TrackerProbe {
        field: "supply_logistics_attrition_live",
        mark: |trackers| {
            trackers.supply_logistics_attrition_live.csupply_system_object = Some(0x1234);
        },
        is_marked: |trackers| {
            trackers
                .supply_logistics_attrition_live
                .csupply_system_object
                .is_some()
        },
    },
    // d43, a sweep-generation cache rather than a row diff: the retention
    // question is the same one.
    TrackerProbe {
        field: "market_trade_live",
        mark: |trackers| {
            trackers.market_trade_live.sweep_gen = Some(7);
        },
        is_marked: |trackers| trackers.market_trade_live.sweep_gen.is_some(),
    },
];

fn probe_tag(field: &str) -> u16 {
    DOMAIN_SCOPED_TRACKER_RETENTION
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, tag)| *tag)
        .unwrap_or_else(|| panic!("`{field}` is a domain-scoped tracker field"))
}

/// A rejected domain still loses its pin — the invariant the narrowing must
/// not weaken.
///
/// What production change makes this fail: retaining a field whose domain is
/// in the rejected set (drop the negation from the predicate, or compare
/// against the wrong set).
#[test]
fn a_rejected_domain_loses_its_pin() {
    for probe in TRACKER_PROBES {
        for domain in covered_domains(probe_tag(probe.field)) {
            let mut trackers = CompactPerfGateTrackers::default();
            assert!(
                !(probe.is_marked)(&trackers),
                "`{}` must start at its default",
                probe.field
            );
            (probe.mark)(&mut trackers);
            assert!(
                (probe.is_marked)(&trackers),
                "the marker on `{}` must be observable before the reset",
                probe.field
            );
            let rejected: BTreeSet<u16> = [domain].into_iter().collect();
            reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
            assert!(
                !(probe.is_marked)(&trackers),
                "domain {domain} was rejected, so `{}` must have been dropped",
                probe.field
            );
        }
    }
}

/// The control: a domain nobody rejected keeps its pin. Without this every
/// test above passes trivially under the old blanket reset.
///
/// What production change makes this fail: restoring the unconditional
/// `*trackers = CompactPerfGateTrackers::default()`.
#[test]
fn an_unrejected_domain_keeps_its_pin() {
    for probe in TRACKER_PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.mark)(&mut trackers);
        let rejected: BTreeSet<u16> = [UNRELATED_DOMAIN].into_iter().collect();
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
        assert!(
            (probe.is_marked)(&trackers),
            "no domain covering `{}` was rejected, so its pin must survive the reset",
            probe.field
        );
    }
}

/// One domain's rejection must not take another domain's pin with it. This is
/// the whole point of the change, stated as a test.
///
/// What production change makes this fail: any reset that is not per-tag —
/// including one that drops every field whenever the rejected set is
/// non-empty.
#[test]
fn one_domains_rejection_does_not_reset_another_domains_pin() {
    for probe in TRACKER_PROBES {
        let tag = probe_tag(probe.field);
        let covered = covered_domains(tag);
        // Reject every OTHER table domain at once, and nothing this probe's
        // tag covers.
        let rejected: BTreeSet<u16> = DOMAIN_SCOPED_TRACKER_RETENTION
            .iter()
            .flat_map(|(_, other)| covered_domains(*other))
            .filter(|domain| !covered.contains(domain))
            .collect();
        assert!(
            !rejected.is_empty(),
            "there are other domains to reject alongside `{}`",
            probe.field
        );
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.mark)(&mut trackers);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
        assert!(
            (probe.is_marked)(&trackers),
            "`{}` survived its own bracket, so {} other rejected domains must not have \
             dropped its pin",
            probe.field,
            rejected.len()
        );
    }
}

/// The carve-outs the reset already made must keep working after the
/// narrowing, in both directions — the domain that survived and the domain
/// that did not.
///
/// What production change makes this fail: applying the carry-over AFTER the
/// unconditional carve-outs, which would let a surviving d44 tracker overwrite
/// the install booleans the caller had already saved.
#[test]
fn the_unconditional_carve_outs_still_win_over_the_allowlist() {
    for rejected in [
        BTreeSet::new(),
        [COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE]
            .into_iter()
            .collect::<BTreeSet<u16>>(),
        [UNRELATED_DOMAIN].into_iter().collect::<BTreeSet<u16>>(),
    ] {
        let mut trackers = CompactPerfGateTrackers::default();
        trackers
            .supply_logistics_attrition_live
            .attrition_hook_installed = true;
        trackers
            .supply_logistics_attrition_live
            .tooltip_hook_installed = true;
        let mut ordinals = BTreeMap::new();
        ordinals.insert((20u16, 0u16), 7u64);
        trackers.domain_gap_ordinals = ordinals.clone();

        reset_trackers_after_rejected_bracket(&mut trackers, &rejected);

        assert!(
            trackers
                .supply_logistics_attrition_live
                .attrition_hook_installed,
            "d44's install fact is carried across the reset whether or not d44 was rejected \
             (rejected = {rejected:?})"
        );
        assert!(
            trackers
                .supply_logistics_attrition_live
                .tooltip_hook_installed,
            "d44's tooltip install fact is carried across the reset (rejected = {rejected:?})"
        );
        assert_eq!(
            trackers.domain_gap_ordinals, ordinals,
            "H4DG ordinals are carried across the reset (rejected = {rejected:?})"
        );
    }
}

/// A rejected set naming every domain must reproduce the old blanket reset
/// exactly. This is the compatibility floor: whatever the allowlist says, a
/// frame in which nothing survived behaves as it did before.
///
/// What production change makes this fail: a retention predicate that ignores
/// `rejected_domains` for some field.
#[test]
fn rejecting_every_domain_is_the_old_blanket_reset() {
    let everything: BTreeSet<u16> = (0u16..=1000).collect();
    for probe in TRACKER_PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.mark)(&mut trackers);
        reset_trackers_after_rejected_bracket(&mut trackers, &everything);
        assert!(
            !(probe.is_marked)(&trackers),
            "with every domain rejected, `{}` must take the blanket reset",
            probe.field
        );
    }
}
