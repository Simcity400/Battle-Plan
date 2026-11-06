//! The family's columnar preparation held to the row recording of the same hours, written as the
//! native recorder writes them: every country's reply equals the row reply ([`SpecialProjects`]'s
//! oracle), and the event feed's completions equal the row read's ([`super::row`]) and the
//! completion oracle's ([`super::completion_events`]), across segment starts, a gap, cuts and a
//! resume.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Binding, Projector},
    *,
};
use dashboard_base::{
    products::Product,
    test_fixtures::{CAMPAIGN, continuation_records, file_source, records, source},
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CampaignRow, CharacterRow, CountryLinkRow, CountryRow,
        LexicalTokenRow, ProjectHistoryRow, ProjectProgramRow, ProjectRow, RowCodec, TableRows,
        tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::TableView;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

/// The hours walked: 100 to 130 without 120 and 121.
fn hours() -> Vec<i32> {
    (100..=130).filter(|hour| !(120..=121).contains(hour)).collect()
}

/// A completion's history entry, as the game stores it.
const COMPLETION: u32 = 0x4c17;

/// One hour of three countries. AAA runs project 1 (definition 8) under program 5, whose
/// scientist is character 7, its progress rising every hour, and completes it at 110, dated
/// 110; BBB's project 1 (definition 9) is paused from 105 and completes at 115, its history
/// dating it 113; CCC holds none. BBB's country changes id at 124.
fn tables(hour: i32) -> Tables {
    let bbb = if hour >= 124 { 20 } else { 2 };
    let countries = vec![(1, 1, "AAA"), (bbb, 2, "BBB"), (3, 3, "CCC")];
    let mut links = vec![(1, 1), (bbb, 2), (3, 3)];
    links.sort_unstable();
    let state = |offset: u32, progress: i32| replay_data::rows::ProjectState {
        current_offset: offset,
        not_completed: u8::from(offset != 0xe8),
        prototype_progress: progress,
        ..Default::default()
    };
    let mut projects = vec![ProjectRow {
        owner: 1,
        local_id: 1,
        definition_id: 8,
        program_reference: if hour < 110 { 5 << 32 | 83 } else { 0 },
        started: 1,
        state: state(if hour < 110 { 0x18 } else { 0xe8 }, hour - 100),
        ..Default::default()
    }];
    projects.push(ProjectRow {
        owner: bbb,
        local_id: 1,
        definition_id: 9,
        started: u8::from(hour < 105),
        state: state(
            match hour {
                ..=104 => 0x18,
                105..=114 => 0x168,
                _ => 0xe8,
            },
            40,
        ),
        ..Default::default()
    });
    projects.sort_unstable_by_key(|row| (row.owner, row.local_id));
    let mut history = Vec::new();
    let completed = |owner, date| ProjectHistoryRow {
        owner,
        project: 1,
        ordinal: 0,
        prototype_reward_handle: COMPLETION,
        option_handle: COMPLETION,
        date,
        ..Default::default()
    };
    if hour >= 110 {
        history.push(completed(1, 110));
    }
    if hour >= 115 {
        history.push(completed(bbb, 113));
    }
    history.sort_unstable_by_key(|row| (row.owner, row.project, row.ordinal));
    Tables {
        hour,
        countries,
        links,
        projects,
        history,
    }
}

struct Tables {
    hour: i32,
    countries: Vec<(u32, u32, &'static str)>,
    links: Vec<(u32, u32)>,
    projects: Vec<ProjectRow>,
    history: Vec<ProjectHistoryRow>,
}

fn country_rows(countries: &[(u32, u32, &str)]) -> Vec<CountryRow> {
    let mut rows: Vec<_> = countries
        .iter()
        .map(|&(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect();
    rows.sort_unstable_by_key(|row| row.id);
    rows
}

fn link_rows(links: &[(u32, u32)]) -> Vec<CountryLinkRow> {
    links
        .iter()
        .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect()
}

fn programs() -> Vec<ProjectProgramRow> {
    vec![ProjectProgramRow {
        id: 5,
        owner: 1,
        scientist: 7 << 32 | 73,
        ..Default::default()
    }]
}

fn characters() -> Vec<CharacterRow> {
    vec![CharacterRow {
        id: 7,
        country: 1,
        name_token_id: -1,
        token: b"synthetic_scientist".to_vec(),
        ..Default::default()
    }]
}

fn tokens() -> Vec<LexicalTokenRow> {
    vec![
        LexicalTokenRow {
            token: 8,
            literal: b"project_a".to_vec(),
        },
        LexicalTokenRow {
            token: 9,
            literal: b"project_b".to_vec(),
        },
    ]
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, Some(1));
    let mut put = |table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    put(TableId::Countries, TableRows::Countries(country_rows(&tables.countries)));
    put(TableId::CountryLinks, TableRows::CountryLinks(link_rows(&tables.links)));
    put(TableId::Projects, TableRows::Projects(tables.projects.clone()));
    put(TableId::ProjectHistory, TableRows::ProjectHistory(tables.history.clone()));
    put(TableId::ProjectPrograms, TableRows::ProjectPrograms(programs()));
    put(TableId::Characters, TableRows::Characters(characters()));
    put(TableId::CharacterPortraits, TableRows::CharacterPortraits(Vec::new()));
    put(TableId::LexicalTokens, TableRows::LexicalTokens(tokens()));
    frame.metrics.table_ns[TableId::Projects.index()] = 1;
    frame
}

/// `frame`'s `table` written into `walk` as the recorder writes it.
fn write<T: replay_data::rows::TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame)
where
    T::Row: RowCodec,
{
    let table = T::ID;
    // Projects is read in slices: its read bit is the frame's.
    let read = table != TableId::Projects || frame.metrics.table_ns[table.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*frame.tables[table.index()] {
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
    walk.table(table, input).unwrap();
}

/// The trigger kit's tables of `frame` written into `walk`: what scoped project names read.
fn write_kit(walk: &mut Synthetic, frame: &DecodedFrame) {
    macro_rules! kit { ($($table:ident),* $(,)?) => { $(write::<tables::$table>(walk, frame);)* }; }
    kit!(
        ActiveIdeas,
        AdvisorTokens,
        ArmyOrderPlans,
        ArmyOrders,
        Autonomy,
        CharacterOperatives,
        CharacterRoleTraits,
        CompletedFocuses,
        CountryAppearance,
        CountryBuildingResources,
        CountryCharacters,
        CountryCivilWars,
        CountryCommandPower,
        CountryCounterInputs,
        CountryFactories,
        CountryLeaderRoles,
        CountryResources,
        CountryStabilityInputs,
        CountryRetiredCharacters,
        CountryWars,
        CountryWargoals,
        WargoalTargets,
        Divisions,
        DivisionStatus,
        DivisionTemplateIcons,
        DivisionTemplates,
        DynamicModifiers,
        FactionAdvisors,
        FactionMembers,
        Factions,
        Focus,
        GlobalSavedEventTargets,
        AdvisorRoles,
        HiredAdvisors,
        HumanControllers,
        IndustrialOrganisationDetails,
        IndustrialOrganisations,
        IntelligenceAgencies,
        InvasionPreparations,
        LandTheatres,
        LocalPlayer,
        ModifierDefinitions,
        Nukes,
        OrdersGroupPlans,
        OrdersGroups,
        PartyLeaders,
        PoliticalParties,
        Politics,
        Players,
        PowerBalanceMembers,
        PowerBalanceModifierBreakdowns,
        PowerBalances,
        ProvinceBuildings,
        ProvinceNames,
        Provinces,
        RaidTooltipOperands,
        RaidCountryInputs,
        Raids,
        RelationOpinions,
        ResearchTechnologies,
        ResearchModifiers,
        ReservedCountryNaming,
        ResourceDefinitions,
        ScriptFlags,
        ScriptVariables,
        SimulationFeatures,
        StateBaseResources,
        StateBuildings,
        StateCores,
        StateDynamicModifiers,
        StateResourceInputs,
        StateResources,
        StateResistance,
        States,
        Subjects,
        TechnologyDefinitions,
        TechSharingGroups,
        TechSharingMembers,
        UnitLeaderTraits,
        UnitLeaders,
        WarRelations,
    );
}

/// The projected hours of `frames`, written as the native recorder writes them, a projector
/// starting cold at each of `starts` and at each walk's first hour (after a gap the walk starts
/// again).
fn projected(binding: &Binding, frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let scoped = binding.scoped();
    let first = frames[0].hour;
    let firsts = std::sync::Arc::new(BTreeMap::from([(1, first)]));
    let new = || Projector::new(FormatVersion::CURRENT, binding, firsts.clone()).unwrap();
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = new();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let hour = frame.hour;
        if last.is_none_or(|last| last + 1 != hour) {
            walk = Some(Synthetic::new(facts::tables(scoped), 1, hour).unwrap());
        }
        last = Some(hour);
        let walk = walk.as_mut().unwrap();
        let campaign = [CampaignRow {
            id: b"campaign".to_vec(),
        }];
        walk.table(
            TableId::Campaign,
            Input::Snapshot {
                rows: &campaign[..],
                partial: None,
                read: true,
                sample: Sample::FromReadBit,
            },
        )
        .unwrap();
        write::<tables::Countries>(walk, frame);
        write::<tables::CountryLinks>(walk, frame);
        write::<tables::Projects>(walk, frame);
        write::<tables::ProjectHistory>(walk, frame);
        write::<tables::ProjectPrograms>(walk, frame);
        write::<tables::Characters>(walk, frame);
        write::<tables::CharacterPortraits>(walk, frame);
        write::<tables::LexicalTokens>(walk, frame);
        if scoped {
            write_kit(walk, frame);
        }
        let cold = starts.contains(&hour);
        if cold {
            projector = new();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(scoped),
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(out).unwrap()
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

struct Built {
    pages: Memory,
    root: u64,
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
fn build(
    binding: &Binding,
    hours: Vec<facts::Hour>,
    (cuts, resume, gaps): (&[i32], Option<i32>, Vec<(i32, i32)>),
    budget: &Budget,
) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new(binding.emitted(), first, gaps.clone());
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            &cache,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit =
                Emit::load(binding.emitted(), first, gaps.clone(), &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// The row recording of `frames`, each hour not walked between them a gap, and its gaps.
fn recording(frames: Vec<DecodedFrame>) -> (Source, Vec<(i32, i32)>) {
    let mut parts: Vec<Vec<DecodedFrame>> = Vec::new();
    let mut gaps = Vec::new();
    for frame in frames {
        match parts.last_mut() {
            Some(part) if part.last().unwrap().hour + 1 == frame.hour => part.push(frame),
            Some(part) => {
                gaps.push((part.last().unwrap().hour + 1, frame.hour - 1));
                parts.push(vec![frame]);
            }
            None => parts.push(vec![frame]),
        }
    }
    let mut parts = parts.into_iter();
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, parts.next().unwrap())]).unwrap();
    for part in parts {
        replay = replay
            .append(&[continuation_records(&replay, 1, part)])
            .unwrap();
    }
    (file_source(replay).0, gaps)
}

/// `frames`' feed as the publication of their native walk lists it, cut after each of `cuts`
/// and resumed at `resume`, a segment starting at each of `starts`.
fn native_feed(
    frames: &[DecodedFrame],
    binding: &Binding,
    gaps: &[(i32, i32)],
    (starts, cuts, resume): (&[i32], &[i32], Option<i32>),
) -> query::Feed {
    let budget = Budget::new(256 << 20);
    let hours = projected(binding, frames, starts);
    let built = build(binding, hours, (cuts, resume, gaps.to_vec()), &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    query::feed(&mut snapshot, &directory, &AtomicBool::new(false)).unwrap()
}

/// Every placement of segment starts, cuts and a resume answers each country's projects as the
/// row reply does, through every hour and through an earlier one, and lists the completions the
/// row read lists.
#[test]
fn the_publication_answers_what_the_row_recording_answers_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = hours().into_iter().map(|hour| frame(&tables(hour))).collect();
    let (source, gaps) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    let oracle = completion_events(&source, 1, &context, &cancelled).unwrap();
    let row_feed = row::feed(&source, 1, &context, &cancelled).unwrap();
    let listed = |feed: &query::Feed| {
        feed.changes
            .iter()
            .filter_map(|change| change.completion.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(&row_feed), oracle.events, "the row read lists the oracle's completions");
    for (tag, date) in [("AAA", 110), ("BBB", 113)] {
        assert!(
            oracle
                .events
                .iter()
                .any(|event| event.tag == tag && event.date.hour == date && event.date.exact),
            "{tag}'s completion"
        );
    }
    for (starts, cuts, resume) in [
        (&[][..], &[130][..], None),
        (&[105, 111, 124][..], &[110, 119, 130][..], Some(119)),
        (&[110, 115][..], &[109, 115, 125, 130][..], Some(115)),
        (&hours()[1..], &hours()[..], Some(112)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let projected = projected(&binding, &frames, starts);
        let built = build(&binding, projected, (cuts, resume, gaps.clone()), &budget);
        let cache = Cache::new(&budget);
        let mut snapshot =
            Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let feed = query::feed(&mut snapshot, &directory, &cancelled).unwrap();
        assert_eq!(feed.changes, row_feed.changes, "{label}: the completions");
        assert_eq!(feed.available, row_feed.available, "{label}: available");
        for (tag, roots) in [
            ("AAA", vec![]),
            ("BBB", vec![]),
            ("BBB", vec!["BBB".to_owned()]),
            ("CCC", vec![]),
            ("ZZZ", vec![]),
        ] {
            let params = SpecialProjectsParams {
                branch_id: "1".into(),
                tag: tag.into(),
                selection_tags: roots,
                hour: None,
            };
            let mut reply =
                query::reply(&mut snapshot, &directory, &params, 130, &cancelled).unwrap();
            present_reply(&source, &ArtSource::new(&source, None), &mut reply).unwrap();
            let expected = SpecialProjects::load(&source, &params, &context, &cancelled).unwrap();
            assert_eq!(
                serde_json::to_value(&reply).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{label}: {params:?}"
            );
            // Through every hour, the ledger preparation built answers what building the
            // episodes again from the observations answers.
            for through in 99..=131 {
                let ledger =
                    query::reply(&mut snapshot, &directory, &params, through, &cancelled).unwrap();
                let rebuilt =
                    query::rebuilt_reply(&mut snapshot, &directory, &params, through, &cancelled)
                        .unwrap();
                assert_eq!(
                    serde_json::to_value(&ledger).unwrap(),
                    serde_json::to_value(&rebuilt).unwrap(),
                    "{label}: {params:?} through {through}"
                );
            }
            // Through an earlier hour, nothing after it is read.
            let early = query::reply(&mut snapshot, &directory, &params, 112, &cancelled).unwrap();
            assert!(early.read_hours.iter().all(|&hour| hour <= 112), "{label}");
            assert!(
                early
                    .projects
                    .iter()
                    .flat_map(|episode| &episode.runs)
                    .all(|run| run.last_hour <= 112),
                "{label}: {params:?} through 112"
            );
        }
    }
}

/// Of many countries, an hour that changes one project observes that country alone, sending its
/// progress alone where nothing else of it changed and nothing where its rows changed in nothing
/// it observes.
#[test]
fn an_hour_observes_only_the_countries_whose_projects_changed() {
    let context = LoadContext { content: None };
    let frames: Vec<_> = hours().into_iter().map(|hour| frame(&tables(hour))).collect();
    let (source, _) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    let projected = projected(&binding, &frames, &[]);
    for hour in &projected {
        let observed: Vec<(&str, bool)> = hour
            .facts
            .iter()
            .flat_map(|facts| &facts.countries)
            .map(|country| (&*country.tag, matches!(country.seen, facts::Seen::Whole(_))))
            .collect();
        let whole = |tag| (tag, true);
        let progress = ("AAA", false);
        match hour.hour {
            // A walk's first hour, after the gap, and BBB's new id observe every country.
            100 | 122 | 124 => assert_eq!(
                observed,
                [whole("AAA"), whole("BBB"), whole("CCC")],
                "{}",
                hour.hour
            ),
            // AAA's progress moves every hour until it completes at 110; BBB's project and
            // history change at 105 and 115.
            105 => assert_eq!(observed, [progress, whole("BBB")], "{}", hour.hour),
            110 => assert_eq!(observed, [whole("AAA")], "{}", hour.hour),
            115 => assert_eq!(observed, [whole("BBB")], "{}", hour.hour),
            101..=109 => assert_eq!(observed, [progress], "{}", hour.hour),
            // A completed project's meter is full however its row's progress moves.
            _ => assert!(observed.is_empty(), "{}: {observed:?}", hour.hour),
        }
    }
}

/// A program's change in a field no look reads observes nothing of the country whose project
/// it runs, and the publication still answers what the row recording answers.
#[test]
fn a_program_field_no_look_reads_observes_nothing() {
    let context = LoadContext { content: None };
    let frames: Vec<_> = hours()
        .into_iter()
        .map(|hour| {
            let mut frame = frame(&tables(hour));
            let mut programs = programs();
            programs[0].unread_rewards = hour;
            *frame.tables[TableId::ProjectPrograms.index()] =
                TableView::Complete(TableRows::ProjectPrograms(programs));
            frame
        })
        .collect();
    let (source, gaps) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    let projected = projected(&binding, &frames, &[]);
    for hour in projected.iter().filter(|hour| (101..=104).contains(&hour.hour)) {
        let observed: Vec<(&str, bool)> = hour
            .facts
            .iter()
            .flat_map(|facts| &facts.countries)
            .map(|country| (&*country.tag, matches!(country.seen, facts::Seen::Whole(_))))
            .collect();
        assert_eq!(observed, [("AAA", false)], "{}", hour.hour);
    }
    let cancelled = AtomicBool::new(false);
    let feed = native_feed(&frames, &binding, &gaps, (&[], &[130], None));
    assert_eq!(
        feed.changes,
        row::feed(&source, 1, &context, &cancelled).unwrap().changes
    );
}

/// History rows rewritten every hour in a field no operand reads observe nothing of their
/// owners, and the publication still answers what the row recording answers.
#[test]
fn a_history_field_no_operand_reads_observes_nothing() {
    let context = LoadContext { content: None };
    let frames: Vec<_> = hours()
        .into_iter()
        .map(|hour| {
            let mut tables = tables(hour);
            for row in &mut tables.history {
                row.context.progress = hour;
            }
            frame(&tables)
        })
        .collect();
    let (source, gaps) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    let projected = projected(&binding, &frames, &[]);
    for hour in &projected {
        let observed: Vec<(&str, bool)> = hour
            .facts
            .iter()
            .flat_map(|facts| &facts.countries)
            .map(|country| (&*country.tag, matches!(country.seen, facts::Seen::Whole(_))))
            .collect();
        let whole = |tag| (tag, true);
        match hour.hour {
            100 | 122 | 124 => assert_eq!(
                observed,
                [whole("AAA"), whole("BBB"), whole("CCC")],
                "{}",
                hour.hour
            ),
            105 => assert_eq!(observed, [("AAA", false), whole("BBB")], "{}", hour.hour),
            110 => assert_eq!(observed, [whole("AAA")], "{}", hour.hour),
            115 => assert_eq!(observed, [whole("BBB")], "{}", hour.hour),
            101..=109 => assert_eq!(observed, [("AAA", false)], "{}", hour.hour),
            _ => assert!(observed.is_empty(), "{}: {observed:?}", hour.hour),
        }
    }
    let cancelled = AtomicBool::new(false);
    let feed = native_feed(&frames, &binding, &gaps, (&[], &[130], None));
    assert_eq!(
        feed.changes,
        row::feed(&source, 1, &context, &cancelled).unwrap().changes
    );
}

/// A completion as the oracle's scenarios read it: one country's project, read at 100 and
/// 102, completing with its history dating it `date`.
fn completing(hour: i32, read: bool) -> DecodedFrame {
    let mut frame = source(1, hour, None);
    let fixture = super::tests::frame(hour, read, 1);
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Projects,
        TableId::ProjectPrograms,
        TableId::ProjectHistory,
        TableId::Characters,
        TableId::CharacterPortraits,
    ] {
        frame.tables[table.index()] = fixture.tables[table.index()].clone();
    }
    frame.metrics = fixture.metrics;
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]));
    frame
}

/// `frame`'s projects completed, their history dating them `date`, none where it is unread.
fn complete(frame: &mut DecodedFrame, date: Option<i32>) {
    let mut rows = frame.complete::<tables::Projects>().unwrap().to_vec();
    let owner = rows[0].owner;
    for row in &mut rows {
        row.state.current_offset = 0xe8;
        row.state.not_completed = 0;
    }
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(rows));
    *frame.tables[TableId::ProjectHistory.index()] = match date {
        Some(date) => TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner,
            project: 0,
            prototype_reward_handle: COMPLETION,
            option_handle: COMPLETION,
            date,
            ..Default::default()
        }])),
        None => TableView::Unavailable {
            reason: "not recorded".into(),
        },
    };
}

