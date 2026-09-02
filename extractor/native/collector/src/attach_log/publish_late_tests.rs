//! Lane RY: the `publish_late` tail segment's worker-encoded publications.
//!
//! The frame's serial tail publishes ~29 domains between the D47 publish and
//! the frame-time close, and on `captures/rc_vanilla` that segment measured
//! 7.51 ms/frame avg / 11.61 p95 against a 14.93 ms tail and a 48 ms hour.
//! Almost all of it is encode: a `compact_*_data_payload` builder walking rows
//! that every worker has already materialised. Seven of those domains build a
//! payload that is a pure function of their own read, so
//! [`super::spawn_gated_publishing`] has the producing worker build it after
//! its clock bracket has closed and its reader permit has been released, and
//! the tail keeps only the header write.
//!
//! What must not change is the bytes. Each test below runs BOTH paths over the
//! same read and compares the whole record, header included.

use super::*;

const HOUR: i32 = 1939_09_01;
const SEQ: u64 = 4_242;
const ELAPSED: u64 = 7_919;

/// The start of the next top-level item, used to bound one function's source
/// text. Built from a `char` rather than written as a literal because this repo
/// checks its Rust out with CRLF and a multi-line pattern authored against an LF
/// worktree matches nothing once merged (see `tests::collector_source_lf`).
const NEXT_ITEM: &str = "\nfn ";

/// The end of a top-level `if let` block in the frame builder: a closing brace
/// at four spaces of indent. Same CRLF reason as [`NEXT_ITEM`].
const BLOCK_CLOSE: &str = "\n    }";

/// The inline path and the worker path for one domain, as whole records.
fn assert_same_record(
    domain: &str,
    worker_payload: Vec<u8>,
    domain_id: u16,
    inline: impl FnOnce(&mut Vec<u8>),
) {
    let mut moved = Vec::new();
    append_compact_record(
        &mut moved,
        SEQ,
        HOUR,
        domain_id,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &worker_payload,
    );
    let mut inline_bytes = Vec::new();
    inline(&mut inline_bytes);
    assert_eq!(
        moved, inline_bytes,
        "{domain}: the worker-built record and the inline tail record must be \
         the same bytes",
    );
    assert!(
        !moved.is_empty(),
        "{domain}: an empty record would make the comparison vacuous",
    );
}

