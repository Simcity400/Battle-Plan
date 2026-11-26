use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::{
    event_feed_rows::{
        EventFeedCause, EventFeedCountry, EventFeedCounts, EventFeedEventIdentity, EventFeedRoute, EventFeedScope, EventFeedScript, EventFeedScriptChange, EventPresentation, EventWindows,
        OfferedOption, VerdictEntry, WindowKind, WindowVerdict,
    },
    format::FormatVersion,
    rows::{CountryCounterInputRow, CountryLinkRow, CountryPowerChangeRow, CountryRow, PoliticsCaptureCoverageRow, PowerAction, PowerCause, PowerCurrency, PowerIdentity, RelationTupleRow},
};

const TEXTS: &[&[u8]] = &[b"GER", b"POL", b"", b"dec_a"];
const NONE: u32 = TagHolderRow::NONE;
const GUARANTEE: u32 = engine_native_parts::cguarantee_relation::TYPE;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Feed::default())]);
    writers.start_run();
    writers
}

fn country(reference: u32) -> EventFeedCountry {
    EventFeedCountry { reference, id: reference, instance: 0 }
}

fn occurrence(generation: u64, sequence: u64, hour: i32, source: Source, actor: EventFeedCountry, target: EventFeedCountry) -> EventFeedEvent {
    let scripted = !matches!(source, Source::Declaration | Source::Join | Source::Annex | Source::WarEnded);
    EventFeedEvent {
        sequence,
        generation,
        hour,
        source,
        actor,
        target,
        cause: Recorded::NotRecorded,
        script: scripted.then(EventFeedScript::default),
    }
}

fn decision(mut event: EventFeedEvent) -> EventFeedEvent {
    event.script = Some(EventFeedScript { token_len: 5, ..EventFeedScript::default() });
    event
}

fn dispatched(mut event: EventFeedEvent, option: Option<i32>) -> EventFeedEvent {
    event.script = Some(EventFeedScript { native_event: Some(EventFeedEventIdentity { kind: 3, id: 44 }), instance: Some(5), option, ..EventFeedScript::default() });
    event
}

fn pack(generation: u64, read_hour: i32, coverage: EventFeedCoverage, events: Vec<EventFeedEvent>, tokens: &[u8], scopes: Vec<EventFeedScope>) -> EventFeedHistoryRow {
    let n = events.len() as u64;
    EventFeedHistoryRow {
        generation,
        read_hour,
        coverage,
        counts: Some(EventFeedCounts { acquired: n, settled: n, failed: 0, rejected: 0 }),
        outstanding: 0,
        failure: None,
        script_refusals: 0,
        events,
        script_tokens: tokens.to_vec(),
        script_scopes: scopes,
        windows: Recorded::Value(EventWindows { verdicts: Ok(vec![VerdictEntry { sequence: 2, verdict: WindowVerdict::Requested }]), presentations: Vec::new(), presentations_unrecorded: None }),
    }
}

fn read_at(hour: i32) -> EventFeedHistoryRow {
    let complete = EventFeedCoverage::Complete;
    let (ger, pol, alias) = (country(1), country(2), EventFeedCountry { reference: 1, id: 99, instance: 0 });
    match hour {
        0 => {
            let mut war = occurrence(9, 1, 0, Source::Declaration, ger, pol);
            war.cause = Recorded::Value(EventFeedCause { route: EventFeedRoute::DiplomaticCommand, flags: 1, ..EventFeedCause::default() });
            let mut sent = dispatched(occurrence(9, 2, 0, Source::ScriptEvent, country(0), pol), None);
            sent.script = sent.script.map(|script| EventFeedScript { scope_len: 1, ..script });
            let scope = EventFeedScope { country: 2, country_attempted: true, country_id: Some(2), ..EventFeedScope::default() };
            pack(9, 0, complete, vec![war, sent], b"", vec![scope])
        }
        1 => {
            let chosen = dispatched(occurrence(9, 3, 1, Source::EventOption, pol, country(0)), Some(0));
            let selected = decision(occurrence(9, 4, 1, Source::DecisionSelected, ger, country(0)));
            let mut registry = occurrence(9, 5, 1, Source::ScriptState, country(0), country(0));
            registry.script = Some(EventFeedScript { change: Some(EventFeedScriptChange::PendingBaseline), ..EventFeedScript::default() });
            pack(9, 1, complete, vec![chosen, selected, registry], b"dec_a", Vec::new())
        }
        2 => {
            let completed = decision(occurrence(9, 6, 2, Source::DecisionCompleted, ger, country(0)));
            let joined = occurrence(9, 7, 1, Source::Join, alias, pol);
            pack(9, 2, complete, vec![completed, joined], b"dec_a", Vec::new())
        }
        3 => pack(9, 3, EventFeedCoverage::Partial, vec![decision(occurrence(9, 8, 3, Source::DecisionSelected, ger, country(0)))], b"dec_a", Vec::new()),
        4 => pack(9, 4, complete, vec![decision(occurrence(9, 9, 4, Source::DecisionCompleted, ger, country(0)))], b"dec_a", Vec::new()),
        _ => pack(10, hour, complete, vec![dispatched(occurrence(10, 1, hour, Source::EventOption, pol, country(0)), Some(0))], b"", Vec::new()),
    }
}