/// Sets the tag of `frame`'s one country.
fn rename(frame: &mut DecodedFrame, tag: &str) {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = tag.as_bytes().to_vec();
    }
}

/// The completions the oracle, the row read and the native publication list of `frames`, at
/// every placement of a segment start, a cut and a resume over them.
fn completions_of(frames: Vec<DecodedFrame>) -> (std::sync::Arc<ProjectCompletions>, query::Feed) {
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let (source, gaps) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    let oracle = completion_events(&source, 1, &context, &cancelled).unwrap();
    let row = row::feed(&source, 1, &context, &cancelled).unwrap();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let last = *hours.last().unwrap();
    let mut placements = vec![(vec![], vec![last], None)];
    for &hour in &hours[1..] {
        placements.push((vec![hour], vec![hour - 1, last], Some(hour - 1)));
    }
    for (starts, cuts, resume) in placements {
        let cuts: Vec<i32> = cuts
            .into_iter()
            .filter(|cut| hours.contains(cut))
            .collect();
        let resume = resume.filter(|cut| cuts.contains(cut));
        let native = native_feed(&frames, &binding, &gaps, (&starts, &cuts, resume));
        assert_eq!(native.changes, row.changes, "starts {starts:?}, cuts {cuts:?}");
    }
    assert_eq!(
        row.changes
            .iter()
            .filter_map(|change| change.completion.clone())
            .collect::<Vec<_>>(),
        oracle.events
    );
    (oracle, row)
}

