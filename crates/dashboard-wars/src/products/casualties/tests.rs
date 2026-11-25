use super::*;
use dashboard_base::products::breakdown::{BreakdownAmount, BreakdownState};
use replay_data::rows::CasualtyTotalRow;
use replay_lanes::lanes::Step;

fn key(a: Identity, b: Identity, start: i32) -> u128 {
    CasualtyLifetimeRow { slot_a: a.0, id_a: a.1, slot_b: b.0, id_b: b.1, start_hour: start, ..Default::default() }.key()
}

fn figure(whole: &str) -> Option<BreakdownAmount> {
    Some(BreakdownAmount { numerator: whole.into(), denominator: "1".into() })
}

const ONE: Identity = (1, 11);
const TWO: Identity = (2, 22);
const THREE: Identity = (3, 33);
const FOUR: Identity = (4, 44);

fn complete_history(first: i32, last: i32, columns: Vec<usize>, mut keys: Vec<(u128, Vec<Step>)>) -> History {
    keys.sort_unstable_by_key(|(key, _)| *key);
    History { columns, to: last, texts: Vec::new(), keys, statuses: vec![StatusRun { first, last, status: Status::Complete }], reasons: HashMap::new() }
}

fn step(hour: i32, values: &[i64]) -> Step {
    Step { hour, cells: Some(values.iter().copied().map(Cell::Number).collect::<Vec<_>>().into_boxed_slice()) }
}

fn war_name(text: &str) -> WarName {
    WarName { text: text.into(), known: true, reason: None, key: None }
}

#[test]
fn a_lifetime_key_and_a_holder_key_are_the_row_codecs() {
    let packed = key((0xff_fffe, 7), (3, 0xab_cdef), -5);
    assert_eq!(Lifetime::of(packed), Lifetime { a: (0xff_fffe, 7), b: (3, 0xab_cdef), start: -5 });
    assert_eq!(holder_key((9, 4)), CasualtyTotalRow { slot: 9, id: 4, ..Default::default() }.key());
}

#[test]
fn enemies_are_keyed_by_recorded_slot_unnamed_enemies_are_kept_and_ended_relations_carry_their_last_hour() {
    let live_key = key(THREE, ONE, 100);
    let ended_key = key(THREE, TWO, 100);
    let unnamed_key = key(THREE, FOUR, 100);
    let zero_key = key(THREE, (5, 55), 100);
    let lifetimes = complete_history(
        100,
        150,
        vec![0, 1],
        vec![
            (live_key, vec![step(100, &[40, 0]), step(130, &[60, 0])]),
            (ended_key, vec![step(100, &[25, 0])]),
            (unnamed_key, vec![step(100, &[15, 0])]),
            (zero_key, vec![step(100, &[0, 0])]),
        ],
    );
    let totals = complete_history(100, 150, vec![0], vec![(holder_key(THREE), vec![step(100, &[0])])]);
    let grouping = |first: i32, last: i32, war: &str| vec![Grouping { first, last, war: Some(war.into()), ambiguous: false }];
    let stretch = StretchData {
        first: 100,
        last: 150,
        taken_a_pos: 0,
        taken_b_pos: 1,
        partial_pos: 0,
        lifetimes,
        totals,
        lives: vec![
            TrackedLife { key: live_key, country: THREE, enemy: ONE, is_a: true, born: 100, gone: 150, ended_value: None, groupings: grouping(100, 150, "war:1") },
            TrackedLife { key: ended_key, country: THREE, enemy: TWO, is_a: true, born: 100, gone: 110, ended_value: Some(25), groupings: grouping(100, 110, "war:1") },
            TrackedLife { key: unnamed_key, country: THREE, enemy: FOUR, is_a: true, born: 100, gone: 150, ended_value: None, groupings: grouping(100, 150, "war:1") },
            TrackedLife { key: zero_key, country: THREE, enemy: (5, 55), is_a: true, born: 100, gone: 150, ended_value: None, groupings: grouping(100, 150, "war:2") },
        ],
    };
    let index = finalize_index(
        50,
        100,
        150,
        "THREE".into(),
        Vec::new(),
        Vec::new(),
        vec![HolderRun { first: 100, last: 150, holder: Holder::Found { slot: THREE.0, id: THREE.1 }, starts_segment: true }],
        vec![stretch],
        HashMap::from([(ONE.0, vec![(100, 150, "ONE".into())]), (TWO.0, vec![(100, 110, "TWO".into())])]),
        HashMap::from([("war:1".into(), WarEntry { ordinal: 0, names: vec![(100, 150, war_name("First War"))] })]),
        Vec::new(),
    );

    let at_105 = index.at(105);
    assert_eq!(at_105.from, 50, "from is the branch start, never the 720h window start");
    assert_eq!(at_105.through, 110, "stands quietly until ended_key ends at 111");
    assert_eq!(at_105.breakdown.total.amount, figure("80"));
    assert_eq!(at_105.breakdown.parts.len(), 1);
    let war = &at_105.breakdown.parts[0];
    assert_eq!(war.name, "First War");
    let enemy_by = |key: &str| war.children.iter().find(|part| part.key == key).expect("enemy part");
    assert_eq!((enemy_by("slot:1").ordinal, enemy_by("slot:1").last, enemy_by("slot:1").other), (1, None, false));
    assert_eq!((enemy_by("slot:2").ordinal, enemy_by("slot:2").last, enemy_by("slot:2").other), (2, None, false));
    assert_eq!((enemy_by("slot:4").ordinal, enemy_by("slot:4").name.as_str(), enemy_by("slot:4").other), (4, "Country not named at this hour", true), "unnamed enemies are kept as other");

    let at_115 = index.at(115);
    assert_eq!(at_115.through, 129, "stands quietly until live_key changes at 130");
    let war = &at_115.breakdown.parts[0];
    let ended = war.children.iter().find(|part| part.key == "slot:2").expect("ended enemy");
    assert_eq!(ended.last, Some(110), "ended relation resolves enemy identity at its last active read");
    assert_eq!(ended.name, "TWO");
    let live = war.children.iter().find(|part| part.key == "slot:1").expect("live enemy");
    assert_eq!(live.last, None);
}

