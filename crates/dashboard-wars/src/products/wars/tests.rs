use dashboard_base::products::Country;
use super::{
    inputs::{self, Hour, Input, Relation, Shown, State},
    walk::{self, Seen, Viewed, Walk},
    *,
};
use crate::products::war_fixtures::recorded;
use dashboard_query::identity::HolderRun;
use replay_data::{
    rows::{self, CasualtyLifetimeRow, LocalPlayerRow, TableId, TableRows, WarOverviewCountryRow, WarOwnedRelationRow},
    war_participation::Factors,
};
use replay_frame::frame::TableView;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

thread_local! {
    static WALKS: Cell<usize> = const { Cell::new(0) };
    static GROUPINGS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static PROJECTIONS: Cell<usize> = const { Cell::new(0) };
    static OBSERVATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn count_walk() {
    WALKS.set(WALKS.get() + 1);
}

pub(super) fn count_grouping(hour: i32) {
    GROUPINGS.with_borrow_mut(|hours| hours.push(hour));
}

pub(super) fn count_projection() {
    PROJECTIONS.set(PROJECTIONS.get() + 1);
}

pub(super) fn count_observation() {
    OBSERVATIONS.set(OBSERVATIONS.get() + 1);
}

#[test]
fn the_first_timeline_reply_reuses_a_war_until_scores_change_its_representative() {
    use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records}};
    use replay::campaign::Replay;

    let make = |hour: i32| {
        let mut frame = recorded(hour);
        let TableView::Complete(TableRows::WarOverviewCountries(rows)) = &mut *frame.tables[TableId::WarOverviewCountries.index()] else {
            panic!("the war fixture has overview scores");
        };
        rows[1].major = 0;
        rows[1].divisions = 2 * (hour - 120);
        rows[2].major = 0;
        rows[2].divisions = 5;
        for (table, value) in frame.tables.iter().enumerate() {
            if matches!(&**value, TableView::Complete(_)) {
                frame.metrics.table_ns[table] = 1;
            }
        }
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (120..=126).map(make).collect())])
        .unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "war-observation-count".into(), revision: "1".into() },
        "memory.battleplan".into(), Arc::new(replay), &cancelled,
    ).unwrap();
    PROJECTIONS.set(0);
    OBSERVATIONS.set(0);
    let params = WarTimelineParams { branch_id: "1".into(), viewpoint: Some(Country::from("ONE")), hour: 126 };
    let reply = WarTimelineAt::load(&source, &params, &LoadContext { content: None }, &cancelled).unwrap();
    assert_eq!(reply.wars.len(), 1);
    let observations: Vec<_> = reply.wars[0].observations.iter().map(|seen| {
        let representative = seen.representative.as_ref().map(|[first, second]| [first.as_str(), second.as_str()]);
        (seen.first_hour, seen.last_hour, representative)
    }).collect();
    assert_eq!(observations, [(120, 122, Some(["ONE", "THREE"])), (123, 126, Some(["ONE", "TWO"]))]);
    assert_eq!((PROJECTIONS.get(), OBSERVATIONS.get()), (2, 2));
}

