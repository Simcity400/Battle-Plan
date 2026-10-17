//! The military air ledger, raids and air aces prepared on their own, before
//! the hourly walk, publish and save exactly what hourly preparation does,
//! read only the hours their fields change, refuse where hourly preparation
//! refuses, and answer their queries while the hourly walk has accepted
//! nothing.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::{air_aces, military_air_ledger, raids};
use engine::craid_instance as native;
use independent::{Own, air};
use replay::rows::{
    AirAceRow, AirLossEventRow, AirWingRow, BuildingBombingDamageCause, BuildingBombingDamageRow,
    LexicalTokenRow, ProvinceRow, RaidCountryRow, RaidRow, RaidScope, RaidTargetRow, Recorded,
    ReservedCountryNamingRow, WingEquipmentRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// A recording of branch 1's blocks (each block's hours given), then branch
/// 2's one block when it has hours.
fn recording(branch_1: &[&[i32]], branch_2: &[i32], frame: fn(u64, i32) -> DecodedFrame) -> Replay {
    let frames = |branch: u64, hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| frame(branch, hour)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1, branch_1[0]))]).unwrap();
    for &hours in &branch_1[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(1, hours))])
            .unwrap();
    }
    if !branch_2.is_empty() {
        replay = replay.append(&[records(2, frames(2, branch_2))]).unwrap();
    }
    replay
}

/// Six blocks of four hours from 100 (112 and 113 a gap before the fourth),
/// the first `blocks` of them, then branch 2's one block when `second`.
fn blocks(blocks: usize, second: bool, frame: fn(u64, i32) -> DecodedFrame) -> Replay {
    let hours: [&[i32]; 6] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[108, 109, 110, 111],
        &[114, 115, 116, 117],
        &[118, 119, 120, 121],
        &[122, 123, 124, 125],
    ];
    let second: &[i32] = if second { &[20, 21, 22, 23] } else { &[] };
    recording(&hours[..blocks], second, frame)
}

/// Run `run` with `family` the only family prepared on its own.
fn own<T>(family: &'static dyn Own, run: impl FnOnce() -> T) -> T {
    independent::with_test_families(&[family], run)
}

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Every block committed on its own.
fn measured(source: &Source) {
    run_measured_through(
        source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |_| {},
    )
    .unwrap();
}

/// The hourly walk prepares one block per run.
fn resumed(source: &Source) {
    let flag = AtomicBool::new(false);
    while unprepared_blocks(source, None).unwrap() != Some(0) {
        run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
    }
}

/// Prepare `replays` in turn on one source with `prepare` (each later one
/// appended to the one before), with every flushed branch read back from its
/// saved state when `reload`; the last one's payloads.
fn prepared(
    fixture: &mut Fixture,
    replays: &[&Replay],
    prepare: &dyn Fn(&Source),
    reload: bool,
) -> Payloads {
    super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
    let mut published = None;
    for (position, replay) in replays.iter().enumerate() {
        fixture.replace(replay);
        let source = fixture.source("air-families");
        prepare(&source);
        if position + 1 == replays.len() {
            published = Some(payloads(&source, None));
            clear(source);
        }
    }
    super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
    published.expect("a recording to prepare")
}

/// Sections only one mode writes: the checkpoint (which holds the family's
/// progress) and the family's own coverage pages.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1" || name.starts_with("preparation-family-")
}

/// Every section both modes publish is byte-identical, and hourly
/// preparation published sections starting with `prefix`.
fn assert_same(actual: &Payloads, hourly: &Payloads, prefix: &str, label: &str) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| !mode_specific(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert!(
        hourly.keys().any(|(name, _)| name.starts_with(prefix)),
        "{label}: hourly preparation published nothing of {prefix}"
    );
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
}

/// A family's blocks observed hour by hour and folds that fell back, so far
/// on this thread.
fn notes(family: &str) -> (usize, usize) {
    (
        prepass::noted(&format!("{family}.sequential")),
        prepass::noted(&format!("{family}.fallback")),
    )
}

fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

