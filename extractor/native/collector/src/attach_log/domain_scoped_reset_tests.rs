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
        "d47 discovery cache (roots + sealed byte witness + walk families), carried across the \
         reset UNCONDITIONALLY by its own carve-out (lane MX): its validity is decided by the \
         next frame's witness replay inside that frame's bracket, never by this bracket's \
         verdict; d47 emits no deltas, so there is no staged half to roll back",
    ),
    (
        "acceptance_weather_fault_injected",
        "session-local one-shot, not a per-domain read product",
    ),
    (
        "focus_research_decisions_depth_live",
        "d45 takes PRE-FRAME RESTORATION, not the blanket reset and not the allowlist. Its pin \
         is staged inside the bracket and committed only where the late custody status is \
         final, so a committed value is by construction one the fold applied; its own rejection \
         UNDOES this frame's commits and any other domain's rejection leaves them standing. \
         `Default` is reachable from neither branch -- a wiped pin is BEHIND the fold, which \
         loses data exactly as being ahead does",
    ),
    (
        "tech_country_delta",
        "d13's wire baseline is advanced at EMISSION, after every reader has returned, so no \
         reader bracket can decide whether it survived; the blanket reset is what keeps the pin \
         and the fold's mirror in step",
    ),
    (
        "air_wings",
        "d30 is a member of the AIR-WING TRANSACTIONAL UNIT. Its pin is not retained on its own \
         reader's survival: it rolls back together with d32's and d42's whenever ANY of the \
         three was rejected, through the joint carve-out keyed on \
         `AIR_WING_TRANSACTIONAL_UNIT`, because the fold keys all three on one `AirWingKey`",
    ),
    (
        "air_wing_loadouts",
        "d32, second member of the air-wing transactional unit; same joint carve-out as d30",
    ),
    (
        "air_combat_losses_live",
        "d42, third member of the air-wing transactional unit; it mints from the same wing \
         bodies and its rows carry the same `AirWingKey`",
    ),
    (
        "air_wing_generations",
        "the air-wing unit's SHARED generation authority. Not a per-domain read product and not \
         a delta pin -- it is the body -> generation identity map all three domains key the \
         fold on, monotonic by design, and carried across the reset unconditionally by its own \
         carve-out",
    ),
    (
        "land_combat_detail_events_live",
        "d55 takes the same PRE-FRAME RESTORATION as d45: the snapshot baseline is staged and \
         committed against the same shared contract predicate the fold applies, its own \
         rejection undoes this frame's commit, and any other domain's leaves it standing",
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
    // `spawn_gated_lane_only(` is the country parent since lane KS: one permit,
    // one lane span, NO clock guard. It is listed here because the derivation
    // still has to find the call that consumes the country tracker struct --
    // the tag it carries is the parent's lane tag, and
    // `country_child_bracket_tags` resolves the second hop to the real bracket.
    for (name, tag_arg) in [
        ("spawn_gated(", 3usize),
        ("spawn_gated_lane_only(", 3usize),
        ("lane_time_main(", 1usize),
    ] {
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
    // trackers.fuel, .. }`. The whole struct moves into ONE reader call -- and
    // until lane KS that call's tag, `LW_TAG_COUNTRY_BUNDLE`, was the whole
    // answer for all eight fields. That is exactly what made a torn politics
    // child wipe the fuel baseline.
    //
    // It is a TWO-HOP resolution now, and the second hop is the point: the
    // parent call establishes only that these eight belong to the country
    // bundle, and each field's real bracket is the CHILD bracket the bundle
    // hands its local to. `country_child_bracket_tags` reads that second hop
    // out of the same source text, so a child that moves to another tag, or a
    // tracker handed to the wrong child, fails here rather than silently
    // retaining a baseline whose read was refused.
    assert!(
        spans.iter().any(|span| span.tag == "LW_TAG_COUNTRY_BUNDLE"),
        "the country bundle reader call is present"
    );
    let child_tags = country_child_bracket_tags();
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_suffix(',') {
            if let Some((lhs, rhs)) = rest.split_once(": &mut trackers.") {
                if !lhs.contains(' ') && !rhs.contains(' ') {
                    let tag = child_tags.get(rhs).unwrap_or_else(|| {
                        panic!(
                            "country tracker field `{rhs}` is handed to the country bundle but no \
                             `country_child_bracket` call in this crate takes its local"
                        )
                    });
                    derived.entry(rhs.to_string()).or_default().push(tag.clone());
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

/// The table of table entries, flattened out of the macros.
///
/// TWO macros since lane KS. The country trackers moved to
/// [`for_each_country_scoped_tracker`] because they take a THIRD answer --
/// roll back this frame's staged advance -- rather than the carry/wipe switch
/// the rest take. They are still (field, bracket tag) pairs, and this
/// derivation gate must cover them exactly as it covers the others.
fn table() -> BTreeMap<&'static str, &'static str> {
    let mut table = BTreeMap::new();
    macro_rules! collect_table {
        ($($field:ident => $tag:expr,)*) => {
            $( table.insert(stringify!($field), stringify!($tag)); )*
        };
    }
    for_each_domain_scoped_tracker!(collect_table);
    for_each_country_scoped_tracker!(collect_table);
    table
}

/// `country tracker field -> child bracket tag`, read out of the crate's own
/// source: the bundle's destructuring names the local each field is handed to,
/// and the `country_child_bracket(lanes, TAG, ..)` call that encloses that
/// local names the bracket.
///
/// What production change makes this fail: moving a child to another tag,
/// handing a child the wrong tracker local, or dropping a child's bracket --
/// each of which puts a tracker back under a verdict its own reader did not
/// take.
fn country_child_bracket_tags() -> BTreeMap<String, String> {
    let source = attach_log_source();
    let destructure_start = source
        .find("let CompactCountryGateTrackers {")
        .expect("the parallel bundle destructures its tracker set");
    let destructure_end = source[destructure_start..]
        .find("} = trackers;")
        .expect("the destructuring is closed")
        + destructure_start;
    let mut local_of_field: BTreeMap<String, String> = BTreeMap::new();
    for line in source[destructure_start..destructure_end].lines() {
        let trimmed = line.trim().trim_end_matches(',');
        let Some((field, local)) = trimmed.split_once(": ") else {
            continue;
        };
        if field.contains(' ') || local.contains(' ') {
            continue;
        }
        local_of_field.insert(field.to_string(), local.to_string());
    }
    assert_eq!(
        local_of_field.len(),
        8,
        "the country bundle destructures eight trackers, parsed {local_of_field:?}"
    );

    let mut tag_of_local: BTreeMap<String, String> = BTreeMap::new();
    let needle = "country_child_bracket(";
    let mut search = 0usize;
    let mut calls = 0usize;
    while let Some(at) = source[search..].find(needle) {
        let open = search + at + needle.len() - 1;
        search = open + 1;
        // Skip the definition itself.
        if source[..open].trim_end().ends_with("fn country_child_bracket") {
            continue;
        }
        let Some(close) = matching_paren(&source, open) else {
            continue;
        };
        let call = &source[open + 1..close];
        let mut args = call.splitn(3, ',');
        let _lanes = args.next().unwrap_or("").trim();
        let Some(tag) = args.next().map(str::trim) else {
            continue;
        };
        let call_body = args.next().unwrap_or("");
        calls += 1;
        for local in local_of_field.values() {
            let hit = call_body
                .lines()
                .any(|line| line.trim().trim_end_matches(',') == local.as_str());
            if hit {
                // NO INDIRECTION LEFT TO RESOLVE. There used to be exactly one:
                // the focus+tech child's tag was a function of its roster
                // (`country_focus_tech_bracket_tag`: 12 alone, 13 alone, or the
                // pair), so its call site passed a LOCAL rather than a constant
                // and this had to resolve it. Lane LE split that child in two,
                // and each half names its own domain id as a constant, so every
                // one of the eight call sites now passes a constant directly.
                tag_of_local.insert(local.clone(), tag.to_string());
            }
        }
    }
    assert_eq!(
        calls, 8,
        "each of the eight country children holds its own bracket, found {calls}"
    );

    local_of_field
        .into_iter()
        .map(|(field, local)| {
            let tag = tag_of_local.get(&local).unwrap_or_else(|| {
                panic!("no `country_child_bracket` call takes the country tracker local `{local}`")
            });
            (field, tag.clone())
        })
        .collect()
}

/// Byte index of the `)` closing the `(` at `open`.
fn matching_paren(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
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
        DOMAIN_SCOPED_TRACKER_RETENTION.len() + COUNTRY_SCOPED_TRACKER_RETENTION.len(),
        "the table macros and the flattened constants must describe the same list"
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

/// The division root cache must remain excluded for the stated reason: it is
/// touched somewhere no single reader bracket covers, so no one tag can decide
/// whether it survived.
///
/// What production change makes this fail: making the division root cache a
/// per-reader product, which would make the field eligible and this exclusion
/// stale.
#[test]
fn the_division_root_cache_is_touched_outside_every_reader_bracket() {
    let source = attach_log_source();
    let body = frame_builder_body(&source);
    let derived = derive_field_tags_from_source(body);

    let field = "division_roots";
    let tags = derived
        .get(field)
        .unwrap_or_else(|| panic!("{field} is bound in the frame builder"));
    assert!(
        tags.iter()
            .any(|tag| tag.starts_with("OUTSIDE-EVERY-READER-CALL")),
        "{field} is excluded from the domain-scoped allowlist because it is written          outside every reader bracket; the source now says its uses are {tags:?}, so the          exclusion needs re-deciding rather than keeping"
    );
}

/// d47's discovery cache is excluded from the allowlist for a DIFFERENT reason
/// (lane MX): it is carried across the reset UNCONDITIONALLY by its own
/// carve-out, whether or not its bracket survived, because its validity is
/// decided by the next frame's witness replay and not by this bracket's
/// verdict. Its discovery has run inside the worker's bracket since lane HT,
/// so "written outside every reader bracket" stopped being the reason then.
///
/// What production change makes this fail: putting the field on the
/// domain-scoped allowlist (a claim that bracket survival decides it), or
/// removing the carve-out (the field would then be neither carried nor
/// classified as such — `d47_cache_survives_refusal_tests` holds the runtime
/// half of that).
#[test]
fn the_d47_discovery_cache_is_an_unconditional_carve_out_not_a_bracket_scoped_pin() {
    let field = "characters_leaders_cabinet_live";
    assert!(
        !DOMAIN_SCOPED_TRACKER_RETENTION
            .iter()
            .any(|(name, _)| *name == field),
        "`{field}` must stay off the bracket-scoped allowlist: the witness replay, not the          bracket verdict, decides it"
    );
    let (_, reason) = BLANKET_RESET_TRACKER_FIELDS
        .iter()
        .find(|(name, _)| *name == field)
        .expect("d47's cache carries its classification reason");
    assert!(
        reason.contains("UNCONDITIONALLY") && reason.contains("carve-out"),
        "the classification must state the unconditional carve-out: {reason}"
    );
    let source = attach_log_source();
    let reset_start = source
        .find("fn reset_trackers_after_rejected_bracket(")
        .expect("the reset function is declared");
    let reset_body = &source[reset_start..reset_start + source[reset_start..].find("
}
").expect("closed")];
    assert!(
        reset_body.contains("std::mem::take(&mut trackers.characters_leaders_cabinet_live)")
            && reset_body.contains("trackers.characters_leaders_cabinet_live = d47_discovery_cache;"),
        "the reset must take the d47 cache before the blanket `fresh` and reinstall it after"
    );
}

/// d45 and d55 are held OUT of the allowlist deliberately, and re-adding either
/// must be a decision rather than an edit.
///
/// Both survive their clock bracket and can still acquire a failure-class status
/// afterwards, from hook-reservation custody settled on the main thread. Bracket
/// survival — which is the entire retention predicate — does not decide that
/// question for these two, so the allowlist cannot express what they need.
///
/// What they take instead is PRE-FRAME RESTORATION, not the blanket reset: their
/// pins are staged during the read, committed at emission under the shared
/// `compact_delta_pin_may_advance` predicate, and their OWN rejection undoes
/// this frame's commits rather than wiping the tracker. `Default` is the wrong
/// answer for them in exactly the way an advanced pin is —
/// `no_transactional_domain_is_default_constructed_on_its_own_rejection` holds
/// that half. This test holds the allowlist half.
///
/// What production change makes this fail: putting either field back in the
/// allowlist. That would be a claim about what the fold does with a RETAINED pin
/// across a rejected frame, which is a different question from the one the
/// transaction answers and owes its own fold-equivalence proof.
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
            "`{field}` must stay off the allowlist: bracket survival cannot decide whether the \
             hook-custody status that settles after the reader's bracket closed badged its \
             record failure-class. Pre-frame restoration is what it takes instead"
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
        .chain(COUNTRY_SCOPED_TRACKER_RETENTION.iter())
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

/// A country tracker seen from BOTH sides of its transaction.
///
/// The other probes are one-sided because the other trackers are: the reset
/// either keeps the field or default-constructs it. A country tracker has a
/// committed half the reset may never touch and a staged half it must drop on
/// that child's own rejection, so a one-sided probe cannot tell the new
/// behaviour from either of the two old ones.
struct CountryTrackerProbe {
    field: &'static str,
    /// The COMMITTED baseline: what the consumer already holds.
    committed: fn(&mut CompactPerfGateTrackers),
    is_committed: fn(&CompactPerfGateTrackers) -> bool,
    /// THIS FRAME'S staged advance: what the consumer has not received yet.
    staged: fn(&mut CompactPerfGateTrackers),
    is_staged: fn(&CompactPerfGateTrackers) -> bool,
}

macro_rules! country_tracker_probes {
    ($($field:ident => $tag:expr,)*) => {
        const COUNTRY_TRACKER_PROBES: &[CountryTrackerProbe] = &[
            $(CountryTrackerProbe {
                field: stringify!($field),
                committed: |trackers| {
                    trackers
                        .$field
                        .previous_rows
                        .insert("committed".to_string(), "value".to_string());
                },
                is_committed: |trackers| !trackers.$field.previous_rows.is_empty(),
                staged: |trackers| {
                    trackers
                        .$field
                        .staged
                        .previous_rows
                        .insert("staged".to_string(), "value".to_string());
                },
                is_staged: |trackers| trackers.$field.has_staged_advance(),
            },)*
        ];
    };
}
for_each_country_scoped_tracker!(country_tracker_probes);

fn country_probe_tag(field: &str) -> u16 {
    COUNTRY_SCOPED_TRACKER_RETENTION
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, tag)| *tag)
        .unwrap_or_else(|| panic!("`{field}` is a country-scoped tracker"))
}

const TRACKER_PROBES: &[TrackerProbe] = &[
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

/// THE COLD-START LOOP, STATED AS A TEST: a rejected country child drops only
/// what this frame staged, and the committed baseline the consumer already
/// holds survives.
///
/// This is the whole of deliverable (a) at the reset. Before it, every one of
/// the eight country fields named `LW_TAG_COUNTRY_BUNDLE`, that tag expanded to
/// 10..=17, and so ANY country domain's rejection default-constructed ALL EIGHT
/// baselines -- which made the next read a full 439-country deep read of every
/// domain (233 ms/frame against a 76 ms hour on t55), which guaranteed the next
/// rejection.
///
/// What production change makes this fail: default-constructing a country
/// tracker on rejection (the old wipe); or NOT dropping the staged half, which
/// would let a baseline advance past records that became a `DOMAIN_GAP` and
/// leave the producer permanently ahead of the fold.
#[test]
fn a_rejected_country_child_drops_its_staged_advance_and_keeps_its_baseline() {
    for probe in COUNTRY_TRACKER_PROBES {
        for domain in covered_domains(country_probe_tag(probe.field)) {
            let mut trackers = CompactPerfGateTrackers::default();
            (probe.committed)(&mut trackers);
            (probe.staged)(&mut trackers);
            assert!((probe.is_committed)(&trackers) && (probe.is_staged)(&trackers));
            let rejected: BTreeSet<u16> = [domain].into_iter().collect();
            reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
            assert!(
                (probe.is_committed)(&trackers),
                "domain {domain} was rejected, but `{}`'s COMMITTED baseline is what the fold \
                 holds and no rejection may reach it",
                probe.field
            );
            assert!(
                !(probe.is_staged)(&trackers),
                "domain {domain} was rejected, so `{}`'s staged advance describes records that \
                 became a DOMAIN_GAP and must be dropped",
                probe.field
            );
        }
    }
}

/// A country child that survived keeps BOTH halves: its staged advance is
/// settled later, at the frame's publish/refuse verdict, not here.
///
/// What production change makes this fail: rolling back on a sibling's
/// rejection -- which is what the shared tag-200 coverage did -- or committing
/// here, where no verdict on the frame exists yet.
#[test]
fn a_surviving_country_child_keeps_its_staged_advance_for_the_frame_verdict() {
    for probe in COUNTRY_TRACKER_PROBES {
        let covered = covered_domains(country_probe_tag(probe.field));
        // Reject every country domain this child does NOT read, plus an
        // unrelated one.
        let mut rejected: BTreeSet<u16> = (10u16..=17)
            .filter(|domain| !covered.contains(domain))
            .collect();
        rejected.insert(UNRELATED_DOMAIN);
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.committed)(&mut trackers);
        (probe.staged)(&mut trackers);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
        assert!(
            (probe.is_committed)(&trackers),
            "`{}` survived its own bracket; its baseline must stand",
            probe.field
        );
        assert!(
            (probe.is_staged)(&trackers),
            "`{}` survived its own bracket, so its staged advance is still live and is \
             settled by the frame verdict, not by a sibling's tear",
            probe.field
        );
    }
}

/// Even with EVERY domain rejected a country baseline stands. The blanket reset
/// is deliberately not the floor for these eight any more.
///
/// What production change makes this fail: restoring the wipe for the country
/// fields, which is what put every capture to date in permanent cold start.
#[test]
fn rejecting_every_domain_still_leaves_the_country_baselines_standing() {
    let everything: BTreeSet<u16> = (0u16..=1000).collect();
    for probe in COUNTRY_TRACKER_PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.committed)(&mut trackers);
        (probe.staged)(&mut trackers);
        reset_trackers_after_rejected_bracket(&mut trackers, &everything);
        assert!(
            (probe.is_committed)(&trackers),
            "`{}`'s committed baseline is the consumer's own state; rejecting everything \
             cannot unpublish it",
            probe.field
        );
        assert!(
            !(probe.is_staged)(&trackers),
            "`{}` was rejected, so its staged advance must be dropped",
            probe.field
        );
    }
}

/// A frame refused WHOLE returns the country baselines to the published slot
/// with their staged advance dropped.
///
/// What production change makes this fail: restoring `drop(pending_trackers)`
/// with nothing salvaged (960 recurring wipes on t55 alone), or salvaging the
/// tracker WITHOUT rolling back -- which would claim rows the refusal withheld.
#[test]
fn a_refused_frame_returns_the_country_baselines_and_drops_the_candidate() {
    for probe in COUNTRY_TRACKER_PROBES {
        let mut refused = CompactPerfGateTrackers::default();
        (probe.committed)(&mut refused);
        (probe.staged)(&mut refused);
        let mut published = CompactPerfGateTrackers::default();
        restore_country_trackers_after_refused_frame(&mut refused, &mut published);
        assert!(
            (probe.is_committed)(&published),
            "`{}`'s baseline must come back from a refused frame; `Default` is BEHIND the \
             fold, which loses data exactly as being ahead of it does",
            probe.field
        );
        assert!(
            !(probe.is_staged)(&published),
            "a refused frame published no DATA record, so `{}`'s staged advance must not \
             become a baseline",
            probe.field
        );
    }
}

/// A published frame commits the staged advance -- unless that domain was
/// published DEGRADED, in which case the consumer may discard the record and
/// the advance rolls back instead.
///
/// What production change makes this fail: committing unconditionally (a
/// degraded d11/d12/d13 record is discarded whole by the fold, so its advance
/// would leave the producer ahead of it), or never committing (the baseline
/// would never warm and the cold start would survive the whole change).
#[test]
fn a_published_frame_commits_unless_the_domain_was_degraded() {
    for probe in COUNTRY_TRACKER_PROBES {
        let covered = covered_domains(country_probe_tag(probe.field));

        let mut clean = CompactPerfGateTrackers::default();
        (probe.staged)(&mut clean);
        commit_country_staged_advances(&mut clean, &BTreeSet::new());
        assert!(
            (probe.is_committed)(&clean),
            "`{}` published cleanly, so its staged advance becomes the baseline",
            probe.field
        );
        assert!(!(probe.is_staged)(&clean), "commit drains the staged half");

        for domain in covered {
            let mut degraded = CompactPerfGateTrackers::default();
            (probe.staged)(&mut degraded);
            let withheld: BTreeSet<u16> = [domain].into_iter().collect();
            commit_country_staged_advances(&mut degraded, &withheld);
            assert!(
                !(probe.is_committed)(&degraded),
                "domain {domain} was published degraded, so `{}` must not advance past a \
                 record the consumer may discard",
                probe.field
            );
            assert!(
                !(probe.is_staged)(&degraded),
                "a rolled-back candidate is dropped, never carried into the next frame"
            );
        }
    }
}

/// A parent panic synthesizes brackets ONLY for the children this frame
/// scheduled, and never over a child that already recorded one.
///
/// The country parent holds no clock bracket, so its panic -- visible only at
/// the join -- would otherwise leave its domains unbracketed and
/// `replace_rejected_domain_records_with_gaps` would call that
/// `ClockUnreadable` rather than `WorkerPanic`. Both bounds matter and neither
/// is data loss: synthesizing over a child that finished doubles the bracket
/// count and lets a fabricated outcome outrank a recorded one, and
/// synthesizing for an unscheduled child prices domain-hours the frame never
/// attempted (`lw_bracketed_domains` and the mismatch instrument are both
/// counted in domain-hours).
///
/// What production change makes this fail: dropping the `scheduled` filter;
/// dropping the already-covered check; or going back to one bracket over all
/// of 10..=17.
#[test]
fn a_parent_panic_brackets_only_the_scheduled_and_uncovered_children() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    // A decisions-and-fuel roster where fuel already closed its own bracket.
    let scheduled: BTreeSet<u16> = [LW_TAG_COUNTRY_FUEL, LW_TAG_COUNTRY_DECISIONS]
        .into_iter()
        .collect();
    lanes.record_clock_bracket(LW_TAG_COUNTRY_FUEL, None, None, false);
    lanes.record_worker_panicked_country_brackets(&scheduled);

    let brackets = lanes.take_clock_brackets();
    let synthesized: Vec<u16> = brackets
        .iter()
        .filter(|bracket| bracket.worker_panicked)
        .map(|bracket| bracket.tag)
        .collect();
    assert_eq!(
        synthesized,
        vec![LW_TAG_COUNTRY_DECISIONS],
        "only the scheduled child that left no bracket may be synthesized; fuel already          recorded one and the five unscheduled children attempted nothing"
    );
    let mut covered = Vec::new();
    for bracket in &brackets {
        for_each_bracket_covered_domain(bracket.tag, |domain| covered.push(domain));
    }
    covered.sort_unstable();
    assert_eq!(
        covered,
        vec![COMPACT_DOMAIN_FUEL, COMPACT_DOMAIN_DECISIONS],
        "no domain may be covered twice, and none the frame never scheduled at all"
    );
}

/// The same bound, on the two children whose bracket tag IS a bare domain id.
///
/// Focus and tech are the only country children not tagged with a 20x constant:
/// lane LE split them and gave each its own domain id, 12 and 13, which
/// `for_each_bracket_covered_domain` resolves through its DEFAULT arm rather
/// than a named one. Before that split those values were reachable only on a
/// one-spec roster; on the production roster the child was tag 203 and covered
/// both. So this is the first time a real capture can synthesize a panic
/// bracket over a bare domain id, and it is the case the roster in the test
/// above cannot reach.
///
/// The claim under test is that d13 tearing tells you nothing about d12: a
/// parent panic after focus has closed its own bracket synthesizes for tech
/// ALONE, and the covered set is exactly {12, 13} with neither doubled.
///
/// What production change makes this fail:
///  * Recombining focus and tech under one 203 tag, i.e. taking 12 and 13 out
///    of `COUNTRY_CHILD_BRACKET_TAGS`. The synthesized list goes EMPTY, not to
///    `[203]` — this fixture's scheduled set is `{12, 13}`, and the synthesiser
///    only ever brackets a tag that set contains, so 203 is skipped rather than
///    emitted.
///  * Breaking the `own => visit(own)` default arm, so d12 is no longer seen as
///    covered by the bracket it recorded: BOTH 12 and 13 are then synthesized.
///  * Dropping the already-covered check: focus is bracketed twice, and a
///    fabricated WorkerPanic outranks the clean bracket it recorded.
#[test]
fn a_parent_panic_over_the_split_focus_and_tech_children_brackets_tech_alone() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let scheduled: BTreeSet<u16> = [COMPACT_DOMAIN_FOCUS, COMPACT_DOMAIN_TECH]
        .into_iter()
        .collect();
    // Focus closed cleanly; the technology census is the child that did not.
    lanes.record_clock_bracket(COMPACT_DOMAIN_FOCUS, None, None, false);
    lanes.record_worker_panicked_country_brackets(&scheduled);

    let brackets = lanes.take_clock_brackets();
    let synthesized: Vec<u16> = brackets
        .iter()
        .filter(|bracket| bracket.worker_panicked)
        .map(|bracket| bracket.tag)
        .collect();
    assert_eq!(
        synthesized,
        vec![COMPACT_DOMAIN_TECH],
        "d12 recorded its own bracket, so only d13 may be synthesized -- the whole          point of splitting them is that one cannot speak for the other"
    );
    let mut covered = Vec::new();
    for bracket in &brackets {
        for_each_bracket_covered_domain(bracket.tag, |domain| covered.push(domain));
    }
    covered.sort_unstable();
    assert_eq!(
        covered,
        vec![COMPACT_DOMAIN_FOCUS, COMPACT_DOMAIN_TECH],
        "a bare domain id must cover exactly itself through the default arm"
    );
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