/// Every domain lane RY moved: the payload the worker builds, wrapped by the
/// tail's header write, against the record the tail used to build by itself.
///
/// kills: a swapped or dropped argument in any of the seven encode closures at
/// the spawn sites (the scalars below are pairwise distinct, so a payload built
/// from the wrong field differs in the bytes); a closure that hands the builder
/// a different `elapsed_wall_ms` than the tail does; a `match` arm in the tail
/// that names the wrong `COMPACT_DOMAIN_*` or the wrong record type.
///
/// NOT VACUOUS DESPITE THE EMPTY ROW VECTORS. Every builder writes a fixed
/// scalar header before its rows, and these reads carry twelve to sixteen
/// distinct scalars each; the map-control and map-detail arms additionally
/// carry rows, so the row-encoding half is exercised too.
#[test]
fn every_worker_encoded_publish_late_record_is_byte_identical_to_the_inline_tail_encode() {
    // ---- D42 air combat losses -------------------------------------------
    let mut d42 = compact_air_strength_delta_worker_panic_read(0);
    d42.changed_rows = 3;
    d42.observed_air_wing_rows = 5;
    d42.observed_equipment_rows = 7;
    d42.loss_rows = 11;
    d42.reinforcement_rows = 13;
    d42.deleted_rows = 17;
    d42.observed_region_objects = 19;
    d42.invalid_air_wing_rows = 23;
    d42.scale_mismatch_rows = 29;
    d42.first_invalid_air_wing_ordinal = 31;
    d42.first_invalid_air_wing_reason = 37;
    assert_same_record(
        "air_combat_losses_live",
        compact_air_strength_deltas_data_payload(
            &d42.rows,
            &d42.region_stats,
            ELAPSED,
            d42.status_flags,
            d42.changed_rows,
            d42.observed_air_wing_rows,
            d42.observed_equipment_rows,
            d42.loss_rows,
            d42.reinforcement_rows,
            d42.deleted_rows,
            d42.observed_region_objects,
            d42.invalid_air_wing_rows,
            d42.scale_mismatch_rows,
            d42.first_invalid_air_wing_ordinal,
            d42.first_invalid_air_wing_reason,
            !d42.region_section_declared_absent,
        ),
        COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE,
        |out| {
            append_compact_air_strength_deltas_data_record(
                out,
                SEQ,
                HOUR,
                &d42.rows,
                &d42.region_stats,
                ELAPSED,
                d42.status_flags,
                d42.changed_rows,
                d42.observed_air_wing_rows,
                d42.observed_equipment_rows,
                d42.loss_rows,
                d42.reinforcement_rows,
                d42.deleted_rows,
                d42.observed_region_objects,
                d42.invalid_air_wing_rows,
                d42.scale_mismatch_rows,
                d42.first_invalid_air_wing_ordinal,
                d42.first_invalid_air_wing_reason,
                !d42.region_section_declared_absent,
            )
        },
    );

    // ---- D44 supply logistics attrition ----------------------------------
    let mut d44 = compact_supply_logistics_attrition_worker_panic_read(0);
    d44.changed_rows = 41;
    d44.province_rows = 43;
    d44.division_rows = 47;
    d44.badge_rows = 53;
    d44.csupply_input_rows = 59;
    d44.observed_province_roots = 61;
    d44.observed_division_roots = 67;
    assert_same_record(
        "supply_logistics_attrition_live",
        compact_supply_logistics_attrition_data_payload(&d44, ELAPSED),
        COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE,
        |out| append_compact_supply_logistics_attrition_data_record(out, SEQ, HOUR, &d44, ELAPSED),
    );

    // ---- D48 construction / buildings / state damage ---------------------
    let mut d48 = compact_construction_buildings_state_damage_worker_panic_read(HOUR, 0);
    d48.frame.observed_country_roots = 71;
    d48.frame.observed_state_roots = 73;
    d48.frame.observed_province_roots = 79;
    assert_same_record(
        "construction_buildings_state_damage_live",
        domain48::encode_lane_frame(&d48.frame),
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        |out| {
            append_compact_construction_buildings_state_damage_data_record(
                out,
                SEQ,
                HOUR,
                &d48.frame,
            )
        },
    );

    // ---- Map detail, WITH ROWS -------------------------------------------
    let mut map_detail = compact_map_detail_worker_panic_read(0);
    map_detail.state_rows = vec![CompactMapDetailStateRow {
        state_id: 83,
        resources: None,
        buildings: None,
    }];
    map_detail.province_rows = vec![CompactMapDetailProvinceRow {
        province_id: 89,
        state_id: 83,
        buildings: None,
    }];
    map_detail.changed_rows = 97;
    map_detail.observed_state_rows = 101;
    map_detail.observed_province_rows = 103;
    assert_same_record(
        "map_detail",
        compact_map_detail_data_payload(
            &map_detail.state_rows,
            &map_detail.province_rows,
            ELAPSED,
            map_detail.status_flags,
            map_detail.changed_rows,
            map_detail.observed_state_rows,
            map_detail.observed_province_rows,
        ),
        COMPACT_DOMAIN_MAP_DETAIL,
        |out| {
            append_compact_map_detail_data_record(
                out,
                SEQ,
                HOUR,
                &map_detail.state_rows,
                &map_detail.province_rows,
                ELAPSED,
                map_detail.status_flags,
                map_detail.changed_rows,
                map_detail.observed_state_rows,
                map_detail.observed_province_rows,
            )
        },
    );

    // ---- Weather ----------------------------------------------------------
    let mut weather = compact_weather_worker_panic_read(0);
    weather.changed_rows = 107;
    weather.observed_region_rows = 109;
    weather.observed_province_rows = 113;
    weather.max_water_raw = 127;
    weather.max_snow_raw = 131;
    assert_same_record(
        "weather_live",
        compact_weather_data_payload(
            &weather.rows,
            &weather.province_rows,
            ELAPSED,
            weather.status_flags,
            weather.changed_rows,
            weather.observed_region_rows,
            weather.observed_province_rows,
            weather.max_water_raw,
            weather.max_snow_raw,
        ),
        COMPACT_DOMAIN_WEATHER,
        |out| {
            append_compact_weather_data_record(
                out,
                SEQ,
                HOUR,
                &weather.rows,
                &weather.province_rows,
                ELAPSED,
                weather.status_flags,
                weather.changed_rows,
                weather.observed_region_rows,
                weather.observed_province_rows,
                weather.max_water_raw,
                weather.max_snow_raw,
            )
        },
    );

    // ---- Map control, WITH ROWS ------------------------------------------
    let mut map_control = compact_map_control_worker_panic_read(0);
    map_control.state_rows = vec![CompactMapStateRow {
        state_id: 137,
        owner_tag_db_id: 139,
        controller_tag_db_id: 149,
    }];
    map_control.province_rows = vec![CompactMapProvinceRow {
        province_id: 151,
        controller_tag_db_id: 157,
        state_id: 137,
    }];
    map_control.changed_rows = 163;
    map_control.changed_state_rows = 167;
    map_control.changed_province_rows = 173;
    assert_same_record(
        "map_control",
        compact_map_control_data_payload(
            &map_control.state_rows,
            &map_control.province_rows,
            ELAPSED,
            map_control.status_flags,
            map_control.changed_rows,
            map_control.changed_state_rows,
            map_control.changed_province_rows,
        ),
        COMPACT_DOMAIN_MAP_CONTROL,
        |out| {
            append_compact_map_control_data_record(
                out,
                SEQ,
                HOUR,
                &map_control.state_rows,
                &map_control.province_rows,
                ELAPSED,
                map_control.status_flags,
                map_control.changed_rows,
                map_control.changed_state_rows,
                map_control.changed_province_rows,
            )
        },
    );
}

