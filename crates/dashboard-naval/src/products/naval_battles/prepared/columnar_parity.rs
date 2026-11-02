//! The naval family's columnar walk against its row path, every published section compared.

use super::*;
use crate::products::naval_battles::columnar::{self, Projector};
use dashboard_base::{
    prepared::{
        frame_changes::FrameChangeTracker,
        series::tests::{Fixture, assert_payloads_equal},
    },
    source::SourceKey,
};
use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Item, Operator, Start},
};
use replay_data::rows::{
    CampaignRow, CountryLinkRow, CountryRow, FleetRow, LexicalTokenRow, NavalCombatDetailsRow,
    NavalCombatParticipantRow, NavalCombatResultDetailsRow, NavalCombatRow, NavalCombatSideDetails,
    NavalCombatSummaryInputs, NavalResultCountryRow, NavalResultMemberRow, NavalResultRow,
    NavalResultShip, NavalResultSideDetails, NavalResultSourceRow, Recorded, RowCodec,
    SunkConvoyRow, SunkShipEntry, SunkShipsRow, TableRows, TaskForceRow,
};
use replay_frame::frame::TableView;

/// Hours a row block holds.
const BLOCK: usize = 6;
/// The raw links of the two countries the hours name.
const GER: u32 = 11;
const ENG: u32 = 12;