fn guarantee(owner: u32, target: u32, ordinal: u16, first: u32, second: u32, start: i32) -> RelationTupleRow {
    RelationTupleRow { owner_country: owner, target_country: target, type_id: GUARANTEE, ordinal, first_country: first, second_country: second, start_hour: start, ..RelationTupleRow::default() }
}

fn world(store: &mut Store, hour: i32) {
    if hour == 0 {
        for (id, slot, tag) in [(1, 10, b"GER"), (2, 20, b"POL")] {
            testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..CountryRow::default() }, TEXTS).unwrap();
        }
        for (id, slot) in [(1, 10), (2, 20), (7, 10)] {
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
        }
        testing::put(store, TableId::RelationTuples, &guarantee(1, 2, 0, 1, 2, -50), TEXTS).unwrap();
    }
    for (country, owned) in [(1, 5), (2, if hour >= 3 { 0 } else { 3 })] {
        testing::put(store, TableId::CountryCounterInputs, &CountryCounterInputRow { country, owned_state_count: owned, ..CountryCounterInputRow::default() }, TEXTS).unwrap();
    }
    if hour == 2 {
        testing::put(store, TableId::RelationTuples, &guarantee(2, 1, 0, 2, 1, 2), TEXTS).unwrap();
    }
    if hour == 3 {
        testing::put(store, TableId::RelationTuples, &guarantee(1, 2, 1, 2, 1, 2), TEXTS).unwrap();
    }
    testing::put(store, TableId::EventFeedHistory, &read_at(hour), TEXTS).unwrap();
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, setup: impl FnOnce(&mut Store)) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for table in [TableId::EventFeedHistory, TableId::Countries, TableId::CountryLinks, TableId::CountryCounterInputs, TableId::RelationTuples] {
        store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    }
    setup(store);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn feed(store: &Store) -> Vec<FeedEventRow> {
    let view = store.view(TableId::FeedEvents.index());
    let format = FormatVersion::CURRENT.version();
    let mut rows: Vec<FeedEventRow> = view
        .rows()
        .map(|slot| FeedEventRow::from_wire(view.key(slot).unwrap(), &mut |column| replay_frame::frame::field(&view, slot, column, format)).unwrap())
        .collect();
    rows.sort_unstable_by_key(RowCodec::key);
    rows
}

fn brief(rows: &[FeedEventRow]) -> Vec<(u8, u64, u64, Recorded<u32>, Option<u64>)> {
    rows.iter().map(|row| (row.kind, row.generation, row.sequence, row.actor_slot, row.opener)).collect()
}

fn story() -> Vec<Vec<FeedEventRow>> {
    let (mut store, mut writers) = (testing::store(), writers());
    (0..=5)
        .map(|hour| {
            run(&mut store, &mut writers, hour, |store| world(store, hour));
            feed(&store)
        })
        .collect()
}