/// D41 keeps HALF of its publication in the tail, and this is the half.
///
/// The authority trailer is stamped with the record's own sequence number,
/// which is the one input an encoding worker cannot know, so only the base
/// payload moves. `append_compact_projects_data_record_onto` is the shared
/// statement of everything after the base payload exists.
///
/// THE TRAILER'S LAST THIRTY-TWO BYTES ARE A DIGEST OF THE PAYLOAD UNDER IT,
/// and that is the seam this split could actually break.
/// `append_compact_project_authority_trailer` takes `Sha256` of the payload as
/// it stands when the trailer is appended, so the split is byte-identical only
/// if the bytes the WORKER built are exactly the bytes the tail digests —
/// nothing added, nothing dropped, nothing rebuilt in between. The assertion at
/// the end of this test compares that digest against the base payload directly,
/// which is a sharper statement than "the record got longer".
///
/// THE FIXTURE HAS TO REACH THE TRAILER, and an earlier revision of this test
/// did not. `append_compact_project_authority_trailer` returns early on two
/// gates: `read.d18_authority` must be `Some`, and the PPGF section must be
/// present. `compact_projects_worker_panic_read` leaves `d18_authority` at
/// `None` — correctly, a panicked worker claims no roster authority — so the
/// trailer was never appended, the record was the base payload, and the
/// length assertion was asserting something false about a fixture that had no
/// trailer to find. Both gates are opened below, and the guard immediately
/// after them fails loudly if a later change to those gates closes one again
/// rather than letting this test go quietly vacuous.
///
/// kills: a worker-built D41 payload published WITHOUT the trailer; a trailer
/// whose digest covers something other than the base payload the worker built
/// (an extra section appended in the tail, or a payload rebuilt there); a tail
/// that stamps the trailer with a sequence other than the record's own; a
/// fixture that stops reaching the trailer at all.
#[test]
fn the_projects_publication_splits_at_the_authority_trailer_and_rejoins_identically() {
    let mut read = compact_projects_worker_panic_read(0);
    read.changed_rows = 179;
    read.observed_facility_rows = 181;
    read.observed_project_rows = 191;
    read.observed_scientist_rows = 193;
    read.scan_ms = 197;
    read.bridge_rows_supplied = 199;
    // The trailer's two gates, opened. `owner_progress_countries` alone makes
    // the PPGF section present, so no row type has to be built here.
    read.d18_authority = Some(D41CountryRosterAuthority {
        row_count: 211,
        sha256: [0x5a; 32],
    });
    read.owner_progress_countries = 223;

    let base_only = compact_projects_data_payload(&read, ELAPSED);
    let mut without_trailer = Vec::new();
    append_compact_record(
        &mut without_trailer,
        SEQ,
        HOUR,
        COMPACT_DOMAIN_PROJECTS,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &base_only,
    );

    let mut moved = Vec::new();
    append_compact_projects_data_record_onto(
        &mut moved,
        SEQ,
        HOUR,
        &read,
        compact_projects_data_payload(&read, ELAPSED),
    );
    let mut inline = Vec::new();
    append_compact_projects_data_record(&mut inline, SEQ, HOUR, &read, ELAPSED);
    assert_eq!(
        moved, inline,
        "the base payload built by the worker and finished in the tail must be \
         the record the tail used to build end to end",
    );

    // THE FIXTURE REACHED THE TRAILER. Checked before the digest is compared,
    // so a gate that closes again is reported as a dead fixture rather than as
    // a digest mismatch.
    assert!(
        without_trailer.len() < moved.len(),
        "the fixture must open both of the trailer's gates, or everything below \
         is asserted about a record that has no trailer in it",
    );

    // AND THE DIGEST COVERS THE WORKER'S BYTES AND NOTHING ELSE. The trailer's
    // final field is `Sha256` of the payload beneath it; the payload beneath it
    // must be exactly what the worker handed over.
    let expected: [u8; 32] = Sha256::digest(base_only.as_slice()).into();
    assert!(
        moved.ends_with(&expected),
        "the authority trailer must close on a digest of the base payload the \
         worker built; if it does not, the split is publishing a digest over \
         different bytes from production's",
    );
}