#[test]
fn live_and_ended_lifetimes_for_the_same_war_and_enemy_keep_last_none_while_live() {
    let ended_key = key(THREE, ONE, 100);
    let live_key = key(THREE, ONE, 120);
    let lifetimes = complete_history(100, 150, vec![0, 1], vec![(ended_key, vec![step(100, &[10, 0])]), (live_key, vec![step(120, &[20, 0])])]);
    let totals = complete_history(100, 150, vec![0], vec![(holder_key(THREE), vec![step(100, &[0])])]);
    let grouping = vec![Grouping { first: 100, last: 150, war: Some("war:1".into()), ambiguous: false }];
    let stretch = StretchData {
        first: 100,
        last: 150,
        taken_a_pos: 0,
        taken_b_pos: 1,
        partial_pos: 0,
        lifetimes,
        totals,
        lives: vec![
            TrackedLife { key: ended_key, country: THREE, enemy: ONE, is_a: true, born: 100, gone: 110, ended_value: Some(10), groupings: grouping.clone() },
            TrackedLife { key: live_key, country: THREE, enemy: ONE, is_a: true, born: 120, gone: 150, ended_value: None, groupings: grouping },
        ],
    };
    let index = finalize_index(
        0,
        100,
        150,
        "THREE".into(),
        Vec::new(),
        Vec::new(),
        vec![HolderRun { first: 100, last: 150, holder: Holder::Found { slot: THREE.0, id: THREE.1 }, starts_segment: true }],
        vec![stretch],
        HashMap::from([(ONE.0, vec![(100, 150, "ONE".into())])]),
        HashMap::from([("war:1".into(), WarEntry { ordinal: 0, names: vec![(100, 150, war_name("War"))] })]),
        Vec::new(),
    );
    let at_125 = index.at(125);
    let enemy = &at_125.breakdown.parts[0].children[0];
    assert_eq!(enemy.amount, figure("30"));
    assert_eq!(enemy.last, None, "a live lifetime in the pair keeps the pair live");
}