/// Whether any of `feed`'s changes withdraws a completion for `reason`.
fn refused(feed: &query::Feed, reason: &str) -> bool {
    feed.changes.iter().any(|change| {
        change.completion.is_none()
            && change
                .warnings
                .iter()
                .any(|(held, count)| held == reason && *count == 1)
    })
}

/// A completion dated before a rename is shown by the tag of its date.
#[test]
fn a_completion_keeps_the_tag_its_holder_had_at_its_date() {
    let first = completing(100, true);
    let middle = completing(101, false);
    let mut renamed = completing(102, true);
    rename(&mut renamed, "renamed tag");
    complete(&mut renamed, Some(101));
    let mut later = renamed.clone();
    later.hour = 103;
    let (oracle, feed) = completions_of(vec![first, middle, renamed, later]);
    assert_eq!(oracle.events.len(), 1);
    let completion = feed.changes[0].completion.as_ref().unwrap();
    assert_ne!(completion.tag, "renamed tag");
    assert_eq!(completion.date, ProjectDate { hour: 101, exact: true });
    assert_eq!(completion.observed_hour, 102);
}

/// A completion dated where its holder was another country's is refused, never shown.
#[test]
fn a_completion_whose_holder_was_reassigned_at_its_date_is_refused() {
    let first = completing(100, true);
    let mut reassigned = completing(101, false);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reassigned.tables[TableId::Countries.index()]
    {
        rows[0].id = 6;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reassigned.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 6,
            canonical_slot: 0,
        });
    }
    let mut last = completing(102, true);
    complete(&mut last, Some(101));
    let (oracle, feed) = completions_of(vec![first, reassigned, last]);
    assert!(oracle.events.is_empty());
    assert!(refused(
        &feed,
        "Project country identity could not be resolved at its completion hour."
    ));
}