/// NO CONVERTED DOMAIN ADDS TO ITS PAYLOAD BETWEEN THE BUILDER AND THE RECORD,
/// except D41, which is why D41 is the one that had to be split.
///
/// This is the seam the whole conversion turns on. A worker may build a payload
/// only if the payload the TAIL would have published is the same bytes — so any
/// statement standing between `let payload = <builder>(..)` and
/// `append_compact_record(..)` is either work the worker must also do, or a
/// reason that domain cannot be converted at all. D41's authority trailer is
/// exactly such a statement: it is stamped with the record's own sequence and
/// closes on a digest of the payload beneath it, so the split moved the base
/// payload and left the trailer in the tail
/// (`append_compact_projects_data_record_onto`).
///
/// The landing gate found this the expensive way: the D41 test above was
/// asserting the trailer's presence against a fixture whose gates left it
/// absent. The six other wrappers were checked by hand at the time; this is
/// that check, kept.
///
/// kills: a seventh domain converted whose wrapper appends a trailer, stamps a
/// sequence, or digests its payload in the tail; a future trailer added to any
/// of the six converted wrappers, which would silently make the worker's
/// payload and the tail's disagree.
#[test]
fn no_converted_domain_adds_to_its_payload_between_the_builder_and_the_record() {
    let source = super::tests::collector_source_lf();

    // The six whose wrapper must be `builder` then `append_compact_record`,
    // with nothing between.
    const DIRECT: [&str; 6] = [
        "fn append_compact_air_strength_deltas_data_record(",
        "fn append_compact_supply_logistics_attrition_data_record(",
        "fn append_compact_construction_buildings_state_damage_data_record(",
        "fn append_compact_map_detail_data_record(",
        "fn append_compact_weather_data_record(",
        "fn append_compact_map_control_data_record(",
    ];
    for signature in DIRECT {
        let body = helper_body(&source, signature);
        let between = strip_code(between_builder_and_record(body));
        assert!(
            between.is_empty(),
            "{signature} runs code between building its payload and appending \
             the record: `{between}`. Either the worker that builds this payload \
             must run it too, or this domain cannot be converted.",
        );
    }

    // D41's wrapper delegates, and the delegate holds exactly one statement
    // between the payload and the record: the authority trailer.
    let wrapper = helper_body(&source, "fn append_compact_projects_data_record(");
    assert!(
        wrapper.contains("append_compact_projects_data_record_onto(out, sequence, game_hour_raw, read, payload);"),
        "D41's wrapper must hand its base payload to the shared `_onto` half, \
         so the moved path and the inline path cannot drift",
    );

    // EMPTY SPANS, NOT A STATEMENT COUNT. Counting lines that end in `;` misses
    // a second multi-line call (its opening line ends in `(` and its closing
    // line is `);`), and a semicolon test misses a block expression that needs
    // none. Requiring the spans on either side of the trailer to hold no code
    // at all has neither hole.
    const TRAILER: &str =
        "append_compact_project_authority_trailer(&mut payload, read, sequence, game_hour_raw);";
    let onto = helper_body(&source, "fn append_compact_projects_data_record_onto(");
    assert!(
        onto.contains(TRAILER),
        "the tail's half must stamp the authority trailer with the record's own \
         sequence",
    );
    let before = strip_code(code_between(onto, ") {", TRAILER));
    assert!(
        before.is_empty(),
        "the trailer must be the FIRST thing the tail's half does; found `{before}` \
         ahead of it, which would be digested into the trailer on one path and \
         not the other",
    );
    let after = strip_code(code_between(onto, TRAILER, "append_compact_record("));
    assert!(
        after.is_empty(),
        "nothing may run between the trailer and the record append; found `{after}`",
    );
}

