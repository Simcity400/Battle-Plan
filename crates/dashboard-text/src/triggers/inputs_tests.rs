use super::*;
use crate::triggers::{ContentFacts, World, variables::Store, world::Query};
use replay_data::{
    rows::CompletedFocusRow, rows::CountryCharacterRow, rows::CountryLinkRow, rows::CountryRow,
    rows::FactionMemberRow, rows::LexicalTokenRow, rows::PartyLeaderRow, rows::PoliticalPartyRow,
    rows::ScriptFlagRow, rows::SubjectRow, rows::TableRows, rows::TableSchema,
    rows::WarRelationRow,
};
use replay_frame::{frame::Frame, frame::FrameDecoder, frame::FrameEncoder, frame::TableUpdate};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[test]
fn evaluations_record_queries_and_whole_tables_read_outside_them() {
    let frame = crate::test_fixtures::source(1, 10, Some(7));
    let countries = |frame: &DecodedFrame| frame.complete::<tables::Countries>().unwrap().len();
    let (inner, mut outer) = capture(&frame, || {
        // Read inside a query: only the query's result is an input.
        let rows = query(|| Key::Table(TableId::States), || countries(&frame));
        assert_eq!(rows, 1);
        let ((), inner) = capture(&frame, || {
            countries(&frame);
        });
        inner
    });
    assert_eq!(inner.tables().collect::<Vec<_>>(), [TableId::Countries]);
    assert_eq!(
        outer.reads.len(),
        2,
        "the query, and the inner evaluation's table"
    );
    let unchanged = |_: &Key| Some(digest(&1_usize));
    assert!(outer.confirm(&frame, unchanged));
    assert!(!outer.confirm(&frame, |_| Some(digest(&2_usize))));
    let mut changed = frame.clone();
    *changed.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::empty(TableId::Countries));
    assert!(!outer.confirm(&changed, unchanged));
    // Shared builds and reads outside an evaluation record nothing.
    let ((), shared) = capture(&frame, || unrecorded(|| assert_eq!(countries(&frame), 1)));
    assert_eq!(shared.reads.len(), 0);
}

/// The recorded values of one hour.
#[derive(Clone, Copy)]
struct State {
    /// Country 101's political power: a field no trigger here reads.
    power: i64,
    /// Its ruling party's popularity: read by none either.
    popularity: i64,
    /// Country 101's flag `x`, which the evaluation reads.
    flag: i16,
    /// Country 102's flag `y`, which it does not.
    other_flag: i16,
    /// When the war between 101 and 102 ends.
    war_end: i32,
    /// Whether country 102 has completed focus `g`.
    other_focus: bool,
    /// Whether country 101 has completed focus `g`.
    own_focus: bool,
    /// Whether country 103, which no link resolves, has completed focus `g`.
    unlinked_focus: bool,
    /// Whether country 102's link resolves to slot 1, as 101's does.
    relinked: bool,
}

const BASE: State = State {
    power: 10,
    popularity: 50,
    flag: 1,
    other_flag: 1,
    war_end: 103,
    other_focus: false,
    own_focus: false,
    unlinked_focus: false,
    relinked: false,
};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

