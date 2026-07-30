//! `wire_format::d99` transcribes the DCS1 grammar rather than importing it, so
//! this is the gate that keeps the transcription and the production carrier from
//! drifting apart.
//!
//! The module itself may not depend on `compact_contract` — that is the
//! `compact_contract_dependency_policy` boundary, which only scans `src`. An
//! integration test can, and this one does exactly one thing with that access:
//! encode with the REAL production encoder and decode with the transcription,
//! then compare field for field.
//!
//! The `@lane-payload-authority` marker in `src/d99.rs` pins the original
//! decoder's source span and digest. That catches a change to the authority.
//! This catches a transcription that is wrong today.

use compact_contract::division_combat_stats as carrier;
use wire_format::d99;

const STATS: carrier::CachedStatBlock = carrier::CachedStatBlock {
    defense_raw: 4_200_000,
    breakthrough_raw: 1_700_000,
    hardness_raw: 30_000,
    soft_attack_raw: 5_500_000,
    hard_attack_raw: 900_000,
};
const SCALE: carrier::ScaleFields = carrier::ScaleFields {
    attack_accumulator_raw: 120_000,
    defence_accumulator_raw: 90_000,
    attack_base_raw: 110_000,
    defence_base_raw: 130_000,
};

fn carrier_row(
    division_id: u32,
    side: u32,
    observation: carrier::RowObservation,
) -> carrier::DivisionCombatStatsRow {
    let after = if observation == carrier::RowObservation::TornRead {
        7
    } else {
        6
    };
    carrier::DivisionCombatStatsRow {
        combat_id: 7,
        generation: 3,
        side,
        division_id,
        observation,
        stats: observation.carries_stats().then_some(STATS),
        scale: observation.carries_scale().then_some(SCALE),
        combat_side_bonus_raw: observation.carries_scale().then_some(15_000),
        modifier_count_before: 6,
        modifier_count_after: after,
    }
}

/// Law: every grammar constant d99 restates equals the carrier's.
///
/// d99 pins each of these to a literal in its own test, so this is the half that
/// makes the literal mean something: without it the two could drift and both
/// suites would stay green.
#[test]
fn every_transcribed_constant_equals_the_carrier() {
    assert_eq!(d99::DOMAIN, u32::from(carrier::DOMAIN_ID));
    assert_eq!(d99::MAGIC, carrier::MAGIC);
    assert_eq!(d99::VERSION, carrier::VERSION);
    assert_eq!(d99::HEADER_LEN, carrier::HEADER_LEN);
    assert_eq!(d99::ROW_LEN, carrier::ROW_LEN);
    assert_eq!(d99::FIXED_SCALE, carrier::FIXED_SCALE);
    assert_eq!(
        d99::MIN_SCALE_ACCUMULATOR_RAW,
        carrier::MIN_SCALE_ACCUMULATOR_RAW
    );
    assert_eq!(d99::MAX_ROWS, carrier::MAX_ROWS);
    assert_eq!(d99::MAX_COMBATS, carrier::MAX_COMBATS);
    assert_eq!(d99::SIDE_ATTACKER, carrier::SIDE_ATTACKER);
    assert_eq!(d99::SIDE_DEFENDER, carrier::SIDE_DEFENDER);
    assert_eq!(d99::EXECUTABLE_SHA256, carrier::EXECUTABLE_SHA256);
    assert_eq!(
        d99::STATUS_TORN_ROWS_PRESENT,
        carrier::DOMAIN99_STATUS_TORN_ROWS_PRESENT
    );
    assert_eq!(
        d99::STATUS_STAT_BLOCK_NULL_PRESENT,
        carrier::DOMAIN99_STATUS_STAT_BLOCK_NULL_PRESENT
    );
    assert_eq!(
        d99::LANE_LOCAL_STATUS_MASK,
        carrier::DOMAIN99_LANE_LOCAL_STATUS_MASK
    );
    // The row-observation wire codes, in both directions.
    for (theirs, mine) in [
        (
            carrier::RowObservation::RoundEntryFold,
            d99::RowObservation::RoundEntryFold,
        ),
        (
            carrier::RowObservation::StatBlockNull,
            d99::RowObservation::StatBlockNull,
        ),
        (
            carrier::RowObservation::TornRead,
            d99::RowObservation::TornRead,
        ),
        (carrier::RowObservation::Unread, d99::RowObservation::Unread),
    ] {
        assert_eq!(theirs.code(), mine.wire_code());
        assert_eq!(theirs.carries_stats(), mine.carries_stats());
        assert_eq!(theirs.carries_scale(), mine.carries_scale());
        assert_eq!(d99::RowObservation::from_wire_code(theirs.code()), Ok(mine));
    }
}