/// The text between the end of `after` and the start of `before`.
fn code_between<'a>(body: &'a str, after: &str, before: &str) -> &'a str {
    let from = expect_at(body, after) + after.len();
    let to = expect_at(body, before);
    assert!(from <= to, "`{after}` must precede `{before}`");
    &body[from..to]
}

/// `text` with comments, blank lines and all whitespace removed, so "no code"
/// is a comparison against the empty string rather than against a spelling.
fn strip_code(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("")
        .split_whitespace()
        .collect()
}

/// The text between the end of a wrapper's `let payload = ..;` statement and
/// its `append_compact_record(` call.
///
/// The builder call spans lines and nests parentheses, so its statement ends at
/// the first `;` at paren depth zero — not at the first `;` in the text.
fn between_builder_and_record(body: &str) -> &str {
    let payload_at = expect_at(body, "let payload = ");
    let rest = &body[payload_at..];
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in rest.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => {
                end = Some(offset + 1);
                break;
            }
            _ => {}
        }
    }
    let end = payload_at + end.expect("the payload statement must be terminated");
    let record_at = expect_at(body, "append_compact_record(");
    assert!(
        end <= record_at,
        "the payload must be built before the record is appended",
    );
    &body[end..record_at]
}

/// A read no worker encoded carries no payload, and the tail's `None` arm is
/// therefore the path a panic substitute takes.
///
/// kills: a `PublishedRead::unencoded` that seeds `Some(Vec::new())`, which
/// would publish an EMPTY payload for a panicked domain instead of the
/// substitute's own encoded rows.
#[test]
fn a_panic_substitute_carries_no_worker_payload() {
    let published = PublishedRead::unencoded(compact_map_control_worker_panic_read(0));
    let (read, payload) = published.split();
    assert!(
        payload.is_none(),
        "a substitute no worker encoded must fall through to the tail's inline \
         encode",
    );
    assert!(
        compact_contract::compact_status_is_failure(read.status_flags),
        "the substitute must still be the failure-badged read the join built",
    );
}