/// Countries 101 (slot 1) and 102 (slot 2) at war, each with a flag, a
/// ruling party and completed focuses.
fn frame(hour: Hour, state: State) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, hour, None);
    set::<tables::Countries>(
        &mut frame,
        [(101, 1, "AAA"), (102, 2, "BBB")]
            .into_iter()
            .map(|(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                political_power_raw: Some(if id == 101 { state.power } else { 0 }),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::CountryLinks>(
        &mut frame,
        [(101, 1), (102, if state.relinked { 1 } else { 2 })]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    );
    set::<tables::Provinces>(&mut frame, Vec::new());
    set::<tables::States>(&mut frame, Vec::new());
    set::<tables::LexicalTokens>(
        &mut frame,
        [(900, "x"), (901, "y")]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    );
    set::<tables::ScriptFlags>(
        &mut frame,
        [(101, 900, state.flag), (102, 901, state.other_flag)]
            .into_iter()
            .map(|(scope_id, token, value)| ScriptFlagRow {
                scope_kind: 1,
                scope_id,
                token,
                value,
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::PoliticalParties>(
        &mut frame,
        [101, 102]
            .into_iter()
            .map(|country| PoliticalPartyRow {
                country,
                id: 1,
                popularity: state.popularity,
                is_ruling: 1,
                token: b"democratic".to_vec(),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::WarRelations>(
        &mut frame,
        vec![WarRelationRow {
            first_country: 101,
            second_country: 102,
            start_hour: 0,
            end_hour: state.war_end,
            ..Default::default()
        }],
    );
    let focus = |country, ordinal, token: &str| CompletedFocusRow {
        country,
        ordinal,
        token: token.as_bytes().to_vec(),
    };
    // In key order: by country, then ordinal.
    set::<tables::CompletedFocuses>(
        &mut frame,
        std::iter::once(focus(101, 0, "f"))
            .chain(state.own_focus.then(|| focus(101, 1, "g")))
            .chain(state.other_focus.then(|| focus(102, 0, "g")))
            .chain(state.unlinked_focus.then(|| focus(103, 0, "g")))
            .collect(),
    );
    frame
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(&World<'_>) -> T) -> T {
    let dlcs = BTreeSet::new();
    let triggers = installed_content_core::trigger_keys::TriggerNames::default();
    let aliases = Arc::default();
    run(&World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    ))
}

/// What an evaluation of country 1's visibility-style triggers reads.
fn read_all(world: &World<'_>) -> String {
    format!(
        "{:?}",
        (
            world.tag(1),
            world.flag(1, 101, b"x"),
            world.government(1),
            world.has_focus(1, b"f"),
            world.at_war(1, 2),
        )
    )
}

fn evaluate(frame: &DecodedFrame) -> (String, Inputs) {
    with_world(frame, |world| capture(frame, || read_all(world)))
}

/// Confirm `inputs` at `frame`, and how many inputs it read again.
fn confirm(inputs: &mut Inputs, frame: &DecodedFrame, since: Option<&Since<'_>>) -> (bool, usize) {
    let mut rereads = 0;
    let confirmed = with_world(frame, |world| {
        inputs.confirm_since(frame, since, |key| {
            rereads += 1;
            match key {
                Key::World(query) => Some(world.reread(query)),
                _ => None,
            }
        })
    });
    (confirmed, rereads)
}

/// Recorded hours, encoded and decoded as a recording stores and reads them.
struct Recording {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
}

impl Recording {
    fn new() -> Self {
        Self {
            encoder: FrameEncoder::new(),
            decoder: FrameDecoder::new(),
        }
    }

    fn hour(&mut self, hour: Hour, state: State) -> (Frame, DecodedFrame) {
        self.record(frame(hour, state))
    }

    fn record(&mut self, frame: DecodedFrame) -> (Frame, DecodedFrame) {
        let wire = self.encoder.encode(frame, false).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (wire, decoded)
    }
}

/// An hour's recorded updates as the tables' changes since the hour before.
struct Updates<'a>(&'a Frame);

impl TableChanges for Updates<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match &self.0.tables[table.index()] {
            TableUpdate::Unchanged { .. } => TableChange::Same,
            TableUpdate::Delta {
                changed, deleted, ..
            } => TableChange::Rows(changed, deleted),
            _ => TableChange::Any,
        }
    }
}

#[test]
fn an_hour_changing_only_unread_fields_reuses_without_reading_again() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    let (value, inputs) = evaluate(&first);
    // Political power, party popularity and another country's flag change:
    // rows of tables the evaluation reads, in fields or rows it does not.
    let (wire, second) = recording.hour(
        101,
        State {
            power: 11,
            popularity: 60,
            other_flag: 2,
            ..BASE
        },
    );
    for table in [
        TableId::Countries,
        TableId::PoliticalParties,
        TableId::ScriptFlags,
    ] {
        assert!(
            matches!(wire.tables[table.index()], TableUpdate::Delta { .. }),
            "{table:?} changed"
        );
    }
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    assert!(inputs.unchanged(&since));
    // Without the hour's changes, every input is read again.
    let (whole, rereads) = confirm(&mut inputs.clone(), &second, None);
    assert!(whole);
    assert!(rereads >= 5, "{rereads} rereads");
    // Check mode reads every skipped input again, and none changed.
    let (confirmed, missed) =
        expecting_missed(|| confirm(&mut inputs.clone(), &second, Some(&since)).0);
    assert!(confirmed);
    assert_eq!(missed, 0);
    let mut reused = inputs;
    assert_eq!(
        reusing(|| confirm(&mut reused, &second, Some(&since))),
        (true, 0)
    );
    assert_eq!(evaluate(&second).0, value);
    // The next hour follows on from this one.
    let (wire, third) = recording.hour(102, BASE);
    let updates = Updates(&wire);
    assert!(reused.unchanged(&Since::hour_before(&third, &updates).unwrap()));
}

#[test]
fn an_hour_changing_a_read_field_or_row_reads_again() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    let (value, inputs) = evaluate(&first);
    // Another country completes a focus: a row of a country that resolves
    // to another slot, which `has_completed_focus` neither reads nor
    // refuses on, so nothing is read again.
    let (wire, second) = recording.hour(
        101,
        State {
            other_focus: true,
            ..BASE
        },
    );
    assert!(matches!(
        wire.tables[TableId::CompletedFocuses.index()],
        TableUpdate::Delta { .. }
    ));
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    assert!(inputs.unchanged(&since));
    // Check mode reads it anyway and finds it the same.
    assert!(confirm(&mut inputs.clone(), &second, Some(&since)).0);
    let mut confirmed = inputs.clone();
    assert_eq!(
        reusing(|| confirm(&mut confirmed, &second, Some(&since))),
        (true, 0)
    );
    // The flag the evaluation reads changes value.
    let flagged = State {
        flag: 2,
        other_focus: true,
        ..BASE
    };
    let (wire, third) = recording.hour(102, flagged);
    let updates = Updates(&wire);
    let since = Since::hour_before(&third, &updates).unwrap();
    assert!(!confirmed.unchanged(&since));
    assert!(!reusing(|| confirm(&mut confirmed, &third, Some(&since))).0);
    assert_ne!(evaluate(&third).0, value);
}

#[test]
fn a_relation_ending_at_an_unchanged_hour_reads_again() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    let (value, mut inputs) = evaluate(&first);
    for hour in [101, 102] {
        let (wire, frame) = recording.hour(hour, BASE);
        assert!(
            wire.tables
                .iter()
                .enumerate()
                .all(|(index, update)| TableId::ALL[index] == TableId::Campaign
                    || !matches!(update, TableUpdate::Delta { .. })),
            "nothing changes"
        );
        let updates = Updates(&wire);
        let since = Since::hour_before(&frame, &updates).unwrap();
        assert!(confirm(&mut inputs, &frame, Some(&since)).0);
        assert_eq!(evaluate(&frame).0, value);
    }
    // At 103 the war ends with no table changing.
    let (wire, ended) = recording.hour(103, BASE);
    let updates = Updates(&wire);
    let since = Since::hour_before(&ended, &updates).unwrap();
    assert!(!inputs.unchanged(&since));
    assert!(!confirm(&mut inputs, &ended, Some(&since)).0);
    assert_ne!(evaluate(&ended).0, value);
}

/// What an evaluation of whether country 1 completed focus `g` reads.
fn evaluate_focus(frame: &DecodedFrame) -> (String, Inputs) {
    with_world(frame, |world| {
        capture(frame, || format!("{:?}", world.has_focus(1, b"g")))
    })
}

#[test]
fn an_owned_read_changes_only_with_its_countrys_rows_an_unlinked_row_or_the_links() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    let (mut value, mut inputs) = evaluate_focus(&first);
    assert_eq!(value, "Ok(false)");
    let other = State {
        other_focus: true,
        ..BASE
    };
    // Each hour's state, whether the read may have changed since the hour
    // before, and its value.
    let hours = [
        // Another slot's country completes `g`.
        (other, false, "Ok(false)"),
        // A country no link resolves completes it: refused.
        (
            State {
                unlinked_focus: true,
                ..other
            },
            true,
            "Err",
        ),
        (other, true, "Ok(false)"),
        // Only the links change: 102, which holds `g`, now resolves to slot 1.
        (
            State {
                relinked: true,
                ..other
            },
            true,
            "Ok(true)",
        ),
        // A row of 102, now slot 1's, goes away.
        (
            State {
                relinked: true,
                ..BASE
            },
            true,
            "Ok(false)",
        ),
        // The country itself completes it.
        (
            State {
                relinked: true,
                own_focus: true,
                ..BASE
            },
            true,
            "Ok(true)",
        ),
    ];
    for (hour, (state, changed, read)) in (101..).zip(hours) {
        let (wire, frame) = recording.hour(hour, state);
        let updates = Updates(&wire);
        let since = Since::hour_before(&frame, &updates).unwrap();
        assert_eq!(!inputs.unchanged(&since), changed, "hour {hour}");
        let (now, next) = evaluate_focus(&frame);
        assert!(now.starts_with(read), "hour {hour}: {now}");
        // Check mode reads a skipped input anyway, and finds it the same.
        assert_eq!(
            confirm(&mut inputs.clone(), &frame, Some(&since)).0,
            now == value,
            "hour {hour}"
        );
        (value, inputs) = (now, next);
    }
}

#[test]
fn expected_miss_accounting_is_thread_local_and_nested() {
    let mut recording = Recording::new();
    let (_, frame) = recording.hour(100, BASE);
    let key = Key::World(Query::Flag(1, 1, b"synthetic".to_vec()));
    let (_, outer) = expecting_missed(|| {
        // This completes inside the outer accounting window, but its miss
        // belongs to the worker. A global before/after delta reports it here.
        std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    let (_, worker) = expecting_missed(|| super::missed(&key, &frame));
                    assert_eq!(worker, 1);
                })
                .join()
                .unwrap();
        });
        let (_, inner) = expecting_missed(|| super::missed(&key, &frame));
        assert_eq!(inner, 1);
        super::missed(&key, &frame);
    });
    assert_eq!(outer, 2);
}

#[test]
fn check_mode_reports_an_owned_read_declared_for_the_wrong_country() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    // Declare, wrongly, that country 1's focus reads slot 2's rows.
    let wrong = |key: &Key| {
        matches!(key, Key::World(Query::HasFocus(..))).then(|| {
            Some(vec![
                TableRead::fields(TableId::CompletedFocuses, &["token"]).owned_by(2, 32),
                TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
            ])
        })
    };
    let (_, inputs) = declaring(wrong, || evaluate_focus(&first));
    let (_, right) = evaluate_focus(&first);
    let (wire, second) = recording.hour(
        101,
        State {
            own_focus: true,
            ..BASE
        },
    );
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    assert!(!right.unchanged(&since));
    assert!(inputs.unchanged(&since));
    let ((confirmed, _), missed) =
        expecting_missed(|| confirm(&mut inputs.clone(), &second, Some(&since)));
    assert!(confirmed);
    assert_eq!(missed, 1);
}

#[test]
fn check_mode_reports_a_dependence_that_misses_a_read() {
    let mut recording = Recording::new();
    let (_, first) = recording.hour(100, BASE);
    // Declare, wrongly, that a flag's value reads no flag row.
    let wrong = |key: &Key| matches!(key, Key::World(Query::Flag(..))).then(|| Some(Vec::new()));
    let (_, inputs) = declaring(wrong, || evaluate(&first));
    let (_, right) = evaluate(&first);
    let (wire, second) = recording.hour(101, State { flag: 2, ..BASE });
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    // The true declaration reads the flag again and finds it changed.
    assert!(!reusing(|| confirm(&mut right.clone(), &second, Some(&since))).0);
    // The wrong one lets the evaluation through unread...
    assert!(inputs.unchanged(&since));
    assert_eq!(
        reusing(|| confirm(&mut inputs.clone(), &second, Some(&since))),
        (true, 0)
    );
    // ...which check mode reads anyway and reports.
    let ((confirmed, _), missed) =
        expecting_missed(|| confirm(&mut inputs.clone(), &second, Some(&since)));
    assert!(confirmed);
    assert_eq!(missed, 1);
    // Outside an expecting test, a missed dependence fails it.
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        confirm(&mut inputs.clone(), &second, Some(&since))
    }));
    assert!(failed.is_err());
}

