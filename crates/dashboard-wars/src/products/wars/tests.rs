use dashboard_base::products::Country;
use super::{
    inputs::{self, Hour, Input, Relation, Shown, State},
    walk::{self, Seen, Viewed, Walk},
    *,
};
use crate::products::war_fixtures::recorded;
use dashboard_query::identity::HolderRun;
use replay_data::{
    rows::{self, CasualtyLifetimeRow, LocalPlayerRow, TableRows, WarOverviewCountryRow, WarOwnedRelationRow},
    war_participation::Factors,
};
use replay_frame::frame::TableView;
use std::collections::BTreeMap;

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
            let named = read.world.representative(blob).map(|p| (p, naming::unknown("Not recorded: no content in this test")));
            walk::observed(&read.world, (&read.tags, &read.identities), read.view, blob, named, hour)
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
    lifetime_columns().unwrap();
}

#[test]
fn a_wars_losses_sum_each_side_and_count_a_countrys_unknown_losses_once_at_their_largest() {
    let membership = |a: u32, b: u32, oriented: bool| WarRelationMembership { key: WarRelationKey::new((a, a), (b, b), 90), oriented, first_hour: 100, last_hour: 110 };
    let (ab, ac, ad) = (membership(1, 2, true), membership(1, 3, true), membership(1, 4, false));
    assert_eq!(losses(&[(&ab, Some([10, 20, 5, 1])), (&ac, Some([30, 40, 7, 2]))]), Some([40, 7, 60, 3]));
    assert_eq!(losses(&[(&ad, Some([10, 20, 5, 1]))]), Some([20, 1, 10, 5]));
    assert_eq!(losses(&[(&ab, Some([10, 20, 5, 1])), (&ac, None)]), None, "a lifetime not known leaves the war's casualties not known, never a smaller sum");
    assert_eq!(losses(&[]), Some([0; 4]));
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
