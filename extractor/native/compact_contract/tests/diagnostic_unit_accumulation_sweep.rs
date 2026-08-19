//! The sweep parses the sources rather than importing a generated table for the
//! the gap between them is one capture, not one release.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::diagnostic_channel::{
    declared_unit_accumulation, unit_accumulation, Accumulation, CUMULATIVE_UNITS,
    DOMAIN47_PREWARM_UNITS, STATEFUL_UNIT_PREFIXES,
};

fn native_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compact_contract sits inside extractor/native")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = native_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn snake_case_literals(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'"' && bytes[end] != b'\n' {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'"' {
            let literal = &text[start..end];
            if !literal.is_empty()
                && literal
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                out.insert(literal.to_string());
            }
            index = end + 1;
        } else {
            index = end;
        }
    }
    out
}

/// Directories every one of whose `.rs` files is scanned.
///
/// `lanes/` is here because a lane crate can build its OWN `data_units_total`
/// map, which the host then merges wholesale — `domain53_*` and `domain47_*`
/// reach the manifest that way and never appear in the three files below. A
/// sweep that read only the host would have let a new unclassified unit into a
/// lane and still passed.
const PRODUCER_DIRS: &[&str] = &[
    "lanes",
    "collector/src",
    "recorder_host/src",
    // `bound_trips.rs` and `status_origins.rs` hold the `bt_*` and `so_*` name
    // tables, so the contract crate is a producer of unit names too.
    "compact_contract/src",
];

/// Individual files outside those trees that construct unit names.
const PRODUCER_SOURCES: &[&str] = &[
    "project_progress_live_lane/src",
    "scientists_live_lane/src",
    "wire_format/src",
    "compact_stream/src",
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Build outputs, not sources.
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn producer_files() -> Vec<PathBuf> {
    let root = native_root();
    let mut files = Vec::new();
    for dir in PRODUCER_DIRS.iter().chain(PRODUCER_SOURCES) {
        collect_rs_files(&root.join(dir), &mut files);
    }
    files.sort();
    files
}

fn producer_literals() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in producer_files() {
        if let Ok(text) = fs::read_to_string(&path) {
            names.extend(snake_case_literals(&text));
        }
    }
    names
}

/// Call sites that put a name on the diagnostic channel.
///
/// A literal is treated as a unit name if it appears as a `name:` field of a
/// `CompactDomainPerfDiagnosticUnit`, or as an argument to one of the host's
/// recording entry points, or as a key written into a lane's own
/// `data_units_total` / `problems` map.
///
/// This is the precise half of the sweep. The over-collecting half
/// ([`snake_case_literals`]) supplies the CANDIDATES; this supplies the proof
/// that a candidate really is a unit, so an exclusion cannot be used to hide
/// one.
const UNIT_CONSTRUCTION_CALLS: &[&str] = &[
    "add_compact_domain_data_unit(",
    "add_compact_domain_cumulative_unit(",
    "add_compact_domain_problem(",
    "add_compact_domain_perf_diagnostic_unit(",
    "data_units_total.entry(",
    "data_units_total.insert(",
    "problems.entry(",
    "problems.insert(",
    "fold_named_unit(",
];