#[test]
fn the_views_share_one_walk_and_group_only_at_relevant_changes() {
    use crate::products::casualties::{CasualtyBreakdown, CasualtyBreakdownParams};
    use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records}};
    use replay::campaign::Replay;

    let frames = (120..=124).map(|hour| {
        let mut frame = recorded(hour);
        let mut set = |rows: TableRows| {
            let table = rows.table();
            frame.tables[table.index()] = TableView::Complete(rows).into();
        };
        let countries = ["ONE", "TWO", "THREE", "FOUR", "FIVE"];
        let slots = 1..=countries.len() as u32;
        set(TableRows::Countries(slots.clone().zip(countries).map(|(slot, tag)| {
            rows::CountryRow { id: slot, canonical_slot: slot, tag: tag.as_bytes().to_vec(), ..Default::default() }
        }).collect()));
        set(TableRows::CountryLinks(slots.map(|slot| rows::CountryLinkRow { id: slot, canonical_slot: slot }).collect()));
        let relation = |a, b| rows::WarRelationRow {
            first_country: a, second_country: b, start_hour: 90, first_was_instigator: 1,
            first_casualties: if b == 3 { 17 } else { 0 }, ..Default::default()
        };
        let mut relations = Vec::new();
        if hour < 122 {
            relations.push(relation(1, 2));
        }
        relations.push(relation(1, 3));
        if hour >= 121 {
            relations.push(relation(4, 5));
        }
        set(TableRows::WarRelations(relations));
        if hour >= 123 {
            set(TableRows::WarOwnedRelations(vec![
                WarOwnedRelationRow { owner: 1, ordinal: 1, first_country: 1, second_country: 3, ..Default::default() },
                WarOwnedRelationRow { owner: 3, ordinal: 0, first_country: 1, second_country: 3, ..Default::default() },
            ]));
        }
        if hour >= 124 {
            set(TableRows::FactionMembers(vec![rows::FactionMemberRow { country: 3, faction: 23, ordinal: 0 }]));
        }
        set(TableRows::WarOverviewCountries((1..=3).map(|country| WarOverviewCountryRow {
            country, major: 1, score_available: u8::from(hour != 121 || country != 1), ..Default::default()
        }).collect()));
        for (table, value) in frame.tables.iter().enumerate() {
            if matches!(&**value, TableView::Complete(_)) {
                frame.metrics.table_ns[table] = 1;
            }
        }
        frame
    }).collect::<Vec<_>>();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "grouping-count".into(), revision: "1".into() },
        "memory.battleplan".into(), Arc::new(replay), &cancelled,
    ).unwrap();
    let context = LoadContext { content: None };
    let country = Country::from("ONE");
    let wars = WarsParams { branch_id: "1".into(), viewpoint: Some(country.clone()) };
    let timeline = WarTimelineParams { branch_id: "1".into(), viewpoint: Some(country.clone()), hour: 124 };
    let losses = CasualtyBreakdownParams { branch_id: "1".into(), country, hour: 124 };
    WALKS.set(0);
    GROUPINGS.with_borrow_mut(Vec::clear);
    let whole = Wars::load(&source, &wars, &context, &cancelled).unwrap();
    assert!(whole.available && !whole.wars.is_empty());
    assert!(Wars::load(&source, &wars, &context, &cancelled).unwrap().available);
    for (hour, has_representative) in [(120, true), (121, false)] {
        let params = WarTimelineParams { hour, ..timeline.clone() };
        let reply = WarTimelineAt::load(&source, &params, &context, &cancelled).unwrap();
        assert!(reply.available);
        let observed: Vec<_> = reply.wars.iter().flat_map(|war| &war.observations).filter(|seen| seen.last_hour == hour).collect();
        assert!(!observed.is_empty() && observed.iter().all(|seen| seen.representative.is_some() == has_representative));
    }
    let casualties = CasualtyBreakdown::load(&source, &losses, &context, &cancelled).unwrap();
    assert!(casualties.breakdown.parts.iter().any(|part| whole.wars.iter().any(|war| part.key == war.id)));
    assert_eq!(WALKS.get(), 1);
    assert_eq!(GROUPINGS.with_borrow_mut(std::mem::take), [120, 122, 123, 124]);
}

fn holders(tags: &[(&str, u32)], first: i32, last: i32) -> Holders {
    Holders::new(
        tags.iter().map(|&(tag, slot)| (tag.to_owned(), vec![HolderRun { first, last, holder: Holder::Found { slot, id: slot }, starts_segment: true }])).collect::<BTreeMap<_, _>>(),
        Vec::new(),
    )
}

fn at_war(enemies: &[u32]) -> State {
    let mut state = State::default();
    for (ordinal, &enemy) in enemies.iter().enumerate() {
        state.relations.insert(1 << 32 | u64::from(enemy), Relation { start: 90, instigator: 1 });
        state.owned.insert(1 << 32 | ordinal as u64, WarOwnedRelationRow { owner: 1, ordinal: ordinal as u32, first_country: 1, second_country: enemy, ..Default::default() });
        state.owned.insert(u64::from(enemy) << 32, WarOwnedRelationRow { owner: enemy, ordinal: 0, first_country: 1, second_country: enemy, ..Default::default() });
    }
    for country in [1].iter().chain(enemies) {
        state.scores.insert(*country, WarOverviewCountryRow { country: *country, major: 1, score_available: 1, ..Default::default() });
    }
    state.rules.insert(0, (b"NATIVE".to_vec(), 1));
    state.local = Some(LocalPlayerRow { country: 1, observer_country: 0 });
    state
}

fn seen(state: &State, held: &Holders, hour: i32) -> Result<(Viewed, Vec<walk::Observed>), String> {
    let names = SlotNames::new(held);
    let link = |raw: u32| Some(raw);
    let read = Hour::build(state, &Resolved { links: &link, names: &names, holders: held, hour }, None)?;
    let observed = read
        .world
        .blobs(read.view)
        .iter()
        .map(|blob| {
            let pairs = read.world.inside(blob);
            let named = read.world.representative(blob, &pairs, &mut Vec::new()).map(|p| (p, naming::unknown("Not recorded: no content in this test")));
            walk::observed(&read.world, (&read.tags, &read.identities), read.view, (blob, &pairs), named, hour)
        })
        .collect();
    Ok((Viewed { countries: Arc::new(read.countries()), view: read.view }, observed))
}

#[test]
fn a_war_relations_lifetime_is_its_casualty_lifetime_key() {
    let row = CasualtyLifetimeRow { slot_a: 1, id_a: 10, slot_b: 3, id_b: 30, start_hour: -5, ..Default::default() };
    assert_eq!(WarRelationKey::new((3, 30), (1, 10), -5).lifetime(), row.key());
    assert_eq!(WarRelationKey::new((1, 10), (3, 30), -5), WarRelationKey::new((3, 30), (1, 10), -5));
}