/// The guarded block IS `spawn_gated`'s worker body, with two identifiers
/// renamed, and this compares the two regions line by line.
///
/// THIS IS THE CLAIM THE WHOLE HELPER RESTS ON. `spawn_gated` lets its
/// `ReaderClockBracketGuard` live to the end of the worker body, so the guard's
/// `Drop` takes the closing clock observation after the read AND after four
/// bookkeeping statements — and that observation decides whether the bracket
/// straddled a game-hour boundary. Anything the publishing helper's block does
/// that `spawn_gated` does not, INCLUDING building a value to hand out of it,
/// sits between the last statement and that observation; at a boundary it can
/// turn a bracket the pre-change code admitted into one it refuses, and the
/// frame then publishes a gap where the stream carried DATA.
///
/// A whitelist of permitted lines stood here and was not enough: it admitted
/// any line someone thought to add to it. An equality against the real
/// `spawn_gated` cannot be widened without widening `spawn_gated`.
///
/// kills: any statement added to or removed from the guarded block; a
/// reordering of the five; the encode hoisted into it; `drop(permit)` moved out
/// of it; a value constructed to be handed out of it; the guard narrowed to the
/// read alone; and — because the reference side is read out of `spawn_gated`
/// itself — a change to `spawn_gated` that this helper was not updated with.
#[test]
fn the_guarded_block_is_the_gated_worker_body_with_two_identifiers_renamed() {
    let source = super::tests::collector_source_lf();

    // Both regions run from the guard's construction to just before the block's
    // value, which is the bare move each ends with.
    let gated = region(
        helper_body(&source, "fn spawn_gated<"),
        "let _clock_guard = ReaderClockBracketGuard::new(lanes, tag);",
        "\n        out\n",
    );
    let publishing = region(
        helper_body(&source, "fn spawn_gated_publishing<"),
        "let _clock_guard = ReaderClockBracketGuard::new(lanes, tag);",
        "\n            read\n",
    );

    // `f` is `read` and `out` is `read`; `end` is `read_end`. Renaming the
    // publishing side INTO `spawn_gated`'s vocabulary is the whole
    // normalisation.
    //
    // COMPARED AS ONE TOKEN STREAM, not line by line, because the two regions
    // sit at different indents and rustfmt wraps the cycle fold's `if let` in
    // one and not the other. Line breaks are not statements; what this compares
    // is the sequence of tokens, comments and blank lines removed.
    let statements = |text: &str, rename: bool| -> String {
        let joined = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .collect::<Vec<_>>()
            .join(" ");
        let joined = if rename {
            joined
                .replace("let read = read();", "let out = f();")
                .replace("read_end", "end")
        } else {
            joined
        };
        joined.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let gated_statements = statements(gated, false);
    let publishing_statements = statements(publishing, true);
    assert!(
        gated_statements.contains("drop(permit);")
            && gated_statements.contains("lanes.record(")
            && gated_statements.contains("lanes.observe_worker_cycles("),
        "the reference region must hold `spawn_gated`'s whole worker body, not a \
         fragment; saw {gated_statements}",
    );
    assert_eq!(
        publishing_statements, gated_statements,
        "the guarded block must be `spawn_gated`'s worker body with `f`/`out` \
         spelled `read` and `end` spelled `read_end`, and nothing else",
    );
    // THE PREPARING HELPER IS HELD TO THE SAME BAR (lane PT, 2026-09-02): its
    // guarded block is the publishing helper's, so the bracket it closes is the
    // one `spawn_gated` would have closed.
    let preparing = region(
        helper_body(&source, "fn spawn_gated_preparing<"),
        "let _clock_guard = ReaderClockBracketGuard::new(lanes, tag);",
        "\n            read\n",
    );
    assert_eq!(
        statements(preparing, true),
        gated_statements,
        "the preparing helper's guarded block must be `spawn_gated`'s worker body \
         with the same two identifiers renamed, and nothing else",
    );
}

/// The encode is after the block, after the permit, and the split's boundary is
/// stamped between them.
///
/// kills: the encode hoisted above the block's close; the split's boundary
/// stamped before the guard drops, which would make `lw_d*_inb_us` less than
/// the bracket and `lw_d*_post_us` more than the encode; the encode's cycle
/// baseline taken inside the block again.
#[test]
fn the_publishing_spawn_encodes_after_the_bracket_and_after_the_permit() {
    let source = super::tests::collector_source_lf();
    let body = helper_body(&source, "fn spawn_gated_publishing<");

    let permit_at = expect_at(body, "drop(permit);");
    let tail_at = expect_at(body, "\n            read\n");
    let closed_at = expect_at(body, "let bracket_closed = lanes.origin.elapsed();");
    let baseline_at = expect_at(body, "let cycles_before_encode = current_thread_cycle_counter();");
    let encode_at = expect_at(body, "let payload = std::panic::catch_unwind(");
    let split_at = expect_at(body, "lanes.record_bracket_split(");

    assert!(
        permit_at < tail_at,
        "the permit must be released inside the guarded block, or the encode \
         runs under it and a queued reader waits on arithmetic",
    );
    assert!(
        tail_at < closed_at && closed_at < baseline_at && baseline_at < encode_at,
        "the bracket must close, then the split's boundary and the encode's \
         cycle baseline are stamped, then the encode runs",
    );
    assert!(
        encode_at < split_at,
        "the split is published after the encode it prices",
    );
    assert!(
        body.contains(
            "let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(&read))).ok();"
        ),
        "the encode must be caught and reduced to an Option, or a latent bug in \
         it changes which record the frame seals",
    );
}

/// The span between `anchor` and `until`, exclusive of `until`.
fn region<'a>(body: &'a str, anchor: &str, until: &str) -> &'a str {
    let from = expect_at(body, anchor);
    let to = expect_at(body, until);
    assert!(from < to, "`{anchor}` must precede `{until}`");
    &body[from..to]
}