/// Could this literal be a diagnostic unit name at all?
fn looks_like_a_unit_name(literal: &str) -> bool {
    !literal.is_empty()
        && literal
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Every literal that is CONSTRUCTED as a unit name somewhere in `text`.
fn constructed_unit_names(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // `const NAME: &str = "literal";` where NAME is later handed to a
    // construction call. The binding is where the literal becomes a name, and
    // the use can be arbitrarily far away -- so a window around the call site
    // would never see the literal, and a name could be introduced without this
    // sweep noticing. Association is required, or every domain wire-name
    // constant in the tree would be treated as a unit.
    let mut bindings: Vec<(String, String)> = Vec::new();
    for keyword in ["const ", "static "] {
        let mut rest = text;
        while let Some(at) = rest.find(keyword) {
            rest = &rest[at + keyword.len()..];
            let Some(line_end) = rest.find('\n') else {
                break;
            };
            let line = &rest[..line_end];
            if !line.contains("&str") {
                continue;
            }
            let Some(colon) = line.find(':') else {
                continue;
            };
            let ident = line[..colon].trim();
            if ident.is_empty()
                || !ident
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                continue;
            }
            let Some(open) = line.find('"') else { continue };
            let body = &line[open + 1..];
            let Some(end) = body.find('"') else { continue };
            if looks_like_a_unit_name(&body[..end]) {
                bindings.push((ident.to_string(), body[..end].to_string()));
            }
        }
    }
    for (ident, literal) in bindings {
        for call in UNIT_CONSTRUCTION_CALLS {
            let mut rest = text;
            let mut used = false;
            while let Some(at) = rest.find(call) {
                rest = &rest[at + call.len()..];
                let window = match rest.find(");") {
                    Some(close) => &rest[..close],
                    None => &rest[..rest.len().min(400)],
                };
                if window.contains(ident.as_str()) {
                    used = true;
                    break;
                }
            }
            if used {
                out.insert(literal.clone());
                break;
            }
        }
    }
    // `name: "..."` — the collector's diagnostic-unit struct literal.
    let mut rest = text;
    while let Some(at) = rest.find("name:") {
        rest = &rest[at + 5..];
        let trimmed = rest.trim_start();
        if let Some(body) = trimmed.strip_prefix('"') {
            if let Some(end) = body.find('"') {
                if looks_like_a_unit_name(&body[..end]) {
                    out.insert(body[..end].to_string());
                }
            }
        }
    }
    // `("literal", value)` — the (name, value) tuple form, which lane crates
    // and the domain-90 block use to build a table that is then iterated into a
    // recording call. Requiring the trailing comma keeps this off ordinary
    // single-argument calls like `compact_domain_id("air_loss_stats_live")`.
    // rustfmt routinely breaks the tuple across lines, so whitespace after the
    // paren is skipped rather than required to be absent.
    let mut rest = text;
    while let Some(at) = rest.find('(') {
        rest = &rest[at + 1..];
        let trimmed = rest.trim_start();
        let Some(body) = trimmed.strip_prefix('"') else {
            continue;
        };
        let Some(end) = body.find('"') else { break };
        if body[end + 1..].starts_with(',') && looks_like_a_unit_name(&body[..end]) {
            out.insert(body[..end].to_string());
        }
    }
    // A string literal in the argument list of a recording call. The window is
    // generous because these calls are routinely formatted across five lines.
    for call in UNIT_CONSTRUCTION_CALLS {
        let mut rest = text;
        while let Some(at) = rest.find(call) {
            rest = &rest[at + call.len()..];
            let window = &rest[..rest.len().min(400)];
            let mut scan = window;
            while let Some(open) = scan.find('"') {
                let body = &scan[open + 1..];
                let Some(end) = body.find('"') else { break };
                if looks_like_a_unit_name(&body[..end]) {
                    out.insert(body[..end].to_string());
                }
                scan = &body[end + 1..];
            }
        }
    }
    out
}

fn constructed_unit_names_everywhere() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in producer_files() {
        if let Ok(text) = fs::read_to_string(&path) {
            out.extend(constructed_unit_names(&text));
        }
    }
    out
}

/// The precise half must see the units the over-collecting half sees, or the
/// exclusion proof would be checking against an empty set and approve anything.
///
/// MUTATION: empty `UNIT_CONSTRUCTION_CALLS` and both this and the exclusion
/// proof fail.
#[test]
fn the_construction_scan_finds_the_units_it_is_meant_to_protect() {
    let constructed = constructed_unit_names_everywhere();
    for expected in [
        // `name:` struct-literal form, in the collector.
        "rs_observations",
        "d75_queue_high_water_mark",
        "heap_sweep_generation",
        // positional-argument form, in the host.
        "convoy_transport_attack_callbacks_seen",
        "fired_event_popups_rows",
        "sunk_ship_writer_events",
        "air_loss_stat_rows",
    ] {
        assert!(
            constructed.contains(expected),
            "{expected} is a real unit but the construction scan did not see it, \
             so it could be silenced by an exclusion"
        );
    }
}

/// No name a producer CONSTRUCTS as a diagnostic unit may be undeclared.
///
/// The candidate set is the set of constructed names, not every snake-case
/// literal in the tree. An earlier version over-collected and carried an
/// exclusion list to cancel the false positives — but an exclusion list is a
/// way to silence a name, and a name can be made into a real unit later
/// (bound to a `const`, then used far from its definition) while its exclusion
/// stays. The list is gone; there is nothing left to abuse.
///
/// `const NAME: &str = "..."` bindings are treated as construction sites for
/// exactly that reason: the binding is where the literal becomes a name, and
/// the use may be anywhere.
#[test]
fn no_producer_unit_name_is_left_undeclared() {
    let undeclared: Vec<String> = constructed_unit_names_everywhere()
        .into_iter()
        .filter(|name| unit_accumulation(name) == Accumulation::Undeclared)
        .collect();
    assert!(
        undeclared.is_empty(),
        "these names sit inside a stateful producer family ({:?}) but no table in \
         compact_contract::diagnostic_channel says whether their value is this \
         frame's quantity or the producer's running state. Classify each one \
         before it reaches a manifest -- a capture containing an undeclared unit \
         is refused by the promotion gate.\n{}",
        STATEFUL_UNIT_PREFIXES,
        undeclared.join("\n")
    );
}