/// One table's hour: its rows, read in part (`Some(missing)`), or unavailable for a reason.
enum Rows<R> {
    Whole(Vec<R>),
    Partial(Vec<R>, u32),
    Unavailable(&'static str),
}

/// Everything one synthetic hour records of the tables the family reads.
struct HourTables {
    countries: Rows<CountryRow>,
    links: Vec<CountryLinkRow>,
    combats: Vec<NavalCombatRow>,
    participants: Rows<NavalCombatParticipantRow>,
    details: Vec<NavalCombatDetailsRow>,
    results: Vec<NavalResultRow>,
    members: Rows<NavalResultMemberRow>,
    result_countries: Vec<NavalResultCountryRow>,
    result_details: Vec<NavalCombatResultDetailsRow>,
    sources: Vec<NavalResultSourceRow>,
    task_forces: Vec<TaskForceRow>,
    fleets: Vec<FleetRow>,
    ships: Rows<SunkShipsRow>,
    convoys: Vec<SunkConvoyRow>,
    /// Whether the hour read the convoys.
    convoys_read: bool,
    tokens: Vec<LexicalTokenRow>,
}

fn detail(combat: u64, province: u32, hour: i32, countries: [u32; 2]) -> NavalCombatDetailsRow {
    NavalCombatDetailsRow {
        combat,
        province,
        duration: hour,
        sides: countries.map(|country| NavalCombatSideDetails {
            positioning: i64::from(hour),
            summary: NavalCombatSummaryInputs {
                countries: vec![country],
                ..Default::default()
            },
            units: vec![9101],
            ..Default::default()
        }),
        custom_sea: Some(vec![]),
        ..Default::default()
    }
}

fn participant(
    combat: u64,
    side: u8,
    slot: u32,
    ship: u64,
    owner: u32,
    state: u32,
) -> NavalCombatParticipantRow {
    NavalCombatParticipantRow {
        combat,
        side,
        battle_slot: slot,
        ship,
        owner,
        state,
    }
}

fn result_rows(
    out: &mut HourTables,
    id: u64,
    (province, result_hour, winning): (u32, i32, u8),
    members: &[(u64, u8, u64)],
    countries: [u32; 2],
) {
    out.results.push(NavalResultRow {
        id,
        province,
        result_hour,
        winning_side: winning,
        losing_side: 3 - winning,
        ..Default::default()
    });
    for (ordinal, &(member, side, presence)) in members.iter().enumerate() {
        if let Rows::Whole(rows) | Rows::Partial(rows, _) = &mut out.members {
            rows.push(NavalResultMemberRow {
                result: id,
                member,
                presence_value: presence,
                ordinal: ordinal as u32,
                side,
            });
        }
    }
    for (list, &country) in countries.iter().enumerate() {
        out.result_countries.push(NavalResultCountryRow {
            result: id,
            country,
            ordinal: 0,
            list: list as u8,
        });
    }
    out.result_details.push(NavalCombatResultDetailsRow {
        result: id,
        sides: std::array::from_fn(|which| NavalResultSideDetails {
            ships: members
                .iter()
                .filter(|&&(_, side, _)| usize::from(side) == which)
                .map(|&(member, _, _)| NavalResultShip {
                    member,
                    is_convoy: 0,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
    });
}

fn entry(name: &[u8], battle: u64, province: u32, sink_hour: i32, victim: u32) -> SunkShipEntry {
    SunkShipEntry {
        ship_name: name.to_vec(),
        class_name: b"recorded class".to_vec(),
        victim_country: victim,
        attacker_country: GER,
        sink_hour,
        class_token: 1,
        province,
        province_present: u8::from(province != 0),
        battle,
        cause_record: u8::from(battle == 0),
        ..Default::default()
    }
}

/// The campaign's hour `hour`: battles, results, sources, sinkings, convoys and gaps in reads.
fn hour_tables(hour: i32) -> HourTables {
    let mut out = HourTables {
        countries: Rows::Whole(vec![
            CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 9002,
                canonical_slot: 8,
                tag: if hour >= 118 {
                    b"UKX".to_vec()
                } else {
                    b"ENG".to_vec()
                },
                war_support_raw: i64::from(hour),
                ..Default::default()
            },
        ]),
        links: [(GER, 7), (ENG, 8), (9001, 7), (9002, 8)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
        combats: Vec::new(),
        participants: Rows::Whole(Vec::new()),
        details: Vec::new(),
        results: Vec::new(),
        members: if hour == 117 {
            Rows::Partial(Vec::new(), 1)
        } else {
            Rows::Whole(Vec::new())
        },
        result_countries: Vec::new(),
        result_details: Vec::new(),
        sources: Vec::new(),
        task_forces: vec![TaskForceRow {
            id: 9101,
            fleet: 9201,
            province: hour as u32,
            ..Default::default()
        }],
        fleets: vec![FleetRow {
            id: 9201,
            name: Recorded::Value(Some(if hour >= 107 {
                b"Grand Fleet".to_vec()
            } else {
                b"Home Fleet".to_vec()
            })),
            ..Default::default()
        }],
        ships: Rows::Whole(Vec::new()),
        convoys: Vec::new(),
        convoys_read: hour % 4 != 2,
        tokens: vec![LexicalTokenRow {
            token: 1,
            literal: b"destroyer".to_vec(),
        }],
    };
    if hour == 124 {
        out.countries = Rows::Unavailable("country identities not read");
    }
    let mut participants = Vec::new();
    let mut combat =
        |id: u64, province: u32, detail_province: u32, rows: Vec<NavalCombatParticipantRow>| {
            out.combats.push(NavalCombatRow {
                id,
                province,
                attacker_positioning: i64::from(hour),
                ..Default::default()
            });
            out.details
                .push(detail(id, detail_province, hour, [GER, ENG]));
            participants.extend(rows);
        };
    if (100..=110).contains(&hour) {
        combat(
            501,
            3001,
            3001,
            vec![
                participant(501, 0, 0, 7001, GER, if hour >= 104 { 0x20 } else { 0 }),
                participant(501, 0, 1, 0, GER, 0),
                participant(501, 1, 0, 7002, ENG, 0),
            ],
        );
    }
    if (106..=114).contains(&hour) {
        let province = if hour >= 112 { 3003 } else { 3002 };
        combat(
            502,
            province,
            if hour == 112 { 3002 } else { province },
            vec![
                participant(502, 0, 0, 7003, ENG, 0),
                participant(502, 1, 0, 7004, GER, if hour >= 113 { 0x20 } else { 0 }),
            ],
        );
    }
    if (120..=126).contains(&hour) {
        let mut rows = vec![
            participant(503, 0, 0, 7005, GER, 0),
            participant(503, 1, 0, 7006, ENG, 0),
        ];
        if hour >= 123 {
            rows.push(participant(503, 1, 1, 7007, 99, 0));
        }
        combat(503, 3004, 3004, rows);
    }
    participants.sort_by_key(RowCodec::key);
    out.participants = if hour == 108 {
        Rows::Unavailable("participants not read")
    } else {
        Rows::Whole(participants)
    };
    if (111..=147).contains(&hour) {
        result_rows(
            &mut out,
            801,
            (3001, 110, 1),
            &[(7001, 0, 1), (7002, 1, 0)],
            [GER, ENG],
        );
    }
    if (115..=129).contains(&hour) {
        result_rows(
            &mut out,
            802,
            (3003, 114, 2),
            &[(7003, 0, u64::from(hour >= 121)), (7004, 1, 1)],
            [ENG, GER],
        );
    }
    if (127..=132).contains(&hour) {
        // Dated after its read at first: that read is refused.
        let result_hour = if hour == 127 { 128 } else { 127 };
        result_rows(
            &mut out,
            803,
            (3005, result_hour, 1),
            &[(7005, 0, 0)],
            [GER, ENG],
        );
    }
    let source = |id: u64, game_hour: i32, result: u64, combat: u64| NavalResultSourceRow {
        id,
        game_hour,
        result,
        combat,
        ..Default::default()
    };
    out.sources = match hour {
        111 => vec![source(1, 110, 801, 501)],
        115 => vec![source(2, 114, 802, 502)],
        127 => vec![NavalResultSourceRow {
            merged: true,
            transient: true,
            combat_created_hour: Some(127),
            ..source(3, 127, 803, 0xdead)
        }],
        // Repeats the last identity: refused.
        128 => vec![source(3, 128, 803, 503)],
        _ => Vec::new(),
    };
    let alpha = entry(b"Alpha", 501, 3001, 103, ENG);
    let beta = entry(b"Beta", 502, 3002, 108, GER);
    let late = entry(b"Late", 0, 0, 118, ENG);
    let gamma = entry(b"Gamma", 503, 3004, 120, 99);
    let entries = match hour {
        ..104 => vec![],
        104..=108 => vec![alpha],
        109..=112 => vec![alpha.clone(), beta, alpha],
        113..=124 => vec![alpha.clone(), beta, alpha, late],
        _ => vec![beta, alpha, late, gamma],
    };
    out.ships = if hour == 119 {
        Rows::Unavailable("sunk ships not read")
    } else {
        Rows::Whole(vec![SunkShipsRow { entries }])
    };
    let convoys = |killer: u32, owner: u32, convoys: i32| SunkConvoyRow {
        month: 1000,
        killer_country: killer,
        owner_country: owner,
        convoys,
        ordinal: 0,
    };
    out.convoys.push(convoys(
        GER,
        ENG,
        match hour {
            ..105 => 2,
            105 => 3,
            // Changed at an hour that does not read them.
            106..=111 => 4,
            _ => 6,
        },
    ));
    if hour >= 120 {
        out.convoys.push(convoys(ENG, GER, 2));
    }
    out.convoys.sort_by_key(RowCodec::key);
    if hour >= 109 {
        out.tokens.push(LexicalTokenRow {
            token: 2,
            literal: b"cruiser".to_vec(),
        });
    }
    out
}

/// The hours a scenario records.
type Tables = fn(i32) -> HourTables;

/// The decoded frame of `hour` of `branch` of `tables`, as the recording keeps it.
fn frame(tables: Tables, branch: u64, hour: i32) -> DecodedFrame {
    let recorded = tables(hour);
    let mut frame = dashboard_core::test_fixtures::source(branch, hour, Some(1));
    let mut set = |table: TableId, view: TableView| *frame.tables[table.index()] = view;
    fn view<R>(rows: Rows<R>, wrap: impl Fn(Vec<R>) -> TableRows) -> TableView {
        match rows {
            Rows::Whole(rows) => TableView::Complete(wrap(rows)),
            Rows::Partial(rows, missing) => TableView::Truncated {
                rows: wrap(rows),
                missing,
            },
            Rows::Unavailable(reason) => TableView::Unavailable {
                reason: reason.into(),
            },
        }
    }
    set(
        TableId::Campaign,
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.to_vec(),
        }])),
    );
    set(
        TableId::Countries,
        view(recorded.countries, TableRows::Countries),
    );
    set(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(recorded.links)),
    );
    set(
        TableId::NavalCombats,
        TableView::Complete(TableRows::NavalCombats(recorded.combats)),
    );
    set(
        TableId::NavalCombatParticipants,
        view(recorded.participants, TableRows::NavalCombatParticipants),
    );
    set(
        TableId::NavalCombatDetails,
        TableView::Complete(TableRows::NavalCombatDetails(recorded.details)),
    );
    set(
        TableId::NavalResults,
        TableView::Complete(TableRows::NavalResults(recorded.results)),
    );
    set(
        TableId::NavalResultMembers,
        view(recorded.members, TableRows::NavalResultMembers),
    );
    set(
        TableId::NavalResultCountries,
        TableView::Complete(TableRows::NavalResultCountries(recorded.result_countries)),
    );
    set(
        TableId::NavalCombatResultDetails,
        TableView::Complete(TableRows::NavalCombatResultDetails(recorded.result_details)),
    );
    set(
        TableId::NavalResultSources,
        TableView::Complete(TableRows::NavalResultSources(recorded.sources)),
    );
    set(
        TableId::TaskForces,
        TableView::Complete(TableRows::TaskForces(recorded.task_forces)),
    );
    set(
        TableId::Fleets,
        TableView::Complete(TableRows::Fleets(recorded.fleets)),
    );
    set(
        TableId::SunkShips,
        view(recorded.ships, TableRows::SunkShips),
    );
    set(
        TableId::SunkConvoys,
        TableView::Complete(TableRows::SunkConvoys(recorded.convoys)),
    );
    set(
        TableId::LexicalTokens,
        TableView::Complete(TableRows::LexicalTokens(recorded.tokens)),
    );
    for table in PREPARED_TABLES {
        if *table != TableId::SunkConvoys || recorded.convoys_read {
            frame.metrics.table_ns[table.index()] = 1;
        }
    }
    frame
}