/// Prepare `replay` with the hourly walk stalled at its first commit, so
/// only the families prepared on their own are accepted.
fn stalled(source: &Source, replay: &Replay) {
    let flag = AtomicBool::new(false);
    let mut commits = 0;
    let error = run_checked(
        source,
        None,
        &flag,
        |_| {},
        |_| {
            commits += 1;
            flag.store(true, Ordering::Release);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(commits, 1);
    assert_eq!(
        unprepared_blocks(source, None).unwrap(),
        Some(replay.index.len()),
        "the hourly walk accepted no block"
    );
}

/// Each of `requests` answers, while the hourly walk has accepted nothing,
/// what it answers once hourly preparation prepared every family.
fn assert_answers_while_stalled(
    family: &'static dyn Own,
    replay: &Replay,
    requests: &[(&str, serde_json::Value)],
) {
    let mut fixture = Fixture::new(replay);
    let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("air-stalled-hourly");
        plain(&source);
        let replies = requests
            .iter()
            .map(|(product, params)| query(&source, product, params.clone()).unwrap())
            .collect();
        clear(source);
        replies
    });
    own(family, || {
        let source = fixture.source("air-stalled");
        stalled(&source, replay);
        for ((product, params), expected) in requests.iter().zip(&expected) {
            assert!(
                !expected.is_null(),
                "{product} {params}: hourly preparation answers"
            );
            assert_eq!(
                &query(&source, product, params.clone()).unwrap(),
                expected,
                "{product} {params}: answered from the family's own coverage"
            );
        }
        clear(source);
    });
}

/// Hours of `replay`.
fn hours(replay: &Replay) -> usize {
    replay
        .index
        .iter()
        .map(|block| (block.last_hour - block.first_hour + 1) as usize)
        .sum()
}

// The military air ledger.

fn loss(id: u64, game_hour: i32, country: u32, amount_raw: i64) -> AirLossEventRow {
    AirLossEventRow {
        id,
        game_hour,
        country,
        region: 40_001,
        native_cause: 1,
        amount_raw,
        opponent: Some(B),
        ..Default::default()
    }
}

fn damage(id: u64, game_hour: i32) -> BuildingBombingDamageRow {
    BuildingBombingDamageRow {
        id,
        game_hour,
        attacker: Some(A),
        victim: B,
        state: 1,
        building_type: 9001,
        cause: BuildingBombingDamageCause::AirMission,
        amount_raw: 12_345,
        ..Default::default()
    }
}

/// The ledger's tables at `hour`, with every country's balances changing
/// every hour. Air losses at 101 and 103; none at 104, a block's first hour,
/// before 105's; losses at 108 (a block's first hour); the table unread at
/// 109 and 110, carried by the gap into the next block, whose first hour
/// reads nothing and whose 115 restarts the identities within the unread
/// interval (a new epoch); C renamed at 120 (a block's third hour). Bombing
/// at 102, 116 and 121. `repeat` repeats an earlier identity at 105, which
/// hourly preparation refuses.
fn ledger_frame_with(branch: u64, hour: i32, repeat: bool) -> DecodedFrame {
    let mut observed = frame(branch, hour, 3);
    if hour >= 120
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"ZZZ".to_vec();
    }
    let losses = match (branch, hour) {
        (1, 101) => Some(vec![loss(1, 100, A, 5), loss(2, 101, B, 7)]),
        (1, 103) => Some(vec![loss(3, 103, A, 2)]),
        (1, 105) => Some(vec![loss(if repeat { 1 } else { 4 }, 104, C, 1)]),
        (1, 108) => Some(vec![loss(5, 108, B, 3)]),
        (1, 109 | 110) => None,
        (1, 115) => Some(vec![loss(1, 110, A, 4), loss(2, 115, C, 6)]),
        (1, 116) => Some(vec![loss(3, 116, B, 1)]),
        (1, 119) => Some(vec![loss(4, 119, A, 2)]),
        (1, 123) => Some(vec![loss(5, 122, C, 9)]),
        (2, 21) => Some(vec![loss(1, 21, A, 1)]),
        _ => Some(Vec::new()),
    };
    *observed.tables[TableId::AirLossEvents.index()] = match losses {
        Some(rows) => TableView::Complete(TableRows::AirLossEvents(rows)),
        None => TableView::Unavailable {
            reason: "air losses unread".into(),
        },
    };
    let bombing = match (branch, hour) {
        (1, 102) => vec![damage(1, 102)],
        (1, 116) => vec![damage(2, 115)],
        (1, 121) => vec![damage(3, 121)],
        _ => Vec::new(),
    };
    *observed.tables[TableId::BuildingBombingDamage.index()] =
        TableView::Complete(TableRows::BuildingBombingDamage(bombing));
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 9001,
            literal: b"industrial_complex".to_vec(),
        }]));
    observed
}

