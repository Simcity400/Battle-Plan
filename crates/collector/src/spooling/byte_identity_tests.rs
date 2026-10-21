//! Spool bytes of a fixed synthetic campaign driven through the spool path.
//!
//! `EXPECTED_DIGEST` covers format 117's record envelopes and daily slot-12
//! training carry markers, the registered tables and
//! unavailable faction-companion outcomes alongside the fixture's populated inputs.
//! The campaign runs with
//! no, one and two helpers. It mixes projected unchanged and changed rows,
//! row-count changes, a failed and a truncated table, shared complete rows
//! that are reused or replaced, and carried staggered markers; any change that
//! alters one spool byte of conversion, carry, projection, comparison,
//! encoding or record framing fails here.

use super::tests::{captured, session};
use super::*;
use engine_native::msvc_string::Text;
use replay::spool::recover_prefix;
use replay_data::rows::TableId;
use replay_frame::{frame::FrameDecoder, frame::TableUpdate};

const FIRST_HOUR: i32 = 100;
const HOURS: i32 = 30;
/// FNV-1a 64 over every spool segment's bytes in segment order.
// Record-envelope version changes update the golden; container aliases do not affect spools.
const EXPECTED_DIGEST: u64 = 0xf60a_ce4a_0064_1cb3;

fn text<const N: usize>(value: &[u8]) -> Text<N> {
    let mut result = Text {
        len: value.len(),
        ..Text::default()
    };
    result.bytes[..value.len()].copy_from_slice(value);
    result
}