#[test]
fn every_column_the_overview_reads_is_in_its_table() {
    for (table, names) in inputs::READ {
        inputs::columns(table, names).unwrap();
    }
}

#[test]
fn an_hour_the_overview_cannot_read_names_why_never_an_empty_war_list() {
    let held = holders(&[("ONE", 1), ("TWO", 2)], 0, 1000);
    let mut state = at_war(&[2]);
    assert!(seen(&state, &held, 100).is_ok());
    state.rules.insert(0, (b"NATIVE".to_vec(), 2));
    assert_eq!(seen(&state, &held, 100).err().as_deref(), Some("Not recorded: war overview viewpoint differs from local player"));
    let mut state = at_war(&[2]);
    state.apply(&inputs::Change { hour: 100, input: Input::Faction, key: 2, shown: Shown::Unknown });
    assert!(seen(&state, &held, 100).err().unwrap().contains("not recorded"));
    let unheld = holders(&[("ONE", 1)], 0, 1000);
    assert_eq!(seen(&at_war(&[2]), &unheld, 100).err().as_deref(), Some("Not recorded: the country in slot 2 holds no tag at this hour"));
}

#[test]
fn a_walk_carries_a_war_through_a_join_and_a_capitulation_and_a_gap_starts_another() {
    let held = holders(&[("ONE", 1), ("TWO", 2), ("THREE", 3)], 0, 1000);
    let mut walk = Walk::new(1, 100);
    let two = at_war(&[2]);
    walk.observe(Seen { hour: 100, read: seen(&two, &held, 100), capitulations: Ok(&[]) });
    walk.extend(102);
    let mut three = at_war(&[2, 3]);
    three.capitulations.insert(3, (1, 103));
    walk.observe(Seen { hour: 103, read: seen(&three, &held, 103), capitulations: Ok(&three.capitulated()) });
    walk.extend(104);
    walk.observe(Seen { hour: 107, read: seen(&three, &held, 107), capitulations: Ok(&[]) });
    walk.extend(108);
    let reply = walk.finish(108);
    assert_eq!(reply.unobserved, vec![HourSpan { first_hour: 105, last_hour: 106 }]);
    assert_eq!(reply.wars.len(), 2, "a gap ends what the walk knows of a war");
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour, war.started_by, war.ended_by), (100, 104, true, true));
    let observed: Vec<_> = war.observations.iter().map(|o| (o.first_hour, o.last_hour, o.viewpoint_side.clone(), o.opposing_side.clone(), o.attackers_are_viewpoint)).collect();
    assert_eq!(observed, vec![
        (100, 102, vec!["ONE".to_owned()], vec!["TWO".to_owned()], Some(true)),
        (103, 104, vec!["ONE".to_owned()], vec!["TWO".to_owned(), "THREE".to_owned()], Some(true)),
    ]);
    let runs: Vec<_> = war.participants.iter().map(|run| (run.tag.as_str(), run.side, run.first_hour, run.last_hour, run.joined_by)).collect();
    assert_eq!(runs, vec![("ONE", WarSide::Attacker, 100, 104, true), ("TWO", WarSide::Defender, 100, 104, true), ("THREE", WarSide::Defender, 103, 104, false)]);
    assert_eq!(war.capitulations, vec![WarCapitulation { tag: "THREE".into(), hour: 103 }]);
    let relations: Vec<_> = war.relations.iter().map(|m| (m.key.clone(), m.oriented, m.first_hour, m.last_hour)).collect();
    assert_eq!(relations, vec![
        (WarRelationKey::new((1, 1), (2, 2), 90), true, 100, 104),
        (WarRelationKey::new((1, 1), (3, 3), 90), true, 103, 104),
    ]);
    let after = &reply.wars[1];
    assert_eq!((after.first_hour, after.last_hour, after.started_by), (107, 108, true));
    assert_eq!(reply.viewpoints.iter().map(|run| (run.tag.as_str(), run.first_hour, run.last_hour)).collect::<Vec<_>>(), vec![("ONE", 100, 104), ("ONE", 107, 108)]);
    let early = super::at::at(&reply, 102);
    assert_eq!(early.wars.len(), 1, "a war begun after the hour is not in the reply");
    let runs: Vec<_> = early.wars[0].participants.iter().map(|run| (run.tag.as_str(), run.last_hour, run.left_by)).collect();
    assert_eq!(runs, vec![("ONE", 102, false), ("TWO", 102, false)], "a participant who joins after the hour is not there, and one held at it ends there");
    assert!(early.wars[0].capitulations.is_empty() && early.wars[0].last_hour == 102 && !early.wars[0].ended_by);
    assert_eq!(early.next_change, Some(103));
    let gap = super::at::at(&reply, 106);
    assert_eq!((gap.wars.len(), gap.wars[0].last_hour, gap.wars[0].ended_by), (1, 104, true), "inside the unread hours its end is not known");
    assert_eq!(gap.unobserved, vec![HourSpan { first_hour: 105, last_hour: 106 }]);
    let late = super::at::at(&reply, 108);
    assert_eq!(late.wars.iter().map(|war| war.first_hour).collect::<Vec<_>>(), vec![107, 100], "ongoing wars first, then the ended ones");
    assert_eq!(late.wars[1].capitulations, vec![WarCapitulation { tag: "THREE".into(), hour: 103 }]);
    assert_eq!(late.next_change, Some(109), "nothing changes before the hour after the last one recorded");
}