fn ledger_frame(branch: u64, hour: i32) -> DecodedFrame {
    ledger_frame_with(branch, hour, false)
}

fn ledger_repeat_frame(branch: u64, hour: i32) -> DecodedFrame {
    ledger_frame_with(branch, hour, true)
}

/// `ledger_frame` without events: nothing the ledger reads changes, while
/// every country's balances do.
fn ledger_quiet_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = ledger_frame(branch, hour);
    *observed.tables[TableId::AirLossEvents.index()] =
        TableView::Complete(TableRows::AirLossEvents(Vec::new()));
    *observed.tables[TableId::BuildingBombingDamage.index()] =
        TableView::Complete(TableRows::BuildingBombingDamage(Vec::new()));
    observed
}

/// `ledger_quiet_frame` with C's tag empty at 101 to 103, which the
/// ledger's identity read refuses every hour, without any field changing
/// at 102 and 103.
fn ledger_unnamed_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = ledger_quiet_frame(branch, hour);
    if (101..=103).contains(&hour)
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
    {
        rows[2].tag = Vec::new();
    }
    observed
}

/// Two quiet blocks of six hours.
fn quiet_blocks(frame: fn(u64, i32) -> DecodedFrame) -> Replay {
    recording(
        &[
            &[100, 101, 102, 103, 104, 105],
            &[106, 107, 108, 109, 110, 111],
        ],
        &[],
        frame,
    )
}

#[test]
fn air_ledger_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = blocks(6, true, ledger_frame);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, &[&replay], &plain, false));
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let before = (notes("air_ledger"), military_air_ledger::prepared::counts());
        let actual = own(&air::AIR_LEDGER, || {
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    prepared(&mut fixture, &[&replay], &plain, false)
                })
            })
        });
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&actual, &hourly, "military-air-ledger-", &label);
        assert_eq!(
            notes("air_ledger"),
            before.0,
            "{label}: every block folded from its extract"
        );
        // The fold observed each block's first hour on the carried ledger
        // and nothing else; the extracts were given frames only at the hours
        // the ledger's fields changed.
        let (observed, frames) = military_air_ledger::prepared::counts();
        assert_eq!(observed - before.1.0, replay.index.len(), "{label}");
        let frames = frames - before.1.1;
        assert!(
            frames > replay.index.len() && frames < hours(&replay),
            "{label}: {frames} frames of {} hours",
            hours(&replay)
        );
    }
}

