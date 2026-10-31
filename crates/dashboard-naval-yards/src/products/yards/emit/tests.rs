//! The publication of synthetic walks: what it holds of every owner at every hour is what the
//! recording gives there observed whole, and it is the same publication however the walk is cut
//! into segments, published and resumed.

use super::*;
use crate::{
    products::yards::{
        layout::{line, port, ship},
        management::TABLES,
        segment::{Refits, Repairs},
    },
    tests::rows::{Canon, Rows, whole},
};
use dashboard_keyed::segment as keyed;
use dashboard_series::{
    Budget, Cache, EventOrder, EventQuery, Identity, Publication, Selection, Snapshot,
    codec::Binary,
    memory::{Memory, publish},
    query::EventView,
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator as _},
};
use replay_data::{rows::*, scope::CaptureMode};
use replay_frame::frame::TableView;
use std::collections::BTreeSet;

pub(crate) const SHIP: u64 = (9_u64 << 32) | 61;
const SECOND: u64 = (9_u64 << 32) | 62;
const TASK: u64 = (3_u64 << 32) | 61;
const LINE: u64 = (5_u64 << 32) | 71;
const COUNTRY: &str = "MOD@7";

/// What a scenario hour holds of the yard frame.
#[derive(Clone, Copy)]
struct At {
    /// The first ship's hull and its refit's progress.
    strength: i64,
    progress: i64,
    /// Whether the repair queue is recorded, and the queue and refit tables read.
    queue: bool,
    read: bool,
    /// A second ship of the same task force queued and refitted, its design and maximum hull
    /// not recorded: two refusals its owner first names at once.
    second: bool,
    /// The ships' class token spelled, and the port's state named by its owner: naming tables
    /// the changes engine does not follow.
    spelled: bool,
    named: bool,
    /// Countries not recorded: no owner resolves.
    refused: bool,
    /// The first ship named by its naming operands rather than its cached name.
    rebuilt: bool,
}

const BASE: At = At {
    strength: 120_000,
    progress: 1_000,
    queue: true,
    read: true,
    second: false,
    spelled: false,
    named: false,
    refused: false,
    rebuilt: false,
};