#[test]
fn each_occurrence_is_a_row_of_its_read_hour_keyed_by_the_collectors_identity_and_registry_changes_are_none() {
    let seen = story();
    let derived = |hour: i32, ordinal: u64| u64::from(hour as u32) << 32 | ordinal;
    let (declaration, dispatch, option, selected, completed, join) =
        (Source::Declaration as u8, Source::ScriptEvent as u8, Source::EventOption as u8, Source::DecisionSelected as u8, Source::DecisionCompleted as u8, Source::Join as u8);
    let value = Recorded::Value;
    assert_eq!(
        seen.iter().map(|rows| brief(rows)).collect::<Vec<_>>(),
        [
            vec![(declaration, 9, 1, value(10), None), (dispatch, 9, 2, value(NONE), None)],
            vec![(option, 9, 3, value(20), Some(2)), (selected, 9, 4, value(10), None)],
            vec![(completed, 9, 6, value(10), Some(4)), (join, 9, 7, Recorded::NotRecorded, None), (FeedEventRow::GUARANTEE_GRANTED, FeedEventRow::DERIVED, derived(2, 0), value(20), None)],
            vec![
                (selected, 9, 8, value(10), None),
                (FeedEventRow::READ_INCOMPLETE, FeedEventRow::DERIVED, derived(3, 0), value(NONE), None),
                (FeedEventRow::CEASED_TO_EXIST, FeedEventRow::DERIVED, derived(3, 1), value(20), None),
            ],
            vec![(completed, 9, 9, value(10), None)],
            vec![(option, 10, 1, value(20), None)],
        ],
        "rows live their read's hour; a choice continues its dispatch and a completion its selection; \
         a partial read opens no decision and is a row of its own; a new generation closes every opening; an id the link's holder \
         does not have resolves no slot; the guarantee held at the run's first hour and its second listing are none"
    );
}

#[test]
fn a_rows_operands_are_typed_and_what_was_not_recorded_stays_so() {
    let seen = story();
    let war = &seen[0][0];
    assert_eq!((war.hour, war.read_hour, war.target_slot), (0, 0, Recorded::Value(20)));
    assert_eq!((war.route, war.flags, war.ally_slot), (Recorded::Value(EventFeedRoute::DiplomaticCommand as u8), Recorded::Value(1), Recorded::Value(None)));
    assert_eq!((war.token.clone(), war.political_power), (Recorded::Value(Vec::new()), Recorded::Value(None)), "an occurrence without script has none to name");
    let sent = &seen[0][1];
    assert_eq!((sent.native_event_kind, sent.native_event_id, sent.instance), (Some(3), Some(44), Some(5)));
    match &sent.scopes {
        Recorded::Value(scopes) => assert_eq!(scopes.iter().map(|scope| (scope.country, scope.country_id)).collect::<Vec<_>>(), [(2, Some(2))]),
        Recorded::NotRecorded => panic!("the dispatch's scope graph was recorded"),
    }
    assert_eq!((seen[1][0].opener_hour, seen[2][0].opener_hour), (Some(0), Some(1)), "a continuing row names its opener's read hour");
    let partial = &seen[3][1];
    assert_eq!(
        (partial.read_hour, partial.coverage, partial.outstanding, partial.refusals, partial.failure.clone()),
        (3, Some(EventFeedCoverage::Partial as u32), Some(0), Some(0), Vec::new()),
        "a read not complete names its coverage"
    );
    assert_eq!(partial.incomplete, Some(EventFeedCoverage::Partial as u32), "its facet: coverage in the low byte, no family refused");
    assert_eq!(FeedEventRow::incomplete_of(EventFeedCoverage::Unavailable as u8, 0b101), 0b101 << FeedEventRow::REFUSALS_SHIFT);
    assert!(seen.iter().flatten().filter(|row| row.kind != FeedEventRow::READ_INCOMPLETE).all(|row| row.incomplete.is_none()), "only an incomplete read is counted by the facet");
    let selected = &seen[1][1];
    assert_eq!(selected.token, Recorded::Value(b"dec_a".to_vec()));
    assert_eq!((selected.political_power, selected.command_power), (Recorded::NotRecorded, Recorded::NotRecorded), "no power changes were read: no charge is proven");
    let joined = &seen[2][1];
    assert_eq!((joined.hour, joined.read_hour), (1, 2), "an occurrence keeps its own hour beside its read's");
    assert_eq!((joined.route, joined.reason, joined.ally_reference), (Recorded::NotRecorded, Recorded::NotRecorded, Recorded::NotRecorded), "a cause not recorded leaves every cause column so");
    let granted = &seen[2][2];
    assert_eq!((granted.actor_reference, granted.target_reference, granted.target_slot, granted.dated), (2, 1, Recorded::Value(10), Some(2)));
    let ceased = &seen[3][2];
    assert_eq!((ceased.actor_reference, ceased.dated), (2, Some(2)), "dated at the last read POL still owned states");
}