fn participation() -> replay_frame::frame::DecodedFrame {
    let mut frame = recorded(100);
    let ledgers = [(2, 0, 100_000), (2, 1, 100_000), (3, 0, 200_000), (3, 1, 300_000)]
        .into_iter()
        .map(|(second, direction, damage)| rows::WarParticipationRow {
            war_first: 1,
            war_second: second,
            direction,
            equipment_damage: damage,
            first_country: if direction == 0 { 1 } else { second },
            second_country: if direction == 0 { second } else { 1 },
            ..Default::default()
        })
        .collect();
    frame.tables[TableId::WarParticipation.index()] = TableView::Complete(TableRows::WarParticipation(ledgers)).into();
    frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Complete(TableRows::WarLendLeaseHistories(vec![
        rows::WarLendLeaseHistoryRow { country: 2, target: 3, industry_sent: 100_000, ..Default::default() },
        rows::WarLendLeaseHistoryRow { country: 3, target: 2, industry_received: 100_000, ..Default::default() },
    ]))
    .into();
    frame
}

const FACTORS: Factors = Factors { industry_given: 100_000, fuel_given: 100_000, industry_received: 100_000, fuel_received: 100_000 };

#[test]
fn contributions_credit_each_member_its_own_ledgers_and_same_side_deliveries_for_each_viewpoint() {
    let frame = participation();
    let held = holders(&[("ONE", 1), ("TWO", 2), ("THREE", 3)], 0, 1000);
    for view in [None, Some(Country::from("TWO")), Some(Country::Slot(2))] {
        let groups = contributions::groups(&frame, &held, view.as_ref(), Ok(FACTORS)).unwrap();
        let group = &groups[0];
        assert!(group.reason.is_none(), "{:?}", group.reason);
        let row = |tag: &str| group.participants.iter().find(|row| row.tag == tag).unwrap();
        assert_eq!((row("TWO").score.as_str(), row("THREE").score.as_str()), ("200000", "200000"));
        assert_eq!((row("TWO").participation.as_str(), row("THREE").participation.as_str()), ("50000", "50000"));
        assert_eq!(row("THREE").lend_lease_received, "-100000");
    }
    let mut incomplete = frame;
    let remaining = incomplete.complete::<rows::tables::WarParticipation>().unwrap().iter().filter(|row| row.direction == 1).cloned().collect();
    incomplete.tables[TableId::WarParticipation.index()] = TableView::Complete(TableRows::WarParticipation(remaining)).into();
    let group = contributions::groups(&incomplete, &held, Some(&Country::from("ONE")), Ok(FACTORS)).unwrap().remove(0);
    assert_eq!(group.participants.len(), 2, "the opposite side still has both directional ledgers");
    let equipment = group.measures.iter().find(|m| m.key == WarMeasureKey::EquipmentDamage).unwrap();
    assert_eq!(group.reason.as_deref(), Some("Directional participation ledger not recorded"));
    assert_eq!(equipment.viewpoint.total, None);
    assert!(equipment.viewpoint.members.is_empty());
    assert_eq!(equipment.opposing.members[0].value, "300000");
    assert_eq!(equipment.opposing.members[0].share_of_side, Some(7500));
    assert_eq!(equipment.opposing.members[0].share_of_war, None);
}

