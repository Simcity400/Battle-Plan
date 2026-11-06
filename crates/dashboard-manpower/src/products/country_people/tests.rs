//! The manpower family's publication of synthetic walks: what it holds of every country at every
//! hour is what the frame reader gives there, however the walk is cut into segments, published
//! and resumed.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, TABLES},
    query,
};
use dashboard_base::products::country_people::{Operands, frame_operands};
use dashboard_series::{
    Blob, Budget, Cache, Identity, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirWingRow, CountryCounterInputRow, CountryLinkRow, CountryRow, CountryStabilityInputRow,
        DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow, DivisionTemplateIconRow,
        FleetRow, LexicalTokenRow, ModifierDefinitionRow, Recorded, SentVolunteerRow, ShipRow,
        SimulationFeatureRow, SliceReadRow, StateCoreRow, StateResourceInputRow, StateRow,
        TableId, TableRows, TableSchema, TaskForceRow, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::collections::BTreeSet;

fn view(rows: TableRows) -> replay_frame::frame::SharedView {
    TableView::Complete(rows).into()
}

/// Two countries, Armenia (id 11, slot 1, also named by its alias 33) and Burma (id 22, slot 2),
/// with every table the operands read at `hour`.
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unwritten".into(),
            }
            .into()
        }),
    };
    let set = |frame: &mut DecodedFrame, table: TableId, rows: TableRows| {
        frame.tables[table.index()] = view(rows);
        frame.metrics.table_ns[table.index()] = 1;
    };
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![
        CountryRow { id: 11, canonical_slot: 1, tag: b"AAA".to_vec(), manpower_residual: 1_000, ..Default::default() },
        CountryRow { id: 22, canonical_slot: 2, tag: b"BBB".to_vec(), manpower_residual: 500, ..Default::default() },
    ]));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![
        CountryLinkRow { id: 11, canonical_slot: 1, ..Default::default() },
        CountryLinkRow { id: 22, canonical_slot: 2, ..Default::default() },
        CountryLinkRow { id: 33, canonical_slot: 1, ..Default::default() },
    ]));
    set(&mut frame, TableId::States, TableRows::States(vec![
        StateRow { id: 1, manpower_available: 100, manpower_locked: Recorded::Value(10), population: Recorded::Value(1_000), ..Default::default() },
        StateRow { id: 2, manpower_available: 50, manpower_locked: Recorded::Value(5), population: Recorded::Value(400), ..Default::default() },
    ]));
    set(&mut frame, TableId::StateResourceInputs, TableRows::StateResourceInputs(vec![
        StateResourceInputRow {
            state: 1,
            local_manpower: Recorded::Value(Some(10_000)),
            local_noncore_manpower: Recorded::Value(Some(0)),
            recruitment_add: Recorded::Value(Some(2_000)),
            recruitment_factor: Recorded::Value(Some(0)),
            ..Default::default()
        },
        StateResourceInputRow {
            state: 2,
            local_manpower: Recorded::Value(Some(0)),
            local_noncore_manpower: Recorded::Value(None),
            recruitment_add: Recorded::Value(Some(0)),
            recruitment_factor: Recorded::Value(Some(0)),
            ..Default::default()
        },
    ]));
    set(&mut frame, TableId::StateCores, TableRows::StateCores(vec![
        StateCoreRow { state: 1, cores: vec![33], ..Default::default() },
        StateCoreRow { state: 2, cores: vec![22], ..Default::default() },
    ]));
    // Armenia controls both states and a third States does not hold.
    set(&mut frame, TableId::CountryCounterInputs, TableRows::CountryCounterInputs(vec![
        CountryCounterInputRow {
            country: 11,
            controlled_states: Recorded::Value(Some(vec![1, 2])),
            recruitment_modifiers: Recorded::Value(Some([Some(5_000), Some(2_500), None, Some(150_000)])),
            sent_expeditionaries: Recorded::Value(Some(vec![7, 0, 99].into())),
            ..Default::default()
        },
        CountryCounterInputRow {
            country: 22,
            controlled_states: Recorded::Value(Some(vec![3])),
            recruitment_modifiers: Recorded::Value(None),
            sent_expeditionaries: Recorded::Value(Some(Vec::new().into())),
            ..Default::default()
        },
    ]));
    set(&mut frame, TableId::CountryStabilityInputs, TableRows::CountryStabilityInputs(vec![
        CountryStabilityInputRow {
            country: 33,
            recruitment_ratio: Recorded::Value(Some(4_000_000)),
            keys: vec![u32::MAX; CountryStabilityInputRow::KEY_NAMES.len()],
            ..Default::default()
        },
    ]));
    set(&mut frame, TableId::SliceReads, TableRows::SliceReads(vec![SliceReadRow {
        table: TableId::CountryStabilityInputs.index() as u32,
        owner: 33,
        sample_hour: hour - hour.rem_euclid(24),
        ..Default::default()
    }]));
    set(&mut frame, TableId::Divisions, TableRows::Divisions(vec![
        DivisionRow { id: 1, roster_owner: 11, logical_owner: 11, template: 100, ..Default::default() },
        DivisionRow { id: 2, roster_owner: 33, logical_owner: 11, template: 101, ..Default::default() },
        DivisionRow { id: 7, roster_owner: 22, logical_owner: 22, expeditionary_owner: 11, template: 100, ..Default::default() },
    ]));
    set(&mut frame, TableId::DivisionLandInputs, TableRows::DivisionLandInputs(vec![
        DivisionLandInputsRow { division: 1, manpower: vec![DivisionManpowerContributor { country: 11, amount: 900 }], ..Default::default() },
        DivisionLandInputsRow {
            division: 2,
            manpower: vec![
                DivisionManpowerContributor { country: 22, amount: 5 },
                DivisionManpowerContributor { country: 33, amount: 400 },
            ],
            ..Default::default()
        },
        DivisionLandInputsRow { division: 7, manpower: vec![DivisionManpowerContributor { country: 11, amount: 300 }], ..Default::default() },
    ]));
    set(&mut frame, TableId::DivisionTemplateIcons, TableRows::DivisionTemplateIcons(vec![
        DivisionTemplateIconRow { template: 100, manpower_exile: Recorded::Value(false), ..Default::default() },
        DivisionTemplateIconRow { template: 101, manpower_exile: Recorded::Value(false), ..Default::default() },
    ]));
    set(&mut frame, TableId::SentVolunteers, TableRows::SentVolunteers(vec![
        SentVolunteerRow { country: 22, ordinal: 0, division: 1 },
    ]));
    set(&mut frame, TableId::Fleets, TableRows::Fleets(vec![FleetRow { id: 1, owner: 33, ..Default::default() }]));
    set(&mut frame, TableId::TaskForces, TableRows::TaskForces(vec![TaskForceRow { id: 5, fleet: 1, ..Default::default() }]));
    set(&mut frame, TableId::Ships, TableRows::Ships(vec![ShipRow {
        id: 9,
        task_force: 5,
        acquired_manpower: Recorded::Value(Some(50)),
        required_manpower: Recorded::Value(Some(60)),
        ..Default::default()
    }]));
    set(&mut frame, TableId::AirWings, TableRows::AirWings(vec![
        AirWingRow { id: 1, owner: 11, exile_country: Recorded::Value(Some(0)), acquired_manpower: Recorded::Value(Some(20)), ..Default::default() },
        AirWingRow { id: 2, owner: 22, exile_country: Recorded::Value(Some(33)), acquired_manpower: Recorded::Value(Some(30)), ..Default::default() },
    ]));
    // The four recruitment modifiers; non-core manpower clamps its maximum.
    let names = ["mobilization_speed", "conscription", "conscription_factor", "non_core_manpower"];
    set(&mut frame, TableId::ModifierDefinitions, TableRows::ModifierDefinitions(
        (0..4)
            .map(|at| ModifierDefinitionRow {
                index: at,
                token: at + 1,
                flags: if at == 3 { engine_clamp() } else { 0 },
                ..Default::default()
            })
            .collect(),
    ));
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(
        names
            .iter()
            .zip(1..)
            .map(|(name, token)| LexicalTokenRow { token, literal: name.as_bytes().to_vec() })
            .collect(),
    ));
    set(&mut frame, TableId::SimulationFeatures, TableRows::SimulationFeatures(vec![SimulationFeatureRow { dlc_mask: 0, ..Default::default() }]));
    frame
}