const TABLES: [TableId; 4] = [TableId::FeedEvents, TableId::FeedOpenEvents, TableId::FeedOpenDecisions, TableId::FeedGuarantees];

fn story_tables() -> Vec<Vec<Vec<(u128, Vec<String>)>>> {
    let (mut store, mut writers) = (testing::store(), writers());
    (0..=5)
        .map(|hour| {
            run(&mut store, &mut writers, hour, |store| world(store, hour));
            TABLES.iter().map(|&table| testing::rows(&store, table)).collect()
        })
        .collect()
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let expected = story_tables();
    for stop in 0..=4 {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour, |store| world(store, hour));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=5 {
            run(&mut resumed, &mut after, hour, |store| world(store, hour));
            let tables: Vec<_> = TABLES.iter().map(|&table| testing::rows(&resumed, table)).collect();
            assert_eq!(tables, expected[hour as usize], "stopped after hour {stop}, hour {hour}: the open cards and guarantees come back from their tables");
        }
    }
}

#[test]
fn the_open_tables_hold_open_cards_only_and_a_guarantee_is_held_once() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    for hour in 0..=5 {
        run(&mut store, &mut writers, hour, |store| world(store, hour));
        let keys = |table| testing::rows(&store, table).into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        seen.push((keys(TableId::FeedOpenEvents), keys(TableId::FeedOpenDecisions), keys(TableId::FeedGuarantees)));
    }
    let dispatch = 3_u128 << 96 | 44 << 64 | 5 << 32 | 2;
    let selection = 9_u128 << 64 | 4;
    let guarantee = 2_u128 << 64 | 1 << 32 | 2;
    assert_eq!(
        seen,
        [
            (vec![dispatch], vec![], vec![]),
            (vec![], vec![selection], vec![]),
            (vec![], vec![], vec![guarantee]),
            (vec![], vec![], vec![guarantee]),
            (vec![], vec![], vec![guarantee]),
            (vec![], vec![], vec![guarantee]),
        ],
        "a dispatch lives until its choice, a selection until its completion; a partial read opens no decision; \
         the guarantee is held once though listed again"
    );
}

