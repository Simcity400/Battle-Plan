//! The family's columnar preparation held to the peace scan of the row frames of the same hours,
//! written as the native recorder writes them: the reply its publication gives, complete and as
//! each cut's prefix, across segment starts, a gap, cuts and a resume, equals the scan's.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    test_fixtures::{frame, replay},
    tests::settled_row,
    *,
};
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
    format::FormatVersion,
    rows::{
        LexicalTokenRow, PeaceConferenceRow, SubjectRow, TableRows, TableSchema, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

const REFUSAL: &str = "synthetic refusal";
const FIRST: i32 = 100;
const LAST: i32 = 220;

/// Whether `hour` is recorded: all from [`FIRST`] to [`LAST`] but a gap at 150 and 151.
fn recorded(hour: i32) -> bool {
    !(150..=151).contains(&hour)
}

/// The hour's tables over four countries, with fields the observer does not read moving
/// every hour (state manpower, province controllers, elections, subject autonomy).
///
/// Conference 9 is listed from 110 at turn 1 (the reads at 113, 117 and 121 find it
/// unchanged); at 126 it finalizes, the first war records its end and the roster reads it
/// finalized at 129; state 1 passes from the first country to the second at 127, the
/// settlement row is published at 128, and the read at 133 finds the conference gone. The
/// fourth country's subjection to the third ends at 140, inside a quiet span. At 170 state 3
/// passes from the third country to the fourth as their war ends, with no conference. The
/// second country is renamed at 180, the settlements are not read at 190 and 191, and the first
/// country's ruling party changes at 200. The roster is read at every hour 1 past a multiple
/// of 4.
fn tables_at(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour, 4, false, &[]);
    let mut owners = [0_i32; 5];
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        for row in rows.iter_mut() {
            row.manpower_available = hour;
            if row.id == 1 && hour >= 127 {
                row.owner = 1_002;
            }
            if row.id == 3 && hour >= 170 {
                row.owner = 1_004;
            }
            owners[(row.owner - 1_000) as usize] += 1;
        }
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        for row in rows.iter_mut() {
            row.controller = 1_001 + hour.rem_euclid(4) as u32;
        }
    }
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *frame.tables[TableId::WarRelations.index()]
    {
        for row in rows.iter_mut() {
            row.end_hour = match row.first_country {
                1_001 if hour >= 126 => 126,
                1_003 if hour >= 170 => 170,
                _ => 0,
            };
        }
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows.iter_mut() {
            row.owned_state_count = owners[(row.country - 1_000) as usize];
        }
    }
    if hour >= 180
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows[1].tag = b"renamed".to_vec();
    }
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        for row in rows.iter_mut() {
            row.last_election = hour;
            if row.country == 1_001 && hour >= 200 {
                row.ruling_party = Some(4);
            }
        }
    }
    let conference = |current_turn, finalized| PeaceConferenceRow {
        id: 9,
        ordinal: 0,
        current_turn,
        finalized,
    };
    *frame.tables[TableId::PeaceConferences.index()] =
        TableView::Complete(TableRows::PeaceConferences(match hour {
            110..=125 => vec![conference(1, 0)],
            126..=129 => vec![conference(2, 1)],
            _ => Vec::new(),
        }));
    *frame.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 1_003,
            subject: 1_004,
            start_hour: 90,
            end_hour: 140,
            cancel: 0,
            autonomy_ratio: i64::from(hour),
            state: b"authored subject".to_vec(),
        }]));
    let mut settled = settled_row();
    settled.hour = Some(126);
    *frame.tables[TableId::PeaceSettlements.index()] = if (190..=191).contains(&hour) {
        TableView::Unavailable {
            reason: REFUSAL.into(),
        }
    } else {
        TableView::Complete(TableRows::PeaceSettlements(if hour >= 128 {
            vec![settled]
        } else {
            Vec::new()
        }))
    };
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 77,
                literal: b"take_states".to_vec(),
            },
            LexicalTokenRow {
                token: 991,
                literal: b"custom_frontier_demand".to_vec(),
            },
        ]));
    frame
}

