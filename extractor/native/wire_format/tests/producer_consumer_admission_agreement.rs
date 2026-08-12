//! The producer half of the cross-workspace admission-agreement gate.
//!
//! # What this pins and why
//!
//! `extractor/native` and `replay_interface/native` are separate cargo
//! workspaces with no dependency on each other, so an admission rule that both
//! sides apply has to be written twice. Twice that has silently cost data: a
//! lane widened a producer-side rule, the rows were then collected correctly,
//! and the replay side re-applied the identical superseded rejection and threw
//! them away. The producer-only fix "worked" and produced nothing.
//!
//! `portable_admission` is the crate BOTH workspaces path-depend on, so it can
//! hold the value once. Some producer-side copies can be pointed straight at it
//! and now are. The copies pinned HERE are the ones that cannot be: they live
//! in `compact_contract`, which is deliberately kept to serde_json + sha2 and
//! whose dependency surface is itself policy
//! (`tests/compact_contract_dependency_policy.rs`). Taking a new path
//! dependency there to share a constant would be a policy change, not an
//! incidental import.
//!
//! So for those, the mechanism is this test rather than the type system: the
//! value stays written twice, and any edit that makes the two spellings
//! disagree fails here by name instead of silently discarding rows at replay.
//!
//! The consumer half is
//! `replay_interface/native/replay_core/src/world.rs`'s
//! `producer_consumer_admission_agreement` module. Both halves must exist:
//! a rule enforced on one side and not its sibling is not enforced.

/// `compact_contract::calendar` is the extractor's canonical calendar. It
/// cannot depend on `portable_admission` (see above), so its agreement with the
/// shared window is asserted instead of compiled in.
#[test]
fn compact_contract_calendar_agrees_with_the_shared_admission_window() {
    assert_eq!(
        compact_contract::calendar::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "compact_contract::calendar::DATE_EPOCH_CONST drifted from \
         portable_admission::DATE_EPOCH_CONST. The replay consumer converts \
         raw dates with the shared value; a producer that stamps hours with a \
         different epoch has every row rejected as an out-of-window date."
    );

    // Agreement on the endpoints is not enough: the two copies must also agree
    // on whether each endpoint is INSIDE the window. A copy that spells the
    // upper comparison `>=` instead of `>` rejects exactly one value that every
    // other copy admits, and nothing else would notice.
    for probe in [
        portable_admission::RAW_DATE_MIN - 1,
        portable_admission::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN + 1,
        portable_admission::RAW_DATE_MAX - 1,
        portable_admission::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX + 1,
    ] {
        let producer = compact_contract::calendar::hour_index_from_raw(probe);
        let shared = portable_admission::hour_index_from_raw(probe);
        assert_eq!(
            producer, shared,
            "compact_contract::calendar::hour_index_from_raw and \
             portable_admission::hour_index_from_raw disagree at raw date \
             {probe}. One side admits an hour the other discards."
        );
    }
}

/// `wire_format::d67` carries its own copy of the same window, with source
/// comments citing `compact_contract/src/calendar.rs` as its authority.
#[test]
fn d67_raw_date_window_agrees_with_the_shared_admission_window() {
    assert_eq!(
        wire_format::d67::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "wire_format::d67::DATE_EPOCH_CONST drifted from the shared epoch"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN,
        "wire_format::d67::RAW_DATE_MIN drifted from the shared window"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX,
        "wire_format::d67::RAW_DATE_MAX drifted from the shared window"
    );
}

/// The owner-identity bound. `compact_contract::build_profile` is the producer
/// authority; `replay_core` re-applies the same ceiling when it reads the rows
/// back, and now takes it from `portable_admission`.
#[test]
fn compact_owner_tag_db_id_bound_agrees_with_the_shared_bound() {
    assert_eq!(
        compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
        "compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID drifted \
         from portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID. Raising it here \
         alone means the extra owners are collected and then discarded by the \
         replay consumer, which still applies the shared ceiling."
    );
}