#[test]
fn air_ledger_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    let first = blocks(3, false, ledger_frame);
    let grown = blocks(6, true, ledger_frame);
    let modes: [(&str, &dyn Fn(&Source), bool); 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let hourly = independent::with_test_hourly(|| {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        let actual = own(&air::AIR_LEDGER, || {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        assert_same(&actual, &hourly, "military-air-ledger-", mode);
    }
}

/// Quiet hours build no frame: a block whose ledger fields never change is
/// read at its first hour alone, however its countries' balances move.
#[test]
fn air_ledger_quiet_hours_build_no_frame() {
    let replay = quiet_blocks(ledger_quiet_frame);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, &[&replay], &plain, false));
    let before = military_air_ledger::prepared::counts();
    let actual = own(&air::AIR_LEDGER, || {
        prepass::with_test_prepass(2, None, || {
            prepared(&mut fixture, &[&replay], &plain, false)
        })
    });
    assert_same(&actual, &hourly, "military-air-ledger-", "quiet");
    let (observed, frames) = military_air_ledger::prepared::counts();
    assert_eq!(
        (observed - before.0, frames - before.1),
        (2, 2),
        "one frame per block, at its first hour"
    );
}

/// An identity read the ledger refuses is read again every hour although
/// nothing it reads changes: those hours are the extract's deadlines.
#[test]
fn air_ledger_reads_refused_identities_again_every_hour_as_hourly_preparation() {
    let replay = quiet_blocks(ledger_unnamed_frame);
    let named = quiet_blocks(ledger_quiet_frame);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, &[&replay], &plain, false));
    let mut frames = |replay: &Replay| {
        let before = military_air_ledger::prepared::counts().1;
        let actual = own(&air::AIR_LEDGER, || {
            prepass::with_test_prepass(2, None, || prepared(&mut fixture, &[replay], &plain, false))
        });
        (actual, military_air_ledger::prepared::counts().1 - before)
    };
    let (actual, unnamed_frames) = frames(&replay);
    assert_same(
        &actual,
        &hourly,
        "military-air-ledger-",
        "refused identities",
    );
    let (_, named_frames) = frames(&named);
    // Named, each block is read at its first hour alone. Unnamed, 101
    // empties C's tag, 102 and 103 are read again, refused, and 104 restores
    // the tag.
    assert_eq!(named_frames, 2);
    assert_eq!(
        unnamed_frames,
        named_frames + 4,
        "the refused hours were read again"
    );
}

#[test]
fn air_ledger_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = blocks(6, true, ledger_repeat_frame);
    let mut fixture = Fixture::new(&replay);
    let modes: [(&str, &dyn Fn(&Source)); 2] = [("run", &plain), ("measured", &measured)];
    for (mode, prepare) in modes {
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            let source = fixture.source("ledger-refusal");
            prepare(&source);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
            let result = (payloads(&source, None), refused);
            clear(source);
            result
        });
        let refusal = hourly_refused[&1].0["air_ledger"].clone();
        assert_eq!(refusal.hour, 105, "{mode}");
        let fallback = notes("air_ledger").1;
        own(&air::AIR_LEDGER, || {
            let source = fixture.source("ledger-refusal");
            prepass::with_test_prepass(2, None, || prepare(&source));
            let actual = payloads(&source, None);
            // The branch coverage pages carry the hourly walk's refusals;
            // the ledger's is on its own page, checked below.
            let walked = |payloads: &Payloads| -> Payloads {
                payloads
                    .iter()
                    .filter(|((name, _), _)| !name.starts_with("preparation-coverage-"))
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect()
            };
            assert_same(
                &walked(&actual),
                &walked(&hourly),
                "military-air-ledger-",
                mode,
            );
            // The carried ordinal refuses the block's first read with
            // events, which the fold then observes hour by hour.
            assert_eq!(notes("air_ledger").1 - fallback, 1, "{mode}");
            let page: CoveragePage = serde_json::from_slice(
                &actual[&(independent::coverage_section("air_ledger", 1), 0)],
            )
            .unwrap();
            assert_eq!(page.refused.0["air_ledger"].hour, 105, "{mode}");
            assert_eq!(page.refused.0["air_ledger"].error, refusal.error, "{mode}");
            let params = serde_json::json!({"branchId": "1", "family": "airLosses"});
            let error = query(&source, "military-air-ledger", params).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{mode}");
            assert_eq!(error.to_string(), refusal.error, "{mode}");
            let params = serde_json::json!({"branchId": "2", "family": "airLosses"});
            assert!(
                !query(&source, "military-air-ledger", params)
                    .unwrap()
                    .is_null(),
                "{mode}: branch 2 answers"
            );
            clear(source);
        });
    }
}