#[test]
fn a_guarantee_is_held_while_a_relation_row_holds_it_and_written_once() {
    let held = |owner: u32, ordinal: u16, start: i32, cancel: u8| RelationTupleRow { cancel, ..guarantee(owner, 3 - owner, ordinal, 2, 1, start) };
    let key = |row: &RelationTupleRow| row.key();
    let (g, k) = (2_u128 << 64 | 1 << 32 | 1, 2_u128 << 64 | 1 << 32 | 7);
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    for hour in 0..=10 {
        store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
        let relations = if hour == 9 { Status::Truncated { missing: 1 } } else { Status::Complete };
        for table in [TableId::EventFeedHistory, TableId::Countries, TableId::CountryLinks, TableId::CountryCounterInputs] {
            store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        }
        store.status(TableId::RelationTuples.index(), relations, true, Some(hour), true).unwrap();
        let put = |store: &mut Store, row: RelationTupleRow| testing::put(store, TableId::RelationTuples, &row, TEXTS).unwrap();
        let gone = |store: &mut Store, row: RelationTupleRow| testing::remove(store, TableId::RelationTuples, key(&row)).unwrap();
        match hour {
            0 => {
                for (id, slot, tag) in [(1, 10, b"GER"), (2, 20, b"POL")] {
                    testing::put(&mut store, TableId::Countries, &CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..CountryRow::default() }, TEXTS).unwrap();
                    testing::put(&mut store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
                }
                put(&mut store, guarantee(1, 2, 0, 1, 2, -50));
            }
            1 => put(&mut store, held(2, 0, 1, 0)),
            2 => {
                gone(&mut store, held(2, 0, 1, 0));
                put(&mut store, held(2, 1, 1, 0));
                put(&mut store, held(1, 1, 1, 0));
            }
            3 => put(&mut store, held(2, 1, 1, 1)),
            4 => put(&mut store, held(2, 1, 1, 0)),
            5 => gone(&mut store, held(2, 1, 1, 0)),
            6 => gone(&mut store, held(1, 1, 1, 0)),
            7 => put(&mut store, held(2, 0, 7, 1)),
            8 => put(&mut store, held(2, 0, 7, 0)),
            9 => gone(&mut store, held(2, 0, 7, 0)),
            _ => {}
        }
        store.settle();
        writers.hour(1, hour, hour == 0, &mut store).unwrap();
        store.end_hour(&mut Discard).unwrap();
        let tracked = store.view(TableId::FeedGuarantees.index());
        let rows = tracked.rows().map(|slot| (tracked.key(slot).unwrap(), tracked.number(0, slot), tracked.number(1, slot).unwrap())).collect::<Vec<_>>();
        let granted = feed(&store).iter().filter(|row| row.kind == FeedEventRow::GUARANTEE_GRANTED).map(|row| row.dated).collect::<Vec<_>>();
        seen.push((rows, granted));
    }
    assert_eq!(
        seen,
        [
            (vec![], vec![]),
            (vec![(g, Some(1), 1)], vec![Some(1)]),
            (vec![(g, Some(1), 2)], vec![]),
            (vec![(g, Some(1), 2)], vec![]),
            (vec![(g, Some(1), 2)], vec![]),
            (vec![(g, Some(1), 1)], vec![]),
            (vec![], vec![]),
            (vec![(k, None, 1)], vec![]),
            (vec![(k, Some(8), 1)], vec![Some(7)]),
            (vec![(k, Some(8), 1)], vec![]),
            (vec![], vec![]),
        ],
        "written once at its first uncancelled read; held through a move, a second slice, a cancel and its return; \
         held by its other row, gone with its last, or at the whole read after a read not complete"
    );
}