#[test]
fn a_block_read_by_its_changes_confirms_through_changed_and_quiet_hours() {
    use replay::container::BlockChange;
    // Political power changes every hour; the flag at 10.
    let state = |hour: Hour| State {
        power: i64::from(hour),
        flag: if hour < 10 { 1 } else { 2 },
        war_end: 0,
        ..BASE
    };
    let replay =
        crate::test_fixtures::replay((1..=30).map(|hour| frame(hour, state(hour))).collect());
    let index = &replay.index[0];
    let watched = [
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"]).unwrap(),
        TableFields::named(TableId::CountryLinks, &["canonical_slot"]).unwrap(),
        TableFields::named(TableId::LexicalTokens, &["literal"]).unwrap(),
        TableFields::named(TableId::ScriptFlags, &["value"]).unwrap(),
        TableFields::named(TableId::PoliticalParties, &["is_ruling", "token"]).unwrap(),
        TableFields::named(TableId::CompletedFocuses, &["token"]).unwrap(),
        TableFields::named(TableId::WarRelations, &["start_hour", "end_hour"]).unwrap(),
        TableFields::keys(TableId::Provinces),
        TableFields::keys(TableId::States),
    ];
    let tables: Vec<TableId> = watched.iter().map(|watch| watch.table).collect();
    let mut alone = std::collections::BTreeMap::new();
    replay
        .read_block_alone(
            index,
            &tables,
            |_, frame| {
                alone.insert(frame.hour, evaluate(frame).0);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    for checking in [false, true] {
        let run = || {
            let mut held: Option<(String, Inputs)> = None;
            let (mut visited, mut evaluated, mut rereads) = (Vec::new(), 0, 0);
            let mut last = None;
            let mut held_frame: Option<DecodedFrame> = None;
            replay
                .read_block_changes(
                    index,
                    &watched,
                    |change| {
                        match change {
                            BlockChange::Changed {
                                frame,
                                tables: changed,
                                rebuilt,
                            } => {
                                visited.push(frame.hour);
                                let block = BlockTables::new(&watched, changed, rebuilt);
                                let since = Since::hour_before(frame, &block);
                                let reused = held.as_mut().is_some_and(|(_, inputs)| {
                                    let (confirmed, read) = confirm(inputs, frame, since.as_ref());
                                    rereads += read;
                                    confirmed
                                });
                                if !reused {
                                    evaluated += 1;
                                    held = Some(evaluate(frame));
                                }
                                assert_eq!(held.as_ref().unwrap().0, alone[&frame.hour]);
                                last = Some(frame.hour);
                                held_frame = Some(frame.clone());
                            }
                            BlockChange::Quiet(quiet) => {
                                let quiet_tables =
                                    QuietTables::new(&watched, quiet, quiet.hours().len());
                                let since = Since::span(
                                    1,
                                    last.unwrap(),
                                    quiet.last(),
                                    held_frame.as_ref(),
                                    &quiet_tables,
                                );
                                let (value, inputs) = held.as_mut().unwrap();
                                assert!(inputs.advance(&since), "quiet {:?}", quiet.hours());
                                for hour in quiet.hours() {
                                    assert_eq!(*value, alone[hour]);
                                }
                                last = Some(quiet.last());
                            }
                        }
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
            (visited, evaluated, rereads)
        };
        let (visited, evaluated, rereads) = if checking { run() } else { reusing(run) };
        // Countries change every hour, in a field nothing watches.
        assert_eq!(visited, [1, 10]);
        assert_eq!(evaluated, 2, "the first hour, and the flag's change");
        if !checking {
            assert_eq!(rereads, 1, "the flag alone is read again");
        }
    }
}

#[test]
fn block_changes_name_what_may_have_changed() {
    let watched = [TableFields::named(TableId::Countries, &["tag"]).unwrap()];
    let tag = TableRead::fields(TableId::Countries, &["tag"]);
    let power = TableRead::fields(TableId::Countries, &["political_power_raw"]);
    let flags = TableRead::keys(TableId::ScriptFlags);
    let frame = crate::test_fixtures::source(1, 10, None);
    let changed = |changed: &[TableId], rebuilt: &[TableId], read: &TableRead| {
        let block = BlockTables::new(&watched, changed, rebuilt);
        Since::hour_before(&frame, &block).unwrap().changed(read)
    };
    // Rebuilt for an unwatched field only.
    assert!(!changed(&[], &[TableId::Countries], &tag));
    assert!(changed(&[], &[TableId::Countries], &power));
    // A watched field, key or availability changed.
    assert!(changed(&[TableId::Countries], &[TableId::Countries], &tag));
    // An unwatched table rebuilt may differ anywhere; one carried in none.
    assert!(changed(&[], &[TableId::ScriptFlags], &flags));
    assert!(!changed(&[], &[], &flags));
}

/// A frame where every declared query below reads rows: the scenario's
/// countries with party leaders, characters, factions, a subject and ideas.
fn rich_frame() -> DecodedFrame {
    let mut frame = frame(100, BASE);
    set::<tables::PartyLeaders>(
        &mut frame,
        vec![PartyLeaderRow {
            country: 101,
            party: 1,
            ordinal: 0,
            character: Some(5),
            role: Some(1),
            leader_id: 3,
        }],
    );
    set::<tables::CountryCharacters>(
        &mut frame,
        vec![CountryCharacterRow {
            country: 101,
            ordinal: 0,
            character: Some(5),
        }],
    );
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    set::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 102,
            subject: 101,
            start_hour: 0,
            end_hour: 0,
            ..Default::default()
        }],
    );
    set::<tables::ActiveIdeas>(
        &mut frame,
        vec![replay_data::rows::ActiveIdeaRow {
            country: 101,
            ordinal: 0,
            token: b"idea".to_vec(),
        }],
    );
    set::<tables::CountryAppearance>(
        &mut frame,
        [(101, 0), (102, 7)]
            .into_iter()
            .map(
                |(country, original)| replay_data::rows::CountryAppearanceRow {
                    country,
                    original_tag_reference: replay_data::rows::Recorded::Value(Some(original)),
                    ..Default::default()
                },
            )
            .collect(),
    );
    for table in [
        TableId::AdvisorTokens,
        TableId::HiredAdvisors,
        TableId::FactionAdvisors,
        TableId::Characters,
        TableId::UnitLeaders,
        TableId::CountryCounterInputs,
        TableId::CountryCivilWars,
        TableId::Politics,
        TableId::Focus,
        TableId::Projects,
    ] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    frame
}

/// A query of every declared world kind, on the rich frame's countries.
fn declared_world_queries() -> Vec<Query> {
    vec![
        Query::TableStatus(TableId::Countries),
        Query::CountrySlots,
        Query::StateIds,
        Query::StateContinentToken(1),
        Query::StateExists(1),
        Query::FocusTree(101),
        Query::CoreGap,
        Query::TableGap("States".into()),
        Query::Slot(101),
        Query::Tag(1),
        Query::StateRow(1),
        Query::ProvinceRow(1),
        Query::StateController(1),
        Query::ProvinceController(1),
        Query::Token(900),
        Query::TokenOf(b"x".to_vec()),
        Query::Unlinked("Table".into(), 9),
        Query::Faction(1),
        Query::Faction(0),
        Query::SameFaction(1, 2),
        Query::AtWar(1, 2),
        Query::Overlord(1),
        Query::OriginalReference(1),
        Query::OriginalReference(0),
        Query::SameTag(101, 102),
        Query::HasFocus(1, b"f".to_vec()),
        Query::CompletedProject(1, b"x".to_vec()),
        Query::Government(1),
        Query::NamedIdeology(b"democratic".to_vec()),
        Query::IdeologyToken(1),
        Query::ScopedRulingIdeology(1),
        Query::HasElections(1),
        Query::Flag(1, 101, b"x".to_vec()),
        Query::Exists(1),
        Query::OwnedStateCount(2),
        Query::CountryCounterInputs(1),
        Query::ControlledStateCount(1),
        Query::IsAllyWith(1, Some(2)),
        Query::IsAllyWith(1, None),
        Query::Variable(Store::Global, b"v".to_vec()),
        Query::Variable(Store::Country(1), b"v".to_vec()),
        Query::VariableSeed(Store::State(3)),
        Query::VariableSeed(Store::Character(5)),
        Query::Character(5),
        Query::CharacterExists(5),
        Query::UnitLeader(5 << 32 | 4713),
        Query::PartyLeaders(1),
        Query::CountryCharacters(1),
        Query::CivilWarCountries(101),
        Query::AdvisorToken(b"idea".to_vec()),
        Query::FactionAdvisors(7),
        Query::HasIdea(1, "idea".into()),
        Query::Enemies(1),
    ]
}

/// The tables `key` declares, with the world's.
fn declared_tables(key: &Key) -> Vec<TableId> {
    key.dependence()
        .expect("declared")
        .iter()
        .map(|read| read.table)
        .collect()
}

/// Each key's digest at a frame, None when it cannot be read there.
pub(crate) type Digests<'a> = dyn Fn(&DecodedFrame, &[Key]) -> Vec<Option<u64>> + 'a;

/// Reads `keys` on `frame` with each table in turn made unavailable: a key
/// whose declaration lacks that table reads the same. For each key that a
/// table it declares changed, the first such table.
pub(crate) fn tables_each_key_reads(
    frame: &DecodedFrame,
    keys: &[Key],
    digests: &Digests<'_>,
) -> BTreeMap<usize, TableId> {
    let before = digests(frame, keys);
    let mut differed = BTreeMap::new();
    for table in TableId::ALL {
        let mut without = frame.clone();
        *without.tables[table.index()] = TableView::Unavailable {
            reason: "made unavailable".into(),
        };
        let after = digests(&without, keys);
        for (at, key) in keys.iter().enumerate() {
            if !declared_tables(key).contains(&table) {
                assert_eq!(
                    after[at], before[at],
                    "{key:?} reads {table:?}, which it does not declare"
                );
            } else if after[at] != before[at] {
                differed.entry(at).or_insert(table);
            }
        }
    }
    differed
}

/// An edit of some fields of one row (or of every row) of one table.
pub(crate) struct Change {
    pub table: TableId,
    pub fields: &'static [&'static str],
    pub row: Option<u128>,
    pub edit: Box<dyn Fn(&mut TableRows)>,
}