/// Writes `frame`'s table `T` into the walk as the native recorder writes it.
fn write<T: replay_data::rows::TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = if read {
        Sample::FromReadBit
    } else {
        Sample::Carried(frame.hour - 1)
    };
    let input = match &*frame.tables[T::ID.index()] {
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

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame) {
    macro_rules! tables { ($($table:ident),* $(,)?) => { $(write::<tables::$table>(walk, frame);)* }; }
    tables!(
        Campaign,
        Countries,
        CountryLinks,
        NavalCombats,
        NavalCombatParticipants,
        NavalCombatDetails,
        NavalResults,
        NavalResultMembers,
        NavalResultCountries,
        NavalCombatResultDetails,
        NavalResultSources,
        TaskForces,
        Fleets,
        SunkShips,
        SunkConvoys,
        LexicalTokens,
    );
}

/// The recording of `hours`, in row blocks of [`BLOCK`] hours ending at gaps and branches.
fn recording(tables: Tables, hours: &[(u64, i32)]) -> Source {
    let mut blocks: Vec<(u64, Vec<i32>)> = Vec::new();
    for &(branch, hour) in hours {
        match blocks.last_mut() {
            Some((last, block))
                if *last == branch
                    && block.len() < BLOCK
                    && block.last().is_some_and(|previous| previous + 1 == hour) =>
            {
                block.push(hour)
            }
            _ => blocks.push((branch, vec![hour])),
        }
    }
    let mut replay: Option<Replay> = None;
    for (branch, block) in blocks {
        let frames = block
            .iter()
            .map(|&hour| frame(tables, branch, hour))
            .collect();
        replay = Some(match replay {
            None => Replay::from_records(CAMPAIGN.into(), &[records(branch, frames)]).unwrap(),
            Some(replay) if replay.branches.iter().any(|start| start.id == branch) => replay
                .append(&[continuation_records(&replay, branch, frames)])
                .unwrap(),
            Some(replay) => replay.append(&[records(branch, frames)]).unwrap(),
        });
    }
    Source::from_replay(
        SourceKey {
            source_id: "naval-columnar-parity".into(),
            revision: "1".into(),
        },
        "naval-columnar-parity.battleplan".into(),
        Arc::new(replay.unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Each path's published sections: the row path's, then the projected walk's.
struct Published {
    row: Fixture,
    projected: Fixture,
}

/// A columnar walk: its hours, segment starts, whether parked projectors spill, spills inside,
/// and whether its scans are saved and loaded again where a segment ends, as a native branch
/// is after each batch.
struct Walk<'a> {
    tables: Tables,
    hours: &'a [(u64, i32)],
    restart: &'a dyn Fn(usize) -> bool,
    spill: bool,
    inside: &'a dyn Fn(usize) -> bool,
    reload: bool,
}

impl<'a> Walk<'a> {
    fn campaign(hours: &'a [(u64, i32)], restart: &'a dyn Fn(usize) -> bool) -> Self {
        Self {
            tables: hour_tables,
            hours,
            restart,
            spill: false,
            inside: &never,
            reload: false,
        }
    }
}

fn never(_: usize) -> bool {
    false
}

/// `projector` as the run's store takes it up after spilling it.
fn spilled<'a>(projector: Projector<'a>, make: &dyn Fn() -> Projector<'a>) -> Projector<'a> {
    assert!(
        projector.carried().is_some(),
        "a parked projector names its bytes"
    );
    let mut bytes = Vec::new();
    assert!(
        projector.save(&mut bytes).unwrap(),
        "a parked projector is spilled"
    );
    let mut loaded = make();
    loaded.load(&bytes).unwrap();
    loaded
}

/// Publishes `walk` by row blocks and by its segments, handing each projected hour to `seen`.
fn parity_seen(label: &str, walk: Walk<'_>, seen: &mut dyn FnMut(&columnar::Hour)) -> Published {
    let hours = walk.hours;
    let source = Arc::new(recording(walk.tables, hours));
    let context = Arc::new(LoadContext { content: None });
    let content = ships_sunk::prepared_content(&source, &context);
    let replay = source.replay_ref();
    let flag = AtomicBool::new(false);
    let (mut ends, mut firsts) = (BTreeMap::new(), BTreeSet::new());
    let mut ordinals = BTreeMap::<u64, usize>::new();
    for block in &replay.index {
        let ordinal = ordinals.entry(block.branch).or_default();
        ends.insert((block.branch, block.last_hour), *ordinal);
        firsts.insert((block.branch, block.first_hour));
        *ordinal += 1;
    }
    let published = Published {
        row: Fixture::new(),
        projected: Fixture::new(),
    };
    let mut writers = (published.row.writer(), published.projected.writer());
    let mut rows: BTreeMap<u64, Scan> = BTreeMap::new();
    let mut projected: BTreeMap<u64, Scan> = BTreeMap::new();
    let mut tracker = FrameChangeTracker::new();
    let make = || {
        Projector::new(
            replay.format,
            replay.capture_mode,
            &source,
            &context,
            &content,
        )
        .unwrap()
    };
    let mut emit = |items: &mut Vec<columnar::Hour>, writer: &mut store::Writer, reload: bool| {
        for hour in std::mem::take(items) {
            seen(&hour);
            assert!(hour.heap() < 1 << 30, "{label}: an hour names its bytes");
            let (branch, at) = (hour.branch, hour.hour);
            let scan = projected.entry(branch).or_default();
            scan.configure(&source, branch, &content).unwrap();
            scan.observe_projected(hour, &source, &flag).unwrap();
            if let Some(&ordinal) = ends.get(&(branch, at)) {
                scan.publish_configured(replay, writer, branch, ordinal, &flag)
                    .unwrap();
            }
        }
        if reload {
            // As a native branch's cut and save leave it: its histories settled, then reloaded.
            for scan in projected.values_mut() {
                if let Some(last) = scan.last_hour {
                    scan.settle(last);
                }
                *scan = serde_json::from_slice(&serde_json::to_vec(&*scan).unwrap()).unwrap();
            }
        }
    };
    let mut items: Vec<columnar::Hour> = Vec::new();
    let mut synthetic: Option<Synthetic> = None;
    let mut world_held: Option<dashboard_world::World> = None;
    let mut walking: Option<Projector> = None;
    for (ordinal, &(branch, hour)) in hours.iter().enumerate() {
        let previous = ordinal.checked_sub(1).map(|at| hours[at]);
        let new_branch = previous.is_none_or(|(last, _)| last != branch);
        let run = !new_branch && previous.is_some_and(|(_, last)| last + 1 != hour);
        let opens = new_branch || run || (walk.restart)(ordinal);
        let inside = !opens && (walk.inside)(ordinal);
        if (opens || inside)
            && let Some(done) = walking.as_mut()
        {
            done.end(&mut items).unwrap();
            emit(&mut items, &mut writers.1, walk.reload && opens);
        }
        let carry = |going| {
            if walk.spill {
                spilled(going, &make)
            } else {
                going
            }
        };
        let mut fresh = false;
        if new_branch || run {
            synthetic = Some(Synthetic::new(columnar::tables(), branch, hour).unwrap());
            let start = Start::Run { continues: run };
            match walking.take().filter(|_| run) {
                // A later run's walk goes on from the run before, its tables empty.
                Some(going) if !going.starts_at(start) => {
                    let mut going = carry(going);
                    going.cross(hour).unwrap();
                    walking = Some(going);
                }
                _ => {
                    let mut started = make();
                    assert!(started.starts_at(start), "{label}: a branch's first run");
                    started.start(start, branch, hour).unwrap();
                    walking = Some(started);
                    fresh = true;
                }
            }
        } else if opens {
            let mut started = make();
            let state = synthetic.as_ref().unwrap().state();
            if started.starts_at(Start::Restart(state)) {
                started.start(Start::Restart(state), branch, hour).unwrap();
                walking = Some(started);
                fresh = true;
            } else {
                walking = walking.take().map(carry);
            }
        } else if inside {
            walking = walking.take().map(|going| spilled(going, &make));
        }
        if ordinal == 0 || firsts.contains(&(branch, hour)) {
            // A row block is read alone, with a decoder and change facts of its own.
            tracker.invalidate();
            rows.entry(branch).or_default().forget_views();
        }
        let synthetic = synthetic.as_mut().unwrap();
        let input = frame(walk.tables, branch, hour);
        write_hour(synthetic, &input);
        let projector = walking.as_mut().unwrap();
        synthetic
            .end(|state, manifest, frame, starts| {
                let world = dashboard_world::World::at(
                    &mut world_held,
                    starts,
                    &columnar::needs(),
                    (
                        replay_data::format::FormatVersion::CURRENT,
                        replay_data::scope::CaptureMode::Full,
                    ),
                    state,
                    manifest,
                )
                .unwrap();
                let scan = rows.entry(branch).or_default();
                scan.configure(&source, branch, &content).unwrap();
                let changes = tracker.observe(frame, replay.format);
                scan.observe_prepared(frame, &changes);
                scan.observe_details_read(&source, &context, frame, &flag, &changes, true)
                    .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch,
                            hour,
                            run: 0,
                            ordinal: ordinal as u32,
                            starts: starts || fresh,
                            opens,
                            state,
                            manifest,
                            world,
                        },
                        &mut items,
                    )
                    .unwrap();
            })
            .unwrap();
        if let Some(&block) = ends.get(&(branch, hour)) {
            rows.get_mut(&branch)
                .unwrap()
                .publish_configured(replay, &mut writers.0, branch, block, &flag)
                .unwrap();
        }
    }
    if let Some(done) = walking.as_mut() {
        done.end(&mut items).unwrap();
        emit(&mut items, &mut writers.1, false);
    }
    drop(emit);
    writers.0.commit().unwrap();
    writers.1.commit().unwrap();
    let expected = published.row.payloads();
    for section in [
        "naval-battles-1",
        "naval-battle-ranking-1",
        "naval-battle-records-1",
        "naval-battle-windows-1",
        "naval-battle-reads-1",
        "naval-battle-fleets-1",
        "naval-battle-ship-reads-1",
        "ships-sunk-manifest-1",
    ] {
        assert!(
            expected.keys().any(|(name, _)| name == section),
            "{label}: the row path publishes {section}"
        );
    }
    assert_payloads_equal(&published.projected.payloads(), &expected, label);
    for (branch, scan) in &mut rows {
        let walked = projected.get_mut(branch).unwrap().losses().0;
        // A reloaded scan's losses handed what they wrote to its series before each save.
        let (row, walked) = if walk.reload {
            (scan.losses().0.state(), walked.state())
        } else {
            (scan.losses().0.pending(), walked.pending())
        };
        assert_eq!(walked, row, "{label}: branch {branch}'s naval losses");
    }
    published
}