#[test]
fn a_partial_participation_read_keeps_the_war_and_side_with_all_their_ledgers() {
    let mut frame = recorded(100);
    let wars = [(1, 2), (3, 4)];
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=4).map(|id| rows::CountryLinkRow { id, canonical_slot: id }).collect(),
    )).into();
    frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(wars.map(|(first_country, second_country)| {
        rows::WarRelationRow { first_country, second_country, start_hour: 90, first_was_instigator: 1, ..Default::default() }
    }).into())).into();
    frame.tables[TableId::WarOwnedRelations.index()] = TableView::Complete(TableRows::WarOwnedRelations(wars.into_iter().flat_map(|(first_country, second_country)| {
        [first_country, second_country].map(|owner| WarOwnedRelationRow { owner, first_country, second_country, ..Default::default() })
    }).collect())).into();
    frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Complete(TableRows::WarLendLeaseHistories(vec![])).into();
    let mut ledgers: Vec<_> = wars.into_iter().flat_map(|(war_first, war_second)| [0, 1].map(|direction| {
        rows::WarParticipationRow { war_first, war_second, direction, equipment_damage: 127 * i64::from(war_first + direction), ..Default::default() }
    })).collect();
    frame.tables[TableId::WarParticipation.index()] = TableView::Complete(TableRows::WarParticipation(ledgers.clone())).into();
    let held = holders(&[("ONE", 1), ("TWO", 2), ("THREE", 3), ("FOUR", 4)], 0, 1000);
    let group = |frame: &replay_frame::frame::DecodedFrame, view| contributions::groups(frame, &held, Some(&Country::Slot(view)), Ok(FACTORS)).unwrap().remove(0);
    let expected = group(&frame, 1);
    assert_eq!(expected.participants.len(), 2);
    assert!(expected.reason.is_none());

    ledgers.pop();
    frame.tables[TableId::WarParticipation.index()] = TableView::Truncated { rows: TableRows::WarParticipation(ledgers), missing: 1 }.into();
    assert_eq!(group(&frame, 1), expected);
    let affected = group(&frame, 3);
    assert!(affected.reason.as_deref().is_some_and(|reason| reason.contains("1 rows not read")));
    assert_eq!(affected.participants.len(), 1);
    assert_eq!(affected.participants[0].tag, "THREE");
    let equipment = affected.measures.iter().find(|measure| measure.key == WarMeasureKey::EquipmentDamage).unwrap();
    assert_eq!(equipment.viewpoint.total.as_deref(), Some("381"));
    assert_eq!(equipment.opposing.total, None);
    assert_eq!(equipment.viewpoint.share_of_war, None);
}

#[test]
fn contributions_without_their_histories_name_why_instead_of_counting_zero() {
    let mut frame = participation();
    frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Unavailable { reason: "synthetic missing history".into() }.into();
    let held = holders(&[("ONE", 1), ("TWO", 2), ("THREE", 3)], 0, 1000);
    let group = contributions::groups(&frame, &held, None, Ok(FACTORS)).unwrap().remove(0);
    assert!(group.participants.is_empty());
    assert!(group.reason.unwrap().contains("synthetic missing history"));
    let group = contributions::groups(&participation(), &held, None, Err("no content".into())).unwrap().remove(0);
    assert_eq!(group.reason.as_deref(), Some("no content"));
}