fn fnv1a(digest: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(digest, |digest, &byte| {
        (digest ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// One synthetic hour. Most rows repeat; a few change on fixed steps.
fn hour_frame(step: i32) -> Captured {
    let hour = FIRST_HOUR + step;
    let mut source = captured(hour);
    // A fixed world: tests that run the deletion stub advance the global
    // generation, and a change between two frames starts a branch.
    source.world_generation = 1;
    source.total_ns = 1_000 + step as u64;
    source.table_ns[TableId::Countries.index()] = 10 + step as u64;
    source.pool_bytes = 4096;
    source.countries[0] = engine_native::ccountry::Row {
        politics_witness: None,
        id: 7,
        canonical_slot: 1,
        tag: text(b"GER"),
        political_power_raw: Some(i64::from(step / 5)),
        command_power_raw: 0,
        stability_raw: 500_000,
        war_support_raw: 250_000,
        convoys_raw: Some(12),
        peace_conference_participant: 0,
        manpower_residual: 3,
        border_land_doctrine_level: None,
    };
    source.countries_result = if step == 7 {
        Outcome::Unavailable {
            reason: "synthetic country fault",
        }
    } else {
        Outcome::Rows { len: 1 }
    };
    source.links[0] = engine_native::ccountry::LinkRow {
        id: 2,
        canonical_slot: 1,
    };
    source.links_result = if step == 11 {
        Outcome::Truncated {
            len: 1,
            unread_rows: 4,
        }
    } else {
        Outcome::Rows { len: 1 }
    };
    source.provinces.rows[0] = engine_native::cprovince::Row {
        id: 6,
        state: 4,
        owner: 7,
        controller: if step >= 20 { 2 } else { 7 },
        victory_points: 3,
        ..Default::default()
    };
    source.provinces_result = Outcome::Rows { len: 1 };
    source.states[0] = engine_native::cstate::Row {
        id: 4,
        owner: 7,
        controller: 7,
        manpower_available: step / 3,
        ..Default::default()
    };
    source.states_result = Outcome::Rows { len: 1 };
    source.divisions[0] = engine_native::carmy::Row {
        id: 4713,
        roster_owner: 7,
        logical_owner: 7,
        expeditionary_owner: 0,
        province: 6,
        template: 9,
        // Whole display steps (replay_data::display): the wire holds the same
        // tenths the digest was taken over.
        hit_points: 10_000 * (1_000 - i64::from(step)),
        organisation: 5_000_000,
        orders_group: 0,
        reveal_skip: 0,
    };
    source.divisions[1] = engine_native::carmy::Row {
        id: 51,
        hit_points: 9_000_000,
        ..source.divisions[0]
    };
    source.divisions_result = Outcome::Rows {
        len: if (12..16).contains(&step) { 1 } else { 2 },
    };
    source.wars[0] = engine_native::cwar_relation::Row {
        first_country: 2,
        second_country: 7,
        start_hour: 50,
        end_hour: -1,
        cancel: 0,
        first_casualties: i64::from(step / 4),
        second_casualties: 0,
        first_unknown_casualties: 0,
        second_unknown_casualties: 0,
        first_was_instigator: 1,
    };
    source.wars_result = Outcome::Rows { len: 1 };
    source.factions[0] = engine_native::cfaction::Row {
        id: 88,
        name_start: 0,
        name_len: 3,
        ..Default::default()
    };
    source.faction_names[..3].copy_from_slice(b"Axs");
    source.faction_names_len = 3;
    source.factions_result = Outcome::Rows { len: 1 };
    source.faction_members[0] = engine_native::cfaction_system::MemberRow {
        country: 7,
        faction: 88,
        ordinal: 0,
    };
    source.faction_members_result = Outcome::Rows { len: 1 };
    source
        .extended
        .set_intel_shadow_fixture(if step < 17 { 10 } else { 20 });
    source
}

/// Every update kind the campaign must exercise, counted over all frames.
#[derive(Debug, Default)]
struct Coverage {
    snapshots: usize,
    deltas: usize,
    unchanged: usize,
    truncated: usize,
    unavailable: usize,
    /// Unchanged staggered tables: carried markers or equal shared rows.
    staggered_unchanged: usize,
    /// Hourly tables the comparison before encoding found equal.
    compared_equal: u64,
    /// Intel shadow hits and misses: the lent conversion cache on a helper
    /// decides as the spool thread's does.
    intel_shadow: (u64, u64),
}

fn run_campaign(directory: &Path, helpers: usize) -> (u64, Coverage) {
    let _ = fs::remove_dir_all(directory);
    let mut processor = Processor::new(directory, PreparedCampaigns::default(), session()).unwrap();
    for _ in 0..helpers {
        processor.set_helper(crate::spool_helper::SpoolHelper::start().unwrap());
    }
    for step in 0..HOURS {
        processor.set_backlogged(step % 3 == 0);
        processor.frame(&hour_frame(step)).unwrap();
        processor.flush().unwrap();
    }
    processor.detach().unwrap();
    let compared_equal = processor.comparison.total_decisions().0;
    let status = serde_json::to_value(processor.status().unwrap()).unwrap();
    let shadow = &status["spool_work"]["intel_shadow"];
    let intel_shadow = (
        shadow["hits"].as_u64().unwrap(),
        shadow["misses"].as_u64().unwrap(),
    );
    let campaign = directory.join(std::str::from_utf8(CAMPAIGN).unwrap());
    let mut segments: Vec<_> = fs::read_dir(&campaign)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("spool."))
        })
        .collect();
    segments.sort();
    assert!(!segments.is_empty());
    let mut digest = 0xcbf2_9ce4_8422_2325;
    let mut coverage = Coverage {
        compared_equal,
        intel_shadow,
        ..Coverage::default()
    };
    for segment in segments {
        let bytes = fs::read(segment).unwrap();
        digest = fnv1a(digest, &bytes);
        let recovered = recover_prefix(&bytes);
        assert!(recovered.damage.is_none());
        let mut decoder = FrameDecoder::new();
        for record in recovered.records {
            let Record::Frame(frame) = record else {
                continue;
            };
            decoder.apply(&frame).unwrap();
            for (table, update) in TableId::ALL.into_iter().zip(&frame.tables) {
                if replay_data::cadence::is_staggered(table)
                    && matches!(update, TableUpdate::Unchanged { .. })
                {
                    coverage.staggered_unchanged += 1;
                }
                match update {
                    TableUpdate::Snapshot(_) => coverage.snapshots += 1,
                    TableUpdate::Delta { .. } => coverage.deltas += 1,
                    TableUpdate::Unchanged { .. } => coverage.unchanged += 1,
                    TableUpdate::Truncated { .. } => coverage.truncated += 1,
                    TableUpdate::Unavailable { .. } => coverage.unavailable += 1,
                    TableUpdate::Incompatible { .. } => panic!("current writer"),
                }
            }
        }
    }
    let _ = fs::remove_dir_all(directory);
    (digest, coverage)
}

const CAMPAIGN: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

#[test]
fn a_fixed_synthetic_campaign_writes_the_same_spool_bytes() {
    let directory =
        std::env::temp_dir().join(format!("hoi4-spool-byte-identity-{}", std::process::id()));
    let (digest, coverage) = run_campaign(&directory, 0);
    println!("spool byte digest {digest:#018x}, coverage {coverage:?}");
    assert_eq!(
        run_campaign(&directory, 0).0,
        digest,
        "the campaign is deterministic"
    );
    // Helpers convert map tables and the helper packs and share the
    // comparison before encoding; the spool bytes stay those of one thread.
    for helpers in [1, 2] {
        let (helped, helped_coverage) = run_campaign(&directory, helpers);
        assert_eq!(
            helped, digest,
            "{helpers} conversion helpers changed the spool"
        );
        assert_eq!(helped_coverage.intel_shadow, coverage.intel_shadow);
    }
    assert!(coverage.intel_shadow.0 > 0, "{coverage:?}");
    assert!(coverage.snapshots > 0, "{coverage:?}");
    assert!(coverage.deltas > 0, "{coverage:?}");
    assert!(coverage.unchanged > 0, "{coverage:?}");
    assert!(coverage.truncated > 0, "{coverage:?}");
    assert!(coverage.unavailable > 0, "{coverage:?}");
    assert!(coverage.staggered_unchanged > 0, "{coverage:?}");
    // Equal hourly tables reached the encoder as comparisons, not rows.
    assert!(coverage.compared_equal > 0, "{coverage:?}");
    assert_eq!(digest, EXPECTED_DIGEST);
}

/// Conversion with the helper fills every table slot the single-thread order
/// fills, with equal values, including a failed ordinary job's refusal, which
/// must cover helper packs converted after the spool thread's share.
#[test]
fn the_conversion_helper_reproduces_single_thread_conversion() {
    use crate::extended::capture_live_shape;
    use crate::projection::{Projection, ProjectionScratch};
    let first = crate::spool_helper::SpoolHelper::start().unwrap();
    let second = crate::spool_helper::SpoolHelper::start().unwrap();
    let helper_calls = crate::spool_phases::HELPER
        .phase(crate::spool_phases::Phase::ConvertPacks)
        .0;
    for ordinary_failure in [None, Some("synthetic ordinary acquisition failure")] {
        let mut source = hour_frame(3);
        capture_live_shape(&mut source.extended);
        source.extended.set_ordinary_failure(ordinary_failure);
        let clock = source.clock.as_ref().unwrap().copy_owned().unwrap();
        let encoder = FrameEncoder::new();
        let convert = |helper| {
            let scratch = ProjectionScratch::default();
            let mut projection = Projection::new(&encoder, 1, clock.hour, false, &scratch);
            decoded_with_costs(
                &source,
                clock.borrowed(),
                1,
                &mut SpoolConversion {
                    variables_baseline: None,
                    variables_scratch: &mut ScriptVariableComparison::default(),
                    characters: None,
                    lexical: None,
                    intel: None,
                    projection: Some(&mut projection),
                    helper,
                },
            )
            .0
        };
        let serial = convert(None);
        for (backlogged, second) in [(false, None), (true, None), (false, Some(&second))] {
            let split = convert(Some(crate::spool_helper::Helpers {
                first: &first,
                second,
                backlogged,
            }));
            assert_eq!(split.tables, serial.tables, "{ordinary_failure:?}");
            assert_eq!(split.metrics, serial.metrics);
        }
        if let Some(reason) = ordinary_failure {
            assert!(serial.tables.iter().any(|table| matches!(
                table,
                TableInput::Sampled(TableView::Unavailable { reason: actual }) if actual == reason
            )));
        }
        assert!(matches!(
            serial.tables[TableId::Ships.index()],
            TableInput::Sampled(TableView::Complete(_))
        ));
    }
    assert!(
        crate::spool_phases::HELPER
            .phase(crate::spool_phases::Phase::ConvertPacks)
            .0
            >= helper_calls + 7
    );
}