/// A completion dated in a gap is refused, never shown.
#[test]
fn a_completion_dated_in_a_gap_is_refused() {
    let first = completing(100, true);
    let mut after = completing(103, true);
    complete(&mut after, Some(101));
    let (oracle, feed) = completions_of(vec![first, after]);
    assert!(oracle.events.is_empty());
    assert!(refused(&feed, "Project completion hour was not recorded."));
}

/// A segment start, a cut or a resume in an unchanged dictionary keeps its revision.
#[test]
fn segment_starts_keep_an_unchanged_dictionary_revision() {
    let context = LoadContext { content: None };
    let frames: Vec<_> = (100..=110).map(|hour| completing(hour, true)).collect();
    let (source, _) = recording(frames.clone());
    let binding = Binding::load(&source, &context).unwrap();
    for starts in [&[][..], &[103, 104, 107][..]] {
        let digests: Vec<u128> = projected(&binding, &frames, starts)
            .into_iter()
            .filter_map(|hour| hour.dictionary.map(|dictionary| dictionary.digest))
            .collect();
        assert!(!digests.is_empty());
        assert!(digests.windows(2).all(|pair| pair[0] == pair[1]), "{starts:?}");
    }
}

/// A completion dated where the dictionary was not read names no token, as at that hour.
#[test]
fn a_completion_dated_where_the_dictionary_was_unread_names_no_token() {
    let first = completing(100, true);
    let mut unread = completing(101, false);
    *unread.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let mut last = completing(102, true);
    complete(&mut last, Some(101));
    let (oracle, feed) = completions_of(vec![first, unread, last]);
    assert_eq!(oracle.events.len(), 1);
    let completion = feed.changes[0].completion.as_ref().unwrap();
    assert_eq!(completion.item_token, None);
    assert_eq!(completion.name, None);
}