impl Change {
    pub(crate) fn new(
        table: TableId,
        fields: &'static [&'static str],
        row: Option<u128>,
        edit: impl Fn(&mut TableRows) + 'static,
    ) -> Self {
        Self {
            table,
            fields,
            row,
            edit: Box::new(edit),
        }
    }
}

/// Applies each change to `frame`: a key none of whose declared reads of
/// that table reads a changed field of a changed row reads the same. How
/// many reads changed.
pub(crate) fn changes_each_key_reads(
    frame: &DecodedFrame,
    keys: &[Key],
    digests: &Digests<'_>,
    changes: &[Change],
) -> usize {
    let before = digests(frame, keys);
    let mut differed = 0;
    for change in changes {
        let mut changed = frame.clone();
        match &mut *changed.tables[change.table.index()] {
            TableView::Complete(rows) => (change.edit)(rows),
            _ => panic!("{:?} is not complete", change.table),
        }
        assert!(
            changed.tables[change.table.index()] != frame.tables[change.table.index()],
            "the edit of {:?} changes it",
            change.table
        );
        let read = TableRead::fields(change.table, change.fields);
        let after = digests(&changed, keys);
        for (at, key) in keys.iter().enumerate() {
            let affected = key.dependence().expect("declared").iter().any(|declared| {
                declared.table == change.table
                    && declared.fields & read.fields != 0
                    && change.row.is_none_or(|row| declared.reads_row(row, frame))
            });
            if !affected {
                assert_eq!(
                    after[at], before[at],
                    "{key:?} reads {:?} {:?}, which it does not declare",
                    change.table, change.fields
                );
            }
            differed += usize::from(after[at] != before[at]);
        }
    }
    differed
}

/// Check mode reads every input a declaration lets through: each key,
/// declared to read nothing, is let through unread by the hour after
/// `frame` that makes the table `tables` names for it unavailable, and
/// check mode counts its changed value as a missed dependence; its own
/// declaration reads it again.
pub(crate) fn check_mode_reports_each_wrong_declaration(
    frame: &DecodedFrame,
    keys: &[Key],
    tables: &BTreeMap<usize, TableId>,
    digests: &Digests<'_>,
) {
    for (&at, &table) in tables {
        let key = &keys[at];
        let mut edited = frame.clone();
        edited.hour = frame.hour + 1;
        *edited.tables[table.index()] = TableView::Unavailable {
            reason: "made unavailable".into(),
        };
        let record = || {
            capture(frame, || {
                query(
                    || key.clone(),
                    || digests(frame, std::slice::from_ref(key))[0],
                )
            })
            .1
        };
        let wanted = key.clone();
        let wrong = declaring(
            move |declared| (*declared == wanted).then(|| Some(Vec::new())),
            record,
        );
        let right = record();
        let changes = Differing {
            before: frame,
            after: &edited,
        };
        let since = Since::hour_before(&edited, &changes).unwrap();
        assert!(!right.unchanged(&since), "{key:?} declares {table:?}");
        assert!(wrong.unchanged(&since), "{key:?}");
        let reread = |key: &Key| Some(digest(&digests(&edited, std::slice::from_ref(key))[0]));
        let (confirmed, missed) =
            expecting_missed(|| wrong.clone().confirm_since(&edited, Some(&since), reread));
        assert!(confirmed, "{key:?}");
        assert_eq!(missed, 1, "{key:?} read the same without {table:?}");
    }
}

/// Each world query's digest, read in the content-free world.
fn world_digests(frame: &DecodedFrame, keys: &[Key]) -> Vec<Option<u64>> {
    with_world(frame, |world| {
        keys.iter()
            .map(|key| match key {
                Key::World(query) => Some(world.reread(query)),
                _ => None,
            })
            .collect()
    })
}

fn world_keys(queries: Vec<Query>) -> Vec<Key> {
    queries.into_iter().map(Key::World).collect()
}

#[test]
fn a_declared_query_reads_no_table_it_does_not_declare() {
    let keys = world_keys(declared_world_queries());
    let differed = tables_each_key_reads(&rich_frame(), &keys, &world_digests);
    // Most queries read rows here, so a table they declare matters.
    assert!(differed.len() * 4 >= keys.len() * 3, "{differed:?}");
}