/// Every `domain47_prewarm_*` unit a producer emits must be CLASSIFIED, not
/// merely summed by default.
///
/// The general rule of this file is "no producer name is `Undeclared`", and
/// `Undeclared` only exists inside a stateful family. Outside one, an
/// unclassified name silently defaults to `PerFrame` and is SUMMED — which is
/// how `domain47_prewarm_clock_verdict` (an enum code) and
/// `domain47_prewarm_known_families` (a `FAMILY_*` bitmask) reached t36's
/// manifest as 219 and 20469, two figures that cannot be read as anything.
///
/// This family is held to the stricter rule because its names carry three
/// different value kinds — counts, codes, masks — and none of them says which.
/// The stricter rule is MEMBERSHIP in `DOMAIN47_PREWARM_UNITS`, not merely
/// "some rule classifies it". The weaker spelling would have let an entry be
/// deleted and pass: `domain47_prewarm_hold_reason` still resolves through the
/// `_reason` suffix, and it is exactly that suffix — which took the
/// `families_dark_without_reason` MASK for a raw word — that this table is
/// there to take precedence over. The whole family declared in one place is the
/// property; "declared somewhere" is not it.
///
/// The candidate set is read from the SOURCES rather than from a list kept
/// here, so a unit added to the collector tomorrow is covered without anybody
/// remembering to widen this test. It comes from the OVER-COLLECTING half
/// ([`producer_literals`]) rather than the precise half, because the precise
/// half only recognises a literal `name: "..."` field and a `name: SOME_CONST`
/// emit site would walk straight past it. Over-collecting is the safe
/// direction here for the reason stated at the top of this file: the worst a
/// non-unit literal in this family's namespace can do is demand a table entry
/// for a name nothing emits, which is noticed at once. Under-collecting would
/// let a real unit through, which is the whole failure mode.
///
/// Aliveness is still proved by the PRECISE half: an entry in the table must be
/// constructed by a producer, so a retired or misspelled entry is reported
/// rather than left as a class nobody can check.
///
/// Scope: this family only. The same demand across every family would mean
/// classifying ~460 domain data units whose per-frame sum is exactly right, and
/// the default exists for them.
///
/// MUTATION: delete ANY `DOMAIN47_PREWARM_UNITS` entry — including one the
/// suffix rules would still classify — and this names it.
#[test]
fn every_domain47_prewarm_unit_is_explicitly_classified() {
    // The bare prefix is a literal in this crate's own guards, not a unit.
    const PREFIX: &str = "domain47_prewarm_";
    let named_in_sources: Vec<String> = producer_literals()
        .into_iter()
        .filter(|name| name.starts_with(PREFIX) && name.len() > PREFIX.len())
        .collect();
    // The scan must actually reach the sources, or an empty candidate set would
    // make this pass by finding nothing.
    assert!(
        named_in_sources.len() >= 25,
        "only {} domain47_prewarm_* names found in the producer sources; the scan is \
         not reading collector/src/attach_log.rs: {named_in_sources:?}",
        named_in_sources.len()
    );
    for pinned in [
        "domain47_prewarm_clock_verdict",
        "domain47_prewarm_known_families",
        "domain47_prewarm_families_dark_without_reason",
        "domain47_prewarm_shadow_walk_state",
    ] {
        assert!(
            named_in_sources.iter().any(|name| name == pinned),
            "{pinned} is emitted by the collector but the scan did not see it, so this \
             guard would not have caught its misclassification"
        );
    }

    let untabled: Vec<&String> = named_in_sources
        .iter()
        .filter(|name| !DOMAIN47_PREWARM_UNITS.iter().any(|(unit, _)| unit == name))
        .collect();
    assert!(
        untabled.is_empty(),
        "these d47 prewarm names are not in DOMAIN47_PREWARM_UNITS, so this family's \
         class is decided somewhere else -- by a suffix that may not know what the \
         value is, or by the PerFrame default, which SUMS it. A count may well be \
         right: say so in the table. A code or a mask summed is a figure with no \
         referent, which is what t36 published for two of them:\n{}",
        untabled
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    // Membership implies a declaration, but state it: this is the property the
    // table exists to give, and the two are separate mechanisms.
    for (name, class) in DOMAIN47_PREWARM_UNITS {
        assert_eq!(
            declared_unit_accumulation(name),
            Some(*class),
            "{name} is in the table but does not resolve to it"
        );
    }

    // The table must not drift into naming units nothing emits: a dead entry is
    // a class nobody can check, and a MISSPELLED entry is a dead entry whose
    // real unit is back on the default. Proved against the PRECISE scan, which
    // is what shows a name is really constructed as a unit.
    let constructed = constructed_unit_names_everywhere();
    let dead: Vec<&str> = DOMAIN47_PREWARM_UNITS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !constructed.contains(*name))
        .collect();
    assert!(
        dead.is_empty(),
        "declared in DOMAIN47_PREWARM_UNITS but no producer constructs them -- retired, \
         or misspelled: {dead:?}"
    );
}