/// One country's ships in a port with a repair queue and a refit line, as `at` holds them.
fn yard_frame(hour: i32, at: At) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    for &table in TABLES {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let set = |frame: &mut DecodedFrame, table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    if at.refused {
        *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "Synthetic countries refusal".into(),
        };
    } else {
        set(
            &mut frame,
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: 11,
                canonical_slot: 7,
                tag: b"MOD".to_vec(),
                ..Default::default()
            }]),
        );
    }
    set(
        &mut frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        }]),
    );
    set(
        &mut frame,
        TableId::CountryAppearance,
        TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            ..Default::default()
        }]),
    );
    if at.spelled {
        set(
            &mut frame,
            TableId::LexicalTokens,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0,
                literal: b"mod_class".to_vec(),
            }]),
        );
    }
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(vec![TaskForceRow {
            id: TASK,
            roster_owner: 11,
            ..Default::default()
        }]),
    );
    let mut ships = vec![ShipRow {
        id: SHIP,
        task_force: TASK,
        strength: at.strength,
        name: Recorded::Value((!at.rebuilt).then(|| b"Cached".to_vec())),
        naming: Recorded::Value(at.rebuilt.then(|| ShipNamingOperands {
            kind: 1,
            order: 2,
            ordered: 1,
            ..Default::default()
        })),
        ..Default::default()
    }];
    let mut models = vec![ShipModelRow {
        ship: SHIP,
        counter: Some(ShipCounterInputs {
            maximum_strength: 200_000,
            ..Default::default()
        }),
        ..Default::default()
    }];
    let mut queue = vec![
        NavalRepairQueueRow {
            province: 9,
            ordinal: 0,
            ship: SHIP,
        },
        NavalRepairQueueRow {
            province: 9,
            ordinal: 1,
            ship: SHIP,
        },
    ];
    let mut refitted = vec![NavalRefitShipRow {
        ship: SHIP,
        line: LINE,
    }];
    if at.second {
        ships.push(ShipRow {
            id: SECOND,
            task_force: TASK,
            strength: 50_000,
            ..Default::default()
        });
        models.push(ShipModelRow {
            ship: SECOND,
            variant: 77,
            ..Default::default()
        });
        queue.push(NavalRepairQueueRow {
            province: 9,
            ordinal: 2,
            ship: SECOND,
        });
        refitted.push(NavalRefitShipRow {
            ship: SECOND,
            line: LINE,
        });
    }
    set(&mut frame, TableId::Ships, TableRows::Ships(ships));
    set(&mut frame, TableId::ShipModels, TableRows::ShipModels(models));
    set(
        &mut frame,
        TableId::NavalBases,
        TableRows::NavalBases(vec![NavalBaseRow {
            province: 9,
            country: 11,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::NavalRepairBases,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 9,
            priority: -3,
            allocated_dockyards: 4,
        }]),
    );
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(vec![ProvinceRow {
            id: 9,
            state: 1,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::States,
        TableRows::States(vec![StateRow {
            id: 1,
            owner: 11,
            custom_name: if at.named {
                b"Harbour".to_vec()
            } else {
                Vec::new()
            },
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::NavalRefitShips,
        TableRows::NavalRefitShips(refitted),
    );
    set(
        &mut frame,
        TableId::NavalRefitProgress,
        TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
            line: LINE,
            progress_raw: at.progress,
        }]),
    );
    set(
        &mut frame,
        TableId::ProductionLines,
        TableRows::ProductionLines(vec![ProductionLineRow {
            handle: LINE,
            country: 11,
            cost_raw: 1_000_000,
            active_factories: 3,
            ..Default::default()
        }]),
    );
    if at.queue {
        set(
            &mut frame,
            TableId::NavalRepairQueue,
            TableRows::NavalRepairQueue(queue),
        );
    } else {
        *frame.tables[TableId::NavalRepairQueue.index()] = TableView::Unavailable {
            reason: "Synthetic repair queue refusal".into(),
        };
    }
    for table in [
        TableId::NavalRepairQueue,
        TableId::NavalRefitShips,
        TableId::NavalRefitProgress,
    ] {
        frame.metrics.table_ns[table.index()] = u64::from(at.read);
    }
    frame
}

/// The hours of the scenario, from 100: a hull repaired over hours read and unread; the queue
/// refused at an unread hour nothing else changes, quiet while refused, then recorded again;
/// the lines' daily read at 104 and the designs' at 118 with nothing changed; a second ship
/// arriving whose owner first names two refusals at once, leaving and coming back; the class
/// token spelled and the port's state named, which the changes engine does not follow; an hour
/// whose countries are not recorded, then recorded again on the same rows; the first ship's name
/// rebuilt for two hours, then cached again; a refit's progress; quiet hours between; countries
/// not recorded again, then recorded on rows changed.
pub(crate) fn scenario() -> Vec<DecodedFrame> {
    let unread = At { read: false, ..BASE };
    let repaired = At {
        strength: 150_000,
        ..BASE
    };
    let refused = At {
        queue: false,
        read: false,
        ..repaired
    };
    let progressed = At {
        strength: 180_000,
        progress: 2_000,
        ..BASE
    };
    let second = At {
        second: true,
        read: false,
        ..progressed
    };
    let spelled = At {
        spelled: true,
        ..second
    };
    let named = At {
        named: true,
        ..spelled
    };
    let left = At {
        second: false,
        ..named
    };
    let back = At {
        second: true,
        ..left
    };
    let later = At {
        strength: 190_000,
        progress: 3_000,
        ..back
    };
    let hours = [
        BASE,
        unread,
        repaired,
        refused,
        refused,
        At {
            read: false,
            ..repaired
        },
        At {
            read: false,
            ..repaired
        },
        progressed,
        second,
        second,
        spelled,
        named,
        left,
        At {
            refused: true,
            ..left
        },
        left,
        back,
        At {
            strength: 190_000,
            rebuilt: true,
            ..back
        },
        At {
            strength: 190_000,
            rebuilt: true,
            ..back
        },
        At {
            strength: 190_000,
            ..back
        },
        At {
            read: true,
            strength: 190_000,
            ..back
        },
        later,
        later,
        At {
            refused: true,
            ..later
        },
        At {
            strength: 200_000,
            progress: 4_000,
            ..later
        },
    ];
    hours
        .iter()
        .enumerate()
        .map(|(offset, &at)| yard_frame(100 + offset as i32, at))
        .collect()
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
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
    write::<tables::Campaign>(walk, frame);
    write::<tables::Countries>(walk, frame);
    write::<tables::CountryLinks>(walk, frame);
    write::<tables::CountryAppearance>(walk, frame);
    write::<tables::ReservedCountryNaming>(walk, frame);
    write::<tables::LexicalTokens>(walk, frame);
    write::<tables::TaskForceCounterInputs>(walk, frame);
    write::<tables::VariantDefinitions>(walk, frame);
    write::<tables::NavalRepairCountries>(walk, frame);
    write::<tables::CountryFactories>(walk, frame);
    write::<tables::NavalBases>(walk, frame);
    write::<tables::NavalRepairBases>(walk, frame);
    write::<tables::NavalRepairQueue>(walk, frame);
    write::<tables::NavalRefitShips>(walk, frame);
    write::<tables::NavalRefitProgress>(walk, frame);
    write::<tables::ProductionLines>(walk, frame);
    write::<tables::Ships>(walk, frame);
    write::<tables::ShipModels>(walk, frame);
    write::<tables::TaskForces>(walk, frame);
    write::<tables::NavalRepairTaskForces>(walk, frame);
    write::<tables::NavalRepairChildren>(walk, frame);
    write::<tables::Provinces>(walk, frame);
    write::<tables::ProvinceNames>(walk, frame);
    write::<tables::States>(walk, frame);
}

/// The hours `frames` give `V`'s operators, a fresh operator starting a segment at each of
/// `restarts` (ordinals).
pub(crate) fn project<V: View>(frames: &[DecodedFrame], restarts: &BTreeSet<usize>) -> Vec<Hour<V>> {
    let format = FormatVersion::CURRENT;
    let mut walk = Synthetic::new(V::TABLES, 1, frames[0].hour).unwrap();
    let mut world = None;
    let needs = keyed::needs::<V>().unwrap();
    let mut operator = keyed::Operator::<V>::new(format).unwrap();
    let (mut hours, mut first) = (Vec::new(), 0);
    for (ordinal, frame) in frames.iter().enumerate() {
        if ordinal > 0 && restarts.contains(&ordinal) {
            operator = keyed::Operator::new(format).unwrap();
            first = ordinal;
        }
        write_hour(&mut walk, frame);
        walk.end(|state, manifest, decoded, starts| {
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &needs,
                (format, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            operator
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
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
    hours
}

pub(crate) fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: SCHEMA,
    }
}

/// A publication of `hours`, cut after each of `cuts`' hours; at each of `resume`'s, the emit
/// is saved and goes on from the publication, as a preparation of a grown recording does.
pub(crate) fn build<V: View>(hours: Vec<Hour<V>>, cuts: &[i32], resume: &[i32]) -> (Memory, u64) {
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(&budget, identity(), first).unwrap();
    let mut emit = Emit::<V>::new(FormatVersion::CURRENT);
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
            emit = Emit::load(FormatVersion::CURRENT, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

/// A cell read whole: a text or list by what it holds.
fn read(snapshot: &mut Snapshot<'_, Memory>, cell: Cell) -> Canon {
    let bytes = |snapshot: &mut Snapshot<'_, Memory>, blob: Blob| {
        let mut bytes = vec![0; blob.len as usize];
        if blob.len != 0 {
            snapshot.cell_bytes(blob, 0, &mut bytes).unwrap();
        }
        bytes
    };
    match cell {
        Cell::Null => Canon::Null,
        Cell::Integer(value) => Canon::Integer(value),
        Cell::Unsigned(value) => Canon::Unsigned(value),
        Cell::Boolean(value) => Canon::Unsigned(u64::from(value)),
        Cell::Text(blob) => Canon::Token(String::from_utf8(bytes(snapshot, blob)).unwrap()),
        Cell::List(blob) => {
            let bytes = bytes(snapshot, blob);
            Canon::List(
                bytes
                    .chunks_exact(Cell::SIZE)
                    .map(|chunk| read(snapshot, Cell::decode(&mut Decode::new(chunk)).unwrap()))
                    .collect(),
            )
        }
        Cell::Number(_) => panic!("the yards write no number"),
    }
}

#[derive(Debug, PartialEq)]
struct Decoded {
    hour: i32,
    read: Option<i32>,
    kind: u64,
    cells: Vec<Canon>,
}

/// Every event of `key`, in order, its fields read whole.
fn events(snapshot: &mut Snapshot<'_, Memory>, key: KeyHandle, span: Span) -> Vec<Decoded> {
    let mut raw = Vec::new();
    snapshot
        .each_event(
            EventQuery {
                selection: &[Selection {
                    key,
                    membership: None,
                }],
                native: span,
                as_of: None,
                order: EventOrder::Native,
            },
            1 << 20,
            |event| {
                let EventView::Family(event) = event else {
                    panic!("the yards present no evidence");
                };
                raw.push(event);
                Ok(())
            },
        )
        .unwrap();
    raw.sort_by_key(|event| (event.native_hour, event.sequence));
    raw.into_iter()
        .map(|event| {
            let mut bytes = vec![0; event.fields.len as usize];
            if event.fields.len != 0 {
                snapshot.cell_bytes(event.fields, 0, &mut bytes).unwrap();
            }
            let cells = bytes
                .chunks_exact(Cell::SIZE)
                .map(|chunk| Cell::decode(&mut Decode::new(chunk)).unwrap())
                .collect::<Vec<_>>();
            Decoded {
                hour: event.native_hour,
                read: event.read_hour,
                kind: event.kind,
                cells: cells.into_iter().map(|cell| read(snapshot, cell)).collect(),
            }
        })
        .collect()
}

fn evidence(snapshot: &mut Snapshot<'_, Memory>, key: KeyHandle, span: Span) -> Vec<Evidence> {
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .evidence_page(key, span, cursor, 1 << 16, |row| {
                rows.push(row);
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            return rows;
        }
    }
}

fn runs(snapshot: &mut Snapshot<'_, Memory>, key: KeyHandle, span: Span) -> Vec<(Span, i128)> {
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .run_page(
                Selection {
                    key,
                    membership: None,
                },
                span,
                cursor,
                1 << 16,
                |run| {
                    rows.push((run.span, run.value));
                    Ok(())
                },
            )
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            return rows;
        }
    }
}

/// The cells of a stream's rows that name tokens.
fn labels(stream: u64, refits: bool) -> &'static [usize] {
    match stream {
        layout::HOLDERS if refits => &[line::TARGET_NAME],
        layout::HOLDERS => &[port::NAME, port::POLICY],
        layout::SHIPS => &[
            ship::DESIGN_NAME,
            ship::CLASS,
            ship::TASK_NAME,
            ship::POLICY,
            ship::REFIT_INTENT_NAME,
            ship::AFTER_REPAIR_INTENT_NAME,
            ship::NAME,
            ship::NAME_REBUILT,
        ],
        _ => &[],
    }
}

/// The publication read back.
struct Published {
    /// What it holds at every walked hour; none where its owners do not resolve.
    states: BTreeMap<i32, Option<Rows>>,
    /// Every key's events, evidence and runs, by its place.
    dump: BTreeMap<(u64, u64, Option<u64>), (Vec<Decoded>, Vec<Evidence>, Vec<(Span, i128)>)>,
    /// Each token's spelling, by its number.
    tokens: Vec<String>,
}

/// What the publication at `root` holds of `V` at every one of `hours`.
fn published<V: View>(pages: &Memory, root: u64, hours: &[i32]) -> Published {
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let span = Span::new(hours[0], *hours.last().unwrap()).unwrap();
    let mut places = Vec::new();
    for actor in (0..directory.actors.len() as u64).chain([FAMILY]) {
        for measure in 0..4 {
            if let Some(key) = directory.key(actor, measure, None) {
                places.push(((actor, measure, None), key));
            }
        }
    }
    for measure in [layout::SOURCE, layout::TOKEN] {
        for &(dimension, key) in directory.dimensions(FAMILY, measure) {
            places.push(((FAMILY, measure, Some(dimension)), key));
        }
    }
    let mut dump = BTreeMap::new();
    for (place, key) in places {
        let held = (
            events(&mut snapshot, key, span),
            evidence(&mut snapshot, key, span),
            runs(&mut snapshot, key, span),
        );
        dump.insert(place, held);
    }
    let mut tokens = Vec::new();
    for number in 0.. {
        let Some((events, _, _)) = dump.get(&(FAMILY, layout::TOKEN, Some(number))) else {
            break;
        };
        let [event] = &events[..] else {
            panic!("token {number} is spelled once: {events:?}");
        };
        assert_eq!(event.kind, kind::TOKEN);
        let [Canon::Token(text)] = &event.cells[..] else {
            panic!("a token is its text");
        };
        tokens.push(text.clone());
    }
    let token = |cell: &Canon| match cell {
        Canon::Unsigned(number) => Canon::Token(tokens[*number as usize].clone()),
        Canon::Null => Canon::Null,
        other => panic!("not a token: {other:?}"),
    };
    // The observation's known hours, as runs of hours one after another across cuts.
    let observed = &dump[&(FAMILY, layout::OBSERVED, None)].1;
    let mut spans: Vec<Span> = observed
        .iter()
        .filter(|row| row.known == Knownness::Known)
        .map(|row| row.span)
        .collect();
    spans.sort_unstable_by_key(|span| span.first);
    let mut known: Vec<(i32, i32)> = Vec::new();
    for span in spans {
        match known.last_mut() {
            Some((_, last)) if *last + 1 == span.first => *last = span.last,
            _ => known.push((span.first, span.last)),
        }
    }
    // Each stream's rows after each of its events, checking its chains.
    let mut after: Vec<(String, u64, Vec<(i32, BTreeMap<Canon, Vec<Canon>>)>)> = Vec::new();
    for (actor, named) in directory.actors.iter().enumerate() {
        let owner = String::from_utf8(named.name.to_vec()).unwrap();
        for stream in [
            layout::HOLDERS,
            layout::SHIPS,
            layout::TOTALS,
            layout::REFUSALS,
        ] {
            let Some((events, _, _)) = dump.get(&(actor as u64, stream, None)) else {
                continue;
            };
            let width = cells_of(stream, V::REFITS).unwrap();
            let mut rows = BTreeMap::new();
            let mut chain: Option<(i32, u8)> = None;
            let mut states = Vec::new();
            for event in events {
                let &(start, _) = known
                    .iter()
                    .find(|(first, last)| (*first..=*last).contains(&event.hour))
                    .expect("a stream writes within the observation's coverage");
                let header = match event.kind {
                    kind::KEYFRAME => {
                        rows.clear();
                        chain = Some((event.hour, 1));
                        0
                    }
                    kind::DELTA => {
                        let (first, length) = chain.expect("a delta follows its keyframe");
                        assert_eq!(event.cells[0], Canon::Integer(i128::from(first)));
                        assert!(length < layout::CHAIN, "a chain is CHAIN events at most");
                        assert!(first >= start, "a chain starts within its coverage");
                        chain = Some((first, length + 1));
                        1
                    }
                    other => panic!("unexpected event kind {other}"),
                };
                let body = &event.cells[header..];
                assert_eq!(body.len() % width, 0, "rows are {width} cells each");
                for cells in body.chunks_exact(width) {
                    let Canon::Unsigned(number) = cells[0] else {
                        panic!("a row's key is unsigned");
                    };
                    let key = if stream == layout::REFUSALS {
                        Canon::Token(tokens[number as usize].clone())
                    } else {
                        Canon::Unsigned(number)
                    };
                    if cells[1] == Canon::Unsigned(1) {
                        let mut row: Vec<Canon> = cells[2..].to_vec();
                        for &at in labels(stream, V::REFITS) {
                            row[at - 2] = token(&row[at - 2]);
                        }
                        rows.insert(key, row);
                    } else {
                        assert_eq!(event.kind, kind::DELTA, "a keyframe lists held rows");
                        assert!(cells[2..].iter().all(|cell| *cell == Canon::Null));
                        assert!(rows.remove(&key).is_some(), "a delta removes a held row");
                    }
                }
                states.push((event.hour, rows.clone()));
            }
            for &(start, _) in &known {
                let held = states
                    .iter()
                    .rev()
                    .find(|(at, _)| *at <= start)
                    .is_some_and(|(_, rows)| !rows.is_empty());
                assert!(
                    !held
                        || events
                            .iter()
                            .any(|event| event.hour == start && event.kind == kind::KEYFRAME),
                    "{owner} {stream}: rows held where coverage starts at {start} are keyed"
                );
            }
            after.push((owner.clone(), stream, states));
        }
    }
    let mut states = BTreeMap::new();
    for &hour in hours {
        let known = observed
            .iter()
            .find(|row| row.span.contains(hour))
            .is_some_and(|row| row.known == Knownness::Known);
        states.insert(
            hour,
            known.then(|| {
                let mut held = Rows::new();
                for (owner, stream, after) in &after {
                    let rows = after
                        .iter()
                        .rev()
                        .find(|(at, _)| *at <= hour)
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default();
                    for (key, row) in rows {
                        held.insert((owner.clone(), *stream, key), row);
                    }
                }
                held
            }),
        );
    }
    Published {
        states,
        dump,
        tokens,
    }
}

/// The ordinals at which each placement starts a segment, and how often it cuts and resumes.
pub(crate) struct Placement {
    pub(crate) label: &'static str,
    pub(crate) restarts: BTreeSet<usize>,
    pub(crate) every: usize,
    pub(crate) resume: bool,
}

pub(crate) fn placements(hours: usize) -> Vec<Placement> {
    vec![
        Placement {
            label: "one segment, one cut",
            restarts: BTreeSet::new(),
            every: usize::MAX,
            resume: false,
        },
        Placement {
            label: "segments every fourth hour, cuts every sixth",
            restarts: (1..hours).filter(|ordinal| ordinal % 4 == 0).collect(),
            every: 6,
            resume: false,
        },
        Placement {
            label: "a segment at every hour, cuts every fifth, resumed at each",
            restarts: (1..hours).collect(),
            every: 5,
            resume: true,
        },
        Placement {
            label: "segments where names move, cuts every third, resumed at each",
            restarts: [3, 8, 10, 11, 13, 14, 15].into(),
            every: 3,
            resume: true,
        },
    ]
}

fn publication<V: View>(frames: &[DecodedFrame], placement: &Placement) -> Published {
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let mut cuts: Vec<i32> = hours
        .iter()
        .copied()
        .enumerate()
        .filter(|&(at, _)| placement.every != usize::MAX && at % placement.every == placement.every - 1)
        .map(|(_, hour)| hour)
        .collect();
    if cuts.last() != hours.last() {
        cuts.push(*hours.last().unwrap());
    }
    let resume = if placement.resume { cuts.clone() } else { Vec::new() };
    let (pages, root) = build(project::<V>(frames, &placement.restarts), &cuts, &resume);
    published::<V>(&pages, root, &hours)
}

fn holds_what_the_recording_gives<V: View>(label: &str) {
    let frames = scenario();
    let mut first: Option<Published> = None;
    for placement in placements(frames.len()) {
        let published = publication::<V>(&frames, &placement);
        for frame in &frames {
            assert!(
                published.states[&frame.hour] == whole::<V>(frame),
                "{label}, {}: hour {}",
                placement.label,
                frame.hour
            );
        }
        // The same events, evidence and runs, wherever segments start, cuts fall or the
        // emit resumes.
        match &first {
            None => first = Some(published),
            Some(first) => assert!(first.dump == published.dump, "{label}, {}", placement.label),
        }
    }
}

#[test]
fn every_owner_holds_what_the_recording_gives_at_every_hour_however_it_is_walked() {
    holds_what_the_recording_gives::<Repairs>("repairs");
    holds_what_the_recording_gives::<Refits>("refits");
}

/// The source key of `table`'s evidence.
fn source<'a>(published: &'a Published, table: TableId) -> &'a [Evidence] {
    &published.dump[&(FAMILY, layout::SOURCE, Some(table.index() as u64))].1
}