#[test]
fn air_ledger_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = blocks(6, true, ledger_frame);
    let requests: Vec<(&str, serde_json::Value)> = ["1", "2"]
        .into_iter()
        .flat_map(|branch| {
            ["airLosses", "bombing"].into_iter().map(move |family| {
                (
                    "military-air-ledger",
                    serde_json::json!({"branchId": branch, "family": family}),
                )
            })
        })
        .collect();
    assert_answers_while_stalled(&air::AIR_LEDGER, &replay, &requests);
}

// Raids.

const RAID_TYPE: u32 = 77;

fn raid_row(id: u32, parent_slot: u32, phase: i32, outcome: i32) -> RaidRow {
    RaidRow {
        id,
        parent_slot,
        phase,
        outcome,
        ..Default::default()
    }
}

fn target(id: u32, province: u32) -> RaidTargetRow {
    RaidTargetRow {
        id,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        raid_type: RAID_TYPE,
        ..Default::default()
    }
}

/// The raid tables at `hour`, with every country's balances, every raid
/// country's used types and every province's movement gate changing every
/// hour. Raid 1 (A's, on province 500) prepares at 101 to 104 (its hours
/// moving), flies at 105 to 107 (its distance moving) and has ended at 108 to
/// 110, a success; province 500's controller changes at 116 with no raid
/// live; raid 2 (C's, on province 501) is live at 118 to 121, its
/// preparation moving at 119 and stopping at 120 with nothing the scan reads
/// changing (hourly preparation, observing the equal rows again, publishes
/// the ended preparation run there). Branch 2 has no raids.
fn raids_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 3);
    *observed.tables[TableId::RaidCountries.index()] =
        TableView::Complete(TableRows::RaidCountries(
            (0..3)
                .map(|slot| RaidCountryRow {
                    canonical_slot: slot,
                    country: (A + slot) as i32,
                    eligible_types: vec![RAID_TYPE],
                    used_types: vec![hour as u32],
                })
                .collect(),
        ));
    *observed.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 500,
            state: 2,
            owner: B,
            controller: if hour >= 116 { A } else { B },
            move_gate: (hour % 2) as u8,
            ..Default::default()
        },
        ProvinceRow {
            id: 501,
            state: 3,
            owner: C,
            controller: C,
            move_gate: (hour % 3) as u8,
            ..Default::default()
        },
    ]));
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: RAID_TYPE,
            literal: b"raid_airborne".to_vec(),
        }]));
    let mut live = Vec::new();
    let mut targets = Vec::new();
    if branch == 1 && (101..=110).contains(&hour) {
        let mut raid = match hour {
            101..=104 => raid_row(1, 0, native::PHASE_PREPARING, native::OUTCOME_NONE),
            105..=107 => raid_row(1, 0, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE),
            _ => raid_row(1, 0, native::PHASE_ENDED, native::OUTCOME_SUCCESS),
        };
        raid.preparation_hours = (hour - 100).min(4);
        raid.travelled = (hour >= 105).then(|| i64::from(hour.min(108) - 104) * 1000);
        live.push(raid);
        targets.push(target(1, 500));
    }
    if branch == 1 && (118..=121).contains(&hour) {
        let mut raid = raid_row(2, 2, native::PHASE_PREPARING, native::OUTCOME_NONE);
        raid.preparation_hours = (hour - 117).min(2);
        live.push(raid);
        targets.push(target(2, 501));
    }
    *observed.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(live));
    *observed.tables[TableId::RaidTargets.index()] =
        TableView::Complete(TableRows::RaidTargets(targets));
    complete_empty(&mut observed, &[TableId::CountryAppearance]);
    // A singleton: the reserved country zero's one row.
    *observed.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow::default(),
        ]));
    observed
}

