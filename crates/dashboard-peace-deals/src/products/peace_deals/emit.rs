//! The peace family's emit: the peace observer ([`super::prepared`]) stepped through each
//! projected hour, on the family's series publication. Each deal the observer completes and
//! each finalizer outcome it captures is an event at the hour it left the observer; at each cut
//! the observer's continuation is an event at the cut's last hour, which a reply joins with the
//! events before it ([`super::query`]).
//!
//! An hour that hands on no reads changed none: the observer takes the step of such hours at
//! once where it can, and observes each on the reads it holds otherwise.

use super::{
    facts,
    prepared::{Event, Prepared},
    reads::HourReads,
};
use dashboard_base::prepared::series::{compress, decompress};
use dashboard_series::{
    Cell, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    family::{Actors, Directory, Place},
};
use replay::container::Replay;
use std::io;

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's keys.
pub const FAMILY: u64 = u64::MAX;
/// The family's keys: the deals and finalizer outcomes, and the continuations.
pub const EVENTS: u64 = 0;
pub const CONTINUATION: u64 = 1;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds, each `[Text]`: a deal's or outcome's JSON, or a compressed continuation.
pub mod kind {
    pub const DEAL: u64 = 0;
    pub const FINALIZER: u64 = 1;
    pub const CONTINUATION: u64 = 2;
}

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// The family's emit on one branch.
pub struct Emit {
    prepared: Prepared,
    actors: Actors,
    /// EVENTS and CONTINUATION.
    keys: Option<[KeyHandle; 2]>,
    /// The observer holds reads: an hour that changed none can follow.
    observed: bool,
    /// Hours that changed no read, not yet stepped through: the first, the last and the
    /// roster's reads.
    quiet: Option<(i32, i32, Vec<i32>)>,
    events: Vec<Event>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    /// The emit of `branch` of `replay`.
    pub fn new(replay: &Replay, branch: u64) -> io::Result<Self> {
        Ok(Self::of(Prepared::new(replay, branch)?))
    }

    fn of(prepared: Prepared) -> Self {
        Self {
            prepared,
            actors: Actors::new(SCHEMA),
            keys: None,
            observed: false,
            quiet: None,
            events: Vec::new(),
            last: None,
            first: None,
        }
    }

    fn keys(&mut self, writer: &mut Writer) -> io::Result<[KeyHandle; 2]> {
        if let Some(keys) = self.keys {
            return Ok(keys);
        }
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([EVENTS, CONTINUATION]) {
            *key = self.actors.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
                (RunMode::Exact, 0),
                Some(DOMAIN),
            )?;
        }
        self.keys = Some(keys);
        Ok(keys)
    }

    /// `bytes` as an event of `kind` on `key` at `at`.
    fn event(
        writer: &mut Writer,
        key: KeyHandle,
        at: i32,
        kind: u64,
        bytes: &[u8],
    ) -> io::Result<()> {
        let text = writer.blob(&mut &bytes[..], bytes.len() as u64)?;
        let fields = writer.cells([Cell::Text(text)])?;
        let sequence = writer.sequence()?;
        writer.event(dashboard_series::Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind,
            fields,
        })
    }

    /// The events the observer left at `at`, in the order it left them.
    fn publish(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        if self.events.is_empty() {
            return Ok(());
        }
        let [key, _] = self.keys(writer)?;
        for event in self.events.drain(..) {
            let kind = match &event {
                Event::Deal { .. } => kind::DEAL,
                Event::Finalizer { .. } => kind::FINALIZER,
            };
            Self::event(writer, key, at, kind, &serde_json::to_vec(&event)?)?;
        }
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("peace.emit", None);
        let at = hour.hour;
        if self.last.is_some_and(|last| at <= last) {
            return Err(invalid("A peace hour does not follow the hour before"));
        }
        let follows = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        self.last = Some(at);
        self.first.get_or_insert(at);
        let Some(reads) = hour.reads else {
            if !follows || !self.observed {
                return Err(invalid("A peace hour without reads follows none"));
            }
            let (_, last, reads) = self.quiet.get_or_insert_with(|| (at, at, Vec::new()));
            *last = at;
            if hour.read {
                reads.push(at);
            }
            return Ok(());
        };
        self.settle(writer)?;
        self.prepared
            .observe_changed(at, *reads, hour.read, &mut self.events)?;
        self.observed = true;
        self.publish(writer, at)
    }

    /// Steps the observer through the hours after the last frame not yet stepped through.
    fn settle(&mut self, writer: &mut Writer) -> io::Result<()> {
        let Some((first, last, reads)) = self.quiet.take() else {
            return Ok(());
        };
        if self.prepared.advance_quiet(first, last, &reads) {
            return Ok(());
        }
        for at in first..=last {
            let read = reads.binary_search(&at).is_ok();
            self.prepared
                .observe_changed(at, HourReads::default(), read, &mut self.events)?;
            self.publish(writer, at)?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next: the observer's continuation, and the hours since the last cut as known.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.settle(writer)?;
        let [_, key] = self.keys(writer)?;
        let continuation = self.prepared.continuation()?;
        Self::event(writer, key, last, kind::CONTINUATION, &continuation)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut: the observer and the last hour kept. Its next
    /// hour hands every read on, as a segment's first does.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if self.quiet.is_some() {
            return Err(invalid("A peace emit is saved with hours not stepped through"));
        }
        out.extend(compress(&serde_json::to_vec(&(&self.prepared, self.last))?, 3)?);
        Ok(())
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn resume(directory: &Directory, saved: &[u8]) -> io::Result<Self> {
        let (prepared, last): (Prepared, Option<i32>) =
            serde_json::from_slice(&decompress(saved)?)?;
        let mut emit = Self::of(prepared);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([EVENTS, CONTINUATION]) {
            *key = directory
                .key(FAMILY, measure, None)
                .ok_or_else(|| invalid("Saved peace emit is not valid"))?;
        }
        emit.keys = Some(keys);
        emit.last = last;
        Ok(emit)
    }
}