/// The bound's DERIVATION, not just its value.
///
/// `MAX_COMPACT_OWNER_TAG_DB_ID` is not a number somebody picked: it is the
/// `countrydatabase.cpp:100`) admits an id iff `-1 < i && i < size`, and both
/// the index and `size` are signed 32-bit ints, so a loaded database's size is
/// at most `i32::MAX` and its largest valid id is `i32::MAX - 1`. Both
/// workspaces must agree on the SIZE constant too, or the derivation is
/// re-stated rather than shared.
///
/// kills: redefining either `MAX_STRUCTURAL_TAG_DB_SIZE` away from the engine's
/// signed index type, or letting the two workspaces' copies of it drift.
#[test]
fn the_structural_tag_db_size_is_the_engines_signed_index_type_on_both_sides() {
    assert_eq!(
        compact_contract::country_tag_db::MAX_STRUCTURAL_TAG_DB_SIZE,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        "the producer and the shared crate disagree about the width of the \
         engine's country-database index type"
    );
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        i32::MAX as u32,
        "the engine's accessor compares a signed int index against a signed \
         int size"
    );
    // `id < size` and `size <= i32::MAX`, so the largest admissible id is
    // exactly one below the largest admissible size. One higher admits a word
    // no loaded database can index; one lower discards the last country of a
    // database that fills the id space.
    assert_eq!(
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE
    );
    assert_eq!(
        portable_admission::MAX_COUNTRY_TABLE_ROWS,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        "a country ROW COUNT is bounded by the size, not by the size minus one"
    );
    assert_eq!(
        compact_contract::build_profile::MAX_COUNTRY_TABLE_ROWS,
        portable_admission::MAX_COUNTRY_TABLE_ROWS
    );
}

/// EVERY consumer that re-applies a country tag-db-id ceiling must apply the
/// SAME one.
///
/// This is the province family's `every_province_and_state_consumer_applies_the_shared_envelope`
/// for the tag-db-id family, and it exists for the same reason: the producer
/// bounds a `tag_db_id` by the live country-database size
/// (`country_tag_db::tag_db_id_in_table`), so on an overhaul it legitimately
/// emits ids far above the vanilla country count — and every one of the
/// consumers below used to refuse them at 1_024, 2_048 or 4_096, AFTER the
/// recorder had collected them correctly. A producer-widened /
/// consumer-still-narrow split is invisible on the producer side, which is why
/// it needs a test rather than a review.
///
/// Pinned as an equality against the shared bound AND against the literal: the
/// equality catches one spelling drifting, the literal catches all of them
/// moving together to a number nobody chose.
///
/// kills: reverting any listed constant to 1_024 / 2_048 / 4_096 (or to any
/// other value than the shared engine-structural bound).
#[test]
fn every_country_identity_consumer_applies_the_shared_tag_db_bound() {
    for (name, value) in [
        ("wire_format::d46::ace::MAX_TAG_DB_ID", wire_format::d46::ace::MAX_TAG_DB_ID),
        ("wire_format::d48::MAX_COMPACT_OWNER_TAG_DB_ID", wire_format::d48::MAX_COMPACT_OWNER_TAG_DB_ID),
        ("wire_format::d50::MAX_OWNER_TAG_DB_ID", wire_format::d50::MAX_OWNER_TAG_DB_ID),
        ("wire_format::d51::MAX_COUNTRY_TAG_DB_ID", wire_format::d51::MAX_COUNTRY_TAG_DB_ID),
        ("wire_format::d56::MAX_OWNER_TAG_DB_ID", wire_format::d56::MAX_OWNER_TAG_DB_ID),
        ("wire_format::d62::MAX_COUNTRY_COUNT", wire_format::d62::MAX_COUNTRY_COUNT),
        ("wire_format::d67::MAX_OWNER_TAG_DB_ID", wire_format::d67::MAX_OWNER_TAG_DB_ID),
        ("wire_format::d68::MAX_PLAYER_TAG_DB_ID", wire_format::d68::MAX_PLAYER_TAG_DB_ID),
        (
            "wire_format::d11::MAX_PRODUCTION_LICENSE_TAG_DB_ID",
            wire_format::d11::MAX_PRODUCTION_LICENSE_TAG_DB_ID,
        ),
        ("wire_format::d94::MAX_POV_COUNTRY_ID", wire_format::d94::MAX_POV_COUNTRY_ID),
        ("wire_format::d95::MAX_POV_COUNTRY_ID", wire_format::d95::MAX_POV_COUNTRY_ID),
        (
            "compact_contract::pov_visibility::MAX_POV_COUNTRY_ID",
            compact_contract::pov_visibility::MAX_POV_COUNTRY_ID,
        ),
        (
            "compact_contract::spotted_units::MAX_POV_COUNTRY_ID",
            compact_contract::spotted_units::MAX_POV_COUNTRY_ID,
        ),
        (
            "compact_contract::raid_instances::MAX_TAG_DB_ID",
            compact_contract::raid_instances::MAX_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN66_MAX_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN66_MAX_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN68_MAX_PLAYER_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN68_MAX_PLAYER_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN69_MAX_MEMBER_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN69_MAX_MEMBER_TAG_DB_ID,
        ),
        // DOMAIN53_MAX_COUNTRY_INDEX is deliberately ABSENT. It holds the same
        // value, but this list asserts that its members apply the country
        // DATABASE's bound, and nothing establishes that d53's CID/faction-group
        // words are database ids -- see its own doc, and residual R-GV-8. Listing
        // it here would launder a representation bound into a proven one.
    ] {
        assert_eq!(
            value,
            portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
            "{name} does not apply the shared country-identity bound. A \
             consumer narrower than the producer discards rows the recorder \
             collected, and nothing on the producer side reports it."
        );
    }
    assert_eq!(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID, 2_147_483_646);
}

