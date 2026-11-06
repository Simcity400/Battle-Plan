//! The family's columnar preparation held to the row scan ([`super::Scan`] over row frames) on
//! the same hours, written as the native recorder writes them: each tag's whole history, its
//! slot states at every hour and its grants at every hour as a supplying read admits them,
//! across segment starts, a gap, cuts and a resume.

use super::{
    Scan,
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    grants::{self, GrantInventory},
    query::{ResearchStateSlot, grants_at, history, slots_at},
    test_fixtures::research_frame,
};
use crate::{ResearchHistory, ResearchProgress, test_fixtures::fixture_research_inputs};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    format::FormatVersion,
    rows::{
        LexicalTokenRow, ResearchAppliedBonusRow, ResearchModifierRow as R, RowCodec,
        SliceReadRow, TableId, TableRows,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

/// Day 1000's first hour.
const BASE: i32 = 1000 * 24;
const TAG: &str = "GER";

/// GER (raw ids 11 and 9001, canonical slot 7) is read every four hours, each read carried into
/// the hours after it, until +32; that read is held through +61, so its one-day carry ends
/// inside the silence. Its first slot researches tech 1 until the read at +12, then tech 2
/// (completed at the read at +32); its second slot researches tech 3
/// from +12. Progress moves at every read. Its grant inventory, read with its slots, names a
/// restriction token; the tokens are unavailable from +50 to +52, for another reason at +52. GER
/// is renamed GEX from +74 to +76. Hours +24 and +25 are a gap.
fn frame_at(hour: i32) -> DecodedFrame {
    let at = hour - BASE;
    let read_at = if (33..62).contains(&at) {
        32
    } else {
        at - at.rem_euclid(4)
    };
    let read = BASE + read_at;
    let assignments: &[Option<u32>] = if read_at < 12 {
        &[Some(1), None]
    } else if read_at < 32 {
        &[Some(2), Some(3)]
    } else {
        &[None, Some(3)]
    };
    let completed: &[u32] = if read_at >= 32 { &[2] } else { &[] };
    let mut at_read = research_frame(1, read, assignments, completed);
    fixture_research_inputs(&mut at_read);
    let mut bonused = None;
    if let TableView::Complete(TableRows::ResearchInputs(rows)) =
        &mut *at_read.tables[TableId::ResearchInputs.index()]
    {
        for row in rows.iter_mut().filter(|row| row.technology_present != 0) {
            row.progress_raw = i64::from(read_at + 1) * 1_000;
        }
        // The second slot's technology holds one applied bonus.
        if let Some(row) = rows.get_mut(1).filter(|row| row.technology_present != 0) {
            row.bonus_count = 1;
            bonused = Some(row.technology_handle);
        }
    }
    if let Some(technology_handle) = bonused {
        *at_read.tables[TableId::ResearchAppliedBonuses.index()] =
            TableView::Complete(TableRows::ResearchAppliedBonuses(vec![
                ResearchAppliedBonusRow {
                    country: 11,
                    index: 1,
                    ordinal: 0,
                    technology_handle,
                    id: 4,
                    resolved: 1,
                    name: b"bonus_a".to_vec(),
                    speed_bonus_raw: 5_000,
                    ..Default::default()
                },
            ]));
    }
    let mut carried = research_frame(1, hour, assignments, completed);
    for table in [TableId::ResearchInputs, TableId::ResearchAppliedBonuses] {
        carried.tables[table.index()] = at_read.tables[table.index()].clone();
    }
    // The holder's own id holds the grant inventory's read hour too.
    let TableView::Complete(TableRows::SliceReads(mut reads)) =
        (*at_read.tables[TableId::SliceReads.index()]).clone()
    else {
        panic!("fixture read hours");
    };
    reads.push(SliceReadRow {
        table: TableId::ResearchModifiers.index() as u32,
        owner: 9001,
        sample_hour: read,
    });
    reads.sort_by_key(|row| (row.table, row.owner));
    *carried.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(reads));
    let inventory = engine_research::research_modifiers::grants::INVENTORY_INDEX;
    let mut modifiers = vec![
        R {
            kind: R::GRANT_COUNTER,
            raw: i64::from(read_at),
            ..Default::default()
        },
        R {
            kind: R::SPEED_GRANT,
            source_id: 7,
            name: b"mod_a".to_vec(),
            raw: 10,
            ..Default::default()
        },
        R {
            kind: R::GRANT_USES,
            source_id: 7,
            raw: 3,
            auxiliary: 1,
            ..Default::default()
        },
        R {
            kind: R::GRANT_TECHNOLOGY,
            source_id: 7,
            raw: 5,
            auxiliary: 3,
            ..Default::default()
        },
    ];
    for (ordinal, row) in modifiers.iter_mut().enumerate() {
        row.country = 11;
        row.index = inventory;
        row.ordinal = ordinal as u32;
    }
    // Each held slot's speed breakdown, its category speed raised at the read at +20, and the
    // country's own scope; in key order around the inventory.
    let scope = |index: u32, handle: u32, rows: &[(u8, i64, u32)]| -> Vec<R> {
        (0..)
            .zip(rows)
            .map(|(ordinal, &(kind, raw, source_id))| R {
                country: 11,
                index,
                ordinal,
                technology_handle: handle,
                kind,
                raw,
                source_id,
                ..Default::default()
            })
            .collect()
    };
    let category = if read_at < 20 { -10_000 } else { 5_000 };
    let mut scoped = Vec::new();
    for (index, handle) in (0..).zip(assignments) {
        if let Some(handle) = *handle {
            scoped.extend(scope(
                index,
                handle,
                &[
                    (R::TECHNOLOGY, i64::from(handle), 0),
                    (R::YEAR, 1938, 0),
                    (R::CATEGORY_SPEED, category, 0),
                    (R::DESIGN_TEAM, 5_000, 0),
                    (R::EXPERIENCE, 0, 0),
                    (R::LICENSE, 0, 0),
                    (R::SCIENTIST, 0, 0),
                ],
            ));
        }
    }
    scoped.append(&mut modifiers);
    scoped.extend(scope(
        u32::MAX,
        0,
        &[
            (R::COUNTRY_SPEED, 25_000, 0),
            (R::AHEAD_FACTOR, 200_000, 0),
            (R::MINIMUM_SPEED, 10_000, 0),
            (R::SHARING_MAXIMUM, 50_000, 0),
            (R::SHARING_MULTIPLIER, 100_000, 0),
            (R::MODIFIER_METADATA, 0, 0),
            (R::MODIFIER_METADATA, 0, 4),
        ],
    ));
    *carried.tables[TableId::ResearchModifiers.index()] =
        TableView::Complete(TableRows::ResearchModifiers(scoped));
    if (74..=76).contains(&at)
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *carried.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.tag = b"GEX".to_vec();
        }
    }
    *carried.tables[TableId::LexicalTokens.index()] = if (50..=52).contains(&at) {
        TableView::Unavailable {
            reason: if at == 52 { "later tokens" } else { "synthetic tokens" }.into(),
        }
    } else {
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 5,
            literal: b"tech_5".to_vec(),
        }]))
    };
    carried
}

