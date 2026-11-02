//! Frames walked as a native recording holds them, through the family's operator, emit and
//! publication: once in one segment, and once restarting every other hour, cut and resumed at
//! each restart, so a test's replies hold however the walk is cut.

use super::{
    Part,
    emit::{Emit, SCHEMA},
    facts::{self, Projector, TABLES},
};
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
    rows::{TableSchema, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let sample = Sample::FromReadBit;
    let read = true;
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
    write::<tables::Campaign>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::Politics>(walk, input);
    write::<tables::CountryStabilityInputs>(walk, input);
    write::<tables::SliceReads>(walk, input);
    write::<tables::PowerBalances>(walk, input);
    write::<tables::PowerBalanceMembers>(walk, input);
    write::<tables::CountryCivilWars>(walk, input);
    write::<tables::CountryCounterInputs>(walk, input);
    write::<tables::CountryWars>(walk, input);
    write::<tables::WarRelations>(walk, input);
}

/// The projected hours of `frames`, each run of consecutive hours a walk of its own, a
/// projector of its own starting a segment every `restart` hours of it; and the parts each read.
fn project(frames: &[DecodedFrame], restart: Option<usize>) -> (Vec<facts::Hour>, Vec<(i32, Part)>) {
    let format = FormatVersion::CURRENT;
    let mut hours = Vec::new();
    let mut observed = Vec::new();
    let mut at = 0;
    while at < frames.len() {
        let mut end = at + 1;
        while end < frames.len() && frames[end].hour == frames[end - 1].hour + 1 {
            end += 1;
        }
        let run = &frames[at..end];
        let mut walk = Synthetic::new(TABLES, 1, run[0].hour).unwrap();
        let mut held = None;
        let mut projector = Projector::new().unwrap();
        projector
            .start(Start::Run { continues: at > 0 }, 1, run[0].hour)
            .unwrap();
        let mut first = 0;
        for (ordinal, input) in run.iter().enumerate() {
            if ordinal > 0 && restart.is_some_and(|every| ordinal % every == 0) {
                projector.end(&mut hours).unwrap();
                observed.append(&mut projector.observed);
                projector = Projector::new().unwrap();
                projector
                    .start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
                first = ordinal;
            }
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, walk_starts| {
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
                            hour: input.hour,
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
        observed.append(&mut projector.observed);
        at = end;
    }
    (hours, observed)
}

fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: SCHEMA,
    }
}

/// The publication of `hours`, cut after each of `cuts`, the emit saved and resumed from the
/// publication at each but the last.
fn build(hours: Vec<facts::Hour>, cuts: &[i32]) -> (Memory, u64) {
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
        if Some(&cut) != cuts.last() {
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

/// One publication of a walk.
pub(crate) struct Published {
    pages: Memory,
    root: u64,
}

impl Published {
    pub(crate) fn read<T>(
        &self,
        read: impl FnOnce(&mut Snapshot<'_, Memory>, &Directory) -> T,
    ) -> T {
        let budget = Budget::new(1 << 30);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(self.pages.clone(), &cache, self.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        read(&mut snapshot, &directory)
    }
}

/// Frames to walk, in hour order.
#[derive(Default)]
pub(crate) struct Walk {
    frames: Vec<DecodedFrame>,
}

impl Walk {
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        assert!(
            self.frames.last().is_none_or(|last| last.hour < frame.hour),
            "frames walk in hour order"
        );
        self.frames.push(frame.clone());
    }

    /// The parts the operator read at each hour of one unbroken walk.
    pub(crate) fn observed(&self) -> Vec<(i32, Part)> {
        project(&self.frames, None).1
    }

    /// The walk published whole, and cut into segments every other hour.
    pub(crate) fn publish(&self) -> [Published; 2] {
        let last = self.frames.last().expect("a walk of hours").hour;
        let (whole, _) = project(&self.frames, None);
        let (cut, _) = project(&self.frames, Some(2));
        // Every other walked hour is cut, each cut but the last resumed from the publication.
        let mut cuts: Vec<i32> = self
            .frames
            .iter()
            .skip(1)
            .step_by(2)
            .map(|frame| frame.hour)
            .collect();
        cuts.push(last);
        cuts.dedup();
        [build(whole, &[last]), build(cut, &cuts)].map(|(pages, root)| Published { pages, root })
    }
}