#[test]
fn coverage_gaps_holder_unknowns_truncation_missing_ended_and_unread_totals_never_become_complete_zero() {
    let live_key = key(THREE, ONE, 100);
    let missing_ended = key(THREE, TWO, 100);
    let lifetimes = History {
        columns: vec![0, 1],
        to: 140,
        texts: Vec::new(),
        keys: vec![(live_key, vec![step(100, &[10, 0])])],
        statuses: vec![
            StatusRun { first: 100, last: 109, status: Status::Complete },
            StatusRun { first: 110, last: 119, status: Status::Truncated { missing: 2 } },
            StatusRun { first: 120, last: 129, status: Status::NotRecorded },
            StatusRun { first: 130, last: 140, status: Status::Complete },
        ],
        reasons: HashMap::new(),
    };
    let totals = History {
        columns: vec![0],
        to: 140,
        texts: Vec::new(),
        keys: vec![(holder_key(THREE), vec![step(100, &[0])])],
        statuses: vec![
            StatusRun { first: 100, last: 129, status: Status::Complete },
            StatusRun { first: 130, last: 140, status: Status::NotRecorded },
        ],
        reasons: HashMap::new(),
    };
    let stretch = StretchData {
        first: 100,
        last: 140,
        taken_a_pos: 0,
        taken_b_pos: 1,
        partial_pos: 0,
        lifetimes,
        totals,
        lives: vec![
            TrackedLife { key: live_key, country: THREE, enemy: ONE, is_a: true, born: 100, gone: 140, ended_value: None, groupings: Vec::new() },
            TrackedLife { key: missing_ended, country: THREE, enemy: TWO, is_a: true, born: 100, gone: 104, ended_value: None, groupings: Vec::new() },
        ],
    };
    let index = finalize_index(
        0,
        100,
        140,
        Country::Slot(3).key().into_owned(),
        vec![(100, 101, "tag table unreadable".into())],
        vec![(102, 103)],
        vec![HolderRun { first: 104, last: 140, holder: Holder::Found { slot: THREE.0, id: THREE.1 }, starts_segment: true }],
        vec![stretch],
        HashMap::from([(ONE.0, vec![(100, 140, "ONE".into())]), (TWO.0, vec![(100, 140, "TWO".into())])]),
        HashMap::new(),
        Vec::new(),
    );

    let unknown_holder = index.at(100);
    assert_eq!(unknown_holder.breakdown.total.state, BreakdownState::NotRecorded);
    assert!(unknown_holder.breakdown.total.reason.as_deref().is_some_and(|r| r.contains("tag table unreadable")));

    let unlinked_slot = index.at(102);
    assert_eq!(unlinked_slot.breakdown.total.state, BreakdownState::NotRecorded);
    assert!(unlinked_slot.breakdown.total.reason.as_deref().is_some_and(|r| r.contains(TAG_UNLINKED)));

    let missing_end = index.at(106);
    assert_eq!(missing_end.breakdown.total.state, BreakdownState::AtLeast, "missing ended counter is NotRecorded, making total AtLeast");
    let war = &missing_end.breakdown.parts[0];
    let ended_part = war.children.iter().find(|part| part.key == "slot:2").expect("missing ended enemy kept");
    assert_eq!(ended_part.state, BreakdownState::NotRecorded);

    let truncated = index.at(115);
    assert_eq!(truncated.breakdown.total.state, BreakdownState::AtLeast, "truncated lifetimes add an Unread war relations leaf");
    assert!(truncated.breakdown.parts.iter().any(|part| part.key == "~unread" && part.other && part.state == BreakdownState::NotRecorded));
    assert!(truncated.breakdown.parts.iter().all(|part| part.share.is_none()));

    let not_recorded = index.at(125);
    assert_eq!(not_recorded.breakdown.total.state, BreakdownState::NotRecorded);

    let unread_totals_empty = finalize_index(
        0,
        130,
        140,
        "THREE".into(),
        Vec::new(),
        Vec::new(),
        vec![HolderRun { first: 130, last: 140, holder: Holder::Found { slot: THREE.0, id: THREE.1 }, starts_segment: true }],
        vec![StretchData {
            first: 130,
            last: 140,
            taken_a_pos: 0,
            taken_b_pos: 1,
            partial_pos: 0,
            lifetimes: complete_history(130, 140, vec![0, 1], Vec::new()),
            totals: History { columns: vec![0], to: 140, texts: Vec::new(), keys: Vec::new(), statuses: vec![StatusRun { first: 130, last: 140, status: Status::NotRecorded }], reasons: HashMap::new() },
            lives: Vec::new(),
        }],
        HashMap::new(),
        HashMap::new(),
        Vec::new(),
    );
    let reply = unread_totals_empty.at(135);
    assert_eq!(reply.breakdown.total.state, BreakdownState::Partial, "unread CasualtyTotals never asserts a complete zero");
    assert!(reply.breakdown.total.reason.is_some());
}

#[test]
fn product_answers_at_the_hour_with_branch_start_and_no_future_warnings() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use std::sync::Arc;
    let frames = (1_000..=1_800).map(|hour| source(1, hour, Some(1))).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "casualties".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let reply = CasualtyBreakdown::load(&source, &CasualtyBreakdownParams { branch_id: "1".into(), country: "GER".into(), hour: 1_750 }, &load, &cancelled).unwrap();
    assert_eq!(reply.from, 1_000, "from stays branch start even in the second 720h window");
    assert_eq!(reply.hour, 1_750);
    assert_eq!(reply.through, 1_800, "quiet stretch stands through the window end");
    assert!(reply.warnings.is_empty());
}