/// `wire_format::d52`'s ceiling is the `_CountryLinkTags` index space, NOT the
/// country database's, and the two must not be collapsed even though they
/// currently carry the same width.
///
/// They are the same NUMBER for the same REASON — both vectors are subscripted
/// by the engine with a signed 32-bit index — and they are still different
/// QUESTIONS, with different live sizes (`*(int32*)(db + 0x0C)` versus
/// `u32[CGameState + 0x364]`) and different bound-trip attributions
/// (`BOUND_TRIP_COUNTRY_LINK_INDEX` versus the database's own). d52 is
/// therefore deliberately absent from the list above.
///
/// kills: narrowing `wire_format::d52::MAX_TAG_DB_ID` back to a content-scale
/// ceiling, or widening it past the signed subscript the link vector's own size
/// assert establishes.
///
/// NOT killed by this test, and deliberately so: re-spelling d52's constant as
/// `portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID` would pass, because the
/// two currently hold the same value for parallel reasons. A numeric assertion
/// cannot see a provenance change, and claiming otherwise would be the vacuous
/// half of a `kills:` line. What holds that contract is the source itself: d52
/// spells its own derivation, and `wire_format` deliberately does not import
/// `compact_contract` in library code, a boundary
/// `tests/compact_contract_dependency_policy.rs` enforces mechanically.
#[test]
fn the_link_vector_index_bound_is_derived_separately_from_the_database_bound() {
    assert_eq!(
        wire_format::d52::MAX_TAG_DB_ID,
        (i32::MAX as u32) - 1,
        "the link vector's own structural width: a signed 32-bit subscript"
    );
}

/// The boundary the family turns on, exercised through a real consumer's
/// admission path rather than only against the constants.
///
/// d94 is the walk: its POV country id is a plain country-database id, its
/// header parser is the admission gate, and the three probes are the OLD bound
/// (2_048, which must still be admitted), the FIRST id the old bound discarded
/// (2_049 — an ordinary country tag in an overhaul, and the whole defect), the
/// NEW bound, and ONE PAST it (which must be REFUSED, never truncated or
/// clamped to something in range).
///
/// kills: reverting `d94::MAX_POV_COUNTRY_ID` to a content-scale ceiling, and
/// replacing d94's `pov_country_id > MAX_POV_COUNTRY_ID` refusal with a clamp
/// or a saturating cast.
#[test]
fn a_consumer_admission_path_walks_the_old_bound_the_new_bound_and_one_past() {
    let admitted = |pov: u32| wire_format::d94::MAX_POV_COUNTRY_ID >= pov && pov != 0;

    assert!(admitted(2_048), "the old ceiling is still an ordinary id");
    assert!(
        admitted(2_049),
        "the first id the old ceiling discarded is an ordinary country tag"
    );
    assert!(admitted(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID));
    assert!(
        !admitted(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1),
        "one past the ceiling must be refused, never truncated"
    );
    assert!(
        !admitted(portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE),
        "no loaded database can index its own size"
    );
    assert!(!admitted(0x8000_0000), "a negative index to the engine's accessor");
    assert!(!admitted(u32::MAX));

    // The refusal is the CONSUMER's, byte for byte: a real FOW1 payload whose
    // POV sits one past the ceiling is rejected by the shipping header
    // grammar, and one carrying the ceiling is admitted. This is the half a
    // comparison against the constant cannot show — it is what proves the
    // widened bound is actually reachable through the decode path, and that
    // overflow REFUSES rather than truncating the word into range.
    for admitted_pov in [1u32, 2_048, 2_049, portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID] {
        let payload = fow1_payload(admitted_pov, &[0, 1, 2]);
        let (header, levels) = wire_format::d94::split_payload(&payload)
            .unwrap_or_else(|reason| panic!("d94 refused pov {admitted_pov}: {reason}"));
        assert_eq!(
            header.pov_country_id, admitted_pov,
            "the POV must survive the decode unchanged, never clamped"
        );
        assert_eq!(levels.len(), 3);
    }
    for refused_pov in [
        0u32,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        0x8000_0000,
        u32::MAX,
    ] {
        assert!(
            wire_format::d94::split_payload(&fow1_payload(refused_pov, &[0, 1, 2])).is_err(),
            "d94 admitted pov_country_id {refused_pov}"
        );
    }
}

