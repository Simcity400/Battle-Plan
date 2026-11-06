//! The place-names family's publication of synthetic walks: what it holds of every province,
//! state and name table at every hour is what the frame reader reads there, however the walk is
//! cut into segments, published and resumed.

use super::{
    FrameInputs, Inputs,
    emit::{Emit, SCHEMA},
    facts::{self, TABLES},
    query::{Held, walked},
};
use dashboard_series::{
    Blob, Identity, Publication, Snapshot, Span, Writer,
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
        CountryAppearanceRow, CountryLinkRow, CountryRow, ProvinceNameRow, ProvinceRow, Recorded,
        ReservedCountryNamingRow, StateRow, TableId, TableRows, TableSchema, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::collections::BTreeSet;

const PROVINCES: [u32; 7] = [1, 2, 3, 4, 5, 6, 7];
const STATES: [u32; 5] = [1, 2, 3, 4, 9];

/// Two countries, AAA (id 11, slot 1, also linked as 33) and BBB (id 22, slot 2, originally
/// OLD); five provinces in three states, one stateless, one in a state States lacks; a renamed
/// province; a state whose owner links to no country, and one without an owner.
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
        frame.tables[table.index()] = TableView::Complete(rows).into();
        frame.metrics.table_ns[table.index()] = 1;
    };
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![
        CountryRow { id: 11, canonical_slot: 1, tag: b"AAA".to_vec(), ..Default::default() },
        CountryRow { id: 22, canonical_slot: 2, tag: b"BBB".to_vec(), ..Default::default() },
    ]));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![
        CountryLinkRow { id: 11, canonical_slot: 1, ..Default::default() },
        CountryLinkRow { id: 22, canonical_slot: 2, ..Default::default() },
        CountryLinkRow { id: 33, canonical_slot: 1, ..Default::default() },
    ]));
    set(&mut frame, TableId::CountryAppearance, TableRows::CountryAppearance(vec![
        CountryAppearanceRow {
            country: 11,
            original_tag: Recorded::Value(None),
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        },
        CountryAppearanceRow {
            country: 22,
            original_tag: Recorded::Value(Some(b"OLD".to_vec())),
            original_tag_reference: Recorded::Value(Some(5)),
            ..Default::default()
        },
    ]));
    set(&mut frame, TableId::ReservedCountryNaming, TableRows::ReservedCountryNaming(vec![
        ReservedCountryNamingRow {
            tag: b"ZZZ".to_vec(),
            original_tag: Recorded::Value(None),
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        },
    ]));
    set(&mut frame, TableId::Provinces, TableRows::Provinces(vec![
        ProvinceRow { id: 1, state: 1, ..Default::default() },
        ProvinceRow { id: 2, state: 1, ..Default::default() },
        ProvinceRow { id: 3, state: 2, ..Default::default() },
        ProvinceRow { id: 4, state: 0, ..Default::default() },
        ProvinceRow { id: 5, state: 9, ..Default::default() },
        ProvinceRow { id: 6, state: 3, ..Default::default() },
    ]));
    set(&mut frame, TableId::ProvinceNames, TableRows::ProvinceNames(vec![
        ProvinceNameRow { province: 2, name: b"PROVINCE_RENAMED".to_vec() },
    ]));
    set(&mut frame, TableId::States, TableRows::States(vec![
        StateRow { id: 1, owner: 11, owner_name: b"AAA's state".to_vec(), ..Default::default() },
        StateRow { id: 2, owner: 33, custom_name: b"Custom".to_vec(), ..Default::default() },
        StateRow { id: 3, owner: 44, ..Default::default() },
        StateRow { id: 4, owner: 0, ..Default::default() },
    ]));
    frame
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

rows!(province_rows, Provinces, ProvinceRow);
rows!(rename_rows, ProvinceNames, ProvinceNameRow);
rows!(state_rows, States, StateRow);
rows!(appearance_rows, CountryAppearance, CountryAppearanceRow);
rows!(reserved_rows, ReservedCountryNaming, ReservedCountryNamingRow);

fn unavailable(frame: &mut DecodedFrame, table: TableId) {
    frame.tables[table.index()] = TableView::Unavailable {
        reason: "unread".into(),
    }
    .into();
}