#[test]
fn a_decision_charge_is_proven_only_for_one_execution_of_a_country_whose_power_was_captured() {
    let charge = |country: u32, sequence: u64, currency: PowerCurrency, delta: i64| CountryPowerChangeRow {
        country,
        sequence,
        hour: 1,
        currency,
        balance_before_raw: 100_000,
        balance_after_raw: replay_data::rows::RetiredI64(0),
        realized_delta_raw: replay_data::rows::Recorded::Value(delta),
        cause: PowerCause::Decision,
        identity: PowerIdentity { action: PowerAction::Execute, item_key: b"dec_a".to_vec(), ..PowerIdentity::default() },
        ..CountryPowerChangeRow::default()
    };
    let (mut store, mut writers) = (testing::store(), writers());
    let (ger, pol) = (country(1), country(2));
    run(&mut store, &mut writers, 0, |store| {
        world(store, 0);
        testing::put(store, TableId::EventFeedHistory, &pack(9, 0, EventFeedCoverage::Complete, Vec::new(), b"", Vec::new()), TEXTS).unwrap();
    });
    run(&mut store, &mut writers, 1, |store| {
        for table in [TableId::CountryPowerChanges, TableId::PoliticsCaptureCoverage] {
            store.status(table.index(), Status::Complete, true, Some(1), true).unwrap();
        }
        for row in [charge(1, 1, PowerCurrency::Political, -50_000), charge(1, 2, PowerCurrency::Command, -5_000), charge(2, 3, PowerCurrency::Political, -1), charge(2, 4, PowerCurrency::Command, -2)] {
            testing::put(store, TableId::CountryPowerChanges, &row, TEXTS).unwrap();
        }
        let streams = PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::CP;
        testing::put(store, TableId::PoliticsCaptureCoverage, &PoliticsCaptureCoverageRow { country: 1, since_hour: 0, through_hour: 10, streams, ..PoliticsCaptureCoverageRow::default() }, TEXTS).unwrap();
        testing::put(store, TableId::PoliticsCaptureCoverage, &PoliticsCaptureCoverageRow { country: 2, since_hour: 0, through_hour: 10, streams: PoliticsCaptureCoverageRow::PP, ..PoliticsCaptureCoverageRow::default() }, TEXTS).unwrap();
        let events = vec![
            decision(occurrence(9, 1, 1, Source::DecisionSelected, ger, country(0))),
            decision(occurrence(9, 2, 1, Source::DecisionSelected, pol, country(0))),
            occurrence(9, 3, 1, Source::Declaration, ger, pol),
        ];
        testing::put(store, TableId::EventFeedHistory, &pack(9, 1, EventFeedCoverage::Complete, events, b"dec_a", Vec::new()), TEXTS).unwrap();
    });
    let powers: Vec<_> = feed(&store).into_iter().map(|row| (row.sequence, row.political_power, row.command_power)).collect();
    assert_eq!(
        powers,
        [
            (1, Recorded::Value(Some(-50_000)), Recorded::Value(Some(-5_000))),
            (2, Recorded::Value(Some(-1)), Recorded::NotRecorded),
            (3, Recorded::Value(None), Recorded::Value(None)),
        ],
        "GER's power was captured over the hour; POL's political power was and its command power was not, so each currency says so; a war declaration is charged nothing"
    );
    run(&mut store, &mut writers, 2, |store| {
        for table in [TableId::CountryPowerChanges, TableId::PoliticsCaptureCoverage] {
            store.status(table.index(), Status::Complete, true, Some(2), true).unwrap();
        }
        for row in [CountryPowerChangeRow { hour: 2, ..charge(1, 5, PowerCurrency::Political, -10) }, CountryPowerChangeRow { hour: 2, ..charge(1, 6, PowerCurrency::Political, -10) }] {
            testing::put(store, TableId::CountryPowerChanges, &row, TEXTS).unwrap();
        }
        let streams = PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::CP;
        testing::put(store, TableId::PoliticsCaptureCoverage, &PoliticsCaptureCoverageRow { country: 1, since_hour: 0, through_hour: 10, streams, ..PoliticsCaptureCoverageRow::default() }, TEXTS).unwrap();
        let selected = decision(occurrence(9, 4, 2, Source::DecisionSelected, ger, country(0)));
        testing::put(store, TableId::EventFeedHistory, &pack(9, 2, EventFeedCoverage::Partial, vec![selected], b"dec_a", Vec::new()), TEXTS).unwrap();
    });
    let partial: Vec<_> = feed(&store).into_iter().filter(|row| row.generation == 9).map(|row| (row.sequence, row.political_power, row.command_power)).collect();
    assert_eq!(partial, [(4, Recorded::NotRecorded, Recorded::NotRecorded)], "a read that may have lost a second execution proves no charge for the one it kept");
}

#[test]
fn an_occurrence_before_the_last_one_read_withholds_the_log() {
    let (mut store, mut writers) = (testing::store(), writers());
    run(&mut store, &mut writers, 0, |store| world(store, 0));
    run(&mut store, &mut writers, 1, |store| {
        let event = occurrence(9, 2, 1, Source::Declaration, country(1), country(2));
        testing::put(store, TableId::EventFeedHistory, &pack(9, 1, EventFeedCoverage::Complete, vec![event], b"", Vec::new()), TEXTS).unwrap();
    });
    assert!(matches!(store.view(TableId::FeedEvents.index()).status(), Status::Unavailable { .. }), "sequence 2 was read at hour 0");
}

#[test]
fn a_backlog_read_is_told_apart_from_the_hours_events_and_still_opens_its_cards() {
    let mut store = testing::store();
    store.begin_hour(0, TEXTS).unwrap();
    for table in [TableId::EventFeedHistory, TableId::Countries, TableId::CountryLinks, TableId::CountryCounterInputs, TableId::RelationTuples] {
        store.status(table.index(), Status::Complete, true, Some(0), true).unwrap();
    }
    world(&mut store, 0);
    store.settle();
    let events = store.lend(TableId::FeedEvents.index()).unwrap();
    let hour = Hour::new(&store, 1, 0, true, true, &[]);
    let mut feed = Feed::default();
    let (backlog, opener) = (replay_data::rows::column::<FeedEventRow>("backlog"), replay_data::rows::column::<FeedEventRow>("opener"));
    let key = |sequence: u64| 9_u128 << 64 | u128::from(sequence);
    let (mut rows, mut ordinal, mut open) = (Rows::lending(&store, vec![events]), 0, Open::default());
    feed.occurrences(&hour, &Pack { row: read_at(0), backlog: true }, &mut open, &mut ordinal, &mut rows).unwrap();
    feed.occurrences(&hour, &Pack { row: read_at(1), backlog: false }, &mut open, &mut ordinal, &mut rows).unwrap();
    let view = rows.table(TableId::FeedEvents);
    let cell = |sequence: u64, column: usize| view.slot(key(sequence)).and_then(|slot| view.number(column, slot));
    assert_eq!((cell(1, backlog), cell(2, backlog)), (Some(1), Some(1)), "the list as the run began is no event of its read");
    assert_eq!(cell(3, backlog), Some(0));
    assert_eq!(cell(3, opener), Some(2), "a choice continues a dispatch listed when the run began");
}

