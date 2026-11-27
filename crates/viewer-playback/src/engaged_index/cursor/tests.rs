use super::*;
use replay_capture_spool::spool::Record;
use replay_data::rows::{CampaignRow, ClockRow, CombatEngagementRefreshRow, CountryLinkRow, CountryRow, FactionMemberRow, LandCombatRow, LandCombatSideRow, NavalCombatDetailsRow, NavalCombatRow, TableRows, TaskForceRow};
use replay_frame::{branches::{BranchReason, BranchStart, Parentage}, frame::{Frame, Metrics, TableUpdate, TableView, current_absence}};
use viewer_engagements::{engaged::Engaged, engagement_history::{LAND, NAVAL, Provenance, combats}};
use viewer_foundation::visibility::Relations;

const FIRST: Hour = 40;
const COMBAT: u64 = 73;
const NAVAL_COMBAT: u64 = 74;
const CAMPAIGN: &[u8] = b"surface engagement";

fn replay() -> Arc<Replay> {
    replay_from(FIRST - 3, BranchReason::SessionStart)
}

fn replay_from(first: Hour, reason: BranchReason) -> Arc<Replay> {
    let mut records = vec![
        Record::SessionStart(viewer_layer_core::test_support::sample_playback::session(1)),
        Record::BranchStart(BranchStart { id: 1, session: 1, loaded_hour: (reason == BranchReason::LoadSignal).then_some(first), first_hour: first, parent: Parentage::Unknown, reason }),
    ];
    records.extend((first..=FIRST + 5).map(|hour| {
        let withdrawn = hour > FIRST;
        let allied = hour >= FIRST + 2;
        let mut naval = NavalCombatDetailsRow { combat: NAVAL_COMBAT, ..Default::default() };
        naval.sides[0].units = if withdrawn { vec![12] } else { vec![11, 12] };
        naval.sides[0].summary.countries = if withdrawn { vec![2] } else { vec![1, 2] };
        naval.sides[1].units = vec![13];
        naval.sides[1].summary.countries = vec![3];
        let rows = [
            TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.to_vec() }]),
            TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]),
            TableRows::CountryLinks((1..=3).map(|id| CountryLinkRow { id, canonical_slot: id }).collect()),
            TableRows::Countries((1..=3).map(|id| CountryRow { id, canonical_slot: id, tag: format!("C{id}").into_bytes(), ..Default::default() }).collect()),
            TableRows::FactionMembers(if allied { [1, 2].into_iter().enumerate().map(|(ordinal, country)| FactionMemberRow { country, faction: 17, ordinal: ordinal as u32 }).collect() } else { Vec::new() }),
            TableRows::LandCombats(vec![LandCombatRow { id: COMBAT, attackers: if withdrawn { vec![12] } else { vec![11, 12] }, attacker_classes: if withdrawn { vec![0] } else { vec![0, 0] }, defenders: vec![13], defender_classes: vec![0], ..Default::default() }]),
            TableRows::LandCombatSides(vec![LandCombatSideRow { combat: COMBAT, attacker_primary: if withdrawn { vec![2] } else { vec![1, 2] }, defender_primary: vec![3], ..Default::default() }]),
            TableRows::NavalCombats(vec![NavalCombatRow { id: NAVAL_COMBAT, ..Default::default() }]),
            TableRows::NavalCombatDetails(vec![naval]),
            TableRows::TaskForces((1..=3).map(|country| TaskForceRow { id: 10 + u64::from(country), logical_owner: country, roster_owner: country, ..Default::default() }).collect()),
            TableRows::CombatEngagementRefreshes(if hour == FIRST + 1 { [(CombatEngagementRefreshRow::LAND_SIDE, COMBAT), (CombatEngagementRefreshRow::NAVAL_SIDE, NAVAL_COMBAT)].map(|(kind, target)| CombatEngagementRefreshRow { target, kind, side: 0, viewer: 1, triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED | CombatEngagementRefreshRow::TRIGGER_UNITS }).to_vec() } else { Vec::new() }),
        ];
        let mut tables = std::array::from_fn(|index| match current_absence(TableId::ALL[index]) {
            Some(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
            Some(TableView::Unavailable { reason }) => TableUpdate::Unavailable { reason },
            None => TableUpdate::Snapshot(TableRows::empty(TableId::ALL[index])),
            _ => unreachable!("an absent table has a named refusal"),
        });
        for rows in rows {
            let table = rows.table();
            tables[table.index()] = TableUpdate::Snapshot(if hour < FIRST && matches!(table, TableId::LandCombats | TableId::LandCombatSides | TableId::NavalCombats | TableId::NavalCombatDetails) { TableRows::empty(table) } else { rows });
        }
        Record::Frame(Frame { branch: 1, hour, keyframe: true, metrics: Metrics::default(), tables })
    }));
    Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.to_vec(), &[records]).unwrap())
}