/// Two countries' projects named by scoped text that reads a country variable, raised at 102,
/// as `history_tests` content names them; each hour's frame holds the world that text reads.
fn scoped(hour: i32) -> DecodedFrame {
    let mut frame = super::history_tests::frame(hour, true, 2);
    for table in [TableId::Countries, TableId::CountryLinks] {
        match &mut *frame.tables[table.index()] {
            TableView::Complete(TableRows::Countries(rows)) => {
                rows.iter_mut().for_each(|row| row.canonical_slot = row.id);
            }
            TableView::Complete(TableRows::CountryLinks(rows)) => {
                rows.iter_mut().for_each(|row| row.canonical_slot = row.id);
            }
            _ => unreachable!(),
        }
    }
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![replay_data::rows::StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![replay_data::rows::ProvinceRow {
            id: 1,
            state: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            std::iter::once(replay_data::rows::ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![],
            })
            .chain((1..=2).map(|id| replay_data::rows::ScriptVariablesRow {
                scope_kind: 1,
                scope_id: id,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![replay_data::rows::ScriptVariableEntry {
                    name: b"project_flag".to_vec(),
                    value_raw: if hour < 102 { 0 } else { 100_000 },
                }],
            }))
            .collect(),
        ));
    frame
}

/// `frames` recorded under `identity`'s content.
fn content_recording(frames: Vec<DecodedFrame>, identity: replay_data::identity::Identity) -> Source {
    let mut records = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    super::history_tests::source(Replay::from_records(CAMPAIGN.into(), &[records]).unwrap(), "1")
}

