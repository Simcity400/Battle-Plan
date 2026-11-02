//! A keyed family's native publication held to the walk's scan of the same hours: the frames
//! written as the native recorder writes them, read by segments' operators, published with cuts
//! and a resume, and read back as a reply reads it; and the scan observing each frame whole.

use super::{Emit, Native, SCHEMA, catalogued, changes, runs};
use crate::{
    KNOWN, Observer, READ, Scan,
    query::{self, KeyedWindow, windowed},
    segment::{self, needs},
};
use dashboard_series::{
    Blob, Budget, Cache, Identity, Observation, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use dashboard_world::World;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self as driver, Operator as _},
};
use replay_data::{format::FormatVersion, rows::TableSchema, scope::CaptureMode};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

static BUDGET: Budget = Budget::new(256 << 20);

/// Writes `T`'s table of `frame` into `walk` as the recorder writes it: its rows, or why it is
/// unavailable, read where the frame's metrics read it.
pub fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        _ => panic!("a synthetic walk writes complete or unavailable tables"),
    };
    walk.table(T::ID, input).unwrap();
}

/// The hours `frames` give `O`'s operators: a segment starts at each of `starts` (frame
/// indices), and the walk again after each gap. `write` writes a frame's tables.
pub fn operate<O: Observer>(
    frames: &[DecodedFrame],
    starts: &[usize],
    write: fn(&mut Synthetic, &DecodedFrame),
) -> Vec<segment::Hour<Native<O>>> {
    let format = FormatVersion::CURRENT;
    let needs = needs::<Native<O>>().unwrap();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut operator = segment::Operator::<Native<O>>::new(format).unwrap();
    let mut hours = Vec::new();
    for (ordinal, frame) in frames.iter().enumerate() {
        let gap = ordinal > 0 && frames[ordinal - 1].hour + 1 != frame.hour;
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(O::TABLES, frame.branch, frame.hour).unwrap());
        }
        let cold = ordinal == 0 || gap || starts.contains(&ordinal);
        if cold {
            operator = segment::Operator::new(format).unwrap();
        }
        let walk = walk.as_mut().unwrap();
        write(walk, frame);
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = World::at(
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
                    &driver::Hour {
                        branch: frame.branch,
                        hour: decoded.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
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

fn identity() -> Identity {
    Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication of a branch's hours.
pub struct Published {
    pages: Memory,
    root: u64,
}

/// `hours` of `branch` published, cut after each of `cuts`' hours, the last of which is the
/// last hour; where `resume` names one of them, the emit is saved there and goes on from the
/// publication.
pub fn publish_hours<O: Observer>(
    hours: Vec<segment::Hour<Native<O>>>,
    branch: u64,
    cuts: &[i32],
    resume: Option<i32>,
) -> Published {
    let mut published = publish_bound::<O>(hours, branch, cuts, resume, &mut |_| {});
    published.pop().unwrap()
}

/// [`publish_hours`], `bind` binding each emit made, new or loaded, as the driver binds it; the
/// publication as of each cut.
pub fn publish_bound<O: Observer>(
    hours: Vec<segment::Hour<Native<O>>>,
    branch: u64,
    cuts: &[i32],
    resume: Option<i32>,
    bind: &mut dyn FnMut(&mut Emit<O>),
) -> Vec<Published> {
    let cache = Cache::new(&BUDGET);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(&BUDGET, identity(), first).unwrap();
    let mut emit = Emit::<O>::new(FormatVersion::CURRENT, branch).unwrap();
    bind(&mut emit);
    let mut published = Vec::with_capacity(cuts.len());
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        let root = publish(
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
        published.push(Published {
            pages: pages.clone(),
            root,
        });
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, &BUDGET, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(FormatVersion::CURRENT, branch, &directory, &saved).unwrap();
            bind(&mut emit);
        }
    }
    assert!(hours.next().is_none(), "the last cut is the last hour");
    published
}

impl Published {
    /// The scan the publication catalogues and its window `from..=to` of every name, read
    /// only through `through`, as a reply reads them.
    pub fn read<O: Observer>(
        &self,
        from: i32,
        to: i32,
        through: Option<i32>,
    ) -> (Scan<O>, KeyedWindow) {
        let cache = Cache::new(&BUDGET);
        let mut snapshot =
            Snapshot::open(self.pages.clone(), &cache, self.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let (scan, handles) = catalogued::<O>(&directory).unwrap();
        let keyed: Vec<(String, u64)> = scan
            .keyed()
            .map(|(name, key)| (name.to_owned(), key))
            .collect();
        let mut keys = vec![KNOWN, READ];
        keys.extend(keyed.iter().map(|(_, key)| *key));
        let runs = runs(&mut snapshot, &handles, &keys, from, to).unwrap();
        (scan, windowed(keyed, runs, from, to, through).unwrap())
    }

    /// Every scan key's changes over the ascending `hours` as the reader gives them, and as one
    /// lookup of each key at each hour gives them.
    pub fn changes<O: Observer>(&self, hours: &[i32]) -> [Vec<Vec<(usize, Option<i128>)>>; 2] {
        let cache = Cache::new(&BUDGET);
        let mut snapshot =
            Snapshot::open(self.pages.clone(), &cache, self.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let (scan, handles) = catalogued::<O>(&directory).unwrap();
        let mut keys = vec![KNOWN, READ];
        keys.extend(scan.keyed().map(|(_, key)| key));
        let read = changes(&mut snapshot, &handles, &keys, hours).unwrap();
        let looked = keys
            .iter()
            .map(|&key| {
                let mut changes = Vec::new();
                let Some(&handle) = handles.get(key as usize) else {
                    return changes;
                };
                let mut held = None;
                for (at, &hour) in hours.iter().enumerate() {
                    let now = match snapshot.value_at(handle, hour).unwrap() {
                        Observation::Value { run, .. } => Some(run.value),
                        _ => None,
                    };
                    if now != held {
                        changes.push((at, now));
                    }
                    held = now;
                }
                changes
            })
            .collect();
        [read, looked]
    }
}

/// The walk's scan of `frames`, each observed whole, and its pages' window `from..=to` of every
/// name.
pub fn walked<O: Observer>(frames: &[DecodedFrame], from: i32, to: i32) -> (Scan<O>, KeyedWindow) {
    let format = FormatVersion::CURRENT;
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    for frame in frames {
        let read = dashboard_text::metadata::slot_read(format, &frame.metrics, O::READ, frame.hour);
        scan.prepare(format, frame, read).unwrap();
    }
    let branch = frames[0].branch;
    let mut writer = fixture.writer();
    scan.flush(&mut writer, branch, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened = query::open::<O>(&fixture.store, &fixture.key, branch, &flag)
        .unwrap()
        .unwrap();
    let window = opened.window(from, to, None, |_| true).unwrap();
    (opened.scan, window)
}

/// Holds the publication of `frames` — segments starting at each of `starts`, cut after each
/// of `cuts`' hours and resumed at `resume` — to the walk's scan of them: the names in the
/// scan's order, and every name's runs, the known hours and the reads. The scans both read, the
/// native first.
pub fn assert_published_as_walked<O: Observer>(
    frames: &[DecodedFrame],
    write: fn(&mut Synthetic, &DecodedFrame),
    starts: &[usize],
    cuts: &[i32],
    resume: Option<i32>,
    label: &str,
) -> (Scan<O>, Scan<O>) {
    let (from, to) = (frames[0].hour, frames[frames.len() - 1].hour);
    let hours = operate::<O>(frames, starts, write);
    let published = publish_hours::<O>(hours, frames[0].branch, cuts, resume);
    let (native, window) = published.read::<O>(from, to, None);
    let (walked, expected) = walked::<O>(frames, from, to);
    assert_eq!(native.names(), walked.names(), "{label}: names");
    assert_eq!(window.known, expected.known, "{label}: known hours");
    assert_eq!(window.reads, expected.reads, "{label}: reads");
    assert_eq!(window.series.len(), expected.series.len(), "{label}: series");
    for (series, expected) in window.series.iter().zip(&expected.series) {
        assert_eq!(series.name, expected.name, "{label}: series order");
        assert_eq!(series.runs, expected.runs, "{label}: {}'s runs", series.name);
    }
    // Where segments start never changes what the observer counts.
    let observer = |starts: &[usize]| {
        let hours = operate::<O>(frames, starts, write);
        let published = publish_hours::<O>(hours, frames[0].branch, &[to], None);
        serde_json::to_value(&published.read::<O>(from, to, None).0.observer).unwrap()
    };
    assert_eq!(observer(starts), observer(&[]), "{label}: the observer");
    (native, walked)
}