fn flag(cursor: &mut Cursor, hour: Hour, epoch: u64, country: u32, kind: usize) -> (Engaged, Provenance) {
    let country_choice = Country::Slot { slot: country, tag: format!("C{country}") };
    let (read, history) = cursor.read(1, hour, epoch, &country_choice, Some(FIRST)).unwrap();
    let SeekResult::Frame(frame) = read else { panic!("the synthetic hour is recorded") };
    let links = (1..=3).map(|id| (id, id)).collect();
    let relations = Relations::read(&frame, &links);
    let (flags, provenance) = combats(&frame, kind, country, &links, relations.as_ref().map_err(|reason| *reason), history.as_deref()).unwrap();
    let combat = if kind == LAND { COMBAT } else { NAVAL_COMBAT };
    if hour >= FIRST + 2 && country == 1 {
        let (unindexed, _) = combats(&frame, kind, country, &links, relations.as_ref().map_err(|reason| *reason), None).unwrap();
        assert_eq!(unindexed[&combat], Engaged::Yes);
    }
    (flags[&combat], provenance)
}

#[test]
fn the_battle_surface_keeps_a_withdrawn_country_disengaged_after_it_joins_the_other_attacker() {
    for kind in [LAND, NAVAL] {
        let mut cursor = Cursor::new(replay(), &[]);
        assert_eq!(flag(&mut cursor, FIRST, 0, 1, kind), (Engaged::Yes, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST + 1, 0, 1, kind), (Engaged::No, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST + 2, 0, 1, kind), (Engaged::No, Provenance::Indexed));
    }
}

#[test]
fn the_battle_surface_rebuilds_its_country_history_on_forward_and_backward_seeks() {
    for kind in [LAND, NAVAL] {
        let mut cursor = Cursor::new(replay(), &[]);
        assert_eq!(flag(&mut cursor, FIRST + 4, 0, 1, kind), (Engaged::No, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST, 1, 1, kind), (Engaged::Yes, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST + 2, 2, 1, kind), (Engaged::No, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST + 2, 2, 2, kind), (Engaged::Yes, Provenance::Indexed));
    }
}

#[test]
fn a_missing_seed_keeps_engagement_unknown_unless_a_native_load_proves_it() {
    for kind in [LAND, NAVAL] {
        let mut cursor = Cursor::new(replay_from(FIRST, BranchReason::SessionStart), &[]);
        assert_eq!(flag(&mut cursor, FIRST, 0, 1, kind), (Engaged::Unknown, Provenance::Indexed));
        let mut cursor = Cursor::new(replay_from(FIRST + 2, BranchReason::SessionStart), &[]);
        assert_eq!(flag(&mut cursor, FIRST + 2, 0, 1, kind), (Engaged::Unknown, Provenance::Indexed));
        assert_eq!(flag(&mut cursor, FIRST + 3, 0, 1, kind), (Engaged::Unknown, Provenance::Indexed));
        let mut cursor = Cursor::new(replay_from(FIRST, BranchReason::LoadSignal), &[]);
        assert_eq!(flag(&mut cursor, FIRST, 0, 1, kind), (Engaged::Yes, Provenance::Indexed));
    }
}

#[test]
fn a_forward_step_folds_one_hour_and_a_seek_folds_its_battle_prefix_and_seed_once() {
    let mut cursor = Cursor::new(replay(), &[]);
    let folded = |cursor: &Cursor| cursor.held.as_ref().unwrap().pass.check.summary.hours;
    flag(&mut cursor, FIRST + 2, 0, 1, LAND);
    assert_eq!(folded(&cursor), 4);
    flag(&mut cursor, FIRST + 3, 0, 1, LAND);
    assert_eq!(folded(&cursor), 5);
    cursor.declare(&[TableId::Divisions]);
    flag(&mut cursor, FIRST + 3, 0, 1, LAND);
    assert_eq!(folded(&cursor), 5);
    flag(&mut cursor, FIRST + 5, 1, 1, LAND);
    assert_eq!(folded(&cursor), 7);
    flag(&mut cursor, FIRST + 5, 1, 1, LAND);
    assert_eq!(folded(&cursor), 7);
    flag(&mut cursor, FIRST + 1, 2, 1, LAND);
    assert_eq!(folded(&cursor), 3);
}