/// A panic inside the moved encode lands where it landed before the move.
///
/// Left to unwind, a panicking encode would be caught by the scoped join, the
/// domain's `..._worker_panic_read` would be substituted, and the frame would
/// PUBLISH a badged record — where the pre-change code let the same bug unwind
/// the main thread mid-publish and take the frame with it. That is a change in
/// what gets sealed, so the helper swallows the unwind and leaves the payload
/// absent; the tail then encodes inline and the bug reaches the main thread at
/// the statement it always reached.
///
/// kills: deleting the `catch_unwind`; replacing `.ok()` with an `unwrap`;
/// seeding the caught case with `Some(Vec::new())`, which would publish an
/// EMPTY payload for the panicking domain instead of re-running the encode.
#[test]
fn a_panicking_encode_leaves_no_payload_so_the_tail_re_runs_it() {
    let source = super::tests::collector_source_lf();
    let body = helper_body(&source, "fn spawn_gated_publishing<");
    assert!(
        body.contains(
            "let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(&read))).ok();"
        ),
        "the encode must be caught and reduced to an Option, or a latent bug in \
         it changes which record the frame seals",
    );

    // AND THE SHAPE IS REAL, not just present in the source: a payload that is
    // `None` sends the tail down the arm that encodes inline.
    let published: PublishedRead<u8> = PublishedRead {
        read: 7,
        payload: None,
    };
    let (read, payload) = published.split();
    assert_eq!(read, 7);
    assert!(
        payload.is_none(),
        "an absent payload is what makes the tail re-run the encode",
    );
}

/// Production is wired through the helper, and each converted domain hands the
/// worker exactly the arguments its tail would have handed the same builder.
///
/// The byte-identity test above compares two encodes; it says nothing about
/// whether production performs either of them, nor about whether the two
/// production sites agree. This says both: it reads the spawn site's encode
/// closure and the tail's inline arm out of the collector's own source and
/// requires the SEQUENCE of `read.<field>` mentions to match.
///
/// kills: a spawn site restored to `spawn_gated` (the closure would still
/// compile and the frame would still publish, encoding in the tail again); a
/// tail block that drops the `published_payload` arm and always encodes inline;
/// a spawn site whose encode closure names a different builder from the one its
/// tail falls back to; a spawn site whose closure swaps, drops or duplicates
/// one of the builder's `read.<field>` arguments.
#[test]
fn the_seven_converted_domains_are_wired_through_the_publishing_helper() {
    let source = super::tests::collector_source_lf();
    let body = frame_builder_body(&source);

    // (handle binding, the payload builder both paths must name, the read
    //  binding the tail matches on, the domain constant the tail publishes)
    const WIRED: &[(&str, &str, &str, &str)] = &[
        (
            "let air_combat_losses_live_handle =",
            "compact_air_strength_deltas_data_payload(",
            "air_combat_losses_live_read",
            "COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE",
        ),
        (
            "let supply_logistics_attrition_live_handle =",
            "compact_supply_logistics_attrition_data_payload(",
            "supply_logistics_attrition_live_read",
            "COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE",
        ),
        (
            "let construction_buildings_state_damage_live_handle =",
            "domain48::encode_lane_frame(",
            "construction_buildings_state_damage_live_read",
            "COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE",
        ),
        (
            "let projects_handle =",
            "compact_projects_data_payload(",
            "projects_read",
            "COMPACT_DOMAIN_PROJECTS",
        ),
        (
            "let map_detail_handle =",
            "compact_map_detail_data_payload(",
            "map_detail_read",
            "COMPACT_DOMAIN_MAP_DETAIL",
        ),
        (
            "let weather_handle =",
            "compact_weather_data_payload(",
            "weather_read",
            "COMPACT_DOMAIN_WEATHER",
        ),
        (
            "let map_control_handle =",
            "compact_map_control_data_payload(",
            "map_control_read",
            "COMPACT_DOMAIN_MAP_CONTROL",
        ),
    ];

    for (handle, builder, read_binding, domain) in WIRED {
        let spawn = balanced_after(body, handle, '(', ')')
            .unwrap_or_else(|| panic!("{handle} must still be one balanced call"));
        assert!(
            spawn.contains("spawn_gated_publishing("),
            "{handle} must spawn through the publishing helper, or its encode is \
             back on the frame's serial tail",
        );
        assert!(
            spawn.contains(*builder),
            "{handle} must name {builder} at the spawn site, so the worker and \
             the tail fall back to the same builder",
        );

        let opener = format!("    if let Some(published) = {read_binding} {{");
        let block_at = body.find(&opener).unwrap_or_else(|| {
            panic!("{read_binding} must still be published from a `published` binding")
        });
        let block_end = body[block_at..]
            .find(BLOCK_CLOSE)
            .map(|at| block_at + at)
            .unwrap_or(body.len());
        let block = &body[block_at..block_end];
        assert!(
            block.contains("let (read, published_payload) = published.split();"),
            "{read_binding}'s block must take the worker's payload",
        );

        if *read_binding == "projects_read" {
            // D41 keeps the sequence-stamped trailer in the tail, so its arm is
            // the shared `_onto` helper rather than a `match`, and both paths
            // reach the SAME builder call in the tail's own text -- there is no
            // second argument list to compare.
            assert!(
                block.contains("append_compact_projects_data_record_onto(")
                    && block.contains("published_payload"),
                "projects must finish the worker's base payload in the tail",
            );
            continue;
        }

        assert!(
            block.contains("Some(payload) => append_compact_record(")
                && block.contains("&payload,")
                && block.contains(*domain)
                && block.contains("COMPACT_RECORD_TYPE_DOMAIN_DATA"),
            "{read_binding}'s block must publish the worker's payload verbatim \
             under {domain}",
        );

        // THE TWO ARGUMENT LISTS, COMPARED. The spawn site's closure and the
        // tail's inline arm hand the same builder its inputs; if they ever
        // disagree the worker publishes one thing and a panic substitute
        // publishes another, and no byte comparison over hand-written
        // arguments would notice.
        let inline_at = block
            .find("None => ")
            .unwrap_or_else(|| panic!("{read_binding} must keep an inline fallback arm"));
        let worker_fields = read_field_sequence(spawn);
        let inline_fields = read_field_sequence(&block[inline_at..]);
        assert_eq!(
            worker_fields, inline_fields,
            "{read_binding}: the worker's encode and the tail's inline encode \
             must hand the builder the same fields in the same order",
        );
        if worker_fields.is_empty() {
            // A builder that takes the WHOLE read has no field arguments, so
            // the equality above holds vacuously for it. What has to match
            // instead is that both sites hand it the read itself rather than
            // something derived from it.
            assert!(
                spawn.contains("(read, elapsed_wall_ms)")
                    && block[inline_at..].contains("&read,"),
                "{read_binding}: a builder that takes the whole read must be \
                 given the whole read on both paths",
            );
        }
    }
}