/// Make `tables` complete and empty: every table a family watches is
/// complete in these recordings, so only its changes make hours read.
fn complete_empty(observed: &mut DecodedFrame, tables: &[TableId]) {
    for &table in tables {
        *observed.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
}

#[test]
fn raids_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = blocks(6, true, raids_frame);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, &[&replay], &plain, false));
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let before = (notes("raids"), raids::prepared::kept_hours());
        let actual = own(&air::RAIDS, || {
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    prepared(&mut fixture, &[&replay], &plain, false)
                })
            })
        });
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&actual, &hourly, "raids-", &label);
        assert_eq!(
            notes("raids"),
            before.0,
            "{label}: every block folded from its extract"
        );
        // Kept: each block's first hour, raid 1's moving hours and its end,
        // the controller change and raid 2's start and end. Raid 2's
        // unmoving hours and every raidless hour are quiet, whatever moves
        // that the scan does not read.
        let kept = raids::prepared::kept_hours() - before.1;
        assert!(
            kept > replay.index.len() && kept < hours(&replay),
            "{label}: {kept} kept hours of {}",
            hours(&replay)
        );
    }
}

#[test]
fn raids_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = blocks(3, false, raids_frame);
    let grown = blocks(6, true, raids_frame);
    let modes: [(&str, &dyn Fn(&Source), bool); 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let hourly = independent::with_test_hourly(|| {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        let actual = own(&air::RAIDS, || {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        assert_same(&actual, &hourly, "raids-", mode);
    }
}

#[test]
fn raids_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = blocks(6, true, raids_frame);
    let requests: Vec<(&str, serde_json::Value)> = ["1", "2"]
        .into_iter()
        .flat_map(|branch| {
            ["log", "catalogue"].into_iter().map(move |view| {
                (
                    "raids",
                    serde_json::json!({
                        "branchId": branch,
                        "view": view,
                        "tags": [],
                        "top": null,
                    }),
                )
            })
        })
        .collect();
    assert_answers_while_stalled(&air::RAIDS, &replay, &requests);
}

// Air aces.

/// The roster's daily read slot.
const ACE_SLOT: i32 = 21;

/// The ace tables at `hour`, with every wing's planes and equipment amount
/// and every country's balances changing every hour. Ace 1 (A's, flying
/// wing 10) dies at 140, between reads; wing 10 is renamed at 150; ace 2
/// (B's) enters at 166, after the read at 165. The roster's slot hours are
/// read with its own timing at 117, 165 and branch 2's 21, and at 141 and
/// 189 without it, as a shared read is.
fn aces_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 3);
    let mut aces = vec![AirAceRow {
        id: 1,
        country: A,
        assigned_wing: 10,
        alive: u8::from(hour < 140),
        name: b"Erich".to_vec(),
        ..Default::default()
    }];
    if hour >= 166 {
        aces.push(AirAceRow {
            id: 2,
            country: B,
            name: b"Adolf".to_vec(),
            ..Default::default()
        });
    }
    *observed.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(aces));
    if hour.rem_euclid(24) == ACE_SLOT && matches!(hour, 117 | 165 | 21) {
        observed.metrics.table_ns[TableId::AirAces.index()] = 1;
    }
    *observed.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 10,
            owner: A,
            planes: hour,
            name: Recorded::Value(Some(if hour >= 150 {
                b"Second Wing".to_vec()
            } else {
                b"First Wing".to_vec()
            })),
            ..Default::default()
        }]));
    *observed.tables[TableId::WingEquipment.index()] =
        TableView::Complete(TableRows::WingEquipment(vec![WingEquipmentRow {
            wing: 10,
            ordinal: 0,
            variant: 900,
            amount_raw: i64::from(hour),
        }]));
    complete_empty(
        &mut observed,
        &[
            TableId::Raids,
            TableId::RaidTooltipOperands,
            TableId::CountryAppearance,
            TableId::LexicalTokens,
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantUpgrades,
        ],
    );
    *observed.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            replay::rows::VariantDefinitionRow {
                id: 900,
                display_name: b"Synthetic aircraft".to_vec(),
                equipment_token: 50,
                ..Default::default()
            },
        ]));
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 50,
            literal: b"mod_aircraft".to_vec(),
        }]));
    observed
}