/// The flag a definition clamps its maximum with, as the registry reads it.
fn engine_clamp() -> u32 {
    replay_derive::modifier_registry::CLAMP_MAXIMUM
}

macro_rules! rows {
    ($name:ident, $variant:ident, $row:ty) => {
        fn $name(frame: &mut DecodedFrame) -> &mut Vec<$row> {
            match &mut *frame.tables[TableId::$variant.index()] {
                TableView::Complete(TableRows::$variant(rows)) => rows,
                _ => unreachable!("the fixture's table is complete"),
            }
        }
    };
}

rows!(land_rows, DivisionLandInputs, DivisionLandInputsRow);
rows!(country_rows, Countries, CountryRow);
rows!(state_rows, States, StateRow);
rows!(ship_rows, Ships, ShipRow);
rows!(counter_rows, CountryCounterInputs, CountryCounterInputRow);
rows!(division_rows, Divisions, DivisionRow);
rows!(wing_rows, AirWings, AirWingRow);
rows!(definition_rows, ModifierDefinitions, ModifierDefinitionRow);
rows!(link_rows, CountryLinks, CountryLinkRow);

/// Forty hours: Armenia's first division reinforces each hour, its residual and first state's
/// manpower change daily, an unresolved country shares its tag for a while, it lists a state
/// twice for a while, the counters are unavailable for three hours, Burma leaves the roster
/// for a while and later takes another tag as a state's manpower changes, a ship's task force is
/// lost once, an expeditionary goes home, a wing leaves exile, a modifier stops clamping, the
/// recruitment inputs and cores are unread under an unchanged roster, and an alias moves to
/// another country.
fn walk() -> Vec<DecodedFrame> {
    (100..140)
        .map(|hour| {
            let mut frame = frame(hour);
            let step = hour - 100;
            land_rows(&mut frame)[0].manpower[0].amount += step * 7;
            country_rows(&mut frame)[0].manpower_residual += (step / 24) * 100;
            state_rows(&mut frame)[0].manpower_available += step / 24;
            if (102..105).contains(&hour) {
                country_rows(&mut frame).insert(0, CountryRow {
                    id: 5,
                    canonical_slot: 9,
                    tag: b"AAA".to_vec(),
                    ..Default::default()
                });
            }
            // The recruitment inputs and cores are unread while the roster stays as it was.
            if (106..108).contains(&hour) {
                for table in [TableId::StateResourceInputs, TableId::StateCores] {
                    frame.tables[table.index()] = TableView::Unavailable {
                        reason: "unread".into(),
                    }
                    .into();
                }
            }
            if (114..118).contains(&hour) {
                counter_rows(&mut frame)[0].controlled_states = Recorded::Value(Some(vec![1, 2, 1]));
            }
            if (110..113).contains(&hour) {
                frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
                    reason: "unread".into(),
                }
                .into();
            }
            if (120..125).contains(&hour) {
                country_rows(&mut frame).retain(|row| row.id != 22);
            }
            if (126..129).contains(&hour) {
                country_rows(&mut frame)[1].tag = b"CCC".to_vec();
            }
            // A state's manpower changes in the hour Burma takes its other tag.
            if hour >= 126 {
                state_rows(&mut frame)[1].manpower_available += 7;
            }
            // Armenia's alias names Burma for a while: its core of state 1 moves with it.
            if (131..134).contains(&hour) {
                link_rows(&mut frame)[2].canonical_slot = 2;
            }
            if hour == 130 {
                ship_rows(&mut frame)[0].task_force = 6;
            }
            if hour >= 132 {
                division_rows(&mut frame)[2].expeditionary_owner = 0;
            }
            if hour >= 134 {
                wing_rows(&mut frame)[1].exile_country = Recorded::Value(Some(0));
            }
            if hour >= 136 {
                definition_rows(&mut frame)[3].flags = 0;
            }
            frame
        })
        .collect()
}

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let read = input.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::Divisions>(walk, input);
    write::<tables::DivisionStatus>(walk, input);
    write::<tables::DivisionLandInputs>(walk, input);
    write::<tables::DivisionTemplateIcons>(walk, input);
    write::<tables::SentVolunteers>(walk, input);
    write::<tables::CountryCounterInputs>(walk, input);
    write::<tables::CountryStabilityInputs>(walk, input);
    write::<tables::SliceReads>(walk, input);
    write::<tables::States>(walk, input);
    write::<tables::StateResourceInputs>(walk, input);
    write::<tables::StateCores>(walk, input);
    write::<tables::Ships>(walk, input);
    write::<tables::TaskForces>(walk, input);
    write::<tables::Fleets>(walk, input);
    write::<tables::AirWings>(walk, input);
    write::<tables::ModifierDefinitions>(walk, input);
    write::<tables::LexicalTokens>(walk, input);
    write::<tables::SimulationFeatures>(walk, input);
}