#[test]
fn a_declared_query_reads_no_field_or_row_it_does_not_declare() {
    let keys = world_keys(declared_world_queries());
    // Each change edits rows or fields of one table; a query whose
    // declaration of that table holds none of them reads the same.
    let changes = vec![
        Change::new(
            TableId::Countries,
            &["political_power_raw", "stability_raw"],
            None,
            |rows| {
                if let TableRows::Countries(rows) = rows {
                    for row in rows {
                        row.political_power_raw = Some(-1);
                        row.stability_raw = 77;
                    }
                }
            },
        ),
        Change::new(TableId::Countries, &["tag"], None, |rows| {
            if let TableRows::Countries(rows) = rows {
                rows[0].tag = b"ZZZ".to_vec();
            }
        }),
        Change::new(
            TableId::PoliticalParties,
            &["popularity", "name"],
            None,
            |rows| {
                if let TableRows::PoliticalParties(rows) = rows {
                    for row in rows {
                        row.popularity = 3;
                        row.name = b"renamed".to_vec();
                    }
                }
            },
        ),
        Change::new(
            TableId::ScriptFlags,
            &["set_hour", "remaining_days"],
            None,
            |rows| {
                if let TableRows::ScriptFlags(rows) = rows {
                    for row in rows {
                        row.set_hour = 1;
                        row.remaining_days = 5;
                    }
                }
            },
        ),
        // Country 102's flag row only.
        Change::new(
            TableId::ScriptFlags,
            &["value"],
            Some(1 << 64 | 102 << 32 | 901),
            |rows| {
                if let TableRows::ScriptFlags(rows) = rows {
                    rows[1].value = 9;
                }
            },
        ),
        Change::new(
            TableId::WarRelations,
            &["first_casualties", "cancel"],
            None,
            |rows| {
                if let TableRows::WarRelations(rows) = rows {
                    rows[0].first_casualties = 5;
                    rows[0].cancel = 1;
                }
            },
        ),
    ];
    let differed = changes_each_key_reads(&rich_frame(), &keys, &world_digests, &changes);
    assert!(differed > 0, "a read field changes some query");
}

#[test]
fn check_mode_reports_a_wrong_declaration_of_each_declared_query() {
    let frame = rich_frame();
    let keys = world_keys(declared_world_queries());
    let tables = tables_each_key_reads(&frame, &keys, &world_digests);
    check_mode_reports_each_wrong_declaration(&frame, &keys, &tables, &world_digests);
}