/// Branch 1 in blocks of twelve hours from 110 (158 and 159 a gap before
/// the fifth), the first `blocks` of them; branch 2's one block (a timed
/// read at 21) when `second`.
fn ace_blocks(blocks: usize, second: bool) -> Replay {
    let hours: Vec<Vec<i32>> = vec![
        (110..122).collect(),
        (122..134).collect(),
        (134..146).collect(),
        (146..158).collect(),
        (160..172).collect(),
        (172..184).collect(),
        (184..196).collect(),
    ];
    let hours: Vec<&[i32]> = hours.iter().take(blocks).map(Vec::as_slice).collect();
    let second: Vec<i32> = if second {
        (18..26).collect()
    } else {
        Vec::new()
    };
    recording(&hours, &second, aces_frame)
}

#[test]
fn aces_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = ace_blocks(7, true);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, &[&replay], &plain, false));
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let before = (notes("aces"), air_aces::prepared::kept_hours());
        let actual = own(&air::ACES, || {
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    prepared(&mut fixture, &[&replay], &plain, false)
                })
            })
        });
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&actual, &hourly, "air-aces-", &label);
        assert!(
            actual.keys().any(|(name, _)| name == "air-aces-1-windows"),
            "{label}: the windows were published"
        );
        assert_eq!(
            notes("aces"),
            before.0,
            "{label}: every block folded from its extract"
        );
        // Kept: each block's first hour, the timed reads (117, 165 and 21),
        // the death, the rename and the new ace; the moving planes, amounts
        // and balances keep none.
        let kept = air_aces::prepared::kept_hours() - before.1;
        assert_eq!(kept, replay.index.len() + 6, "{label}");
    }
}

