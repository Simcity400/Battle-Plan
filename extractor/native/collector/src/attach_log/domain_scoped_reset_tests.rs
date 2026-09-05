use super::*;

use std::collections::BTreeMap;

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
         verdict; nothing in this cache is a delta pin -- d47's wire baseline lives in \
         `payload_span_delta`, which rolls back its own staged half",
    ),
    (
        "rail_replay_state_live_geometry",
        "d81 unchanged-geometry memo (a copy image and the rows it decoded to), carried \
         across the reset UNCONDITIONALLY by its own carve-out (lane PW): it is a \
         bytes-to-rows map with NO hour semantics, so a rejected bracket says nothing \
         about whether what it holds is true, and the next frame re-proves its own \
         inputs byte for byte before anything is reused; d81 stages no delta through \
         it, so there is no half to roll back",
    ),
    (
        "rail_replay_state_live_snapshot",
        "d81 preallocated snapshot plan, carried across the reset unconditionally by its \
         explicit carve-out; it owns reusable buffers and plan refusal state, not a \
         published-hour delta, while each changed hour re-arms and validates its own copy",
    ),
    (
        "market_trade_live_snapshot",
        "d43's bounded snapshot plan, arming buffer and sticky refusal (lane PD), carried \
         across the reset unconditionally by its own carve-out for d81's reason: a rejected \
         bracket says nothing about whether the region table still describes the walk, every \
         frame re-copies and re-proves it before anything is published, and nothing in it is \
         a published-hour delta",
    ),
    (
        "acceptance_weather_fault_injected",
        "session-local one-shot, not a per-domain read product",
    ),
    (
        "xmap07_snow_mud",
        "d100 epoch continuity is invalid after any rejected bracket; the blanket reset forces \
         the next successful sample to open a fresh generation",
    ),
    (
        "xmap07_snow_mud_wire_baseline",
        "d100's WIRE baseline takes a roll-back, never the blanket reset and never a plain \
         retention (lane PB). It is not a continuity claim: it names the grid the FOLD holds, \
         and every delta or no-change record names it by SHA-256, so a consumer holding \
         anything else refuses the hour by name rather than decoding a grid nobody observed. \
         Wiping it alongside the epoch tracker would cost 233 MB per capture -- on \
         captures/oz_tip the epoch tracker was blanket-reset 86 times in 470 hours, and each \
         of those hours would have re-sent the whole 2.88 MB grid. The staged half IS dropped \
         when d100's own record became a gap, which is what keeps `committed` equal to what \
         the fold received",
    ),
    (
        "payload_span_delta",
        "the PSD1 wire-delta producer for every domain on `DELTA_ENABLED_DOMAINS` \
         (d43/d47/d53/d81/d93 today; lanes PS, QL, QK, SG). Same classification as d100's wire baseline \
         and for the same reason: it is not a continuity claim, it names the PAYLOAD the fold \
         holds, and every delta or no-change record names that baseline by SHA-256 -- so a \
         consumer holding anything else refuses the hour by name rather than reconstructing \
         a payload nobody observed. It survives the blanket reset and takes a PER-DOMAIN \
         roll-back instead: the staged half is dropped for exactly those enabled domains \
         whose own records became a gap, which is what keeps the committed half equal to \
         what the fold received. Wiping it on every rejected bracket would force a keyframe \
         there, and d53's keyframe is 2.88 MB against a 26 MB delta stream for the whole of \
         captures/oz_tip",
    ),
    (
        "xmap07_snow_mud_generation_counter",
        "d100's monotonic anchor-generation ordinal, carried across the reset unconditionally \
         (lane PB). Holding it INSIDE the epoch tracker is what made the field useless: the \
         reset zeroed it, `saturating_add(1).max(1)` resolved to 1 again, and all 470 records \
         on captures/oz_tip claimed generation 1 across 86 real resets -- so the fold's \
         `hook_epoch` declared one epoch containing 428 duplicate `capture_frontier_ordinal` \
         values and 85 backwards steps. A counter is not continuity state; the continuity \
         fields beside it still take the blanket reset",
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
        "d13 takes PRE-FRAME RESTORATION like d45 and d55: its wire baseline advances at \
         emission with the frame's entry journalled, its own rejection (or a refused frame) \
         restores the entry and re-opens the change gate's baseline obligation, any other \
         domain's rejection leaves the advance standing, and the frame seal drops the entry",
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

fn attach_log_source() -> String {
    super::tests::collector_source_lf()
}

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

struct ReaderCallSpan {
    start: usize,
    end: usize,
    tag: String,
}

fn reader_call_spans(body: &str) -> Vec<ReaderCallSpan> {
    let bytes = body.as_bytes();
    let mut spans = Vec::new();
    for (name, tag_arg) in [
        ("spawn_gated(", 3usize),
        // THE SPLIT READER IS A READER CALL TOO. It was missing here, and the
        // gap was invisible while no domain-scoped tracker used it: d47's and
        // d81's trackers are carried outside this table, so nothing forced the
        // scanner to know the spelling. Domain 101 is the first domain-scoped
        // tracker to take the copy-then-compute seam, and without this entry its
        // perfectly ordinary borrow reads as `OUTSIDE-EVERY-READER-CALL`.
        //
        // The tag sits at the same argument index as `spawn_gated`'s
        // (`scope, gate, lanes, tag, ...`), which is what makes one index right
        // for both.
        ("spawn_gated_split(", 3usize),
        // THE PUBLISHING READER IS A READER CALL TOO (lane RY, 2026-08-24), and
        // for the same reason the split one is: seven domains now spawn through
        // `spawn_gated_publishing`, each handing its own domain-scoped tracker
        // to the read half, and a spelling this scanner does not know makes
        // those borrows read as `OUTSIDE-EVERY-READER-CALL`. `"spawn_gated("`
        // does not cover it -- the pattern requires the parenthesis
        // immediately after the name -- so the entry is not redundant.
        //
        // Same argument index again: `scope, gate, lanes, tag, read, encode`.
        ("spawn_gated_publishing(", 3usize),
        // THE FINISHING READER IS A READER CALL TOO (lane SQ, 2026-08-25), and
        // this entry is not optional: domain 53's tracker IS domain-scoped
        // (`for_each_domain_scoped_tracker` lists
        // `diplomacy_intel_operatives_live`), and d53 is the reader that spawns
        // through `spawn_gated_finishing`. Without the spelling here its borrow
        // reads as `OUTSIDE-EVERY-READER-CALL` and this file's table comparison
        // fails outright -- which is the scanner doing its job, and the reason
        // a new spawn helper has to arrive with a line in this list.
        //
        // `"spawn_gated("` does not cover it: the pattern requires the
        // parenthesis immediately after the name. Same argument index again --
        // `scope, gate, lanes, tag, observe, finish`.
        ("spawn_gated_finishing(", 3usize),
        // THE PREPARING READER IS A READER CALL TOO (lane PT, 2026-09-02): d43
        // and d93 spawn through `spawn_gated_preparing`, each handing its own
        // tracker to the read half. Same argument index -- `scope, gate,
        // lanes, tag, read, prepare`.
        ("spawn_gated_preparing(", 3usize),
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
    let end = body[at..]
        .find('\n')
        .map(|nl| at + nl)
        .unwrap_or(body.len());
    (start, end)
}

fn line_containing(body: &str, at: usize) -> &str {
    let (start, end) = line_bounds(body, at);
    &body[start..end]
}

fn derive_field_tags_from_source(body: &str) -> BTreeMap<String, Vec<String>> {
    let spans = reader_call_spans(body);
    assert!(
        spans.len() > 30,
        "the frame builder should hold dozens of reader calls, found {}",
        spans.len()
    );

    let mut derived: BTreeMap<String, Vec<String>> = BTreeMap::new();

    assert_eq!(
        body.matches("spawn_compact_country_children(").count(),
        1,
        "the frame builder spawns the country children exactly once"
    );
    let child_tags = country_child_bracket_tags();
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_suffix(',') {
            if let Some((lhs, rhs)) = rest.split_once(": &mut trackers.") {
                if !lhs.contains(' ') && !rhs.contains(' ') {
                    let tag = child_tags.get(rhs).unwrap_or_else(|| {
                        panic!(
                            "country tracker field `{rhs}` is handed to the country children but \
                             no `spawn_gated_country_child` call in this crate takes its local"
                        )
                    });
                    derived
                        .entry(rhs.to_string())
                        .or_default()
                        .push(tag.clone());
                }
            }
        }
    }

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
            if trimmed.starts_with(&format!("let {local} = &mut trackers."))
                || trimmed.starts_with(&format!("let {local} = &trackers."))
                || trimmed == format!("let {local} =")
                || trimmed == format!("&mut trackers.{field};")
                || trimmed == format!("&trackers.{field};")
            {
                continue;
            }
            if trimmed == format!("let {local} = {local};") {
                continue;
            }
            let innermost = spans
                .iter()
                .filter(|span| span.start < use_at && use_at < span.end)
                .max_by_key(|span| span.start);
            match innermost {
                Some(span) => entry.push(span.tag.clone()),
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

fn table() -> BTreeMap<&'static str, &'static str> {
    let mut table = BTreeMap::new();
    macro_rules! collect_table {
        ($($field:ident => $tag:expr,)*) => {
            $( table.insert(stringify!($field), stringify!($tag)); )*
        };
    }
    for_each_domain_scoped_tracker!(collect_table);
    for_each_country_scoped_tracker!(collect_table);
    for_each_journalled_baseline_tracker!(collect_table);
    table
}

/// `(tracker field, bracket tag)` for the eight country children, derived from
/// the fan-out's own text: the tracker local each field is destructured into,
/// and the `spawn_gated_country_child(scope, gate, lanes, TAG, ...)` call whose
/// read closure takes that local.
fn country_child_bracket_tags() -> BTreeMap<String, String> {
    let source = attach_log_source();
    let destructure_start = source
        .find("let CompactCountryGateTrackers {")
        .expect("the country fan-out destructures its tracker set");
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
        "the country fan-out destructures eight trackers, parsed {local_of_field:?}"
    );

    let mut tag_of_local: BTreeMap<String, String> = BTreeMap::new();
    let needle = "spawn_gated_country_child(";
    let mut search = 0usize;
    let mut calls = 0usize;
    while let Some(at) = source[search..].find(needle) {
        let open = search + at + needle.len() - 1;
        search = open + 1;
        let Some(close) = matching_paren(&source, open) else {
            continue;
        };
        let args = split_top_level_args(&source[open + 1..close]);
        // `scope, gate, lanes, tag, before_bracket, read, encode`
        let Some(tag) = args.get(3) else {
            continue;
        };
        let call_body = args[4..].join(",");
        calls += 1;
        for local in local_of_field.values() {
            let mut from = 0usize;
            let mut hit = false;
            while let Some(at) = call_body[from..].find(local.as_str()) {
                let use_at = from + at;
                from = use_at + local.len();
                if is_word_boundary(&call_body, use_at, local.len()) {
                    hit = true;
                    break;
                }
            }
            if hit {
                tag_of_local.insert(local.clone(), tag.clone());
            }
        }
    }
    assert_eq!(
        calls, 8,
        "each of the eight country children is spawned through the gated child helper, \
         found {calls}"
    );

    local_of_field
        .into_iter()
        .map(|(field, local)| {
            let tag = tag_of_local.get(&local).unwrap_or_else(|| {
                panic!(
                    "no `spawn_gated_country_child` call takes the country tracker local `{local}`"
                )
            });
            (field, tag.clone())
        })
        .collect()
}

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


/// The victory-point delta pin, over the sequence that has no other witness:
/// a successful observe, a record that then refuses, and the next hour's delta.
///
/// kills: writing d101's advance straight through instead of staging it. Without
/// the rollback the producer holds census B while the fold still holds A, so the
/// next delta patches A with B's changes; the census digest stops that from being
/// published as complete, but the carrier is then unavailable until the next
/// keyframe -- up to a game week -- and if the refused record was the FIRST one,
/// every later delta is refused as `delta_without_baseline` for the rest of the
/// recording. One partial-data refusal must not cost later readable hours.
///
/// (Adversarial review round 3. `finalize_compact_map_furniture_victory_points_read`
/// can turn a clean read into a refusal after `observe` has already advanced, and
/// so can a panic in the compute half.)
#[test]
fn a_refused_victory_point_record_restores_the_census_the_frame_found() {
    use compact_contract::map_furniture_victory_points::VictoryPointRow;

    fn row(province_id: u32, current_value: i32) -> VictoryPointRow {
        VictoryPointRow {
            province_id,
            current_value,
        }
    }

    // Hour 1 published a two-province census.
    let mut tracker = CompactMapFurnitureVictoryPointTracker {
        previous: vec![row(1, 5), row(2, 10)],
        baselined: true,
        last_full_census_hour_raw: 100,
        uncommitted_pin: None,
        topology: None,
    };

    // Hour 2 reads a different census and stages it as a keyframe.
    //
    // THE RETURN VALUE IS CHECKED, and that is the point of taking it: it is the
    // census the delta will be diffed against, and an earlier version of this
    // reader took it out of the tracker BEFORE staging, which left the pin
    // capturing an empty census and every rollback restoring one. Asserting the
    // displaced value here is what makes the pin and the delta provably the same
    // census. (Adversarial review round 4.)
    let displaced = tracker.stage_advance(vec![row(1, 7), row(3, 2)], Some(268));
    assert_eq!(
        displaced,
        vec![row(1, 5), row(2, 10)],
        "staging must hand back the census the frame found"
    );
    assert_eq!(tracker.previous, vec![row(1, 7), row(3, 2)]);
    assert_eq!(tracker.last_full_census_hour_raw, 268);

    // ...and hour 2's record then refuses, so the frame settles it back.
    tracker.roll_back_uncommitted_pin();
    assert_eq!(
        tracker.previous,
        vec![row(1, 5), row(2, 10)],
        "a refused record must leave the producer diffing against the census the \
         fold still holds"
    );
    assert!(tracker.baselined);
    assert_eq!(
        tracker.last_full_census_hour_raw, 100,
        "a refused record is not a keyframe and must not restart the cadence"
    );

    // A second rollback is harmless: nothing is staged.
    tracker.roll_back_uncommitted_pin();
    assert_eq!(tracker.previous, vec![row(1, 5), row(2, 10)]);

    // Hour 3 publishes, and its commit seals the advance against a LATER frame's
    // rollback.
    let displaced = tracker.stage_advance(vec![row(1, 6)], None);
    assert_eq!(displaced, vec![row(1, 5), row(2, 10)]);
    tracker.commit_pin();
    tracker.roll_back_uncommitted_pin();
    assert_eq!(
        tracker.previous,
        vec![row(1, 6)],
        "a committed advance is beyond a later frame's reach"
    );
    assert_eq!(tracker.last_full_census_hour_raw, 100);
}

/// kills: staging the pin twice in one frame and rolling back only to the
/// SECOND stage, which would restore a census the fold never saw either.
#[test]
fn staging_a_victory_point_advance_twice_still_rolls_back_to_the_frames_entry() {
    use compact_contract::map_furniture_victory_points::VictoryPointRow;
    let entry = vec![VictoryPointRow {
        province_id: 1,
        current_value: 5,
    }];
    let mut tracker = CompactMapFurnitureVictoryPointTracker {
        previous: entry.clone(),
        baselined: true,
        last_full_census_hour_raw: 100,
        uncommitted_pin: None,
        topology: None,
    };
    let first = tracker.stage_advance(
        vec![VictoryPointRow {
            province_id: 2,
            current_value: 6,
        }],
        Some(200),
    );
    assert_eq!(first, entry);
    tracker.stage_advance(
        vec![VictoryPointRow {
            province_id: 3,
            current_value: 7,
        }],
        Some(300),
    );
    tracker.roll_back_uncommitted_pin();
    assert_eq!(tracker.previous, entry);
    assert_eq!(tracker.last_full_census_hour_raw, 100);
}

/// kills: letting a never-baselined tracker come back from a refusal claiming a
/// baseline. The first record is the one whose loss is worst -- every later delta
/// would be refused as `delta_without_baseline` -- so the flag has to roll back
/// with the census.
#[test]
fn a_refused_first_victory_point_record_leaves_the_tracker_unbaselined() {
    use compact_contract::map_furniture_victory_points::VictoryPointRow;
    let mut tracker = CompactMapFurnitureVictoryPointTracker::default();
    assert!(!tracker.baselined);
    let displaced = tracker.stage_advance(
        vec![VictoryPointRow {
            province_id: 1,
            current_value: 5,
        }],
        Some(100),
    );
    assert!(
        displaced.is_empty(),
        "a tracker with no baseline displaces nothing"
    );
    assert!(tracker.baselined);
    tracker.roll_back_uncommitted_pin();
    assert!(
        !tracker.baselined,
        "the next record must be a full census, not a delta against nothing"
    );
    assert!(tracker.previous.is_empty());
}

#[test]
fn domain_scoped_tracker_allowlist_matches_the_borrow_sites() {
    let source = attach_log_source();
    let body = frame_builder_body(&source);
    let derived = derive_field_tags_from_source(body);
    let table = table();

    assert_eq!(
        table.len(),
        DOMAIN_SCOPED_TRACKER_RETENTION.len()
            + COUNTRY_SCOPED_TRACKER_RETENTION.len()
            + JOURNALLED_TRACKER_RETENTION.len(),
        "the table macros and the flattened constants must describe the same list"
    );

    let mut disagreements = Vec::new();
    for (field, expected_tag) in &table {
        match derived.get(*field) {
            Some(tags) if tags.as_slice() == [expected_tag.to_string()] => {}
            Some(tags)
                if *field == "diplomacy_intel_operatives_live"
                    && tags.len() == 2
                    && tags.contains(&expected_tag.to_string())
                    && tags
                        .iter()
                        .any(|tag| tag.starts_with("OUTSIDE-EVERY-READER-CALL")) => {}
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

#[test]
fn d53s_outside_bracket_borrow_is_only_the_snapshot_armer() {
    let source = attach_log_source();
    let body = frame_builder_body(&source);
    let arm = body
        .find("let snapshot_arm = arm_compact_diplomacy_intel_operatives_live_snapshot(")
        .expect("d53 snapshot plan is armed before its bracket");
    let spawn = body[arm..]
        .find("spawn_gated_finishing(")
        .map(|at| arm + at)
        .expect("d53 bracket follows its armer");
    assert!(arm < spawn);

    assert!(source.contains("snapshot_plan: self.snapshot_plan.take(),"));
    assert!(source.contains("self.snapshot_plan = carried.snapshot_plan;"));
}

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

#[test]
fn the_d47_discovery_cache_is_an_unconditional_carve_out_not_a_bracket_scoped_pin() {
    let field = "characters_leaders_cabinet_live";
    assert!(
        !DOMAIN_SCOPED_TRACKER_RETENTION
            .iter()
            .any(|(name, _)| *name == field),
        "`{field}` must stay off the bracket-scoped allowlist: the next frame's own \
         input comparison, not this bracket's verdict, decides whether the memo may \
         be used"
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
    let reset_body = &source[reset_start
        ..reset_start
            + source[reset_start..]
                .find(
                    "
}
",
                )
                .expect("closed")];
    assert!(
        reset_body.contains("std::mem::take(&mut trackers.characters_leaders_cabinet_live)")
            && reset_body
                .contains("trackers.characters_leaders_cabinet_live = d47_discovery_cache;"),
        "the reset must take the d47 cache before the blanket `fresh` and reinstall it after"
    );
}

/// Both survive their clock bracket and can still acquire a failure-class status
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

/// Lane PW: d81's geometry memo is an unconditional carve-out, NOT a
/// bracket-scoped pin.
///
/// The landing gate for `90a3dac3` suggested domain-scoping this field under
/// d81's reader tag, "like the other rail_replay_state_live trackers". That
/// would be wrong, and the difference is the whole design: a domain-scoped
/// field is wiped when its reader's bracket is REJECTED, and the memo has no
/// hour semantics to invalidate. It maps a byte image to the rows that image
/// decodes to. A torn bracket means this frame may not publish; it does not
/// make the remembered mapping false, and the next frame re-proves its own
/// inputs byte for byte before reusing anything.
///
/// Wiping it on every tear is exactly the d47 death-spiral shape (lane MT,
/// t83): straddled hours would force a cold walk on the next frame, which
/// straddles the next hour, and the gate would never warm. d47's own cache is
/// classified here for the same reason.
///
/// kills: moving this field to DOMAIN_SCOPED_TRACKER_RETENTION; deleting either
/// half of the take/reinstall carve-out in the reset.
#[test]
fn the_d81_geometry_memo_is_an_unconditional_carve_out_not_a_bracket_scoped_pin() {
    let field = "rail_replay_state_live_geometry";
    assert!(
        !DOMAIN_SCOPED_TRACKER_RETENTION
            .iter()
            .any(|(name, _)| *name == field),
        "`{field}` must stay off the bracket-scoped allowlist: the next frame's own \
         input comparison, not this bracket's verdict, decides whether the memo may \
         be used"
    );
    let (_, reason) = BLANKET_RESET_TRACKER_FIELDS
        .iter()
        .find(|(name, _)| *name == field)
        .expect("the geometry memo carries its classification reason");
    assert!(
        reason.contains("UNCONDITIONALLY") && reason.contains("carve-out"),
        "the classification must state the unconditional carve-out: {reason}"
    );
    let source = attach_log_source();
    let reset_start = source
        .find("fn reset_trackers_after_rejected_bracket(")
        .expect("the reset function is declared");
    let reset_body = &source[reset_start
        ..reset_start
            + source[reset_start..]
                .find(
                    "
}
",
                )
                .expect("closed")];
    assert!(
        reset_body.contains("std::mem::take(&mut trackers.rail_replay_state_live_geometry)")
            && reset_body
                .contains("trackers.rail_replay_state_live_geometry = d81_geometry_memo;"),
        "the reset must take the geometry memo before the blanket `fresh` and reinstall it after"
    );
}

#[test]
fn xmap07_snow_mud_epoch_state_is_blanket_reset_not_retained() {
    // kills: adding d100's epoch tracker to DOMAIN_SCOPED_TRACKER_RETENTION.
    let field = "xmap07_snow_mud";
    assert!(
        !DOMAIN_SCOPED_TRACKER_RETENTION
            .iter()
            .any(|(name, _)| *name == field),
        "a rejected bracket must never preserve d100 continuity"
    );
    assert!(
        BLANKET_RESET_TRACKER_FIELDS
            .iter()
            .any(|(name, reason)| *name == field && reason.contains("fresh generation")),
        "d100 must state why the blanket reset is required"
    );
}

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
        .chain(JOURNALLED_TRACKER_RETENTION.iter())
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

fn covered_domains(tag: u16) -> Vec<u16> {
    let mut domains = Vec::new();
    for_each_bracket_covered_domain(tag, |domain| domains.push(domain));
    domains
}

const UNRELATED_DOMAIN: u16 = 0xBEEF;

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

struct TrackerProbe {
    field: &'static str,
    mark: fn(&mut CompactPerfGateTrackers),
    is_marked: fn(&CompactPerfGateTrackers) -> bool,
}

struct CountryTrackerProbe {
    field: &'static str,
    committed: fn(&mut CompactPerfGateTrackers),
    is_committed: fn(&CompactPerfGateTrackers) -> bool,
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
    TrackerProbe {
        field: "supply_logistics_attrition_live",
        mark: |trackers| {
            trackers
                .supply_logistics_attrition_live
                .csupply_system_object = Some(0x1234);
        },
        is_marked: |trackers| {
            trackers
                .supply_logistics_attrition_live
                .csupply_system_object
                .is_some()
        },
    },
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
        .chain(JOURNALLED_TRACKER_RETENTION.iter())
        .find(|(name, _)| *name == field)
        .map(|(_, tag)| *tag)
        .unwrap_or_else(|| panic!("`{field}` is a domain-scoped tracker field"))
}

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

#[test]
fn one_domains_rejection_does_not_reset_another_domains_pin() {
    for probe in TRACKER_PROBES {
        let tag = probe_tag(probe.field);
        let covered = covered_domains(tag);
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

#[test]
fn a_surviving_country_child_keeps_its_staged_advance_for_the_frame_verdict() {
    for probe in COUNTRY_TRACKER_PROBES {
        let covered = covered_domains(country_probe_tag(probe.field));
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

#[test]
fn a_pre_guard_panic_brackets_only_the_joined_and_uncovered_children() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let scheduled: BTreeSet<u16> = [COMPACT_DOMAIN_FUEL, COMPACT_DOMAIN_DECISIONS]
        .into_iter()
        .collect();
    lanes.record_clock_bracket(COMPACT_DOMAIN_FUEL, None, None, false);
    lanes.record_worker_panicked_country_brackets(&scheduled);

    let brackets = lanes.take_clock_brackets();
    let synthesized: Vec<u16> = brackets
        .iter()
        .filter(|bracket| bracket.worker_panicked)
        .map(|bracket| bracket.tag)
        .collect();
    assert_eq!(
        synthesized,
        vec![COMPACT_DOMAIN_DECISIONS],
        "only the joined child that left no bracket may be synthesized; fuel already          recorded one and the six children the join did not name attempted nothing"
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

#[test]
fn a_pre_guard_panic_over_the_focus_and_tech_children_brackets_tech_alone() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let scheduled: BTreeSet<u16> = [COMPACT_DOMAIN_FOCUS, COMPACT_DOMAIN_TECH]
        .into_iter()
        .collect();
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

fn seeded_tech_tracker(seed: u32) -> CompactTechCountryDeltaTracker {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let dictionary = TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    };
    let rows = [super::domain13_tech_delta_tests::tech_row(1, seed)];
    let _ = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    tracker
}

fn tech_pin_bytes(trackers: &CompactPerfGateTrackers) -> Vec<Vec<u8>> {
    trackers
        .tech_country_delta
        .pin
        .blocks
        .rows
        .iter()
        .map(|block| block.country_row.to_vec())
        .collect()
}

#[test]
fn a_rejected_tech_bracket_restores_the_pin_the_frame_found() {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.tech_country_delta = seeded_tech_tracker(1);
    trackers.tech_country_delta.close_frame_transaction();
    let entry = tech_pin_bytes(&trackers);
    let dictionary = TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    };
    let rows = [super::domain13_tech_delta_tests::tech_row(1, 2)];
    let _ = compact_tech_data_payload_delta(&rows, 1, 0, &mut trackers.tech_country_delta, &dictionary);
    assert_ne!(tech_pin_bytes(&trackers), entry, "the frame advanced the pin");
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_TECH].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    assert_eq!(
        tech_pin_bytes(&trackers),
        entry,
        "d13's own rejection puts the pin back where the frame found it"
    );
    assert!(
        trackers.tech_country_delta.pin.baseline_seeded,
        "the restored pin is the seeded baseline, not Default"
    );
}

#[test]
fn an_unrelated_rejection_keeps_the_advanced_tech_pin() {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.tech_country_delta = seeded_tech_tracker(1);
    trackers.tech_country_delta.close_frame_transaction();
    let dictionary = TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    };
    let rows = [super::domain13_tech_delta_tests::tech_row(1, 2)];
    let _ = compact_tech_data_payload_delta(&rows, 1, 0, &mut trackers.tech_country_delta, &dictionary);
    let advanced = tech_pin_bytes(&trackers);
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_FUEL, UNRELATED_DOMAIN].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    assert_eq!(
        tech_pin_bytes(&trackers),
        advanced,
        "d13 survived its own bracket, so its record reached the fold and the advance stands"
    );
    assert!(
        trackers.tech_country_delta.frame_entry.is_some(),
        "the entry is still journalled until the frame seals"
    );
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
    assert!(
        trackers.tech_country_delta.frame_entry.is_some(),
        "the build's seal settles nothing; the sink has not spoken"
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    assert!(
        trackers.tech_country_delta.frame_entry.is_none(),
        "the sink verdict drops the entry"
    );
    reset_trackers_after_rejected_bracket(&mut trackers, &[COMPACT_DOMAIN_TECH].into_iter().collect());
    assert_eq!(
        tech_pin_bytes(&trackers),
        advanced,
        "a later frame's rejection cannot undo a sealed advance"
    );
}

#[test]
fn a_refused_frame_returns_the_restored_tech_pin() {
    let mut refused = CompactPerfGateTrackers::default();
    refused.tech_country_delta = seeded_tech_tracker(1);
    refused.tech_country_delta.close_frame_transaction();
    let entry = tech_pin_bytes(&refused);
    let dictionary = TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    };
    let rows = [super::domain13_tech_delta_tests::tech_row(1, 2)];
    let _ = compact_tech_data_payload_delta(&rows, 1, 0, &mut refused.tech_country_delta, &dictionary);
    // The build's seal ran before the sink refused the frame; the entry has to
    // outlive it.
    commit_delta_pin_transactions(&mut refused, &mut Vec::new());
    let mut published = CompactPerfGateTrackers::default();
    restore_country_trackers_after_refused_frame(&mut refused, &mut published);
    assert_eq!(
        tech_pin_bytes(&published),
        entry,
        "a refused frame published nothing; the pre-frame pin comes back to the published slot"
    );
    assert!(published.tech_country_delta.pin.baseline_seeded);
}