/// A negative control for the guard above: a NEW unit in the family, of the
/// kind that keeps being added, must be reported rather than summed.
///
/// Without this, deleting the guard's body would look the same as a clean run.
#[test]
fn a_new_domain47_prewarm_unit_would_be_reported() {
    assert_eq!(
        declared_unit_accumulation("domain47_prewarm_a_unit_added_next_quarter"),
        None,
        "an unclassified d47 prewarm name must be visible as unclassified"
    );
    assert_eq!(
        unit_accumulation("domain47_prewarm_a_unit_added_next_quarter"),
        Accumulation::PerFrame,
        "and it must still FOLD as it always did -- the guard fails the build, it does \
         not change an unclassified unit's arithmetic"
    );
}

/// The sweep must be able to SEE the producers, or it would pass by finding
/// nothing. Pins the two families whose misclassification was measured.
///
/// MUTATION: point `PRODUCER_SOURCES` at a file that does not exist, or break
/// `snake_case_literals`, and this fails instead of the sweep silently passing.
#[test]
fn the_sweep_actually_reaches_the_producers() {
    let names = producer_literals();
    assert!(
        names.len() > 500,
        "only {} literals found; the sweep is not reading the producer sources",
        names.len()
    );
    for expected in [
        "convoy_transport_attack_callbacks_seen",
        "rs_observations",
        "fired_event_popups_rows",
        "sunk_ship_writer_events",
        "d75_queue_high_water_mark",
        "heap_sweep_generation",
    ] {
        assert!(
            names.contains(expected),
            "{expected} is emitted by a producer but the sweep did not see it"
        );
        assert_ne!(unit_accumulation(expected), Accumulation::PerFrame);
    }
}