/// Mutates `frame`'s `tag`'s project rows.
fn project_of(frame: &mut DecodedFrame, owner: u32, change: impl FnOnce(&mut Vec<ProjectRow>)) {
    let mut rows = frame.complete::<tables::Projects>().unwrap().to_vec();
    let mut own: Vec<_> = rows.iter().filter(|row| row.owner == owner).cloned().collect();
    rows.retain(|row| row.owner != owner);
    change(&mut own);
    rows.extend(own);
    rows.sort_unstable_by_key(|row| (row.owner, row.local_id));
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(rows));
}

/// Scoped project names read through segment starts as the row recording reads them: a project
/// cancelled by script while its episode is open keeps its name after a cold start, and a name
/// gone at a cold start names nothing at a completion dated while it was gone.
#[test]
fn scoped_names_read_through_segment_starts_as_the_row_recording_reads_them() {
    let frames: Vec<_> = (100..=107)
        .map(|hour| {
            let mut frame = scoped(hour);
            // opaque-2's project is cancelled by script at 103: not started, no program.
            if hour >= 103 {
                project_of(&mut frame, 2, |rows| {
                    for row in rows {
                        row.started = 0;
                        row.program_reference = 0;
                    }
                });
            }
            // opaque-1's project is gone at 103 and 104, and back completed at 105, its
            // history dating it 104.
            match hour {
                103..=104 => project_of(&mut frame, 1, Vec::clear),
                105.. => {
                    project_of(&mut frame, 1, |rows| {
                        for row in rows {
                            row.state.current_offset = 0xe8;
                            row.state.not_completed = 0;
                        }
                    });
                    *frame.tables[TableId::ProjectHistory.index()] =
                        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                            owner: 1,
                            project: 1,
                            ordinal: 0,
                            prototype_reward_handle: COMPLETION,
                            option_handle: COMPLETION,
                            date: 104,
                            ..Default::default()
                        }]));
                }
                _ => {}
            }
            frame
        })
        .collect();
    let (source, context, _folder) = scoped_parity(
        &frames,
        &[
            (&[][..], &[107][..], None),
            (&[103, 105][..], &[102, 104, 107][..], Some(104)),
            (&[104][..], &[103, 107][..], Some(103)),
        ],
        &["opaque-1", "opaque-2"],
    );
    let cancelled = AtomicBool::new(false);
    // The cancelled project is named through every hour it is shown.
    let params = SpecialProjectsParams {
        branch_id: "1".into(),
        tag: "opaque-2".into(),
        selection_tags: Vec::new(),
        hour: None,
    };
    let expected = SpecialProjects::load(&source, &params, &context, &cancelled).unwrap();
    assert!(
        expected
            .presentations
            .iter()
            .all(|info| info.name != "Project name unavailable"),
        "{:?}",
        expected.presentations
    );
}