/// The aces publish a window at each block's end: prepared on their own,
/// it is held until the commit that hourly preparation's block end belongs
/// to, so every commit boundary publishes the same pages.
#[test]
fn aces_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = ace_blocks(3, false);
    let grown = ace_blocks(7, true);
    let modes: [(&str, &dyn Fn(&Source), bool); 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let hourly = independent::with_test_hourly(|| {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        let actual = own(&air::ACES, || {
            prepared(&mut fixture, &[&first, &grown], prepare, reload)
        });
        assert_same(&actual, &hourly, "air-aces-", mode);
    }
}

#[test]
fn aces_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = ace_blocks(7, true);
    let requests: Vec<(&str, serde_json::Value)> = vec![
        ("air-aces", serde_json::json!({"branchId": "1"})),
        (
            "air-aces",
            serde_json::json!({"branchId": "1", "view": "window", "window": 2}),
        ),
        ("air-aces", serde_json::json!({"branchId": "2"})),
    ];
    assert_answers_while_stalled(&air::ACES, &replay, &requests);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "cold, prefix and complete first-view preparation parity"
)]
fn air_ledger_cold_and_prefix_replies_never_claim_unprepared_totals() {
    use crate::products::{LoadContext, Product};
    use military_air_ledger::{MilitaryAirLedger, MilitaryAirParams};
    let replay = blocks(6, false, ledger_frame);
    let mut fixture = Fixture::new(&replay);
    let mut short_fixture = Fixture::new(&blocks(1, false, ledger_frame));
    independent::with_test_hourly(|| {
        let source = fixture.source("air-ledger-prefix");
        let short = short_fixture.source("air-ledger-first-block");
        let flag = AtomicBool::new(false);
        for family in ["airLosses", "bombing"] {
            let params = serde_json::json!({"branchId":"1", "family":family});
            let cold = query(&source, "military-air-ledger", params).unwrap();
            assert!(
                cold["coverage"]["firstHour"].as_i64().is_some_and(|first| {
                    cold["coverage"]["lastHour"].as_i64() == first.checked_sub(1)
                }) && cold["coverage"]["complete"] == false
            );
            assert!(!cold["pending"].as_array().unwrap().is_empty());
            assert_eq!(cold["leaves"], serde_json::json!([]));
        }
        run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        assert_eq!(
            coverage(&source, None, 1, &flag)
                .unwrap()
                .unwrap()
                .last_hour,
            103
        );
        for family in ["airLosses", "bombing"] {
            let params = serde_json::json!({"branchId":"1", "family":family});
            let mut actual = query(&source, "military-air-ledger", params.clone()).unwrap();
            assert_eq!(actual["coverage"]["complete"], false);
            assert_eq!(
                actual["pending"],
                serde_json::json!([
                    {"firstHour":104,"lastHour":111}, {"firstHour":114,"lastHour":125}
                ])
            );
            for key in ["coverage", "pending"] {
                actual.as_object_mut().unwrap().remove(key);
            }
            let selection: MilitaryAirParams = serde_json::from_value(params).unwrap();
            let mut oracle = serde_json::to_value(
                MilitaryAirLedger::load(&short, &selection, &LoadContext { content: None }, &flag)
                    .unwrap(),
            )
            .unwrap();
            // The reply dates the source extent independently of its accepted events.
            oracle["lastHour"] = serde_json::json!(125);
            assert_eq!(actual, oracle);
        }
        plain(&source);
        for family in ["airLosses", "bombing"] {
            let params = serde_json::json!({"branchId":"1", "family":family});
            let mut actual = query(&source, "military-air-ledger", params.clone()).unwrap();
            assert_eq!(actual["pending"], serde_json::json!([]));
            for key in ["coverage", "pending"] {
                actual.as_object_mut().unwrap().remove(key);
            }
            let selection = serde_json::from_value(params).unwrap();
            let oracle = serde_json::to_value(
                MilitaryAirLedger::load(&source, &selection, &LoadContext { content: None }, &flag)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(actual, oracle);
        }
        clear(short);
        clear(source);
    });
}

fn roster_values(reply: &serde_json::Value, hour: i64) -> serde_json::Value {
    let texts = reply["roster"]["texts"].as_array().unwrap();
    let rows: Vec<_> = reply["roster"]["aces"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|ace| {
            let run = ace["runs"].as_array().unwrap().iter().find(|run| {
                run["firstHour"].as_i64().unwrap() <= hour
                    && run["lastHour"].as_i64().unwrap() >= hour
            })?;
            let mut value = run.clone();
            for field in ["name", "callsign", "wingName"] {
                value[field] = value[field]
                    .as_u64()
                    .map_or(serde_json::Value::Null, |index| {
                        texts[index as usize].clone()
                    });
            }
            for field in ["firstHour", "lastHour"] {
                value.as_object_mut().unwrap().remove(field);
            }
            if value["death"].is_object() {
                value["death"]["lastAlive"] = serde_json::Value::Null;
                value["death"]["byHour"] = serde_json::Value::Null;
            }
            Some(serde_json::json!({"id":ace["id"], "value":value}))
        })
        .collect();
    serde_json::json!(rows)
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "cold, prefix and complete first-view preparation parity"
)]
fn aces_cold_roster_matches_prepared_values_and_never_dates_a_first_sighting() {
    let replay = ace_blocks(7, false);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("ace-roster-first-view");
    let params = serde_json::json!({"branchId":"1", "view":"roster", "hour":195});
    let cold = query(&source, "air-aces", params.clone()).unwrap();
    assert_eq!(cold["readHour"], 189);
    assert_eq!(cold["preparing"], serde_json::json!(["firstSeen", "death"]));
    assert!(!cold["roster"]["aces"].as_array().unwrap().is_empty());
    for ace in cold["roster"]["aces"].as_array().unwrap() {
        assert!(ace["firstSeen"].is_null());
    }
    let reads = dashboard_base::prepared::query::frames::reads(&source);
    assert_eq!(query(&source, "air-aces", params.clone()).unwrap(), cold);
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        reads,
        "re-ask reuses the supplying blocks"
    );
    plain(&source);
    let exact = query(&source, "air-aces", params).unwrap();
    assert_eq!(exact["preparing"], serde_json::json!([]));
    assert_eq!(roster_values(&cold, 189), roster_values(&exact, 189));
    let name = |reply: &serde_json::Value| {
        let mut name = reply["manifest"]["variants"]["900"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|name| name["firstHour"].as_i64().unwrap() <= 189)
            .cloned()
            .expect("the supplying variant name");
        name.as_object_mut().unwrap().remove("firstHour");
        name
    };
    assert_eq!(name(&cold)["known"], true);
    assert_eq!(name(&cold), name(&exact));
    clear(source);
}
