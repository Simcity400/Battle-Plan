use super::coverage::{CountryCells, Coverage, Held, RunCells, UnknownReason, classify};
use super::*;
use replay::lanes::Bin;
use replay_data::rows::EquipmentLossCoverageRow as Row;

fn run(drain: &str, unread: u8) -> Option<RunCells> {
    Some(RunCells { drain: Some(drain.to_owned()), unread: Some(unread), counted_from: Some(10), ..RunCells::default() })
}

#[test]
fn a_totals_key_names_its_source_and_design() {
    let (low, high) = identity_range(7, 300);
    let key = low | u128::from(EquipmentLossTotalRow::DESTRUCTION + EquipmentDestructionCause::Attrition as u8) << 33 | 1 << 32 | 42;
    assert!(low <= key && key <= high);
    assert_eq!(leaf_of(key), Some((LossSource::Destruction(Cause(EquipmentDestructionCause::Attrition)), Some(42))));
    assert_eq!(leaf_of(low | 4 << 33), Some((LossSource::Recovered, None)));
    assert_eq!(leaf_of(low | u128::from(EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::SUNK_KIND) << 33), Some((LossSource::LendLeaseSunk, None)));
    assert_eq!(leaf_of(low | 20 << 33), None);
}

#[test]
fn combat_log_mode_shows_the_log_air_and_mixed_causes_and_destruction_mode_every_drain() {
    let log = LossSource::shown(LandSource::CombatLog);
    assert!(log.contains(&LossSource::CombatLog(0)) && log.contains(&LossSource::Recovered));
    assert!(log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::AirRaid))));
    assert!(log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::Unknown))));
    assert!(!log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::LandCombat))));
    let drains = LossSource::shown(LandSource::Destruction);
    assert!(drains.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::LandCombat))));
    assert!(!drains.contains(&LossSource::CombatLog(0)));
    assert!(drains.contains(&LossSource::SunkShips));
}

#[test]
fn a_bin_gives_its_first_value_at_its_first_hour_and_its_last_at_its_last() {
    let bin = |first: i64, last: i64| Bin { hours: 6, first, last, min: first.min(last), max: first.max(last), ..Bin::default() };
    let bins = [bin(0, 5), Bin::default(), bin(5, 5), bin(5, 9)];
    assert_eq!(bin_points(&bins, 0, 6, 0, 23), vec![(0, 0), (5, 5), (23, 9)]);
    assert_eq!(bin_points(&bins, 0, 6, 2, 20), vec![(2, 0), (5, 5), (20, 9)]);
}

#[test]
fn drain_reasons_become_one_span_each_from_their_first_hour_after_counting_starts() {
    let held = Held {
        run: vec![
            (0, 9, Some(RunCells { drain: Some(Row::DRAIN_NOT_READ.to_owned()), ..RunCells::default() })),
            (10, 19, run("", 0)),
            (20, 24, run(Row::DRAIN_NOT_READ, 0)),
            (25, 29, None),
            (30, 39, run(&format!("{}; {}", Row::DRAIN_OWNER_UNKNOWN, Row::DRAIN_SUPPLEMENTAL_NOT_RECORDED), 0)),
            (40, 49, run("", 0)),
        ],
        country: Vec::new(),
    };
    assert_eq!(held.counted_from(), Some(10));
    let spans = held.destruction_unknown(99);
    let kinds: Vec<(UnknownReason, i32, i32)> = spans.iter().map(|span| (span.reason, span.first_hour, span.last_hour)).collect();
    assert!(kinds.contains(&(UnknownReason::DestructionGap, 20, 99)), "{kinds:?}");
    assert!(kinds.contains(&(UnknownReason::DestructionGap, 25, 99)), "the hours the tables were not available: {kinds:?}");
    assert!(kinds.contains(&(UnknownReason::DestructionUnresolved, 30, 99)));
    assert!(kinds.contains(&(UnknownReason::DestructionWithheld, 30, 99)));
    assert_eq!(spans.len(), 4, "a startup hour before counting claims nothing: {kinds:?}");
    let coverage = held.destruction_coverage();
    assert_eq!(coverage.first().map(|span| (span.first_hour, span.reason.clone())), Some((10, None)));
    assert_eq!(coverage.iter().filter(|span| span.reason.is_none()).count(), 2);
}

#[test]
fn a_records_unread_hours_and_unavailable_hours_are_unknown_never_zero() {
    let held = Held {
        run: vec![(0, 4, run("", 0)), (5, 7, run("", Row::UNREAD_SHIPS)), (8, 9, run("", Row::UNREAD_SHIPS | Row::UNREAD_CONVOYS)), (10, 12, None), (13, 20, run("", 0))],
        country: Vec::new(),
    };
    let ships: Vec<(i32, i32, bool)> = held.unread(Row::UNREAD_SHIPS, 20).iter().map(|span| (span.first_hour, span.last_hour, span.detail.is_none())).collect();
    assert_eq!(ships, vec![(5, 9, true), (10, 12, false)]);
    let convoys: Vec<(i32, i32)> = held.unread(Row::UNREAD_CONVOYS, 20).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(convoys, vec![(8, 9), (10, 12)]);
}

#[test]
fn each_ended_exchange_is_a_span_from_the_hour_its_batch_is_lost() {
    let country = |ended: u32, from: Option<i32>| Some(CountryCells { exchanges_ended: Some(ended), ended_from: from, ..CountryCells::default() });
    let held = Held { run: Vec::new(), country: vec![(0, 9, country(0, None)), (10, 19, country(2, Some(8))), (20, 29, country(3, Some(15)))] };
    let spans: Vec<(i32, i32)> = held.ended(40).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(spans, vec![(8, 40), (8, 40), (15, 40)]);
}

#[test]
fn land_coverage_follows_the_identitys_row_and_its_absence_is_unknown() {
    let country = |land: u8, ungrouped: u32| Some(CountryCells { land: Some(land), ungrouped: Some(ungrouped), ..CountryCells::default() });
    let held = Held { run: Vec::new(), country: vec![(0, 4, country(Row::LOGGED, 0)), (5, 9, country(Row::LOGGED, 0)), (10, 14, country(Row::PARTIAL, 3)), (15, 19, None)] };
    let land: Vec<(i32, i32, Coverage)> = held.land().iter().map(|span| (span.first_hour, span.last_hour, span.coverage)).collect();
    assert_eq!(land, vec![(0, 9, Coverage::Logged), (10, 14, Coverage::Partial { ungrouped: 3 }), (15, 19, Coverage::Unknown)]);
    assert_eq!(classify(Row::DRAIN_LOST), UnknownReason::DestructionGap);
    assert_eq!(classify("The recording's own reason"), UnknownReason::DestructionWithheld);
}

/// Exact totals at 1,500 hours, as a view with that many picked hours may ask, are one valid request.
#[test]
fn a_request_reading_1500_exact_hours_is_valid() {
    let params = LossesParams { branch_id: "1".into(), tag: "GER".into(), land_source: LandSource::default(), country: None, from: 0, to: 2_000, width: 64, reads: (0..1_500).collect() };
    assert_eq!(EquipmentLosses::validate(&params), Ok(()));
}