/// The content-backed recording of `frames`, its projects named by scoped text, held to the row
/// recording at each placement of segment starts, cuts and a resume: every country's reply,
/// and the completions the oracle and the row read list. The recording, its content and the
/// folder holding it.
#[allow(clippy::type_complexity)]
fn scoped_parity(
    frames: &[DecodedFrame],
    placements: &[(&[i32], &[i32], Option<i32>)],
    tags: &[&str],
) -> (Source, LoadContext, super::tests::TempRoot) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let folder = super::tests::TempRoot::new("native-scoped");
    super::history_tests::content(&folder.0, "[ROOT.ProjectStage]");
    let (options, recorded) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let context = LoadContext {
        content: Some(options),
    };
    let cancelled = AtomicBool::new(false);
    let source = content_recording(frames.to_vec(), recorded);
    let binding = Binding::load(&source, &context).unwrap();
    assert!(binding.scoped(), "the content names its projects by scoped text");
    let oracle = completion_events(&source, 1, &context, &cancelled).unwrap();
    let row = row::feed(&source, 1, &context, &cancelled).unwrap();
    let budget = Budget::new(256 << 20);
    let last = frames.last().unwrap().hour;
    for &(starts, cuts, resume) in placements {
        let label = format!("starts {starts:?}");
        let hours = projected(&binding, frames, starts);
        let built = build(&binding, hours, (cuts, resume, Vec::new()), &budget);
        let cache = Cache::new(&budget);
        let mut snapshot =
            Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let feed = query::feed(&mut snapshot, &directory, &cancelled).unwrap();
        assert_eq!(feed.changes, row.changes, "{label}: the completions");
        assert_eq!(
            feed.changes
                .iter()
                .filter_map(|change| change.completion.clone())
                .collect::<Vec<_>>(),
            oracle.events,
            "{label}: the oracle's completions"
        );
        for &tag in tags {
            let params = SpecialProjectsParams {
                branch_id: "1".into(),
                tag: tag.into(),
                selection_tags: Vec::new(),
                hour: None,
            };
            let mut reply =
                query::reply(&mut snapshot, &directory, &params, last, &cancelled).unwrap();
            present_reply(
                &source,
                &ArtSource::new(&source, context.content.as_ref()),
                &mut reply,
            )
            .unwrap();
            let expected = SpecialProjects::load(&source, &params, &context, &cancelled).unwrap();
            assert_eq!(
                serde_json::to_value(&reply).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{label}: {tag}"
            );
        }
    }
    (source, context, folder)
}