/// Every country-ROW ceiling the wave widened must admit a roster bigger than
/// the vanilla-plus-DLC guess, and must still refuse a count the payload cannot
/// contain.
///
/// The count ceilings are a different question from the id ceilings above: they
/// bound how many countries a payload carries, not which ids it may name, and
/// several of them used to be the only thing standing between a stream-supplied
/// `u32` and a `Vec::with_capacity`. Widening them to the structural row width
/// is only safe because each decoder now derives its real bound from the
/// payload, so that derivation is what this pins.
///
/// kills: reverting `d92::MAX_ROWS` or `d96::MAX_COUNTRIES` to a content-scale
/// ceiling, and REMOVING either decoder's payload-capacity check — without it a
/// four-byte word decides the allocation, which is the regression the widening
/// would otherwise introduce.
#[test]
fn the_country_row_ceilings_admit_an_overhaul_and_still_refuse_an_impossible_count() {
    // The roster carriers no longer cap a country count at vanilla scale.
    assert!(wire_format::d92::MAX_ROWS > 100_000);
    assert!(wire_format::d96::MAX_COUNTRIES > 100_000);
    assert_eq!(
        wire_format::d92::MAX_ROWS,
        compact_contract::preferred_tactic::MAX_ROWS,
        "the wire mirror and the shipping contract must agree"
    );
    assert_eq!(
        wire_format::d96::MAX_COUNTRIES,
        compact_contract::tactic_weight::MAX_COUNTRIES
    );

    // What actually bounds the allocation is the payload. A row is at least
    // `FIXED_ROW_LEN` bytes (d96's rows also carry a token of at least one),
    // so the capacity is exact and testable rather than a guess.
    assert_eq!(wire_format::d92::max_rows_in(0), 0);
    assert_eq!(
        wire_format::d92::max_rows_in(wire_format::d92::FIXED_ROW_LEN - 1),
        0
    );
    assert_eq!(
        wire_format::d92::max_rows_in(wire_format::d92::FIXED_ROW_LEN * 1_025),
        1_025,
        "an overhaul-sized roster fits when the bytes are actually there"
    );
    assert_eq!(
        wire_format::d96::max_rows_in((wire_format::d96::FIXED_ROW_LEN + 1) * 1_025),
        1_025
    );
    // And the two mirrors agree with the shipping contracts they restate.
    assert_eq!(
        wire_format::d92::max_rows_in(12_000),
        compact_contract::preferred_tactic::max_rows_in(12_000)
    );
    assert_eq!(
        wire_format::d96::max_rows_in(12_000),
        compact_contract::tactic_weight::max_rows_in(12_000)
    );

    // The refusal itself, through the real decoders. Everything above compares
    // constants and helper arithmetic, and none of it would fail if the
    // production capacity check were deleted — which is precisely the vacuity a
    // `kills:` line must not claim its way out of. These two payloads are
    // well-formed headers declaring a row count the body cannot possibly
    // satisfy, and each must be refused BY NAME rather than allocated for.
    let d92_refusal = wire_format::d92::split_payload(&ptc1_header_declaring(1))
        .expect_err("a PTC1 header declaring a row it has no bytes for must be refused");
    assert!(
        d92_refusal.contains("exceeds the payload's capacity"),
        "d92 refused for the wrong reason: {d92_refusal}"
    );
    // A zero-row header is well-formed and must still be admitted, so the
    // check is a capacity rule and not a blanket refusal of empty bodies.
    assert!(
        wire_format::d92::split_payload(&ptc1_header_declaring(0)).is_ok(),
        "a PTC1 header declaring no rows is ordinary"
    );

    // The same walk through d96. The `kills:` line above names BOTH decoders,
    // so both have to be reachable from here: exercising one and asserting
    // constants for the other is what let the 512-row bridge ceiling next door
    // survive three rounds.
    let d96_refusal = wire_format::d96::split_payload(&ptw1_header_declaring(1))
        .expect_err("a PTW1 header declaring a row it has no bytes for must be refused");
    assert!(
        d96_refusal.contains("exceeds the payload's capacity"),
        "d96 refused for the wrong reason: {d96_refusal}"
    );
    if let Err(reason) = wire_format::d96::split_payload(&ptw1_header_declaring(0)) {
        panic!("a PTW1 header declaring no rows is ordinary, got: {reason}");
    }
}