#[test]
fn contribution_history_keeps_joins_departures_capitulation_and_unread_inputs_distinct() {
    use super::history;
    use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records}};
    use replay::campaign::Replay;

    let frames: Vec<_> = (100..=112).filter(|hour| ![109, 111].contains(hour)).map(|hour| {
        let mut frame = participation();
        frame.hour = hour;
        let mut put = |rows: TableRows| {
            let table = rows.table();
            *frame.tables[table.index()] = TableView::Complete(rows);
            frame.metrics.table_ns[table.index()] = 1;
        };
        put(TableRows::Clock(vec![rows::ClockRow { hour, ..Default::default() }]));
        let joins = (102..=105).contains(&hour);
        let state = at_war(if joins { &[2, 3] } else { &[2] });
        put(TableRows::WarOwnedRelations(state.owned.into_values().collect()));
        put(TableRows::WarRelations(state.relations.into_keys().map(|key| rows::WarRelationRow {
            first_country: (key >> 32) as u32, second_country: key as u32, start_hour: 90, first_was_instigator: 1, ..Default::default()
        }).collect()));
        put(TableRows::Capitulation(if hour >= 103 {
            vec![rows::CapitulationRow { country: 3, capitulated: 1, capitulated_hour: 103, ..Default::default() }]
        } else { Vec::new() }));
        let mut ledgers = participation().complete::<rows::tables::WarParticipation>().unwrap().to_vec();
        ledgers.retain(|row| (joins || row.war_second == 2) && (hour != 105 || row.direction != 0));
        if hour >= 103 {
            for row in &mut ledgers {
                if row.war_second == 3 && row.direction == 1 {
                    row.casualties = 137;
                    row.unknown_casualties = 11;
                }
            }
        }
        put(TableRows::WarParticipation(ledgers));
        if hour == 104 {
            *frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Unavailable { reason: "Fixture delivery histories unread".into() };
        }
        if hour == 107 {
            *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable { reason: "Fixture faction membership unread".into() };
        }
        if hour == 110 {
            *frame.tables[TableId::WarRelations.index()] = TableView::Truncated { rows: TableRows::WarRelations(Vec::new()), missing: 1 };
        }
        for (table, view) in frame.tables.iter().enumerate() {
            if matches!(&**view, TableView::Complete(_) | TableView::Truncated { .. } | TableView::Unavailable { .. }) {
                frame.metrics.table_ns[table] = 1;
            }
        }
        frame
    }).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames.clone())])
        .unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "contribution-history".into(), revision: "1".into() },
        "memory.battleplan".into(), Arc::new(replay), &cancelled,
    ).unwrap();
    let wars = walked(&source, 1, None, None, &cancelled).unwrap();
    let war = &wars.wars[0];
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let reply = history::read(&source, &query, &wars, war, 106, Ok(FACTORS), &cancelled).unwrap();
    let three = reply.contributors.iter().find(|member| member.slot == 3).unwrap();
    assert_eq!(three.membership, [(102, 105, 1, "THREE".into())]);
    assert_eq!(reply.war.capitulations, [WarCapitulation { tag: "THREE".into(), hour: 103 }]);
    assert_eq!(three.partial_casualties, [HourSpan { first_hour: 103, last_hour: 103 }, HourSpan { first_hour: 105, last_hour: 105 }]);
    assert_eq!(reply.sides[0].missing[0].first_hour, 104);
    assert!(reply.sides[0].missing[0].reason.contains("Fixture delivery histories unread"));
    assert_eq!(reply.sides[1].missing.len(), 1);
    assert_eq!(reply.sides[0].missing[1].reason, "Directional participation ledger not recorded");
    assert_eq!(reply.war.last_hour, 106);
    let missing = history::read(&source, &query, &wars, war, 102, Err("Fixture defines unavailable".into()), &cancelled).unwrap();
    assert!(missing.sides.iter().all(|side| side.missing[0].reason == "Fixture defines unavailable"));
    assert!(missing.war.capitulations.is_empty());

    let held = holders(&[("ONE", 1), ("TWO", 2), ("THREE", 3)], 100, 108);
    for frame in &frames[..7] {
        let snapshot = contributions::groups(frame, &held, None, Ok(FACTORS)).unwrap().remove(0);
        history::assert_snapshot(&reply, frame.hour, &snapshot);
    }
    assert!(contributions::groups(&frames[7], &held, None, Ok(FACTORS)).unwrap_err().contains("Fixture faction membership unread"));
    let unread = history::read(&source, &query, &wars, war, 107, Ok(FACTORS), &cancelled).unwrap();
    assert_eq!(unread.war.last_hour, 106);
    assert_eq!(unread.gaps, [dashboard_query::series::MissingSpan {
        first_hour: 107, last_hour: 107, reason: "War membership not recorded".into(),
    }]);
    let interrupted = wars.wars.iter().find(|war| war.first_hour == 108).unwrap();
    let truncated = frames.iter().find(|frame| frame.hour == 110).unwrap();
    assert!(contributions::groups(truncated, &held, None, Ok(FACTORS)).is_err());
    let paused = history::read(&source, &query, &wars, interrupted, 111, Ok(FACTORS), &cancelled).unwrap();
    assert_eq!(paused.war.last_hour, 108);
    assert!(paused.war.ended_by, "the war was last seen and its end is unproved");
    let gaps: Vec<_> = paused.gaps.iter().map(|span| (span.first_hour, span.last_hour, span.reason.as_str())).collect();
    assert_eq!(gaps, [
        (109, 109, query.timeline.reason(109).as_str()),
        (110, 110, "War membership is partial because a recorded relation table was truncated"),
        (111, 111, query.timeline.reason(111).as_str()),
    ]);
    assert!(paused.contributors.iter().flat_map(|member| &member.membership).all(|run| run.1 == 108));
}