/// Law: what the production encoder writes, the transcription reads back
/// unchanged — every raw field, on every observation state, on both sides.
///
/// This is the anti-drift gate proper. A transcription with a shifted offset, a
/// swapped field pair, or a wrong row width would round-trip perfectly inside
/// `d99`'s own tests, because those build the bytes with the same (wrong) layout.
/// Only the real encoder can catch it.
#[test]
fn the_production_encoder_round_trips_through_the_transcription() {
    let rows = vec![
        carrier_row(4_211, carrier::SIDE_ATTACKER, carrier::RowObservation::RoundEntryFold),
        carrier_row(4_212, carrier::SIDE_ATTACKER, carrier::RowObservation::StatBlockNull),
        carrier_row(4_213, carrier::SIDE_DEFENDER, carrier::RowObservation::TornRead),
        carrier_row(4_214, carrier::SIDE_DEFENDER, carrier::RowObservation::Unread),
    ];
    let snapshot = carrier::DivisionCombatStatsSnapshot {
        read_time_us: 1_234,
        observed_combat_count: 1,
        status_flags: carrier::DOMAIN99_STATUS_TORN_ROWS_PRESENT
            | carrier::DOMAIN99_STATUS_STAT_BLOCK_NULL_PRESENT,
        source_sequence: 88,
        rows: rows.clone(),
    };
    let payload = carrier::encode_payload(&snapshot).expect("the production encoder accepts this");

    let (header, decoded) =
        d99::decode_success_payload(&payload).expect("the transcription reads the production bytes");
    assert_eq!(header.observed_combat_count, snapshot.observed_combat_count);
    assert_eq!(header.read_time_us, snapshot.read_time_us);
    assert_eq!(header.source_sequence, snapshot.source_sequence);
    assert_eq!(header.row_count as usize, rows.len());
    assert!(header.torn_rows_present);
    assert!(header.stat_block_null_present);

    assert_eq!(decoded.len(), rows.len());
    for (mine, theirs) in decoded.iter().zip(rows.iter()) {
        assert_eq!(mine.combat_id, theirs.combat_id);
        assert_eq!(mine.generation, theirs.generation);
        assert_eq!(mine.side, theirs.side);
        assert_eq!(mine.division_id, theirs.division_id);
        assert_eq!(mine.observation.wire_code(), theirs.observation.code());
        assert_eq!(mine.modifier_count_before, theirs.modifier_count_before);
        assert_eq!(mine.modifier_count_after, theirs.modifier_count_after);
        assert_eq!(mine.combat_side_bonus_raw, theirs.combat_side_bonus_raw);
        assert_eq!(
            mine.stats.map(|s| (
                s.defense_raw,
                s.breakthrough_raw,
                s.hardness_raw,
                s.soft_attack_raw,
                s.hard_attack_raw
            )),
            theirs.stats.map(|s| (
                s.defense_raw,
                s.breakthrough_raw,
                s.hardness_raw,
                s.soft_attack_raw,
                s.hard_attack_raw
            )),
        );
        assert_eq!(
            mine.scale.map(|s| (
                s.attack_accumulator_raw,
                s.defence_accumulator_raw,
                s.attack_base_raw,
                s.defence_base_raw
            )),
            theirs.scale.map(|s| (
                s.attack_accumulator_raw,
                s.defence_accumulator_raw,
                s.attack_base_raw,
                s.defence_base_raw
            )),
        );
    }
}

