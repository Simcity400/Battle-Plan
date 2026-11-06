//! A keyed family's native publication held to the walk's scan of the same hours: the frames
//! written as the native recorder writes them, read by segments' operators, published with cuts
//! and a resume, and read back as a reply reads it; and the scan observing each frame whole.

use super::{Emit, Native, SCHEMA, catalogued, changes, runs};
use crate::{
    KNOWN, Observer, READ, Scan,
    query::{self, KeyedWindow, windowed},
    segment::{self, Segmented, needs},
};
use dashboard_series::{
    Blob, Budget, Cache, Identity, Observation, PageRead, Publication, Snapshot, Span, Writer,
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

const SECTION: &str = "native-keyed";

static BUDGET: Budget = Budget::new(256 << 20);

/// The recording and content a test binds its observers and keepers to, as the driver binds
/// them.
pub struct Binding<O: Observer> {
    content: std::sync::Arc<O::Content>,
}

impl<O: Observer> Binding<O> {
    pub fn new(
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
    ) -> std::io::Result<Self> {
        O::content(source, options, &mut || Ok(())).map(|content| Self {
            content: std::sync::Arc::new(content),
        })
    }
}

fn bound_operator<O: Observer>(binding: Option<&Binding<O>>) -> segment::Operator<Native<O>> {
    let format = FormatVersion::CURRENT;
    match binding {
        Some(binding) => Native::<O>::operator(format, &binding.content).unwrap(),
        None => segment::Operator::new(format).unwrap(),
    }
}

/// Writes `T`'s table of `frame` into `walk` as the recorder writes it: its rows, or why it is
/// unavailable, read where the frame's metrics read it.
pub fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = frame.tables[T::ID.index()]
        .sampled_at()
        .map_or(Sample::FromReadBit, Sample::Carried);
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
        _ => panic!("a synthetic walk writes complete, truncated or unavailable tables"),
    };
    walk.table(T::ID, input).unwrap();
}

/// `starts` (frame indices) with a segment starting after `resume` too, as a preparation that
/// resumes there goes on: at a segment's first hour, which gives every table whole.
pub fn resuming(frames: &[DecodedFrame], starts: &[usize], resume: Option<i32>) -> Vec<usize> {
    let mut starts = starts.to_vec();
    if let Some(after) = resume.and_then(|cut| frames.iter().position(|frame| frame.hour > cut)) {
        starts.push(after);
    }
    starts
}

/// The hours `frames` give `O`'s operators: a segment starts at each of `starts` (frame
/// indices), and the walk again after each gap. `write` writes a frame's tables.
pub fn operate<O: Observer>(
    frames: &[DecodedFrame],
    starts: &[usize],
    write: fn(&mut Synthetic, &DecodedFrame),
) -> Vec<segment::Hour<Native<O>>> {
    operate_bound::<O>(frames, starts, write, None)
}

/// [`operate`] with each segment's keeper bound to `binding`.
pub fn operate_bound<O: Observer>(
    frames: &[DecodedFrame],
    starts: &[usize],
    write: fn(&mut Synthetic, &DecodedFrame),
    binding: Option<&Binding<O>>,
) -> Vec<segment::Hour<Native<O>>> {
    operate_segmented(frames, starts, write, || bound_operator::<O>(binding))
}