fn parity(label: &str, walk: Walk<'_>) -> Published {
    parity_seen(label, walk, &mut |_| {})
}

/// The hours of branch 1 from 100 through 135.
fn campaign() -> Vec<(u64, i32)> {
    (100..=135).map(|hour| (1, hour)).collect()
}

/// A published reply of the projected walk.
fn reply<T: serde::de::DeserializeOwned>(published: &Published, section: &str, page: u64) -> T {
    series::read_reply(
        &published.projected.store,
        &published.projected.key,
        section,
        page,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn whole_walks_publish_what_block_reads_publish() {
    let hours = campaign();
    let published = parity("whole", Walk::campaign(&hours, &|_| false));
    for section in [
        "naval-battle-ships-1",
        "ships-sunk-1",
        "sinking-events-1",
        "sinking-titles-1",
    ] {
        assert!(
            published
                .row
                .payloads()
                .keys()
                .any(|(name, _)| name == section),
            "the campaign publishes {section}"
        );
    }
    // The first window shows the first battle sampled from its first hour.
    let first: NavalBattlesReply = reply(&published, "naval-battles-1", 0);
    assert_eq!(first.interval.first_hour, 100);
    assert!(first.rows.iter().any(|row| row.id == "combat:501:100"));
    // A province change starts the battle's second episode; its result follows the source.
    let window: NavalBattlesReply = reply(&published, "naval-battles-1", 2);
    assert!(window.rows.iter().any(|row| row.id == "combat:502:112"));
    assert!(window.rows.iter().any(|row| row.id == "result:801"));
    // The future-dated result read is refused; the result is shown from its next read.
    let late: NavalBattlesReply = reply(&published, "naval-battles-1", 4);
    assert!(late.warnings.iter().any(|warning| warning.hour == 127
        && warning.reason == "Invalid result selector or future date; read unavailable"));
    let refused = late
        .rows
        .iter()
        .find(|row| row.id == "result:803")
        .expect("the result read after its refused read");
    assert_eq!(refused.runs[0].first_hour, 128);
    // Two copies of one entry, one due late, one pruned and one new at the last change.
    let found: usize = (0..6)
        .map(|window| {
            series::read_reply::<ships_sunk::prepared::ShipsSunkWindowReply>(
                &published.projected.store,
                &published.projected.key,
                "ships-sunk-1",
                window,
            )
            .unwrap()
            .map_or(0, |page| page.indices.len())
        })
        .sum();
    assert_eq!(found, 5);
}

#[test]
fn every_hour_restarting_publishes_what_block_reads_publish() {
    let hours = campaign();
    parity("every hour", Walk::campaign(&hours, &|_| true));
}

#[test]
fn restarts_inside_blocks_publish_what_block_reads_publish() {
    let hours = campaign();
    for restarts in [
        &[1_usize, 4, 7, 11, 12, 19, 20, 25, 27, 28][..],
        &[3, 9, 15, 21, 27, 33],
        &[5, 13, 17, 18, 24, 29, 30, 31],
    ] {
        parity(
            &format!("restarts {restarts:?}"),
            Walk::campaign(&hours, &|at| restarts.contains(&at)),
        );
    }
}

#[test]
fn a_gap_carries_the_retained_results_and_sinkings_into_the_next_run() {
    let mut hours = campaign();
    hours.extend((140..=147).map(|hour| (1, hour)));
    let published = parity("gap", Walk::campaign(&hours, &|at| at % 5 == 0));
    // The result held through the gap starts a new run after it.
    let after: NavalBattlesReply = reply(&published, "naval-battles-1", 6);
    let held = after
        .rows
        .iter()
        .find(|row| row.id == "result:801")
        .expect("the retained result after the gap");
    assert_eq!(held.runs[0].first_hour, 140);
    parity("gap every hour", Walk::campaign(&hours, &|_| true));
}

#[test]
fn a_second_branch_starts_its_own_scan() {
    let mut hours: Vec<(u64, i32)> = (100..=113).map(|hour| (1, hour)).collect();
    hours.extend((104..=117).map(|hour| (2, hour)));
    parity("branches", Walk::campaign(&hours, &|at| at % 4 == 1));
    parity("branches every hour", Walk::campaign(&hours, &|_| true));
}

/// A scan saved and loaded again where each segment ends, as a native branch is after each
/// batch, publishes what the row path does.
#[test]
fn scans_reloaded_where_segments_end_publish_what_block_reads_publish() {
    let mut hours = campaign();
    hours.extend((140..=147).map(|hour| (1, hour)));
    for (label, restart) in [
        ("reloaded every hour", &(|_| true) as &dyn Fn(usize) -> bool),
        ("reloaded inside blocks", &|at| at % 7 == 3),
        ("reloaded at blocks", &|at| at % 6 == 0),
    ] {
        parity(
            label,
            Walk {
                tables: hour_tables,
                hours: &hours,
                restart,
                spill: true,
                inside: &never,
                reload: true,
            },
        );
    }
}

/// A projector spilled where it parks, and inside segments, goes on as it would have.
#[test]
fn spilled_projectors_go_on_as_they_would_have() {
    let mut hours = campaign();
    hours.extend((140..=147).map(|hour| (1, hour)));
    let spill = |label: &str, restart: &dyn Fn(usize) -> bool, inside: &dyn Fn(usize) -> bool| {
        parity(
            label,
            Walk {
                tables: hour_tables,
                hours: &hours,
                restart,
                spill: true,
                inside,
                reload: false,
            },
        )
    };
    let mut parked = 0;
    let counted = parity_seen(
        "parked",
        Walk {
            tables: hour_tables,
            hours: &hours,
            restart: &|at| at == 20,
            spill: true,
            inside: &|_| false,
            reload: false,
        },
        &mut |hour| parked += usize::from(hour.hour == 119 && hour.base.is_some()),
    );
    drop(counted);
    // Hour 119 ends the segment the walk cannot start after: it hands the read on.
    assert_eq!(parked, 1);
    spill("spilled every hour", &|_| true, &|_| false);
    spill("spilled inside", &|at| at % 9 == 0, &|at| at % 4 == 2);
    spill("spilled at every hour inside", &|_| false, &|_| true);
}

/// 1,500 countries whose balances move every hour, two fighting; one renamed at hour 205.
fn crowded_tables(hour: i32) -> HourTables {
    let mut out = hour_tables(100);
    out.countries = Rows::Whole(
        (0..CROWD)
            .map(|at| CountryRow {
                id: 20_000 + at,
                canonical_slot: at,
                tag: if at == 1 && hour >= 205 {
                    b"REN".to_vec()
                } else {
                    format!("C{at:04}").into_bytes()
                },
                political_power_raw: Some(i64::from(hour) * i64::from(at)),
                stability_raw: i64::from(hour),
                ..Default::default()
            })
            .collect(),
    );
    out.links = (0..CROWD)
        .flat_map(|at| [(30_000 + at, at), (20_000 + at, at)])
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect();
    out.links.sort_by_key(|row| row.id);
    let (first, second) = (30_000, 30_001);
    out.combats = vec![NavalCombatRow {
        id: 601,
        province: 3001,
        attacker_positioning: i64::from(hour),
        ..Default::default()
    }];
    out.details = vec![detail(601, 3001, hour, [first, second])];
    out.participants = Rows::Whole(vec![
        participant(601, 0, 0, 7101, first, 0),
        participant(601, 1, 0, 7102, second, u32::from(hour >= 207) * 0x20),
    ]);
    out
}

/// The countries [`crowded_tables`] holds.
const CROWD: u32 = 1_500;

/// The countries are read again only where a slot or tag changes.
#[test]
fn fifteen_hundred_countries_are_read_again_only_where_a_tag_changes() {
    let hours: Vec<(u64, i32)> = (200..=210).map(|hour| (1, hour)).collect();
    let mut read = Vec::new();
    parity_seen(
        "crowded",
        Walk {
            tables: crowded_tables,
            hours: &hours,
            restart: &|at| at == 8,
            spill: false,
            inside: &|_| false,
            reload: false,
        },
        &mut |hour| {
            if matches!(hour.countries, columnar::Countries::Read(_)) {
                read.push(hour.hour);
            }
        },
    );
    // The walk's first hour, the rename, and the segment starting at hour 208.
    assert_eq!(read, [200, 205, 208]);
}

/// What a whole read's comparison finds and newly refuses, as the row path's does.
fn compared(
    before: &[SunkShipEntry],
    before_hour: i32,
    entries: &[SunkShipEntry],
    hour: i32,
) -> (Vec<SunkShipEntry>, BTreeMap<String, usize>) {
    let refusal = |entry: &SunkShipEntry, hour: i32| {
        if entry.sink_hour > hour {
            Some("SunkShips event hour is after its read")
        } else if entry.cause_record > 1 || entry.assisted_kill > 1 {
            Some("SunkShips boolean flag is above 1")
        } else {
            None
        }
    };
    let count = |entries: &[SunkShipEntry], at: i32, refused: bool| {
        let mut counts = Vec::<(SunkShipEntry, usize)>::new();
        for entry in entries
            .iter()
            .filter(|entry| refusal(entry, at).is_some() == refused)
        {
            match counts.iter_mut().find(|(held, _)| held == entry) {
                Some((_, count)) => *count += 1,
                None => counts.push((entry.clone(), 1)),
            }
        }
        counts
    };
    let held = |counts: &[(SunkShipEntry, usize)], entry: &SunkShipEntry| {
        counts
            .iter()
            .find(|(held, _)| held == entry)
            .map_or(0, |(_, count)| *count)
    };
    let (taken, refused) = (
        count(before, before_hour, false),
        count(before, before_hour, true),
    );
    let mut found = Vec::new();
    for (entry, now) in count(entries, hour, false) {
        for _ in held(&taken, &entry)..now {
            found.push(entry.clone());
        }
    }
    let mut added = BTreeMap::new();
    for (entry, now) in count(entries, hour, true) {
        let more = now.saturating_sub(held(&refused, &entry));
        if more > 0 {
            *added
                .entry(refusal(&entry, hour).unwrap().to_owned())
                .or_default() += more;
        }
    }
    (found, added)
}

/// Swap-erased reads with duplicates, future dates and bad flags: positional compares are whole.
#[test]
fn reads_compared_by_their_changed_positions_find_what_whole_reads_do() {
    let mut seed = 0x9e37_79b9_u64;
    let mut next = |below: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % below
    };
    let mut base = sinkings::ShipBase::default();
    let mut held: Vec<SunkShipEntry> = Vec::new();
    let (mut before, mut before_hour) = (Vec::new(), None);
    let mut found_any = 0;
    for read in 0..400 {
        let hour = 1_000 + read * 24;
        for _ in 0..next(4) {
            let name = format!("ship {}", next(40));
            let sink_hour = if next(10) == 0 {
                hour + 1 + next(60) as i32
            } else {
                hour - next(100) as i32
            };
            held.push(SunkShipEntry {
                ship_name: name.into_bytes(),
                class_token: next(3) as u32,
                sink_hour,
                assisted_kill: u8::from(next(25) == 0) * 2,
                ..Default::default()
            });
        }
        for _ in 0..next(3) {
            if !held.is_empty() {
                let at = next(held.len() as u64) as usize;
                held.swap_remove(at);
            }
        }
        // A parked walk's base, spilled and taken up again.
        if read % 37 == 36 {
            base = sinkings::ShipBase::restored(base.saved());
        }
        let (expected, refusals) = compared(&before, before_hour.unwrap_or(hour), &held, hour);
        let (found, refused) = base.compare(held.clone(), hour);
        assert_eq!(found, expected, "read {read}");
        assert_eq!(
            refused.into_iter().collect::<BTreeMap<_, _>>(),
            refusals,
            "read {read}"
        );
        found_any += found.len();
        (before, before_hour) = (held.clone(), Some(hour));
    }
    assert!(found_any > 100, "the reads find ships");
}