/// [`rich_frame`] with a state, a province, rows of every table the other
/// world queries read, and country 102's rows in each table read by
/// country.
fn richer_frame() -> DecodedFrame {
    use replay_data::{
        rows::ActiveIdeaRow, rows::AdvisorTokenRow, rows::ArmyOrderPlanRow, rows::ArmyOrderRow,
        rows::BuildingRow, rows::CharacterRoleTraitRow, rows::CountryBuildingResourceRow,
        rows::CountryCivilWarRow, rows::CountryCounterInputRow, rows::CountryFactoriesRow,
        rows::CountryLeaderRoleRow, rows::CountryWarRow, rows::DivisionRow,
        rows::DivisionStatusRow, rows::DivisionTemplateRow, rows::DynamicModifierRow,
        rows::HiredAdvisorRow, rows::IndustrialOrganisationRow, rows::LandTheatreRow,
        rows::NukeRow, rows::OrdersGroupPlanRow, rows::OrdersGroupRow, rows::ProvinceRow,
        rows::Recorded, rows::RelationOpinionRow, rows::ResearchTechnologyRow,
        rows::ReservedCountryNamingRow, rows::ResourceDefinitionRow, rows::StateBaseResourceRow,
        rows::StateCoreRow, rows::StateDynamicModifierRow, rows::StateResourceInputRow,
        rows::StateResourceRow, rows::StateRow, rows::TechnologyDefinitionRow,
    };
    let mut frame = rich_frame();
    let both = [101_u32, 102];
    set::<tables::States>(
        &mut frame,
        vec![StateRow {
            id: 1,
            owner: 101,
            controller: 102,
            manpower_available: 5,
            ..Default::default()
        }],
    );
    set::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            state: 1,
            owner: 101,
            controller: 101,
            ..Default::default()
        }],
    );
    set::<tables::PartyLeaders>(
        &mut frame,
        both.map(|country| PartyLeaderRow {
            country,
            party: 1,
            ordinal: 0,
            character: Some(5),
            role: Some(1),
            leader_id: 3,
        })
        .into(),
    );
    set::<tables::CountryCharacters>(
        &mut frame,
        both.map(|country| CountryCharacterRow {
            country,
            ordinal: 0,
            character: Some(5),
        })
        .into(),
    );
    set::<tables::ActiveIdeas>(
        &mut frame,
        both.map(|country| ActiveIdeaRow {
            country,
            ordinal: 0,
            token: b"idea".to_vec(),
        })
        .into(),
    );
    set::<tables::HiredAdvisors>(
        &mut frame,
        both.map(|country| HiredAdvisorRow {
            country,
            ordinal: 0,
            idea: b"adv".to_vec(),
            ..Default::default()
        })
        .into(),
    );
    set::<tables::AdvisorTokens>(
        &mut frame,
        vec![AdvisorTokenRow {
            ordinal: 0,
            token: b"adv".to_vec(),
        }],
    );
    set::<tables::TechnologyDefinitions>(
        &mut frame,
        vec![TechnologyDefinitionRow {
            database_index: 4,
            id: b"tech".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::ResearchTechnologies>(
        &mut frame,
        both.map(|country| ResearchTechnologyRow {
            country,
            database_index: 4,
            classifier: engine_research::ctechnology::CLASSIFIER_COMPLETED,
            ..Default::default()
        })
        .into(),
    );
    set::<tables::DynamicModifiers>(
        &mut frame,
        both.map(|country| DynamicModifierRow {
            country,
            ordinal: 0,
            token: b"mod".to_vec(),
            ..Default::default()
        })
        .into(),
    );
    set::<tables::StateDynamicModifiers>(
        &mut frame,
        vec![StateDynamicModifierRow {
            state: 1,
            ordinal: 0,
            token: b"smod".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 1,
            cores: vec![101],
            claims: Recorded::Value(vec![102]),
            ..Default::default()
        }],
    );
    set::<tables::Nukes>(
        &mut frame,
        both.map(|country| NukeRow {
            country,
            nuke_type: 0,
            amount_raw: 3_000_000,
            ..Default::default()
        })
        .into(),
    );
    set::<tables::RelationOpinions>(
        &mut frame,
        vec![RelationOpinionRow {
            owner_country: 101,
            target_country: 102,
            opinion: 10,
        }],
    );
    set::<tables::DivisionTemplates>(
        &mut frame,
        vec![DivisionTemplateRow {
            template: 1,
            owner: 101,
            name: b"Infantry".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::CountryFactories>(
        &mut frame,
        vec![CountryFactoriesRow {
            country: 101,
            civilian_levels: 2,
            ..Default::default()
        }],
    );
    set::<tables::CountryWars>(
        &mut frame,
        vec![CountryWarRow {
            country: 101,
            enemies_read: 1,
            enemies: vec![102],
            ..Default::default()
        }],
    );
    set::<tables::CountryCivilWars>(
        &mut frame,
        vec![CountryCivilWarRow {
            country: 101,
            ordinal: 0,
            target: 102,
        }],
    );
    set::<tables::CountryCounterInputs>(
        &mut frame,
        both.map(|country| CountryCounterInputRow {
            country,
            owned_state_count: 1,
            owned_states: Recorded::Value(Some(vec![1])),
            controlled_states: Recorded::Value(Some(Vec::new())),
            ..Default::default()
        })
        .into(),
    );
    set::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            owned_state_count: Recorded::Value(Some(0)),
            land_theatre_count: Recorded::Value(Some(0)),
            faction: Recorded::Value(Some(0)),
            ..Default::default()
        }],
    );
    set::<tables::LandTheatres>(
        &mut frame,
        vec![LandTheatreRow {
            id: 1,
            owner: 101,
            ordinal: 0,
            ..Default::default()
        }],
    );
    set::<tables::OrdersGroups>(
        &mut frame,
        vec![OrdersGroupRow {
            handle: 10,
            country: 101,
            theatre: 1,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    set::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![OrdersGroupPlanRow {
            group: 10,
            primary_orders: vec![1],
            ..Default::default()
        }],
    );
    set::<tables::ArmyOrders>(
        &mut frame,
        vec![ArmyOrderRow {
            owner_group: 10,
            instance: 1,
            country: 101,
            order_type: 3,
            active: 1,
            path: vec![1],
            ..Default::default()
        }],
    );
    set::<tables::ArmyOrderPlans>(
        &mut frame,
        vec![ArmyOrderPlanRow {
            owner_group: 10,
            instance: 1,
            ..Default::default()
        }],
    );
    set::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 77,
            ..Default::default()
        }],
    );
    set::<tables::DivisionStatus>(
        &mut frame,
        vec![DivisionStatusRow {
            division: 77,
            supply_ratio: 5,
            ..Default::default()
        }],
    );
    set::<tables::IndustrialOrganisations>(
        &mut frame,
        vec![IndustrialOrganisationRow {
            organisation: 55,
            ..Default::default()
        }],
    );
    set::<tables::CountryLeaderRoles>(
        &mut frame,
        vec![CountryLeaderRoleRow {
            character: 5,
            country: 101,
            ideology: 1,
            ..Default::default()
        }],
    );
    set::<tables::CharacterRoleTraits>(
        &mut frame,
        vec![CharacterRoleTraitRow {
            character: 5,
            kind: CharacterRoleTraitRow::COUNTRY_LEADER,
            role: 1,
            tokens: vec![900],
            definitions: vec![1],
        }],
    );
    set::<tables::StateResourceInputs>(
        &mut frame,
        vec![StateResourceInputRow {
            state: 1,
            origin: 1,
            infrastructure_level: Some(3),
            ..Default::default()
        }],
    );
    set::<tables::StateResources>(
        &mut frame,
        vec![StateResourceRow {
            state: 1,
            resource: 900,
            pool: 200_000,
            ..Default::default()
        }],
    );
    set::<tables::StateBaseResources>(
        &mut frame,
        vec![StateBaseResourceRow {
            state: 1,
            resource: 900,
            amount: 300_000,
            ..Default::default()
        }],
    );
    // State 2's building only: state 1's amount reads none.
    set::<tables::StateBuildings>(
        &mut frame,
        vec![BuildingRow {
            location: 2,
            building_type: 900,
            level: 2,
            healthy_level: 2,
            ..Default::default()
        }],
    );
    set::<tables::ProvinceBuildings>(
        &mut frame,
        vec![BuildingRow {
            location: 1,
            building_type: 900,
            level: 1,
            healthy_level: 1,
            ..Default::default()
        }],
    );
    set::<tables::CountryBuildingResources>(
        &mut frame,
        both.map(|country| CountryBuildingResourceRow {
            country,
            ordinal: 0,
            building: 900,
            count: 1,
            resource: 900,
            amount: 100_000,
            ..Default::default()
        })
        .into(),
    );
    set::<tables::ResourceDefinitions>(
        &mut frame,
        vec![ResourceDefinitionRow {
            id: 1,
            token: 900,
            active: 1,
            ..Default::default()
        }],
    );
    for table in [
        TableId::Raids,
        TableId::RaidTooltipOperands,
        TableId::DivisionTemplateIcons,
    ] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    frame
}

/// A query of every kind not in [`declared_world_queries`], and of each
/// kind read from one country's rows, on the richer frame's countries.
fn other_world_queries() -> Vec<Query> {
    use crate::triggers::Scope;
    vec![
        Query::UnitStatus(77),
        Query::RaidWingName(9),
        Query::IndustrialOrganisation(55),
        Query::HasFocus(1, b"f".to_vec()),
        Query::HasTech(1, b"tech".to_vec()),
        Query::HasCountryModifier(1, b"mod".to_vec(), None),
        Query::HasCountryModifier(1, b"mod".to_vec(), Some(Scope::Country(2))),
        Query::HasStateModifier(1, b"smod".to_vec(), None),
        Query::StateResource(1, 900),
        Query::StateResourceAmount(1, 900, Some(1)),
        Query::BuildingLevel(false, 2, 900),
        Query::BuildingLevel(true, 1, 900),
        Query::StateMembers,
        Query::StateOrder(1),
        Query::Exists(0),
        Query::OwnedStateCount(0),
        Query::CountryStateRoster(1, false),
        Query::CountryStateRoster(1, true),
        Query::InvasionOrders(1),
        Query::InvasionOrders(0),
        Query::StateOfValue(100_000),
        Query::PartyLeaders(1),
        Query::CountryCharacters(1),
        Query::LeaderHasTrait(1, "same".into()),
        Query::HasCivilWar(1),
        Query::HasIdea(1, "adv".into()),
        Query::HasIdea(1, "idea".into()),
        Query::Opinion(1, Some(2)),
        Query::Opinion(1, None),
        Query::BuiltinVariable(Store::Country(1), b"num_owned_states".to_vec()),
        Query::BuiltinVariable(Store::Country(1), b"num_owned_controlled_states".to_vec()),
        Query::BuiltinVariable(Store::Country(1), b"num_of_controlled_factories".to_vec()),
        Query::BuiltinVariable(Store::Global, b"x".to_vec()),
        Query::HasTemplate(1, b"Infantry".to_vec()),
        Query::Appearance(1),
        Query::IsCoreOf(1, Some(1)),
        Query::IsClaimedBy(1, Some(2)),
        Query::Nukes(1),
        Query::IsAllyWith(0, Some(1)),
    ]
}

/// Edits of the richer frame: fields no query reads, and country 102's
/// rows of the tables read by country.
fn other_changes() -> Vec<Change> {
    use TableId as T;
    vec![
        Change::new(
            T::ResearchTechnologies,
            &["classifier"],
            Some(102 << 32 | 4),
            |rows| {
                if let TableRows::ResearchTechnologies(rows) = rows {
                    rows[1].classifier = 0;
                }
            },
        ),
        Change::new(
            T::ResearchTechnologies,
            &["level", "progress_raw"],
            None,
            |rows| {
                if let TableRows::ResearchTechnologies(rows) = rows {
                    for row in rows {
                        row.level = 3;
                        row.progress_raw = 7;
                    }
                }
            },
        ),
        Change::new(
            T::DynamicModifiers,
            &["days_remaining", "enabled"],
            None,
            |rows| {
                if let TableRows::DynamicModifiers(rows) = rows {
                    for row in rows {
                        row.days_remaining = 3;
                        row.enabled = 1;
                    }
                }
            },
        ),
        Change::new(T::DynamicModifiers, &["token"], Some(102 << 32), |rows| {
            if let TableRows::DynamicModifiers(rows) = rows {
                rows[1].token = b"other".to_vec();
            }
        }),
        Change::new(T::Nukes, &["amount_raw"], Some(102 << 32), |rows| {
            if let TableRows::Nukes(rows) = rows {
                rows[1].amount_raw = 9;
            }
        }),
        Change::new(T::Nukes, &["ready"], None, |rows| {
            if let TableRows::Nukes(rows) = rows {
                for row in rows {
                    row.ready = 4;
                }
            }
        }),
        Change::new(
            T::CountryAppearance,
            &["red", "graphical_culture"],
            Some(102),
            |rows| {
                if let TableRows::CountryAppearance(rows) = rows {
                    rows[1].red = 9;
                    rows[1].graphical_culture = b"other".to_vec();
                }
            },
        ),
        Change::new(
            T::PartyLeaders,
            &["character", "role"],
            Some(102 << 64 | 1 << 32),
            |rows| {
                if let TableRows::PartyLeaders(rows) = rows {
                    rows[1].character = Some(6);
                    rows[1].role = Some(2);
                }
            },
        ),
        Change::new(
            T::CountryCharacters,
            &["character"],
            Some(102 << 32),
            |rows| {
                if let TableRows::CountryCharacters(rows) = rows {
                    rows[1].character = Some(6);
                }
            },
        ),
        Change::new(T::ActiveIdeas, &["token"], Some(102 << 32), |rows| {
            if let TableRows::ActiveIdeas(rows) = rows {
                rows[1].token = b"other".to_vec();
            }
        }),
        Change::new(
            T::HiredAdvisors,
            &["idea", "slot"],
            Some(102 << 32),
            |rows| {
                if let TableRows::HiredAdvisors(rows) = rows {
                    rows[1].idea = b"other".to_vec();
                    rows[1].slot = b"slot".to_vec();
                }
            },
        ),
        Change::new(T::HiredAdvisors, &["character"], None, |rows| {
            if let TableRows::HiredAdvisors(rows) = rows {
                for row in rows {
                    row.character = 9;
                }
            }
        }),
        Change::new(
            T::CountryBuildingResources,
            &["amount"],
            Some(102 << 32),
            |rows| {
                if let TableRows::CountryBuildingResources(rows) = rows {
                    rows[1].amount = 1;
                }
            },
        ),
        Change::new(
            T::StateBuildings,
            &["healthy_level"],
            Some(2 << 32 | 900),
            |rows| {
                if let TableRows::StateBuildings(rows) = rows {
                    rows[0].healthy_level = 5;
                }
            },
        ),
        Change::new(
            T::States,
            &["movement_infrastructure", "custom_name"],
            None,
            |rows| {
                if let TableRows::States(rows) = rows {
                    for row in rows {
                        row.movement_infrastructure = 4;
                        row.custom_name = b"renamed".to_vec();
                    }
                }
            },
        ),
        Change::new(
            T::CountryCounterInputs,
            &["pride_of_the_fleet", "controlled_state_count"],
            None,
            |rows| {
                if let TableRows::CountryCounterInputs(rows) = rows {
                    for row in rows {
                        row.pride_of_the_fleet = 4;
                        row.controlled_state_count = 2;
                    }
                }
            },
        ),
        Change::new(T::CountryFactories, &["military_total_raw"], None, |rows| {
            if let TableRows::CountryFactories(rows) = rows {
                rows[0].military_total_raw = 4;
            }
        }),
        Change::new(T::DivisionTemplates, &["unit_type"], None, |rows| {
            if let TableRows::DivisionTemplates(rows) = rows {
                rows[0].unit_type = 4;
            }
        }),
        Change::new(
            T::ArmyOrders,
            &["target_states", "paradrop_target"],
            None,
            |rows| {
                if let TableRows::ArmyOrders(rows) = rows {
                    rows[0].target_states = vec![1];
                    rows[0].paradrop_target = 4;
                }
            },
        ),
        Change::new(T::ArmyOrderPlans, &["name"], None, |rows| {
            if let TableRows::ArmyOrderPlans(rows) = rows {
                rows[0].name = b"named".to_vec();
            }
        }),
        Change::new(T::OrdersGroups, &["motorization_level"], None, |rows| {
            if let TableRows::OrdersGroups(rows) = rows {
                rows[0].motorization_level = 2;
            }
        }),
        Change::new(T::StateCores, &["claims"], None, |rows| {
            if let TableRows::StateCores(rows) = rows {
                rows[0].claims = replay_data::rows::Recorded::Value(vec![101]);
            }
        }),
    ]
}

/// The queries content alone decides in the content-free world, which the
/// content world below reads.
// StateResource resolves its controller before the unavailable content arm;
// a missing CountryLinks join is now an observable part of that query.
const CONTENT_DECIDED: [&str; 5] = [
    "StateResourceAmount(1, 900, Some(1))",
    "StateOrder(1)",
    "LeaderHasTrait(1, \"same\")",
    "Opinion(1, None)",
    "BuiltinVariable(Global, [120])",
];

#[test]
fn every_other_world_query_is_declared_and_reads_no_table_field_or_row_it_does_not_declare() {
    let frame = richer_frame();
    let keys = world_keys(other_world_queries());
    let tables = tables_each_key_reads(&frame, &keys, &world_digests);
    let undecided: Vec<String> = keys
        .iter()
        .enumerate()
        .filter(|(at, _)| !tables.contains_key(at))
        .map(|(_, key)| match key {
            Key::World(query) => format!("{query:?}"),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(undecided, CONTENT_DECIDED);
    let differed = changes_each_key_reads(&frame, &keys, &world_digests, &other_changes());
    assert!(differed > 0, "a read field changes some query");
    check_mode_reports_each_wrong_declaration(&frame, &keys, &tables, &world_digests);
}

#[test]
fn content_decided_world_queries_read_no_table_field_or_row_they_do_not_declare() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root =
        std::env::temp_dir().join(format!("dashboard-declared-trait-{}", std::process::id()));
    let file = root.join("common/country_leader/traits.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "leader_traits = { same = {} }").unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let leaders = installed_content_core::leader_traits::read(&layers).unwrap().0;
    std::fs::remove_dir_all(root).unwrap();
    let units = installed_content_core::unit_leader::Traits::default();
    let states: std::collections::BTreeMap<u32, Vec<u32>> = [(1, vec![1])].into();
    let (dlcs, triggers, aliases) = (
        BTreeSet::new(),
        installed_content_core::trigger_keys::TriggerNames::default(),
        Arc::default(),
    );
    let digests = |frame: &DecodedFrame, keys: &[Key]| -> Vec<Option<u64>> {
        let world = World::read(
            frame,
            ContentFacts {
                state_provinces: &states,
                state_content: Ok(()),
                resource_bonuses: Ok([15_000, 20_000]),
                character_traits: Ok((&units, &leaders)),
                ..ContentFacts::content_only(&dlcs, &triggers, &aliases)
            },
        );
        keys.iter()
            .map(|key| match key {
                Key::World(query) => Some(world.reread(query)),
                _ => None,
            })
            .collect()
    };
    let frame = richer_frame();
    let keys = world_keys(vec![
        Query::StateResource(1, 900),
        Query::StateResourceAmount(1, 900, Some(1)),
        Query::StateResourceAmount(1, 900, None),
        Query::StateOrder(1),
        Query::LeaderHasTrait(1, "same".into()),
    ]);
    let tables = tables_each_key_reads(&frame, &keys, &digests);
    assert_eq!(tables.len(), keys.len(), "{tables:?}");
    let differed = changes_each_key_reads(&frame, &keys, &digests, &other_changes());
    // The amounts read the controller's overrides and state 1's pool.
    let read = vec![
        Change::new(TableId::StateResources, &["pool"], None, |rows| {
            if let TableRows::StateResources(rows) = rows {
                rows[0].pool = 700_000;
            }
        }),
        Change::new(
            TableId::CountryBuildingResources,
            &["amount"],
            Some(101 << 32),
            |rows| {
                if let TableRows::CountryBuildingResources(rows) = rows {
                    rows[0].amount = 1;
                }
            },
        ),
    ];
    assert!(differed + changes_each_key_reads(&frame, &keys, &digests, &read) > 0);
    check_mode_reports_each_wrong_declaration(&frame, &keys, &tables, &digests);
}

#[test]
fn every_declaration_names_existing_fields() {
    for query in declared_world_queries() {
        assert!(Key::World(query).dependence().is_some());
    }
    use crate::scope::Query as Scopes;
    for query in [
        Scopes::Slot(101),
        Scopes::RawSlot(0),
        Scopes::RawSlot(101),
        Scopes::PositiveCountry(101),
        Scopes::Tag(1),
        Scopes::Country("AAA".into()),
        Scopes::Alias("AAA".into()),
        Scopes::AliasEntry("AAA".into()),
        Scopes::ResolveTag("AAA".into()),
        Scopes::Roster,
        Scopes::Describe(crate::scope::Target::Country(1)),
    ] {
        assert!(Key::Scopes(query).dependence().is_some());
    }
    for key in [
        Key::ClockSeed,
        Key::FocusTag("AAA".into()),
        Key::Table(TableId::States),
    ] {
        assert!(key.dependence().is_some());
    }
}

#[test]
fn country_scope_queries_declare_tag_substitution_and_only_the_fields_they_read() {
    use crate::scope::{Query as Scopes, RecordedScopes, Target};
    use installed_content_core::{tag_aliases::TagAlias, tag_aliases::TagAliases};
    use replay_data::{
        rows::CountryCounterInputRow, rows::Recorded, rows::ReservedCountryNamingRow,
    };
    let mut frame = frame(100, BASE);
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![
            CountryCounterInputRow {
                country: 101,
                owned_state_count: 0,
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 102,
                owned_state_count: 1,
                ..Default::default()
            },
        ],
    );
    set::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            owned_state_count: Recorded::Value(Some(0)),
            original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![0, 102], vec![]])),
            ..Default::default()
        }],
    );
    let aliases = Arc::new(TagAliases {
        entries: [(b"aaa".to_vec(), ("AAA".into(), TagAlias::default()))].into(),
        ..Default::default()
    });
    let scopes = RecordedScopes::read(&frame, aliases.clone());
    assert_eq!(
        scopes.resolve_tag("AAA"),
        Ok(2),
        "nonowning country substitutes the owning bucket member"
    );
    let mut keys = vec![
        Scopes::ResolveTag("AAA".into()),
        Scopes::ResolveTag("BBB".into()),
        Scopes::Alias("AAA".into()),
        Scopes::AliasEntry("AAA".into()),
        Scopes::Roster,
        Scopes::Describe(Target::Country(1)),
        Scopes::Describe(Target::State(1)),
    ]
    .into_iter()
    .map(Key::Scopes)
    .collect::<Vec<_>>();
    let (_, resolved) = capture(&frame, || {
        scopes.resolve(
            &installed_content_core::scope::Scope::Country("AAA".into()),
            &crate::scope::Context::default(),
        )
    });
    keys.extend(resolved.keys().cloned());
    let digests = |frame: &DecodedFrame, keys: &[Key]| {
        let scopes = RecordedScopes::read(frame, aliases.clone());
        keys.iter()
            .map(|key| match key {
                Key::Scopes(query) => Some(scopes.reread(query)),
                _ => None,
            })
            .collect()
    };
    let tables = tables_each_key_reads(&frame, &keys, &digests);
    let edits = vec![
        Change::new(
            TableId::ReservedCountryNaming,
            &["owned_state_count"],
            None,
            |rows| {
                if let TableRows::ReservedCountryNaming(rows) = rows {
                    rows[0].owned_state_count = Recorded::Value(Some(1));
                }
            },
        ),
        Change::new(
            TableId::ReservedCountryNaming,
            &["country_array"],
            None,
            |rows| {
                if let TableRows::ReservedCountryNaming(rows) = rows {
                    rows[0].country_array = Recorded::Value(Some(vec![102, 101]));
                }
            },
        ),
        Change::new(
            TableId::CountryCounterInputs,
            &["owned_state_count"],
            Some(101),
            |rows| {
                if let TableRows::CountryCounterInputs(rows) = rows {
                    rows[0].owned_state_count = 1;
                }
            },
        ),
        Change::new(
            TableId::CountryCounterInputs,
            &["pride_of_the_fleet"],
            None,
            |rows| {
                if let TableRows::CountryCounterInputs(rows) = rows {
                    rows[0].pride_of_the_fleet = 77;
                }
            },
        ),
        Change::new(
            TableId::ReservedCountryNaming,
            &["original_tag_buckets"],
            None,
            |rows| {
                if let TableRows::ReservedCountryNaming(rows) = rows {
                    rows[0].original_tag_buckets =
                        Recorded::Value(Some(vec![vec![], vec![], vec![]]));
                }
            },
        ),
        Change::new(TableId::Countries, &["tag"], Some(101), |rows| {
            if let TableRows::Countries(rows) = rows {
                rows[0].tag = b"ZZZ".to_vec();
            }
        }),
        Change::new(TableId::Countries, &["political_power_raw"], None, |rows| {
            if let TableRows::Countries(rows) = rows {
                rows[0].political_power_raw = Some(-1);
            }
        }),
        Change::new(
            TableId::CountryLinks,
            &["canonical_slot"],
            Some(102),
            |rows| {
                if let TableRows::CountryLinks(rows) = rows {
                    rows[1].canonical_slot = 1;
                }
            },
        ),
    ];
    assert!(changes_each_key_reads(&frame, &keys, &digests, &edits) > 0);
    check_mode_reports_each_wrong_declaration(&frame, &keys, &tables, &digests);
    let (_, inputs) = capture(&frame, || {
        scopes.resolve_tag("AAA").unwrap();
        scopes.alias_entry("AAA");
    });
    let unchanged = Differing {
        before: &frame,
        after: &frame,
    };
    assert!(inputs.unchanged(&Since::span(
        frame.branch,
        frame.hour,
        frame.hour,
        Some(&frame),
        &unchanged
    )));
}

