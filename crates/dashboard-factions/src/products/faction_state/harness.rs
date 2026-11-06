//! Tests' preparation: hours read from whole frames ([`Walked`]) or a columnar walk, kept by the
//! family's emit in a publication of their own ([`Fixture`]), and the replies it gives.
use super::{
    FactionStateParams, FactionStateReply, LoadContext, columnar,
    emit::{BOOKS, Emit, LANES, SCHEMA},
    prepared::{Kind, Runtime, Supplying, Whole},
    query,
};
use dashboard_base::{
    Source, catalogue, metadata::check_cancel, prepared::engine, prepared::frame_changes::FrameChanges,
    prepared::prefix::Prefix,
};
use dashboard_series::{
    Blob, Identity, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay_frame::frame::DecodedFrame;
use std::{
    cell::RefCell,
    io,
    sync::atomic::AtomicBool,
};

fn identity() -> Identity {
    Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// Hours projected from whole frames, as each hour's own frame holds them, until a fixture
/// publishes them.
#[derive(Default)]
pub(super) struct Walked {
    pub(super) runtime: Runtime,
    hours: Vec<columnar::Hour>,
}

impl Walked {
    pub(super) fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut supplying = Supplying::new(source, frame.branch);
        self.observe_supplied(source, context, frame, changes, &mut supplying, cancelled)
    }

    /// [`Self::observe`] of one of the rising hours `supplying` reads programme operands for.
    pub(super) fn observe_supplied(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        supplying: &mut Supplying<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let cat = context
            .content
            .as_ref()
            .and_then(|options| catalogue::catalogue(source, options).ok());
        let projected = self.runtime.project(
            &Whole::new(frame, supplying),
            changes,
            cat.as_deref(),
            cancelled,
        )?;
        self.hours.push(columnar::Hour {
            branch: frame.branch,
            hour: frame.hour,
            projected,
        });
        Ok(())
    }

    /// An hour the columnar walk projected.
    pub(super) fn observe_projected(&mut self, hour: columnar::Hour) {
        self.hours.push(hour);
    }

    pub(super) fn country_rebuilds(&self) -> usize {
        self.runtime.country_rebuilds()
    }
}

struct Held {
    pages: Memory,
    emit: Emit,
    writer: Option<Writer>,
    root: Option<u64>,
    generation: u64,
    first: Option<i32>,
}

/// One branch's publication, and the emit going on with it.
pub(super) struct Fixture(RefCell<Held>);

impl Fixture {
    pub(super) fn new() -> Self {
        Self(RefCell::new(Held {
            pages: Memory::default(),
            emit: Emit::new(),
            writer: None,
            root: None,
            generation: 0,
            first: None,
        }))
    }

    /// Emits and publishes what `walked` projected since it last did, as a segment's cut.
    pub(super) fn publish(&self, walked: &mut Walked) {
        let mut held = self.0.borrow_mut();
        let held = &mut *held;
        let hours = std::mem::take(&mut walked.hours);
        let Some(last) = hours.last().map(|hour| hour.hour) else {
            return;
        };
        let first = *held.first.get_or_insert(hours[0].hour);
        let writer = held
            .writer
            .get_or_insert_with(|| Writer::new(identity(), first).unwrap());
        for hour in hours {
            held.emit.hour(writer, hour).unwrap();
        }
        held.emit.cut(writer, last).unwrap();
        held.generation += 1;
        let publication = Publication {
            generation: held.generation,
            segment: held.generation - 1,
            accepted: Span::new(first, last).unwrap(),
            reached: last,
            refusal: None,
        };
        held.root =
            Some(publish(writer, &mut held.pages, publication, Blob::EMPTY).unwrap());
    }

    /// The emit and writer going on from what was last published, as a later preparation
    /// takes them up.
    pub(super) fn reload(&self) {
        let mut held = self.0.borrow_mut();
        let held = &mut *held;
        let root = held.root.expect("a publication to go on from");
        let mut saved = Vec::new();
        held.emit.save(&mut saved).unwrap();
        held.writer =
            Some(Writer::resume(&mut held.pages, root, identity()).unwrap());
        let mut snapshot = Snapshot::open(held.pages.clone(), root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        held.emit = Emit::load(&directory, &saved).unwrap();
    }

    /// `params` answered through `prefix`, as the prepared route answers them.
    pub(super) fn reply(
        &self,
        source: &Source,
        params: &FactionStateParams,
        context: &LoadContext,
        prefix: Prefix,
        cancelled: &AtomicBool,
    ) -> io::Result<FactionStateReply> {
        let held = self.0.borrow();
        let Some(root) = held.root else {
            return query::unpublished(source, context, params, prefix, cancelled);
        };
        let mut snapshot = Snapshot::open(held.pages.clone(), root, identity())?;
        let directory = Directory::read(&mut snapshot, SCHEMA)?;
        let mut snapshot = snapshot.with_cancellation(cancelled);
        query::reply(source, context, params, &mut snapshot, &directory, prefix, cancelled)
    }

    /// Every lane the publication holds, in order, with its catalogue and runs, and every goal
    /// book.
    pub(super) fn dump(&self) -> Vec<String> {
        let held = self.0.borrow();
        let Some(root) = held.root else {
            return Vec::new();
        };
        let mut snapshot = Snapshot::open(held.pages.clone(), root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let accepted = snapshot.publication().accepted;
        let mut lanes = Vec::new();
        for (actor, named) in directory.actors.iter().enumerate() {
            let name = String::from_utf8_lossy(&named.name).into_owned();
            for kind in Kind::ALL {
                for &(ordinal, key) in directory.dimensions(actor as u64, kind.measure()) {
                    let runs: Vec<_> = engine::runs(&mut snapshot, key, accepted.first, accepted.last)
                        .unwrap()
                        .into_iter()
                        .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
                        .collect();
                    let meta = directory.scalar(LANES + ordinal);
                    lanes.push((ordinal, format!("{name} {kind:?} {meta:?} {runs:?}")));
                }
            }
            if let Some(book) = directory.scalar(BOOKS + actor as u64) {
                lanes.push((u64::MAX, format!("{name} book {book:?}")));
            }
        }
        lanes.sort();
        lanes.into_iter().map(|(_, lane)| lane).collect()
    }
}