/// A syntactically valid header-only PTW1 payload declaring `row_count` rows
/// and carrying no body at all.
///
/// `observed_country_count` is `max(row_count, 1)`: the roster must cover the
/// declared rows, and a clean PTW1 snapshot must witness a non-empty roster, so
/// this satisfies both rules and leaves the capacity check as the only thing
/// that can refuse a header declaring rows it has no bytes for.
fn ptw1_header_declaring(row_count: u32) -> Vec<u8> {
    use wire_format::d96;
    let mut out = Vec::with_capacity(d96::HEADER_LEN);
    out.extend_from_slice(&0u64.to_le_bytes()); // read_time_us
    out.extend_from_slice(&row_count.max(1).to_le_bytes()); // observed_country_count
    out.extend_from_slice(&row_count.to_le_bytes()); // row_count
    out.extend_from_slice(&0u32.to_le_bytes()); // status_flags
    out.extend_from_slice(&d96::MAGIC);
    out.extend_from_slice(&d96::VERSION.to_le_bytes());
    out.extend_from_slice(&(d96::HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes()); // source_sequence
    out.extend_from_slice(&(d96::FIXED_SCALE as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&0i64.to_le_bytes()); // country_factor_raw
    out.extend_from_slice(&d96::EXECUTABLE_SHA256);
    out.extend_from_slice(&d96::CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::DEFINE_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::LOOKUP_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::RUNTIME_OBSERVATION_SHA256);
    out.extend_from_slice(&d96::TACTIC_KEY_CENSUS_SHA256);
    out.extend_from_slice(&d96::NONZERO_OVERRIDE_OBSERVATION_SHA256);
    assert_eq!(out.len(), d96::HEADER_LEN, "the fixture must build a real header");
    out
}

/// A syntactically valid header-only PTC1 payload declaring `row_count` rows
/// and carrying no body at all.
fn ptc1_header_declaring(row_count: u32) -> Vec<u8> {
    use wire_format::d92;
    let mut out = Vec::with_capacity(d92::HEADER_LEN);
    out.extend_from_slice(&0u64.to_le_bytes()); // read_time_us
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // selected_count
    out.extend_from_slice(&0u32.to_le_bytes()); // status_flags
    out.extend_from_slice(&d92::MAGIC);
    out.extend_from_slice(&d92::VERSION.to_le_bytes());
    out.extend_from_slice(&(d92::HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes()); // source_sequence
    out.extend_from_slice(&d92::EXECUTABLE_SHA256);
    out.extend_from_slice(&d92::CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&d92::WRITER_EVIDENCE_SHA256);
    out.extend_from_slice(&d92::IDENTITY_EVIDENCE_SHA256);
    assert_eq!(out.len(), d92::HEADER_LEN, "the fixture must build a real header");
    out
}

/// A raw FOW1 payload, packed the way `compact_contract::pov_visibility`'s
/// encoder packs one: magic, version, flags, capture sequence, POV country id,
/// province count, then one byte per province.
fn fow1_payload(pov_country_id: u32, levels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(compact_contract::pov_visibility::HEADER_LEN + levels.len());
    out.extend_from_slice(&compact_contract::pov_visibility::CARRIER_MAGIC);
    out.extend_from_slice(&compact_contract::pov_visibility::CARRIER_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes());
    out.extend_from_slice(&pov_country_id.to_le_bytes());
    out.extend_from_slice(&(levels.len() as u32).to_le_bytes());
    out.extend_from_slice(levels);
    out
}

/// The shared crate's OWN invariants, asserted from a suite that runs.
///
/// `portable_replay/native/admission` is a path dependency of both workspaces
/// and a member of neither, so `cargo nextest run --workspace` compiles it and
/// never runs its `#[cfg(test)]` module. Its boundary tests are therefore dead
/// where they stand -- which is a poor place for the one gate that decides
/// whether a row survives the trip from the recorder to replay. These
/// assertions restate the same contract inside the extractor suite.
///
/// The values are pinned ABSOLUTELY. A test that spells a bound as the
/// constant it is testing passes for every value the constant could take,
/// which is precisely the change it exists to catch: the number itself is the
/// cross-workspace contract.
#[test]
fn the_shared_admission_bounds_hold_their_exact_documented_values() {
    assert_eq!(portable_admission::DATE_EPOCH_CONST, 43_799_999);
    assert_eq!(portable_admission::RAW_DATE_MIN, 60_005_999);
    assert_eq!(portable_admission::RAW_DATE_MAX, 63_071_999);
    assert_eq!(portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE, 2_147_483_647);
    assert_eq!(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID, 2_147_483_646);
    assert_eq!(portable_admission::MAX_COUNTRY_TABLE_ROWS, 2_147_483_647);
    assert_eq!(portable_admission::SUPPLY_MAX_PROVINCE_ID, 65_535);
    assert_eq!(
        portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID,
        100_000
    );
    assert_eq!(portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_EVENTS, 65_536);
    assert_eq!(portable_admission::MAX_FRD_DIRECT_TRANSITIONS, 1_000_000);
}

/// The supply family's province ceiling and the shared build profile's must be
/// the SAME envelope.
///
/// They are two spellings of one question -- "what is the largest province id
/// this build can represent" -- reached by two paths that cannot import each
/// other: `portable_admission` sits below `compact_contract` and is depended on
/// by a second workspace, so nothing in the type system makes them agree. They
/// disagreed for as long as the supply one was the vanilla province count, and
/// domain 44 dropped rows domain 30's construction lane admitted from the same
/// map.
///
/// Pinned as an equality AND to the literal: the equality catches a drift, and
/// the literal catches both moving together to a number nobody chose.
#[test]
fn the_supply_province_ceiling_is_the_shared_province_envelope() {
    assert_eq!(
        portable_admission::SUPPLY_MAX_PROVINCE_ID,
        compact_contract::build_profile::MAX_PROVINCE_ID,
        "one build cannot have two largest province ids"
    );
    assert_eq!(compact_contract::build_profile::MAX_PROVINCE_ID, 65_535);
}

/// Every consumer that re-applies a province-id ceiling must apply the SAME
/// envelope.
///
/// The two pinned here were left behind when the producer stopped bounding by
/// the vanilla province count: `d34` still refused above 13 413 — the vanilla
/// count itself, spelled out — and `d62` above 20 000. Both are consumers of
/// rows the recorder had already collected, so the loss was invisible on the
/// producer side and a producer-only widening "worked" and produced nothing.
///
/// The assertion is an equality against the shared bound AND against the
/// literal: the equality catches one spelling drifting, the literal catches all
/// of them moving together to a number nobody chose.
///
/// kills: reverting `wire_format::d34::MAX_PROVINCE_ID` to 13_413, or
/// `wire_format::d62::MAX_PROVINCE_ID` / `::MAX_STATE_ID` to 20_000 (or to any
/// other value than the shared envelope).
#[test]
fn every_province_and_state_consumer_applies_the_shared_envelope() {
    for (name, value) in [
        ("wire_format::d34::MAX_PROVINCE_ID", wire_format::d34::MAX_PROVINCE_ID),
        ("wire_format::d48::MAX_PROVINCE_ID", wire_format::d48::MAX_PROVINCE_ID),
        ("wire_format::d54::MAX_PROVINCE_ID", wire_format::d54::MAX_PROVINCE_ID),
        ("wire_format::d62::MAX_PROVINCE_ID", wire_format::d62::MAX_PROVINCE_ID),
    ] {
        assert_eq!(
            value,
            portable_admission::SUPPLY_MAX_PROVINCE_ID,
            "{name} does not apply the shared province envelope. A consumer \
             narrower than the producer discards rows the recorder collected, \
             and nothing on the producer side reports it."
        );
    }
    assert_eq!(
        wire_format::d62::MAX_STATE_ID,
        compact_contract::build_profile::MAX_STATE_ID,
        "wire_format::d62::MAX_STATE_ID does not apply the shared state envelope"
    );
    assert_eq!(compact_contract::build_profile::MAX_STATE_ID, 65_535);
}

/// The calendar window is closed at BOTH ends. A copy that spells the upper
/// comparison `>=` rejects exactly one value every other copy admits, and
/// nothing else would notice.
#[test]
fn the_shared_calendar_window_is_inclusive_at_both_ends() {
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN - 1),
        None
    );
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN).is_some());
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX).is_some());
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX + 1),
        None
    );
    assert_eq!(portable_admission::hour_index_from_raw(0), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MIN), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MAX), None);
}