/// Forty hours: a state changes hands, renames come and go, the name tables are unavailable or
/// truncated to other rows for a while, a province leaves, an original tag is dropped and slot zero's
/// naming row is unread.
fn walk() -> Vec<DecodedFrame> {
    (100..140)
        .map(|hour| {
            let mut frame = frame(hour);
            if hour >= 105 {
                state_rows(&mut frame)[0].owner = 22;
                state_rows(&mut frame)[0].owner_name = b"BBB's state".to_vec();
            }
            if (110..113).contains(&hour) {
                unavailable(&mut frame, TableId::ProvinceNames);
            }
            if hour >= 115 {
                rename_rows(&mut frame).push(ProvinceNameRow {
                    province: 3,
                    name: Vec::new(),
                });
            }
            if hour >= 118 {
                appearance_rows(&mut frame)[1].original_tag_reference = Recorded::Value(Some(0));
            }
            if (120..123).contains(&hour) {
                unavailable(&mut frame, TableId::States);
            }
            if (123..125).contains(&hour) {
                // Each hour shows other rows under the same status.
                let shown = if hour == 123 { 0..2 } else { 2..4 };
                let rows = state_rows(&mut frame)[shown].to_vec();
                frame.tables[TableId::States.index()] = TableView::Truncated {
                    rows: TableRows::States(rows),
                    missing: 2,
                }
                .into();
            }
            if hour >= 125 {
                province_rows(&mut frame).retain(|row| row.id != 5);
            }
            if (127..130).contains(&hour) {
                unavailable(&mut frame, TableId::ReservedCountryNaming);
            }
            if hour >= 130 {
                state_rows(&mut frame)[2].custom_name = b"Renamed".to_vec();
                reserved_rows(&mut frame)[0].tag = b"YYY".to_vec();
            }
            if hour == 133 {
                unavailable(&mut frame, TableId::CountryAppearance);
            }
            if hour >= 135 {
                province_rows(&mut frame).push(ProvinceRow { id: 7, state: 4, ..Default::default() });
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
    write::<tables::Provinces>(walk, input);
    write::<tables::ProvinceNames>(walk, input);
    write::<tables::States>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::CountryAppearance>(walk, input);
    write::<tables::ReservedCountryNaming>(walk, input);
}

/// The projected hours of `inputs`, a projector of its own starting a segment at each ordinal
/// `restarts` holds.
fn project(inputs: &[DecodedFrame], restarts: &BTreeSet<usize>) -> Vec<facts::Hour> {
    let format = FormatVersion::CURRENT;
    let mut hours = Vec::new();
    let mut walk = Synthetic::new(TABLES, 1, inputs[0].hour).unwrap();
    let mut held = None;
    let needs = facts::needs().unwrap();
    let mut projector = facts::Projector::new().unwrap();
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    let mut first = 0;
    for (ordinal, input) in inputs.iter().enumerate() {
        if ordinal > 0 && restarts.contains(&ordinal) {
            projector.end(&mut hours).unwrap();
            projector = facts::Projector::new().unwrap();
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
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
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
        root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
        if resume.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

/// Everything `inputs` read of the fixture's ids, in a comparable form.
fn read(inputs: &impl Inputs) -> String {
    let mut out = format!(
        "renames {} {:?}; states {:?} {:?}; reserved {:?}\n",
        inputs.renames_complete(),
        inputs.renames_gap(),
        inputs.states_unrecorded(),
        inputs.states_gap(),
        inputs.reserved_tag(),
    );
    for id in PROVINCES {
        out += &format!(
            "province {id}: {:?} {:?}\n",
            inputs.province(id),
            inputs.rename(id).map(<[u8]>::to_vec)
        );
    }
    for id in STATES {
        out += &format!("state {id}: {:?}\n", inputs.state(id).map(|state| state.into_owned()));
    }
    out
}

#[test]
fn every_name_input_at_every_hour_is_the_frame_reader_s_however_the_walk_is_cut() {
    let inputs = walk();
    let last = inputs.last().unwrap().hour;
    for (restarts, cuts, resume) in [
        (BTreeSet::new(), vec![last], vec![]),
        (BTreeSet::from([9, 23]), vec![108, 121, last], vec![121]),
        (BTreeSet::from([1, 26, 28]), vec![100, 111, 127, last], vec![100, 111, 127]),
    ] {
        let (pages, root) = build(project(&inputs, &restarts), &cuts, &resume);
        let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        for frame in &inputs {
            let hour = frame.hour;
            assert!(walked(&mut snapshot, &directory, hour).unwrap(), "hour {hour}");
            let held = Held::at(&mut snapshot, &directory, hour, &PROVINCES, &STATES).unwrap();
            assert_eq!(
                read(&held),
                read(&FrameInputs::new(frame)),
                "hour {hour} cut at {cuts:?}, restarted at {restarts:?}"
            );
        }
        assert!(!walked(&mut snapshot, &directory, last + 1).unwrap());
    }
}

#[test]
fn the_fixture_reads_every_kind_of_input() {
    let frames = walk();
    let at = |hour: i32| FrameInputs::new(&frames[(hour - 100) as usize]);
    let (start, renamed, unread) = (at(100), at(115), at(110));
    assert_eq!(start.province(4), Some(0));
    assert_eq!(start.rename(2), Some(&b"PROVINCE_RENAMED"[..]));
    assert_eq!(renamed.rename(3), Some(&b""[..]));
    assert!(!unread.renames_complete());
    assert_eq!(start.reserved_tag(), Ok("ZZZ"));
    let owner = |inputs: &FrameInputs<'_>, id: u32| inputs.state(id).map(|state| state.owner.clone());
    use dashboard_base::products::place_names::Owner;
    assert_eq!(owner(&start, 1), Some(Owner::Tag(Ok("AAA".into()))));
    assert_eq!(owner(&at(105), 1), Some(Owner::Tag(Ok("OLD".into()))));
    assert_eq!(owner(&at(118), 1), Some(Owner::Tag(Ok("BBB".into()))));
    assert_eq!(owner(&start, 2), Some(Owner::Tag(Ok("AAA".into()))));
    assert_eq!(owner(&start, 3), Some(Owner::Unlinked(44)));
    assert_eq!(owner(&start, 4), Some(Owner::Tag(Ok("ZZZ".into()))));
    assert!(start.state(9).is_none());
    assert!(matches!(owner(&at(128), 4), Some(Owner::Tag(Err(why))) if why.contains("hour 128")));
    assert!(matches!(owner(&at(133), 1), Some(Owner::Tag(Err(_)))));
}