fn frames() -> Vec<DecodedFrame> {
    (BASE..=BASE + 80)
        .filter(|hour| !(BASE + 24..=BASE + 25).contains(hour))
        .map(frame_at)
        .collect()
}

fn put<T: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[T]>, reason: &str) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// Writes `frame`'s tables a projection reads into `walk`.
fn write(walk: &mut Synthetic, frame: &DecodedFrame) {
    for &table in facts::TABLES {
        let view = &*frame.tables[table.index()];
        let reason = match view {
            TableView::Unavailable { reason } => &reason[..],
            _ => "",
        };
        macro_rules! rows {
            ($variant:ident) => {
                match view {
                    TableView::Complete(TableRows::$variant(rows)) => Some(&rows[..]),
                    TableView::Unavailable { .. } => None,
                    _ => panic!("{table:?} is neither complete nor unavailable"),
                }
            };
        }
        match table {
            TableId::Countries => put(walk, table, rows!(Countries), reason),
            TableId::CountryLinks => put(walk, table, rows!(CountryLinks), reason),
            TableId::ResearchTechnologies => {
                put(walk, table, rows!(ResearchTechnologies), reason)
            }
            TableId::ResearchInputs => put(walk, table, rows!(ResearchInputs), reason),
            TableId::ResearchModifiers => put(walk, table, rows!(ResearchModifiers), reason),
            TableId::ResearchAppliedBonuses => {
                put(walk, table, rows!(ResearchAppliedBonuses), reason)
            }
            TableId::SliceReads => put(walk, table, rows!(SliceReads), reason),
            TableId::LexicalTokens => put(walk, table, rows!(LexicalTokens), reason),
            _ => panic!("{table:?} is not read"),
        }
    }
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut last: Option<i32> = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    for (ordinal, frame) in (0..).zip(frames) {
        if walk.is_none() || last.is_some_and(|last| last + 1 != frame.hour) {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        last = Some(frame.hour);
        let walk = walk.as_mut().unwrap();
        write(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
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

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
fn build(
    hours: Vec<facts::Hour>,
    cuts: &[i32],
    resume: Option<i32>,
    budget: &Budget,
) -> (Memory, u64) {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new();
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
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    (pages, root)
}

/// What the row scan holds of GER at each hour: its slots' availability and state, and each
/// slot's latest read with its progress then.
struct Expected {
    slots: BTreeMap<i32, Vec<(u32, bool, Option<crate::ResearchSlotState>)>>,
    reads: BTreeMap<i32, BTreeMap<u32, (i32, Option<ResearchProgress>)>>,
    history: ResearchHistory,
    unjoined: usize,
}

fn scanned(frames: &[DecodedFrame]) -> Expected {
    let mut scan = Scan::default();
    let mut expected = Expected {
        slots: BTreeMap::new(),
        reads: BTreeMap::new(),
        history: ResearchHistory::default(),
        unjoined: 0,
    };
    let mut reads: BTreeMap<u32, (i32, Option<ResearchProgress>)> = BTreeMap::new();
    for frame in frames {
        let hour = frame.hour;
        scan.observe_selected(frame, None).unwrap();
        let Some(country) = scan.countries.get(TAG) else {
            continue;
        };
        let mut slots = Vec::new();
        for slot in &country.history.slots {
            if country.last_read == Some(hour) {
                let progress = slot
                    .progress
                    .last()
                    .filter(|progress| progress.last_hour == hour)
                    .cloned();
                reads.insert(slot.index, (hour, progress));
            }
            slots.push((
                slot.index,
                slot.availability
                    .last()
                    .is_some_and(|span| span.last_hour == hour),
                slot.states
                    .last()
                    .filter(|state| state.last_hour == hour)
                    .cloned(),
            ));
        }
        expected.slots.insert(hour, slots);
        expected.reads.insert(hour, reads.clone());
    }
    let country = &scan.countries[TAG];
    expected.history = country.history.clone();
    expected.history.slots.sort_by_key(|slot| slot.index);
    expected.unjoined = country.unjoined_reads;
    expected
}

/// GER's grants at `hour` as the row read admitted them: a read of its holder at its hour,
/// every owner holding grant rows read then, through each hour after it of an unbroken walk with
/// the same holder, short of a read period.
fn admitted(frames: &[DecodedFrame], hour: i32) -> GrantInventory {
    use replay_data::rows::tables;
    let window: Vec<&DecodedFrame> = frames
        .iter()
        .filter(|frame| frame.hour <= hour && frame.hour > hour - STAGGER_PERIOD)
        .collect();
    let (mut identity, mut previous, mut admitted) = (None, None, false);
    for frame in &window {
        let links = frame.complete::<tables::CountryLinks>().ok();
        let current = frame
            .complete::<tables::Countries>()
            .ok()
            .zip(links)
            .and_then(|(countries, links)| {
                crate::metadata::tagged_country(countries, links, TAG).found()
            })
            .map(|(country, slot)| (country.id, slot));
        if current != identity || previous.and_then(|last: i32| last.checked_add(1)) != Some(frame.hour)
        {
            admitted = false;
        }
        identity = current;
        previous = Some(frame.hour);
        let reads = frame.complete::<tables::SliceReads>().ok();
        let read = current.and_then(|(id, _)| {
            reads.and_then(|reads| {
                replay_data::sliced::read_hour(reads, TableId::ResearchModifiers, id)
            })
        });
        if read == Some(frame.hour) {
            admitted = true;
        }
        let rows = frame.complete::<tables::ResearchModifiers>().ok();
        let matched = current.is_some_and(|(_, slot)| {
            let (Some(links), Some(reads), Some(rows)) = (links, reads, rows) else {
                return false;
            };
            rows.iter()
                .filter(|row| {
                    grants::inventory_row(row)
                        && crate::metadata::link_slot(links, row.country) == Some(slot)
                })
                .all(|row| {
                    replay_data::sliced::read_hour(reads, TableId::ResearchModifiers, row.country)
                        == read
                })
        });
        if !matched
            || rows.is_none()
            || read.is_none_or(|read| read > frame.hour || frame.hour - read >= STAGGER_PERIOD)
        {
            admitted = false;
        }
    }
    match window.last().filter(|frame| frame.hour == hour && admitted) {
        Some(frame) => grants::inventory(frame, TAG, |_| None),
        None => GrantInventory::unavailable(grants::UNADMITTED),
    }
}

/// Cold starts, cuts and the cut a resume goes on from, each placement held to the row scan.
const PLACEMENTS: &[(&[i32], &[i32], Option<i32>)] = &[
    (&[], &[BASE + 80], None),
    (&[BASE + 13, BASE + 40], &[BASE + 20, BASE + 47, BASE + 80], None),
    (&[BASE + 26, BASE + 63], &[BASE + 33, BASE + 63, BASE + 80], Some(BASE + 33)),
    (&[BASE + 52, BASE + 75], &[BASE + 23, BASE + 52, BASE + 70, BASE + 80], Some(BASE + 52)),
];

#[test]
fn columnar_research_publishes_what_the_row_scan_reads() {
    let frames = frames();
    let expected = scanned(&frames);
    assert!(
        expected.history.slots.iter().any(|slot| slot
            .entries
            .iter()
            .any(|entry| entry.outcome == crate::ResearchOutcome::Completed)),
        "the scenario completes an assignment"
    );
    let details: Vec<&crate::ResearchDetails> = expected
        .history
        .slots
        .iter()
        .flat_map(|slot| &slot.states)
        .filter_map(|state| state.details.as_ref())
        .collect();
    let totals: std::collections::BTreeSet<&str> = details
        .iter()
        .filter_map(|details| details.modifiers.as_ref())
        .map(|modifiers| modifiers.total_bonus_raw.as_str())
        .collect();
    assert_eq!(totals.len(), 2, "the scenario's breakdown changes once");
    assert!(
        details
            .iter()
            .any(|details| details.bonuses.as_ref().is_some_and(|bonuses| bonuses.len() == 1)),
        "the scenario attributes a bonus"
    );
    let cancelled = AtomicBool::new(false);
    for &(starts, cuts, resume) in PLACEMENTS {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let budget = Budget::new(64 << 20);
        let (pages, root) = build(projected(&frames, starts), cuts, resume, &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let (prepared, unjoined) = history(&mut snapshot, &directory, TAG, &cancelled).unwrap();
        assert_eq!(prepared, expected.history, "{label}");
        assert_eq!(unjoined, expected.unjoined, "{label}");
        let indexes: Vec<u32> = expected.history.slots.iter().map(|slot| slot.index).collect();
        for frame in &frames {
            let hour = frame.hour;
            let at = &expected.slots[&hour];
            let reads = &expected.reads[&hour];
            let want: Vec<ResearchStateSlot> = indexes
                .iter()
                .map(|&index| {
                    let held = at.iter().find(|(slot, ..)| *slot == index);
                    let read = reads.get(&index);
                    ResearchStateSlot {
                        index,
                        available: held.is_some_and(|(_, available, _)| *available),
                        state: held.and_then(|(_, _, state)| state.clone()),
                        progress: read.and_then(|(_, progress)| progress.clone()).map(
                            |mut progress| {
                                progress.last_hour = progress.last_hour.min(hour);
                                progress
                            },
                        ),
                        read_hour: read.map(|(read, _)| *read),
                    }
                })
                .collect();
            let got = slots_at(&mut snapshot, &directory, TAG, hour).unwrap();
            assert_eq!(
                serde_json::to_value(&got).unwrap(),
                serde_json::to_value(&want).unwrap(),
                "slots at {hour}, {label}"
            );
            let grants = match grants_at(&mut snapshot, &directory, TAG, hour).unwrap() {
                Ok(inventory) => inventory,
                Err(reason) => GrantInventory::unavailable(reason),
            };
            assert_eq!(grants, admitted(&frames, hour), "grants at {hour}, {label}");
        }
    }
}

/// The scenario's grants are admitted at reads and the hours they carry, refused for the
/// tokens while those are unavailable, and not admitted through the silence past a read period.
#[test]
fn the_scenario_admits_refuses_and_lapses_grants() {
    let frames = frames();
    let at = |offset: i32| admitted(&frames, BASE + offset);
    assert!(at(4).unavailable.is_none(), "{:?}", at(4));
    assert_eq!(at(4).grants[0].technologies[0].token, "tech_5");
    assert_eq!(
        at(51).unavailable.as_deref(),
        Some(&*format!(
            "hour {} LexicalTokens is unavailable: synthetic tokens",
            BASE + 51
        ))
    );
    assert_eq!(
        at(52).unavailable.as_deref(),
        Some(&*format!("hour {} LexicalTokens is unavailable: later tokens", BASE + 52))
    );
    assert_eq!(at(26).unavailable.as_deref(), Some(grants::UNADMITTED));
    assert_eq!(at(75).unavailable.as_deref(), Some(grants::UNADMITTED));
    assert_eq!(at(57).unavailable.as_deref(), Some(grants::UNADMITTED));
}