/// Writes `frame`'s rows of `T`'s table into the walk's hour, read where `read`.
fn put<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, read: bool) {
    let sample = Sample::FromReadBit;
    let input = match frame.complete::<T>() {
        Ok(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        Err(_) => Input::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(T::ID, input).unwrap();
}

/// The projected hours, a projector starting cold at each of `starts` and at each walk's
/// first hour (after the gap, the walk starts again), and the row frames of the same hours.
fn projected(starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let needs = facts::needs().unwrap();
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(format).unwrap();
    for (ordinal, hour) in (0..).zip((FIRST..=LAST).filter(|&hour| recorded(hour))) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let tables = tables_at(hour);
        put::<tables::Countries>(walk, &tables, true);
        put::<tables::CountryLinks>(walk, &tables, true);
        put::<tables::States>(walk, &tables, true);
        put::<tables::Provinces>(walk, &tables, true);
        put::<tables::WarRelations>(walk, &tables, true);
        put::<tables::PeaceConferences>(walk, &tables, hour.rem_euclid(4) == 1);
        put::<tables::PeaceSettlements>(walk, &tables, true);
        put::<tables::Subjects>(walk, &tables, true);
        put::<tables::Politics>(walk, &tables, true);
        put::<tables::PoliticalParties>(walk, &tables, true);
        put::<tables::CountryAppearance>(walk, &tables, true);
        put::<tables::CountryCounterInputs>(walk, &tables, true);
        put::<tables::LexicalTokens>(walk, &tables, true);
        let cold = starts.contains(&hour);
        if cold {
            projector = Projector::new(format).unwrap();
        }
        let decoded = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (format, CaptureMode::Full),
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
                decoded.clone()
            })
            .unwrap();
        frames.push(decoded);
    }
    (out, frames)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// The recording whose branch the emit prepares: from [`FIRST`] to [`LAST`].
fn recording() -> Replay {
    replay(vec![frame(FIRST, 1, false, &[]), frame(LAST, 1, false, &[])])
}

/// The scan's reply of `frames` through `last`, as a complete reply or a prepared prefix.
fn scanned(frames: &[DecodedFrame], last: i32, prefix: bool) -> serde_json::Value {
    let mut scan = Scan::new(1, FIRST, LAST, FormatVersion::CURRENT.version());
    for frame in frames.iter().filter(|frame| frame.hour <= last) {
        scan.observe(frame).unwrap();
    }
    scan.last = last;
    scan.prefix = prefix;
    serde_json::to_value(scan.reply()).unwrap()
}

/// The reply the publication at `root` gives through `last`.
fn published(
    pages: &Memory,
    root: u64,
    last: i32,
    prefix: bool,
) -> serde_json::Value {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let reply = super::query::reply(
        &mut snapshot,
        &directory,
        last,
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    serde_json::to_value(reply).unwrap()
}

/// Publishes `hours`, cut after each of `cuts`' hours, each cut's prefix holding what the scan
/// of the frames through it does; when `resume` names a cut, the emit is saved there and goes
/// on from the publication. Returns the last publication's pages and root.
fn build(
    hours: Vec<facts::Hour>,
    frames: &[DecodedFrame],
    cuts: &[i32],
    resume: Option<i32>,
    label: &str,
) -> (Memory, u64) {
    let budget = Budget::new(256 << 20);
    let cache = Cache::new(&budget);
    let recording = recording();
    let mut pages = Memory::default();
    let mut writer = Writer::new(&budget, identity(), FIRST).unwrap();
    let mut emit = Emit::new(&recording, 1).unwrap();
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
                accepted: Span::new(FIRST, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        assert_eq!(
            published(&pages, root, cut, true),
            scanned(frames, cut, true),
            "{label}: the prefix through {cut}"
        );
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, &budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "{label}: every hour published");
    (pages, root)
}

/// Every placement of segment starts, cuts and a resume publishes the scan's reply, complete
/// and through each cut, while the hours no field the observer reads changed hand nothing on.
#[test]
fn the_publication_answers_what_the_scan_of_the_row_frames_does() {
    for (starts, cuts, resume) in [
        (&[][..], &[LAST][..], None),
        (
            &[119, 160, 175][..],
            &[118, 129, 160, 205, LAST][..],
            Some(160),
        ),
        (
            &[105, 114, 126, 133, 141, 171, 190][..],
            &[113, 117, 127, 140, 170, 192, LAST][..],
            Some(127),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resumed at {resume:?}");
        // A resumed projector starts cold, as the walk takes it up after its saved cut.
        let mut starts = starts.to_vec();
        starts.extend(resume.map(|cut| cut + 1));
        let (hours, frames) = projected(&starts);
        let held = hours.iter().filter(|hour| hour.observed()).count();
        assert!(
            held * 3 < hours.len(),
            "{label}: {held} of {} hours hand reads on",
            hours.len()
        );
        let (pages, root) = build(hours, &frames, cuts, resume, &label);
        let complete = published(&pages, root, LAST, false);
        assert_eq!(complete, scanned(&frames, LAST, false), "{label}");
        let deals = complete["deals"].as_array().unwrap();
        assert!(
            deals.iter().any(|deal| deal["conferenceId"] == "9"
                && !deal["captured"].is_null()),
            "{label}: the conference's captured outcome: {complete}"
        );
        assert!(
            deals
                .iter()
                .any(|deal| deal["observation"] == "conferenceNotRecorded"),
            "{label}: the outcome without a conference: {complete}"
        );
    }
}
