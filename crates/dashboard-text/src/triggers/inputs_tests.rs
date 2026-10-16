use super::*;
use crate::triggers::{ContentFacts, World, variables::Store, world::Query};
use replay::frame::{Frame, FrameDecoder, FrameEncoder, TableUpdate};
use replay::rows::{
    CompletedFocusRow, CountryCharacterRow, CountryLinkRow, CountryRow, FactionMemberRow,
    LexicalTokenRow, PartyLeaderRow, PoliticalPartyRow, ScriptFlagRow, SubjectRow, TableRows,
    TableSchema, WarRelationRow,
};
use std::{collections::BTreeSet, sync::Arc};

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
}

const BASE: State = State {
    power: 10,
    popularity: 50,
    flag: 1,
    other_flag: 1,
    war_end: 103,
    other_focus: false,
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
        [(101, 1), (102, 2)]
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
    let focus = |country, token: &str| CompletedFocusRow {
        country,
        ordinal: 0,
        token: token.as_bytes().to_vec(),
    };
    set::<tables::CompletedFocuses>(
        &mut frame,
        std::iter::once(focus(101, "f"))
            .chain(state.other_focus.then(|| focus(102, "g")))
            .collect(),
    );
    frame
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(&World<'_>) -> T) -> T {
    let dlcs = BTreeSet::new();
    let triggers = installed_content::trigger_keys::TriggerNames::default();
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
    let missed = missed_dependences();
    assert!(confirm(&mut inputs.clone(), &second, Some(&since)).0);
    assert_eq!(missed_dependences(), missed);
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
    // Another country completes a focus: a row whose country might not
    // resolve, which `has_completed_focus` refuses on, so it is read again
    // and still reads the same.
    let (wire, second) = recording.hour(
        101,
        State {
            other_focus: true,
            ..BASE
        },
    );
    let updates = Updates(&wire);
    let since = Since::hour_before(&second, &updates).unwrap();
    assert!(!inputs.unchanged(&since));
    let mut confirmed = inputs.clone();
    assert_eq!(
        reusing(|| confirm(&mut confirmed, &second, Some(&since))),
        (true, 1)
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
        vec![replay::rows::ActiveIdeaRow {
            country: 101,
            ordinal: 0,
            token: b"idea".to_vec(),
        }],
    );
    set::<tables::CountryAppearance>(
        &mut frame,
        [(101, 0), (102, 7)]
            .into_iter()
            .map(|(country, original)| replay::rows::CountryAppearanceRow {
                country,
                original_tag_reference: replay::rows::Recorded::Value(Some(original)),
                ..Default::default()
            })
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

#[test]
fn a_declared_query_reads_no_table_it_does_not_declare() {
    let frame = rich_frame();
    let queries = declared_world_queries();
    let before: Vec<u64> = with_world(&frame, |world| {
        queries.iter().map(|query| world.reread(query)).collect()
    });
    let mut differed = BTreeSet::new();
    for table in TableId::ALL {
        let mut without = frame.clone();
        *without.tables[table.index()] = TableView::Unavailable {
            reason: "made unavailable".into(),
        };
        with_world(&without, |world| {
            for (query, before) in queries.iter().zip(&before) {
                let key = Key::World(query.clone());
                let after = world.reread(query);
                if !declared_tables(&key).contains(&table) {
                    assert_eq!(
                        after, *before,
                        "{query:?} reads {table:?}, which it does not declare"
                    );
                } else if after != *before {
                    differed.insert(format!("{query:?}"));
                }
            }
        });
    }
    // Most queries read rows here, so a table they declare matters.
    assert!(differed.len() * 4 >= queries.len() * 3, "{differed:?}");
}

#[test]
fn a_declared_query_reads_no_field_or_row_it_does_not_declare() {
    let frame = rich_frame();
    let queries = declared_world_queries();
    let digests = |frame: &DecodedFrame| -> Vec<u64> {
        with_world(frame, |world| {
            queries.iter().map(|query| world.reread(query)).collect()
        })
    };
    let before = digests(&frame);
    let edit = |table: TableId, change: &dyn Fn(&mut TableRows)| {
        let mut changed = frame.clone();
        if let TableView::Complete(rows) = &mut *changed.tables[table.index()] {
            change(rows);
        }
        changed
    };
    // Each change edits rows or fields of one table; a query whose
    // declaration of that table holds none of them reads the same.
    let changes: Vec<(
        TableId,
        Vec<&str>,
        Option<u128>,
        Box<dyn Fn(&mut TableRows)>,
    )> = vec![
        (
            TableId::Countries,
            vec!["political_power_raw", "stability_raw"],
            None,
            Box::new(|rows| {
                if let TableRows::Countries(rows) = rows {
                    for row in rows {
                        row.political_power_raw = Some(-1);
                        row.stability_raw = 77;
                    }
                }
            }),
        ),
        (
            TableId::Countries,
            vec!["tag"],
            None,
            Box::new(|rows| {
                if let TableRows::Countries(rows) = rows {
                    rows[0].tag = b"ZZZ".to_vec();
                }
            }),
        ),
        (
            TableId::PoliticalParties,
            vec!["popularity", "name"],
            None,
            Box::new(|rows| {
                if let TableRows::PoliticalParties(rows) = rows {
                    for row in rows {
                        row.popularity = 3;
                        row.name = b"renamed".to_vec();
                    }
                }
            }),
        ),
        (
            TableId::ScriptFlags,
            vec!["set_hour", "remaining_days"],
            None,
            Box::new(|rows| {
                if let TableRows::ScriptFlags(rows) = rows {
                    for row in rows {
                        row.set_hour = 1;
                        row.remaining_days = 5;
                    }
                }
            }),
        ),
        (
            TableId::ScriptFlags,
            vec!["value"],
            // Country 102's flag row only.
            Some(1 << 64 | 102 << 32 | 901),
            Box::new(|rows| {
                if let TableRows::ScriptFlags(rows) = rows {
                    rows[1].value = 9;
                }
            }),
        ),
        (
            TableId::WarRelations,
            vec!["first_casualties", "cancel"],
            None,
            Box::new(|rows| {
                if let TableRows::WarRelations(rows) = rows {
                    rows[0].first_casualties = 5;
                    rows[0].cancel = 1;
                }
            }),
        ),
    ];
    let mut differed = 0;
    for (table, names, row, change) in &changes {
        let read = TableRead::fields(*table, names);
        let after = digests(&edit(*table, &**change));
        for ((query, before), after) in queries.iter().zip(&before).zip(&after) {
            let reads = Key::World(query.clone()).dependence().unwrap();
            let affected = reads.iter().any(|declared| {
                declared.table == *table
                    && declared.fields & read.fields != 0
                    && row.is_none_or(|key| declared.holds(key))
            });
            if !affected {
                assert_eq!(
                    after, before,
                    "{query:?} reads {table:?} {names:?}, which it does not declare"
                );
            }
            differed += usize::from(after != before);
        }
    }
    assert!(differed > 0, "a read field changes some query");
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

/// The scenario's countries with character 5, whose unit leader needs
/// `required` experience for its next level and has `skill`.
fn character_frame(hour: Hour, required: i32, skill: i32) -> DecodedFrame {
    use replay::rows::{CharacterRow, UnitLeaderRow};
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
    let canonical = |raw: u32| world.slot(raw).ok_or_else(|| format!("{raw} unlinked"));
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
        row.exile_country = replay::rows::Recorded::Value(4);
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