/// Every `read.<field>` mention in `text`, in order.
///
/// Deliberately field NAMES and not the whole expression: the two sites differ
/// in indentation and in the `&` / `!` decorations Rust needs at each, and the
/// defect this catches is a swapped, dropped or duplicated FIELD.
fn read_field_sequence(text: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let bytes = text.as_bytes();
    let mut from = 0usize;
    while let Some(hit) = text[from..].find("read.") {
        let at = from + hit;
        from = at + "read.".len();
        // Word boundary before `read`, so `published_payload.read.` or an
        // identifier ending in `read` is not mistaken for the binding.
        if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
            continue;
        }
        let rest = &text[from..];
        let end = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            fields.push(&rest[..end]);
        }
    }
    fields
}

/// The frame-builder body, from its signature to the next item at column zero.
fn frame_builder_body(source: &str) -> &str {
    helper_body(source, "fn build_compact_extraction_gate_records(")
}

/// One function's source text, from its signature to the next item at column
/// zero.
fn helper_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start + 1..]
        .find(NEXT_ITEM)
        .map(|at| start + 1 + at)
        .unwrap_or(source.len());
    &source[start..end]
}

/// The first balanced `(..)` group after `anchor`, or `None`.
fn balanced_after<'a>(body: &'a str, anchor: &str, open: char, close: char) -> Option<&'a str> {
    let at = body.find(anchor)?;
    let rest = &body[at..];
    let first = rest.find(open)?;
    let mut depth = 0i32;
    for (offset, ch) in rest[first..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(&rest[first..first + offset + ch.len_utf8()]);
            }
        }
    }
    None
}

fn expect_at(body: &str, needle: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("the helper must still contain `{needle}`"))
}