/// The projected hours of `inputs`, a projector of its own starting a segment at each ordinal
/// `restarts` holds.
fn project(inputs: &[DecodedFrame], restarts: &BTreeSet<usize>) -> Vec<facts::Hour> {
    let format = FormatVersion::CURRENT;
    let mut hours = Vec::new();
    let mut walk = Synthetic::new(TABLES, 1, inputs[0].hour).unwrap();
    let mut held = None;
    let mut projector = facts::Projector::new(format.version()).unwrap();
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    let mut first = 0;
    for (ordinal, input) in inputs.iter().enumerate() {
        if ordinal > 0 && restarts.contains(&ordinal) {
            projector.end(&mut hours).unwrap();
            projector = facts::Projector::new(format.version()).unwrap();
            projector
                .start(Start::Restart(walk.state()), 1, input.hour)
                .unwrap();
            first = ordinal;
        }
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, walk_starts| {
            let world = dashboard_world::World::at(
                &mut held,
                walk_starts,
                &facts::needs(),
                (format, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts: ordinal == first,
                        opens: ordinal == first,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
    }
    projector.end(&mut hours).unwrap();
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(hours).unwrap()
}

fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: SCHEMA,
    }
}

/// The publication of `hours`, cut after each of `cuts`; at each of `resume`'s the emit is saved
/// and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: &[i32]) -> (Memory, u64) {
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(&budget, identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        let publication = Publication {
            generation,
            segment: generation - 1,
            accepted: Span::new(first, cut).unwrap(),
            reached: cut,
            refusal: None,
        };
        root = publish(&mut writer, &mut pages, &cache, publication, Blob::EMPTY).unwrap();
        if resume.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, &budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

/// What the frame reader gives each country at `frame`'s hour, and its holder's slot and id;
/// none where it is not a country.
fn expected(frame: &DecodedFrame, tag: &str) -> Option<(Operands, (u32, u32))> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let tagged = dashboard_base::metadata::tagged_countries(countries, links);
    let (country, slot) = tagged.get(tag.as_bytes()).and_then(|lookup| lookup.found())?;
    let canonical = |raw: u32| links.iter().find(|link| link.id == raw).map(|link| link.canonical_slot);
    let equal = |first: u32, second: u32| {
        first == second
            || (first != 0 && second != 0 && canonical(first).is_some_and(|slot| canonical(second) == Some(slot)))
    };
    Some((
        frame_operands(frame, country, |raw| equal(raw, country.id), equal),
        (slot, country.id),
    ))
}

#[test]
fn every_country_s_operands_at_every_hour_are_the_frame_reader_s_however_the_walk_is_cut() {
    let inputs = walk();
    let last = inputs.last().unwrap().hour;
    for (restarts, cuts, resume) in [
        (BTreeSet::new(), vec![last], vec![]),
        // A resumed emit goes on at a segment's first hour, as a restart's.
        (BTreeSet::from([9, 22]), vec![108, 121, last], vec![121]),
        (BTreeSet::from([1, 12, 30]), vec![100, 111, 129, last], vec![100, 111, 129]),
    ] {
        let (pages, root) = build(project(&inputs, &restarts), &cuts, &resume);
        let budget = Budget::new(1 << 30);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        for frame in &inputs {
            for tag in ["AAA", "BBB", "CCC"] {
                let read = query::operands_at(&mut snapshot, &directory, tag, frame.hour).unwrap();
                assert_eq!(read, expected(frame, tag), "{tag} at {} cut at {cuts:?}", frame.hour);
            }
        }
    }
}

#[test]
fn the_fixture_reads_every_kind_of_operand() {
    let frame = frame(100);
    let (armenia, holder) = expected(&frame, "AAA").unwrap();
    assert_eq!(holder, (1, 11));
    // Divisions 1 and 2 at home, 7 sent as an expeditionary; the volunteer is Burma's.
    assert_eq!(armenia.deployed[0], Some(900 + 400 + 300));
    assert_eq!(armenia.deployed[1..3], [Some(50), Some(60)]);
    assert_eq!(armenia.deployed[3..], [Some(20), Some(30)]);
    assert_eq!(armenia.applied, Some(4_000_000));
    assert_eq!(armenia.read_hour, Some(96));
    // The clamped modifier, and the unread one as zero.
    assert_eq!(armenia.effective, Some([5_000, 2_500, 0, 100_000]));
    let states = armenia.states.unwrap();
    assert_eq!(states[0].1.as_ref().unwrap().core, Some(true));
    assert_eq!(states[1].1.as_ref().unwrap().resources, None);
    let (burma, _) = expected(&frame, "BBB").unwrap();
    // The frame reader keeps a repeated state's every place, and passes over an unresolved
    // holder of the tag.
    let walked = walk();
    let (repeated, _) = expected(&walked[15], "AAA").unwrap();
    assert_eq!(
        repeated.states.unwrap().iter().map(|(state, _)| *state).collect::<Vec<_>>(),
        [1, 2, 1]
    );
    assert_eq!(expected(&walked[3], "AAA").unwrap().1, (1, 11));
    assert!(expected(&walked[27], "CCC").is_some());
    assert_eq!(burma.states, Some(vec![(3, None)]));
    assert_eq!(burma.effective, None);
}

#[test]
fn politics_distinguishes_unprepared_disabled_and_scheduled_elections() {
    use dashboard_base::products::country_people::{PeopleParams, RecruitmentDefines};
    use dashboard_politics::products::politics_state::history::AtHour;

    let at = frame(100);
    let operands = expected(&at, "AAA").unwrap().0;
    let params = PeopleParams { branch_id: "1".into(), tag: "AAA".into(), hour: at.hour };
    let assemble = |politics: Option<&AtHour>| query::assemble(
        &params,
        &operands,
        Ok(RecruitmentDefines { speed: 1_000, noncore_base: 2_000, noncore_max: 100_000 }),
        politics,
        &[],
    );
    let missing = assemble(None);
    assert!(missing.politics.iter().all(|fact| fact.value.is_none()));
    assert_eq!(missing.politics[2].unit, "date");
    assert!(missing.warnings.iter().any(|warning| warning == query::UNPREPARED));

    for (held, expected) in [
        (0, [(Some("Not held"), "text"), (Some("36"), "months"), (Some("Not held"), "text")]),
        (1, [(Some("Allowed"), "text"), (Some("36"), "months"), (Some("123456"), "date")]),
    ] {
        let reply = assemble(Some(&AtHour {
            held: Some(held),
            interval: Some(36),
            next: Some(123456),
            exiles_known: true,
            ..Default::default()
        }));
        let facts: Vec<_> = reply.politics.iter().map(|fact| (fact.value.as_deref(), fact.unit.as_str())).collect();
        assert_eq!(facts, expected);
        assert!(!reply.warnings.iter().any(|warning| warning == query::UNPREPARED
            || warning.contains("refused election") || warning.contains("refused exile status")));
        assert!(reply.exiles.is_empty());
    }
}