/// The hours `frames` give the operators `operator` makes of the keyed family `S`: a segment
/// starts at each of `starts` (frame indices), and the walk again after each gap. `write` writes
/// a frame's tables.
pub fn operate_segmented<S: Segmented>(
    frames: &[DecodedFrame],
    starts: &[usize],
    write: fn(&mut Synthetic, &DecodedFrame),
    operator: impl Fn() -> segment::Operator<S>,
) -> Vec<segment::Hour<S>> {
    let format = FormatVersion::CURRENT;
    let needs = needs::<S>().unwrap();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut current = operator();
    let mut hours = Vec::new();
    for (ordinal, frame) in frames.iter().enumerate() {
        let gap = ordinal > 0 && frames[ordinal - 1].hour + 1 != frame.hour;
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(S::TABLES, frame.branch, frame.hour).unwrap());
        }
        let cold = ordinal == 0 || gap || starts.contains(&ordinal);
        if cold {
            current = operator();
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
            current
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
    // As the family's emit is handed them: written to the spool and read back.
    replay_spool::respool(hours).unwrap()
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

pub struct Stored {
    pub fixture: dashboard_base::prepared::series::tests::Fixture,
    root: u64,
}

impl Stored {
    pub fn open<'a, O: Observer>(&'a self, cancelled: &'a AtomicBool) -> query::Opened<'a, O> {
        let snapshot = self
            .fixture
            .store
            .session(&self.fixture.key)
            .unwrap()
            .unwrap()
            .snapshot();
        let opened = dashboard_base::prepared::published::Opened::from_snapshot(
            snapshot,
            SECTION.into(),
            self.root,
            identity(),
            SCHEMA,
            None,
        )
        .unwrap()
        .unwrap();
        query::published(opened, cancelled).unwrap()
    }
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
    let mut published = publish_bound::<O>(hours, branch, cuts, resume, None);
    published.pop().unwrap()
}

/// [`publish_hours`] by an emit bound to `binding`, new or loaded, as the driver binds it; the
/// publication as of each cut.
pub fn publish_bound<O: Observer>(
    hours: Vec<segment::Hour<Native<O>>>,
    branch: u64,
    cuts: &[i32],
    resume: Option<i32>,
    binding: Option<&Binding<O>>,
) -> Vec<Published> {
    let bind = |emit: &mut Emit<O>| {
        if let Some(binding) = binding {
            emit.bind(&binding.content).unwrap();
        }
    };
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
    pub fn store(mut self) -> Stored {
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        for id in 0..=self.root {
            let mut bytes = vec![0; self.pages.len(id).unwrap() as usize];
            self.pages.read(id, 0, &mut bytes).unwrap();
            assert_eq!(writer.write(SECTION, &bytes[..]).unwrap(), id);
        }
        writer.commit().unwrap();
        Stored {
            fixture,
            root: self.root,
        }
    }

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
    walked_bound::<O>(frames, from, to, None)
}

/// [`walked`] by an observer bound to `binding`.
pub fn walked_bound<O: Observer>(
    frames: &[DecodedFrame],
    from: i32,
    to: i32,
    binding: Option<&Binding<O>>,
) -> (Scan<O>, KeyedWindow) {
    let format = FormatVersion::CURRENT;
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    if let Some(binding) = binding {
        scan.observer.bind(&binding.content).unwrap();
    }
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
    assert_bound_published_as_walked::<O>(frames, write, starts, cuts, resume, label, None)
}

/// [`assert_published_as_walked`] with every observer and keeper bound to `binding`.
pub fn assert_bound_published_as_walked<O: Observer>(
    frames: &[DecodedFrame],
    write: fn(&mut Synthetic, &DecodedFrame),
    starts: &[usize],
    cuts: &[i32],
    resume: Option<i32>,
    label: &str,
    binding: Option<&Binding<O>>,
) -> (Scan<O>, Scan<O>) {
    let (from, to) = (frames[0].hour, frames[frames.len() - 1].hour);
    let hours = operate_bound::<O>(frames, &resuming(frames, starts, resume), write, binding);
    let published = publish_bound::<O>(hours, frames[0].branch, cuts, resume, binding);
    let (native, window) = published.last().unwrap().read::<O>(from, to, None);
    let (walked, expected) = walked_bound::<O>(frames, from, to, binding);
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
        let hours = operate_bound::<O>(frames, starts, write, binding);
        let published = publish_bound::<O>(hours, frames[0].branch, &[to], None, binding);
        serde_json::to_value(&published[0].read::<O>(from, to, None).0.observer).unwrap()
    };
    assert_eq!(observer(starts), observer(&[]), "{label}: the observer");
    (native, walked)
}