fn at(evidence: &[Evidence], hour: i32) -> Evidence {
    *evidence
        .iter()
        .find(|row| row.span.contains(hour))
        .unwrap_or_else(|| panic!("evidence at {hour}"))
}

/// Each source's coverage dates its reads: a daily table's supplying read moves where its slot
/// is read, though no row changes; a table not recorded is unknown for its spelled reason; the
/// observation is unknown where no owner resolves, and the dating table's schedule runs hourly.
#[test]
fn the_views_publish_their_sources_reads_and_observation_as_they_happen() {
    let frames = scenario();
    let placement = &placements(frames.len())[0];
    let refits = publication::<Refits>(&frames, placement);
    let lines = source(&refits, TableId::ProductionLines);
    assert_eq!(at(lines, 100).known, Knownness::Known);
    assert_eq!(at(lines, 103).supplying_read, None);
    assert_eq!(at(lines, 104).supplying_read, Some(104));
    assert_eq!(at(lines, 112).supplying_read, Some(104));
    // No date carries past an hour whose owners do not resolve.
    assert_eq!(at(lines, 114).supplying_read, None);
    let designs = source(&refits, TableId::VariantDefinitions);
    assert_eq!(at(designs, 118).supplying_read, Some(118));
    let repairs = publication::<Repairs>(&frames, placement);
    let queue = source(&repairs, TableId::NavalRepairQueue);
    assert_eq!(at(queue, 102).known, Knownness::Known);
    let Knownness::Unknown(why) = at(queue, 103).known else {
        panic!("a refused queue is unknown");
    };
    assert!(
        repairs.tokens[why as usize].contains("NavalRepairQueue: Synthetic repair queue refusal"),
        "{}",
        repairs.tokens[why as usize]
    );
    assert_eq!(at(queue, 105).known, Knownness::Known);
    for published in [&repairs, &refits] {
        let observed = &published.dump[&(FAMILY, layout::OBSERVED, None)].1;
        assert_eq!(at(observed, 112).known, Knownness::Known);
        assert_eq!(
            at(observed, 113).known,
            Knownness::Unknown(reason::COUNTRIES)
        );
        assert_eq!(at(observed, 114).known, Knownness::Known);
        let schedule = &published.dump[&(FAMILY, READS, None)].2;
        assert_eq!(schedule, &[(Span::new(100, 123).unwrap(), 1)]);
    }
}