fn window(read_hour: i32, presentations: Vec<EventPresentation>, lost: Option<Vec<u8>>) -> EventFeedHistoryRow {
    let mut sent = dispatched(occurrence(9, 2, read_hour, Source::ScriptEvent, country(0), country(2)), None);
    sent.script = sent.script.map(|script| EventFeedScript { scope_len: 0, ..script });
    let mut row = pack(9, read_hour, EventFeedCoverage::Complete, vec![sent], b"", Vec::new());
    row.windows = Recorded::Value(EventWindows { verdicts: Ok(Vec::new()), presentations, presentations_unrecorded: lost });
    row
}

#[test]
fn a_firing_carries_its_window_decision_and_each_built_window_is_a_row_with_what_it_showed() {
    let seen = story();
    assert_eq!(seen[0][1].window, Recorded::Value(Some(u32::from(WindowVerdict::Requested.code()))), "the dispatch carries its decision");
    assert_eq!(seen[0][0].window, Recorded::Value(None), "a declaration has no window to decide");
    let presented = EventPresentation {
        native_hour: Some(0),
        instance: 5,
        event_kind: 3,
        event_id: 44,
        actor: 2,
        kind: WindowKind::Normal,
        shown: Some(true),
        option_count: 2,
        token: Recorded::Value(b"news.44".to_vec()),
        title: Recorded::Value(b"Resolved title".to_vec()),
        description: Recorded::NotRecorded,
        picture: Recorded::Value(None),
        options: Recorded::Value(vec![OfferedOption { ordinal: Some(1), label: Some(b"Second".to_vec()) }]),
        ..EventPresentation::default()
    };
    for (coverage, lost) in [(EventFeedCoverage::Complete, None), (EventFeedCoverage::Partial, Some(b"lost".to_vec()))] {
        let (mut store, mut writers) = (testing::store(), writers());
        let expected = 1;
        let lost_any = lost.is_some();
        let mut read = window(0, vec![presented.clone()], lost);
        read.coverage = coverage;
        run(&mut store, &mut writers, 0, |store| {
            world(store, 0);
            testing::put(store, TableId::EventFeedHistory, &read, TEXTS).unwrap();
        });
        let rows = feed(&store);
        let firing = rows.iter().find(|row| row.kind == Source::ScriptEvent as u8).unwrap();
        assert_eq!(firing.window, Recorded::NotRecorded, "an armed hour without the firing's decision does not guess one");
        let shown: Vec<_> = rows.iter().filter(|row| row.kind == FeedEventRow::WINDOW_PRESENTED).collect();
        assert_eq!(shown.len(), expected);
        if let Some(row) = shown.first() {
            assert_eq!((row.instance, row.native_event_id, row.actor_reference, row.token.clone()), (Some(5), Some(44), 2, Recorded::Value(b"news.44".to_vec())));
            assert_eq!(row.presentation, Recorded::Value(Some(presented.clone())));
        }
        let unrecorded: Vec<_> = rows.iter().filter(|row| row.kind == FeedEventRow::WINDOWS_UNRECORDED).collect();
        assert_eq!(unrecorded.len(), usize::from(lost_any), "a partial read still publishes its windows beside the named loss");
        if let Some(row) = unrecorded.first() {
            assert_eq!((row.incomplete, row.failure.clone()), (Some(FeedEventRow::PRESENTATIONS_UNRECORDED), b"lost".to_vec()));
        }
    }
}