/// A project never started when a cold segment starts, completed at the next hour with its
/// history dating it the cold hour, names what the row recording names there: nothing.
#[test]
fn a_project_never_started_at_a_cold_start_lends_no_name_to_its_completion() {
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = scoped(hour);
            project_of(&mut frame, 1, |rows| {
                for row in rows {
                    if hour <= 101 {
                        row.started = 0;
                        row.program_reference = 0;
                    } else {
                        row.state.current_offset = 0xe8;
                        row.state.not_completed = 0;
                    }
                }
            });
            if hour >= 102 {
                *frame.tables[TableId::ProjectHistory.index()] =
                    TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                        owner: 1,
                        project: 1,
                        ordinal: 0,
                        prototype_reward_handle: COMPLETION,
                        option_handle: COMPLETION,
                        date: 101,
                        ..Default::default()
                    }]));
            }
            frame
        })
        .collect();
    let (source, context, _folder) = scoped_parity(
        &frames,
        &[
            (&[][..], &[103][..], None),
            (&[101][..], &[100, 103][..], Some(100)),
            (&[101, 102][..], &[101, 103][..], Some(101)),
        ],
        &["opaque-1", "opaque-2"],
    );
    let oracle = completion_events(&source, 1, &context, &AtomicBool::new(false)).unwrap();
    assert!(
        oracle
            .events
            .iter()
            .all(|event| event.name.as_deref() != Some("Waiting")),
        "the row recording names no project at the cold hour: {:?}",
        oracle.events
    );
}

/// The history entry dating opaque-1's project's completion `date`.
fn completed_at(frame: &mut DecodedFrame, date: i32) {
    project_of(frame, 1, |rows| {
        for row in rows {
            row.started = 1;
            row.state.current_offset = 0xe8;
            row.state.not_completed = 0;
        }
    });
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 1,
            project: 1,
            ordinal: 0,
            prototype_reward_handle: COMPLETION,
            option_handle: COMPLETION,
            date,
            ..Default::default()
        }]));
}

/// A project cancelled by script at a cold start and gone the hour after names nothing while
/// it is gone, though its open episode named it at the cold hour.
#[test]
fn a_provisional_name_ends_where_its_project_is_gone() {
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = scoped(hour);
            match hour {
                101 => project_of(&mut frame, 1, |rows| {
                    for row in rows {
                        row.started = 0;
                        row.program_reference = 0;
                    }
                }),
                102 => project_of(&mut frame, 1, Vec::clear),
                103.. => completed_at(&mut frame, 102),
                _ => {}
            }
            frame
        })
        .collect();
    scoped_parity(
        &frames,
        &[
            (&[][..], &[104][..], None),
            (&[101][..], &[100, 104][..], Some(100)),
            (&[101, 103][..], &[102, 104][..], Some(102)),
        ],
        &["opaque-1", "opaque-2"],
    );
}

/// A holder renamed at a cold start, its project cancelled by script there, opens no episode of
/// it under its new tag: the new tag's completion dated the cold hour names nothing there.
#[test]
fn a_rename_at_a_cold_start_lends_the_old_tags_episode_no_name() {
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = scoped(hour);
            if hour >= 101 {
                rename_country(&mut frame, 1, "opaque-9");
            }
            match hour {
                101 => project_of(&mut frame, 1, |rows| {
                    for row in rows {
                        row.started = 0;
                        row.program_reference = 0;
                    }
                }),
                102.. => completed_at(&mut frame, 101),
                _ => {}
            }
            frame
        })
        .collect();
    scoped_parity(
        &frames,
        &[
            (&[][..], &[103][..], None),
            (&[101][..], &[100, 103][..], Some(100)),
            (&[101, 102][..], &[101, 103][..], Some(101)),
        ],
        &["opaque-1", "opaque-2", "opaque-9"],
    );
}

/// Sets the tag of `frame`'s country `id`.
fn rename_country(frame: &mut DecodedFrame, id: u32, tag: &str) {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.id == id) {
            row.tag = tag.as_bytes().to_vec();
        }
    }
}