/// Law: each of the four distinct stat members and each of the four distinct
/// scale members lands in its OWN slot.
///
/// The round-trip above compares whole groups, so a transposition inside a group
/// whose fixture values happened to be equal would survive it. Here every member
/// gets a value unique across the whole row, so any swap moves a value into a
/// slot it does not belong in.
#[test]
fn no_two_members_can_be_transposed_without_being_seen() {
    let stats = carrier::CachedStatBlock {
        defense_raw: 1_000_001,
        breakthrough_raw: 1_000_002,
        hardness_raw: 1_000_003,
        soft_attack_raw: 1_000_004,
        hard_attack_raw: 1_000_005,
    };
    let scale = carrier::ScaleFields {
        attack_accumulator_raw: 2_000_001,
        defence_accumulator_raw: 2_000_002,
        attack_base_raw: 2_000_003,
        defence_base_raw: 2_000_004,
    };
    let mut row = carrier_row(
        4_211,
        carrier::SIDE_ATTACKER,
        carrier::RowObservation::RoundEntryFold,
    );
    row.stats = Some(stats);
    row.scale = Some(scale);
    row.combat_side_bonus_raw = Some(3_000_001);
    let payload = carrier::encode_payload(&carrier::DivisionCombatStatsSnapshot {
        read_time_us: 1,
        observed_combat_count: 1,
        status_flags: 0,
        source_sequence: 88,
        rows: vec![row],
    })
    .expect("encodes");

    let (_, decoded) = d99::decode_success_payload(&payload).expect("decodes");
    let decoded = decoded[0];
    let stats = decoded.stats.expect("a round-entry row carries stats");
    assert_eq!(stats.defense_raw, 1_000_001);
    assert_eq!(stats.breakthrough_raw, 1_000_002);
    assert_eq!(stats.hardness_raw, 1_000_003);
    assert_eq!(stats.soft_attack_raw, 1_000_004);
    assert_eq!(stats.hard_attack_raw, 1_000_005);
    let scale = decoded.scale.expect("a round-entry row carries scale");
    assert_eq!(scale.attack_accumulator_raw, 2_000_001);
    assert_eq!(scale.defence_accumulator_raw, 2_000_002);
    assert_eq!(scale.attack_base_raw, 2_000_003);
    assert_eq!(scale.defence_base_raw, 2_000_004);
    assert_eq!(decoded.combat_side_bonus_raw, Some(3_000_001));
}

/// Law: a failure-status payload minted by the production encoder folds to
/// `Unavailable` through the transcription, not to an error and not to an empty
/// snapshot.
///
/// "The hour failed" and "the hour observed no combat" are different facts, and a
/// consumer that cannot tell them apart is the exact defect this domain's typed
/// states exist to prevent.
#[test]
fn a_production_failure_payload_folds_to_unavailable() {
    let payload = carrier::encode_payload(&carrier::DivisionCombatStatsSnapshot {
        read_time_us: 0,
        observed_combat_count: 0,
        status_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        source_sequence: 0,
        rows: Vec::new(),
    })
    .expect("the production encoder accepts a typed failure");
    assert_eq!(
        d99::fold_payload(&d99::schema(), &payload).expect("folds"),
        d99::FoldOutcome::Unavailable {
            status_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        }
    );

    let empty_clean = carrier::encode_payload(&carrier::DivisionCombatStatsSnapshot {
        read_time_us: 0,
        observed_combat_count: 0,
        status_flags: 0,
        source_sequence: 88,
        rows: Vec::new(),
    })
    .expect("an observed-nothing hour is well formed");
    let d99::FoldOutcome::Snapshot(decoded) =
        d99::fold_payload(&d99::schema(), &empty_clean).expect("folds")
    else {
        panic!("an observed-nothing hour is a snapshot, not an unavailability");
    };
    assert!(decoded.division_stats.is_empty());
}