/// Every literal name passed to a recording entry point must match that entry
/// point's class.
///
/// `add_compact_domain_data_unit` and `add_compact_domain_cumulative_unit` each
/// carry a `debug_assert`, but an assert only fires on a path a test actually
/// executes. Domain 75 and domain 86 shipped a mismatch that panicked any
/// debug-build decode of a valid payload and no test reached either call site.
/// This checks EVERY call site by reading them, so coverage of the decoders is
/// not what the property depends on.
///
/// MUTATION: route a cumulative unit back through the per-frame entry point and
/// this fails without needing a decoder test for that domain.
#[test]
fn every_call_site_uses_the_entry_point_matching_its_class() {
    // Production code only. The file's own `#[cfg(test)]` module deliberately
    // calls the wrong entry point to prove the debug_assert fires, and that
    // call is not a production call site.
    let whole = read("recorder_host/src/domain_data_authority.rs");
    let at = whole
        .find("#[cfg(test)]")
        .expect("domain_data_authority.rs no longer has a test module to exclude");
    let text = whole[..at].to_string();
    let mut checked = 0usize;
    for (call, allowed) in [
        (
            "add_compact_domain_data_unit(",
            &[
                Accumulation::PerFrame,
                Accumulation::LastValue,
                Accumulation::BitwiseOr,
            ][..],
        ),
        (
            "add_compact_domain_cumulative_unit(",
            &[
                Accumulation::Cumulative,
                Accumulation::Gauge,
                Accumulation::LastValue,
            ][..],
        ),
    ] {
        let mut rest = text.as_str();
        while let Some(at) = rest.find(call) {
            rest = &rest[at + call.len()..];
            // Bound the window at THIS call's closing `);`, or a name passed as
            // a variable would let the scan run on and attribute the NEXT
            // call's literal to this one.
            let window = match rest.find(");") {
                Some(close) => &rest[..close],
                None => &rest[..rest.len().min(400)],
            };
            // The name is the first string literal in the argument list.
            let Some(open) = window.find('"') else {
                continue;
            };
            let body = &window[open + 1..];
            let Some(end) = body.find('"') else { continue };
            let name = &body[..end];
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                continue;
            }
            let class = unit_accumulation(name);
            assert!(
                allowed.contains(&class),
                "{name} is {class:?} but is recorded through {call} -- a debug build \
                 panics on the first valid payload that reaches this call site"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 200,
        "only {checked} entry-point call sites read; the check is not seeing them"
    );
}

/// A negative control for the sweep itself: a name in a stateful family that no
/// table classifies must be reported. Without this, a sweep that returned an
/// empty set for the wrong reason would look identical to a clean one.
#[test]
fn the_sweep_would_report_an_unclassified_stateful_name() {
    for prefix in STATEFUL_UNIT_PREFIXES {
        let invented = format!("{prefix}unclassified_probe_name");
        assert_eq!(
            unit_accumulation(&invented),
            Accumulation::Undeclared,
            "a new unit under {prefix} must be reported rather than summed"
        );
    }
}

/// A cumulative unit whose whole family is NOT declared stateful, with the
/// reason it cannot be. Each of these is the only running-state unit its
/// domain publishes; every sibling under the same prefix is a genuine
/// per-frame quantity, so declaring the prefix stateful would misclassify
/// the siblings rather than protect the one. They are classified
/// individually by name, and a new sibling correctly defaults to `PerFrame`.
const CUMULATIVE_UNITS_OUTSIDE_A_STATEFUL_FAMILY: &[&str] = &[
    "domain53_sweep_stale_frames",
    "domain53_sweep_generation",
    "domain53_war_validated_generation",
    "capwar60_sweep_stale_frames",
    "h_convoy_resolver_publication_sequence",
];

/// Deleting a family from [`STATEFUL_UNIT_PREFIXES`] must not pass silently.
///
/// `the_sweep_would_report_an_unclassified_stateful_name` iterates the list
/// itself, so it says nothing about a family that has been REMOVED from it --
/// the loop simply stops testing that family and still passes. That is the
/// vacuous shape this project has been bitten by before: a guard whose only
/// test cannot fail when the guard is deleted.
///
/// The prefix list is the build-time protection for units that do not exist
/// yet: inside a stateful family an undeclared name is `Undeclared` and fails
/// the producer sweep, while outside one it silently defaults to `PerFrame`
/// and is SUMMED. Losing a family therefore costs nothing today and corrupts
/// the first new unit somebody adds to it.
///
/// The property that catches this is evidential rather than a copy of the
/// list: a family with proven running-state members must be declared. Every
/// name in `CUMULATIVE_UNITS` is inside some stateful family, except the
/// individually-classified one-offs named above.
///
/// MUTATION: remove `"rs_"` from `STATEFUL_UNIT_PREFIXES`. The ~15 scheduler
/// and sink accounting units are then orphaned and this fails, naming each.
#[test]
fn every_family_with_a_cumulative_member_is_declared_stateful() {
    let orphans: Vec<&str> = CUMULATIVE_UNITS
        .iter()
        .copied()
        .filter(|name| {
            !STATEFUL_UNIT_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
                && !CUMULATIVE_UNITS_OUTSIDE_A_STATEFUL_FAMILY.contains(name)
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "these units are known to carry running state but their family is not in          STATEFUL_UNIT_PREFIXES, so a NEW unit beside them would be summed: {orphans:?}"
    );

    // The exemption list is itself held to the evidence: an entry that has
    // since been covered by a family is stale and must be removed, or it
    // becomes a place to quietly park a name that should have been declared.
    let redundant: Vec<&str> = CUMULATIVE_UNITS_OUTSIDE_A_STATEFUL_FAMILY
        .iter()
        .copied()
        .filter(|name| {
            STATEFUL_UNIT_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
                || !CUMULATIVE_UNITS.contains(name)
        })
        .collect();
    assert!(
        redundant.is_empty(),
        "exempted from the family rule but no longer needs to be, or is no longer a          cumulative unit at all: {redundant:?}"
    );
}