/// Where the observation's coverage starts again, each stream holding rows writes a keyframe,
/// whether the rows are unchanged (114) or changed (123), however the walk is placed.
#[test]
fn a_stream_starts_a_chain_where_the_observation_starts_again() {
    let frames = scenario();
    for placement in placements(frames.len()) {
        let repairs = publication::<Repairs>(&frames, &placement);
        let refits = publication::<Refits>(&frames, &placement);
        for (view, published) in [("repairs", &repairs), ("refits", &refits)] {
            for stream in [layout::HOLDERS, layout::SHIPS, layout::TOTALS] {
                let events = &published.dump[&(0, stream, None)].0;
                for hour in [114, 123] {
                    assert!(
                        events
                            .iter()
                            .any(|event| event.hour == hour && event.kind == kind::KEYFRAME),
                        "{view}, {}: stream {stream} at {hour}",
                        placement.label
                    );
                }
            }
        }
    }
}

/// A stream writes an event only at an hour that changes it, and a quiet hour writes nothing:
/// the second ship's two refusals and its row arrive at 108 and go at 112, and nothing moves at
/// 101 or 106.
#[test]
fn a_stream_writes_only_where_its_rows_change() {
    let frames = scenario();
    let repairs = publication::<Repairs>(&frames, &placements(frames.len())[0]);
    let hours = |stream: u64| -> Vec<i32> {
        repairs.dump[&(0, stream, None)]
            .0
            .iter()
            .map(|event| event.hour)
            .collect()
    };
    for stream in [layout::HOLDERS, layout::SHIPS, layout::TOTALS, layout::REFUSALS] {
        let hours = hours(stream);
        assert!(!hours.contains(&101) && !hours.contains(&106), "{stream}: {hours:?}");
    }
    let refusals = hours(layout::REFUSALS);
    assert!(refusals.contains(&108) && refusals.contains(&112), "{refusals:?}");
    let held = repairs.states[&108].as_ref().unwrap();
    let ship = &held[&(COUNTRY.to_owned(), layout::SHIPS, Canon::Unsigned(SECOND))];
    assert_eq!(ship[ship::HOLDER - 2], Canon::Unsigned(9));
    assert_eq!(ship[ship::HULL - 2], Canon::Integer(50_000));
    assert_eq!(ship[ship::MAXIMUM - 2], Canon::Null);
}