/// The scenario's countries with character 5, whose unit leader needs
/// `required` experience for its next level and has `skill`.
fn character_frame(hour: Hour, required: i32, skill: i32) -> DecodedFrame {
    use replay_data::{rows::CharacterRow, rows::UnitLeaderRow};
    let mut frame = frame(hour, BASE);
    set::<tables::Characters>(
        &mut frame,
        vec![CharacterRow {
            id: 5,
            country: 101,
            name_token_id: 900,
            ..Default::default()
        }],
    );
    set::<tables::UnitLeaders>(
        &mut frame,
        vec![UnitLeaderRow {
            id: 1,
            reference_type: 4713,
            character_id: 5,
            category: 1,
            owner: 101,
            skill_status: 1,
            skill_level: skill,
            skill_required_experience: required,
            ..Default::default()
        }],
    );
    for table in [
        TableId::UnitLeaderTraits,
        TableId::CharacterRoleTraits,
        TableId::CharacterOperatives,
        TableId::CountryRetiredCharacters,
    ] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    frame
}

/// What a character scope's `tag`, `skill` and retirement read.
fn character_reads(world: &World<'_>) -> String {
    let tables = world.character_tables();
    let canonical = |raw: u32| {
        world
            .slot(raw)
            .ok_or_else(|| crate::triggers::Failure::from(format!("{raw} unlinked")))
    };
    format!(
        "{:?}",
        (
            tables.tag(5, 1, canonical),
            tables.skill(5),
            tables.retired(5, canonical),
        )
    )
}