fn naming_frame(hour: i32) -> replay_frame::frame::DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1_000));
    let goal = |side: u32, ordinal: u32, token: u32| rows::WarRelationGoalRow { owner: 1, side, ordinal, resolved: 1, type_token: token, state_count: 1, first_state: 5, ..Default::default() };
    let goals = vec![goal(0, 0, if hour < 110 { 7 } else { 8 }), goal(0, 1, 9), goal(1, 0, 7)];
    let token = |token: u32, text: &str| rows::LexicalTokenRow { token, literal: text.as_bytes().to_vec() };
    let tokens = vec![token(7, "annex"), token(8, if hour < 120 { "puppet" } else { "other" }), token(9, "liberate")];
    let state = rows::StateRow { id: 5, owner_name: if hour < 115 { b"Alpha".to_vec() } else { b"Beta".to_vec() }, ..Default::default() };
    for (table, view) in [
        (TableId::WarRelationGoals, TableRows::WarRelationGoals(goals)),
        (TableId::LexicalTokens, TableRows::LexicalTokens(tokens)),
        (TableId::States, TableRows::States(vec![state])),
    ] {
        frame.tables[table.index()] = TableView::Complete(view).into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

#[test]
fn a_names_goal_its_type_tokens_text_and_its_states_name_are_read_as_they_stood_at_the_hour_it_is_dated() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    use inputs::{Goal, Held};
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (100..=130).map(naming_frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "war-names".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let run = dashboard_base::queries::query(&source, 1).unwrap().timeline.stretches[0].run;
    let wars = run_wars(source.replay_ref().recording(), run).unwrap();
    let key = |side: u128, ordinal: u128| 1u128 << 96 | side << 32 | ordinal;
    let token_at = |side: u128, hour: i32| match wars.goals.at(key(side, 0), hour) {
        Some(Held::Value(Goal { type_token, .. })) => Some(*type_token),
        _ => None,
    };
    assert_eq!([token_at(0, 105), token_at(0, 109), token_at(0, 110), token_at(1, 125)], [Some(7), Some(7), Some(8), Some(7)]);
    assert_eq!(wars.goals.at(key(0, 0), 99), None, "before the run no goal is shown");
    assert_eq!(wars.goals.at(key(0, 1), 105), None, "only each side's first goal is held");
    let text = |token: u128, hour: i32| match wars.tokens.at(token, hour) {
        Some(Held::Value(text)) => Some(String::from_utf8_lossy(text).into_owned()),
        _ => None,
    };
    assert_eq!([text(8, 105), text(8, 119), text(8, 120), text(7, 130)].map(|text| text.unwrap_or_default()), ["puppet", "puppet", "other", "annex"]);
    assert_eq!(text(9, 105), None, "a token only a later goal names is not read");
    let owner = |hour: i32| match wars.states.at(5, hour) {
        Some(Held::Value((custom, owner))) => Some((custom.is_empty(), String::from_utf8_lossy(owner).into_owned())),
        _ => None,
    };
    assert_eq!([owner(114), owner(115)], [Some((true, "Alpha".to_owned())), Some((true, "Beta".to_owned()))]);
}

#[test]
fn a_countrys_naming_operands_at_an_hour_are_its_holders_look_with_every_overlord_it_reaches() {
    use content_reader_country_names::country_names::Operands;
    use recorder_derive_world::looks::Look;
    let ops = |tag: &str, overlord: Option<u32>| Ok(Operands::from_parts(overlord, [tag, "", "", "", "", ""].map(str::to_owned)));
    let look = |slot: u32, chain: Vec<(u32, Result<Operands, String>)>| Look { slot, id: slot, chain, ..Default::default() };
    let looks = RunLooks {
        spans: vec![(1, 10, 100, 130), (2, 20, 100, 109), (2, 21, 110, 130), (3, 30, 100, 130)],
        unknown: vec![(131, 140)],
        looks: HashMap::from([
            (10, look(1, vec![(1, ops("ONE", None))])),
            (20, look(2, vec![(2, ops("TWO", None))])),
            (21, look(2, vec![(2, ops("TWO", Some(1))), (1, ops("ONE", None))])),
            (30, look(9, vec![(9, ops("THREE", None))])),
        ]),
        raw_looks: HashMap::new(),
        tags: HashMap::from([(1, Arc::from("ONE")), (2, Arc::from("TWO")), (3, Arc::from("THREE"))]),
    };
    let naming = RunNaming::new(Arc::new(looks));
    let tags: HashMap<u32, String> = [(1, "ONE"), (2, "TWO"), (3, "THREE")].into_iter().map(|(slot, tag)| (slot, tag.to_owned())).collect();
    let slots = |of: &[u32]| of.iter().copied().collect::<BTreeSet<u32>>();
    let (held, _) = naming.operands(&tags, &slots(&[2]), 105);
    assert_eq!(held.keys().copied().collect::<Vec<_>>(), [2]);
    let (held, _) = naming.operands(&tags, &slots(&[2]), 110);
    assert_eq!(held.keys().copied().collect::<Vec<_>>(), [1, 2], "a subject's name reaches its overlord's operands");
    let (held, _) = naming.operands(&tags, &slots(&[3]), 120);
    assert!(held.is_empty(), "a look held by another slot names no operands of this one");
    let (held, _) = naming.operands(&tags, &slots(&[1]), 135);
    assert_eq!(held.get(&1), Some(&Err(LOOKS_UNKNOWN.to_owned())), "an hour no look is known names why");
}

#[test]
fn a_chosen_viewpoint_whose_holder_is_not_known_is_not_recorded_with_the_recorded_reason() {
    let reason = "Countries was not complete at these hours";
    let held = Holders::new(holders(&[("ONE", 1), ("TWO", 2)], 90, 109).tags, vec![(110, 120, reason.to_owned())]);
    let names = SlotNames::new(&held);
    let link = |raw: u32| Some(raw);
    let state = at_war(&[2]);
    let viewpoint = Country::from("ONE");
    let refused = Hour::build(&state, &Resolved { links: &link, names: &names, holders: &held, hour: 115 }, Some(&viewpoint)).err();
    assert_eq!(refused, Some(format!("Who holds ONE is not known: {reason}")), "never \"not in this recording\"");
    assert!(Hour::build(&state, &Resolved { links: &link, names: &names, holders: &held, hour: 100 }, Some(&viewpoint)).is_ok(), "where its holder is known the viewpoint is read");
}

#[test]
fn war_contributions_holds_quiet_hours_with_dated_reply_and_refreshes_on_edits() {
    use super::contributions::WarContributions;
    use dashboard_base::{
        Source, SourceKey,
        products::{LoadContext, at_hour::{exact_panel, frame_panel}},
        test_fixtures::{CAMPAIGN, materialized_frame, played, records, source, streamed},
    };
    use replay::campaign::Replay;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    let frames: Vec<_> = (100..=106)
        .map(|hour| {
            let mut f = source(1, hour, None);
            let one_tag = if hour >= 105 { b"UNO".to_vec() } else { b"ONE".to_vec() };
            *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                rows::CountryRow { id: 1, canonical_slot: 1, tag: one_tag, ..Default::default() },
                rows::CountryRow { id: 2, canonical_slot: 2, tag: b"TWO".to_vec(), ..Default::default() },
            ]));
            *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                rows::CountryLinkRow { id: 1, canonical_slot: 1 },
                rows::CountryLinkRow { id: 2, canonical_slot: 2 },
            ]));
            *f.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(vec![
                rows::WarRelationRow { first_country: 1, second_country: 2, start_hour: 90, ..Default::default() },
            ]));
            *f.tables[TableId::WarOwnedRelations.index()] = TableView::Complete(TableRows::WarOwnedRelations(vec![
                WarOwnedRelationRow { owner: 1, ordinal: 0, first_country: 1, second_country: 2, ..Default::default() },
                WarOwnedRelationRow { owner: 2, ordinal: 0, first_country: 1, second_country: 2, ..Default::default() },
            ]));
            *f.tables[TableId::WarOverviewCountries.index()] = TableView::Complete(TableRows::WarOverviewCountries(vec![
                WarOverviewCountryRow { country: 1, major: 1, score_available: 1, ..Default::default() },
                WarOverviewCountryRow { country: 2, major: 1, score_available: 1, ..Default::default() },
            ]));
            *f.tables[TableId::WarNameRules.index()] = TableView::Complete(TableRows::WarNameRules(vec![
                rows::WarNameRuleRow { index: 0, prefix: b"NATIVE".to_vec(), viewpoint: 1 },
            ]));
            *f.tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(Vec::new()));
            *f.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(vec![
                LocalPlayerRow { country: 1, observer_country: 0 },
            ]));
            let damage = if hour >= 103 { 250_000 } else { 100_000 };
            *f.tables[TableId::WarParticipation.index()] = TableView::Complete(TableRows::WarParticipation(vec![
                rows::WarParticipationRow { war_first: 1, war_second: 2, direction: 0, first_country: 1, second_country: 2, equipment_damage: damage, ..Default::default() },
                rows::WarParticipationRow { war_first: 1, war_second: 2, direction: 1, first_country: 2, second_country: 1, equipment_damage: 100_000, ..Default::default() },
            ]));
            *f.tables[TableId::WarLendLeaseHistories.index()] = TableView::Complete(TableRows::WarLendLeaseHistories(Vec::new()));
            f
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        SourceKey { source_id: "wc-exact".into(), revision: "1".into() },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let params = json!({"branchId": "1", "viewpoint": null});
    let panel = exact_panel::<WarContributions>(&source, &load, &params).unwrap();
    let exact = panel.exact().unwrap();
    let mut whole = frame_panel::<WarContributions>(&source, &load, &params).unwrap();
    let steps = played(source.replay_ref(), exact, 100, 106);
    for step in &steps {
        let mut expected = streamed(&materialized_frame(&mut *whole, source.replay_ref(), 1, step.hour), step.fresh);
        if !step.fresh && let (Ok(actual), Ok(exp)) = (step.value.as_ref(), &mut expected) {
            exp["hour"] = actual["hour"].clone();
        }
        assert_eq!(streamed(&step.value, step.fresh), expected, "war-contributions at hour {}", step.hour);
    }
    for hour in [101, 102, 104, 106] {
        assert_eq!(steps[(hour - 100) as usize].product_calls, 0, "quiet hour {hour} should hold");
    }
    assert_eq!(steps[5].value.as_ref().as_ref().unwrap()["contributions"][0]["viewpointSide"][0], "UNO");
    let dated_panel = exact_panel::<WarContributions>(&source, &load, &params).unwrap();
    let dated_exact = dated_panel.exact().unwrap();
    let _ = (dated_exact.advance)(100, true, &AtomicBool::new(false), None);
    let held_dated = (dated_exact.advance)(101, true, &AtomicBool::new(false), None);
    assert_eq!(*held_dated.value, materialized_frame(&mut *whole, source.replay_ref(), 1, 101));
    assert_eq!(held_dated.value.as_ref().as_ref().unwrap()["hour"], 101);
    assert_eq!((exact.cold)(103, &AtomicBool::new(false)), materialized_frame(&mut *whole, source.replay_ref(), 1, 103));
}