#[test]
fn character_tables_read_only_their_declared_fields() {
    let frame = character_frame(100, 10, 3);
    let read = |frame: &DecodedFrame| with_world(frame, character_reads);
    let before = read(&frame);
    let mut unread = frame.clone();
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *unread.tables[TableId::UnitLeaders.index()]
    {
        let row = &mut rows[0];
        row.skill_valid = 1;
        row.skill_required_experience = 7;
        row.components = [1, 2, 3, 4];
        row.inherited_text_a = b"a".to_vec();
        row.captured = 1;
        row.visibility_choice = 2;
        row.template_visibility = 3;
        row.exile_country = replay_data::rows::Recorded::Value(4);
    }
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *unread.tables[TableId::Characters.index()]
    {
        let row = &mut rows[0];
        row.roster = 1;
        row.ordinal = 2;
        row.name = b"renamed".to_vec();
        row.token = b"token".to_vec();
        row.gender = 1;
    }
    assert_eq!(read(&unread), before);
    // Fields they read change what they answer.
    assert_ne!(read(&character_frame(100, 10, 4)), before);
}

#[test]
fn an_hour_changing_only_unread_leader_fields_confirms_its_character_reads() {
    let mut recording = Recording::new();
    let (_, first) = recording.record(character_frame(100, 10, 3));
    let (value, inputs) = with_world(&first, |world| capture(&first, || character_reads(world)));
    assert!(
        inputs
            .keys()
            .any(|key| matches!(key, Key::Fields(TableId::UnitLeaders, _)))
    );
    let (wire, second) = recording.record(character_frame(101, 11, 3));
    assert!(matches!(
        wire.tables[TableId::UnitLeaders.index()],
        TableUpdate::Delta { .. }
    ));
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    // A whole-table read would be read again, at a new allocation.
    let mut whole = inputs.clone();
    assert!(!confirm(&mut whole, &second, None).0);
    let mut confirmed = inputs;
    assert!(confirmed.unchanged(&since));
    assert!(confirm(&mut confirmed, &second, Some(&since)).0);
    // Confirmed at this hour's views.
    assert!(confirm(&mut confirmed.clone(), &second, None).0);
    assert_eq!(with_world(&second, character_reads), value);
    let (wire, third) = recording.record(character_frame(102, 11, 4));
    let updates = Updates(&wire);
    let since = Since::hour_before(&third, &updates).unwrap();
    assert!(!confirmed.unchanged(&since));
    assert!(!confirm(&mut confirmed, &third, Some(&since)).0);
}
