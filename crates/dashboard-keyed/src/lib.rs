//! Keyed observation families: a family prepared on its own whose observer
//! turns each hour's frame into a set of named exact values (a region's
//! planes on one mission, a country's mines in one sea, one channel of one
//! region's air statistics), and whose prepared pages hold each name's runs.
//!
//! One crate above `dashboard-base` shared by the
//! product crates whose widgets look values up at playback: each declares an
//! [`Observer`] (its tables, the fields it watches, the table that dates its
//! reads and how one frame reads), and this crate does the rest the same way
//! for all of them: the block pre-pass extract and fold ([`block`]), the
//! carried scan with its series and catalogue ([`Scan`]), and the reads of a
//! query ([`query`]).
//!
//! What the pages say, for every name: a run holds the exact value the
//! observer read from its first to its last hour; an hour inside a known
//! observation (the [`KNOWN`] key) with no run holds no value for that name
//! (the observer read its inputs and the name was not among them); an hour
//! outside a known observation is not recorded. The [`READ`] key holds, at
//! each known hour, the hour of the supplying read the values come from, or
//! nothing when the read before a recording's first hour is not known.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod block;
pub mod countries;
pub mod daily;
pub mod query;
pub mod regions;

#[cfg(test)]
mod tests;

use dashboard_base::prepared::{frame_changes::FrameChanges, prepass::Family, series::Accumulator};
use replay::container::TableFields;
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};

/// The series key holding each known hour's supplying read hour.
pub const READ: u64 = 0;
/// The series key holding 1 at every hour whose inputs the observer read.
pub const KNOWN: u64 = 1;
/// The first key a name is numbered with.
const FIRST: u64 = 2;

/// The values one hour reads, by name. Names are the observer's own keys
/// (`series::field_key` of their parts, for instance); a name read twice in
/// one hour keeps its last value.
#[derive(Default)]
pub struct Values(Vec<(String, i128)>);

impl Values {
    pub fn push(&mut self, name: String, value: i128) {
        self.0.push((name, value));
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, i128)> {
        self.0.iter().map(|(name, value)| (name.as_str(), *value))
    }
}

/// One keyed family's observation of a frame.
pub trait Observer: Default + Serialize + DeserializeOwned + Send + 'static {
    /// The family's store name: its catalogue is `{NAME}-{branch}-catalogue`
    /// and its series `{NAME}-series-{branch}`.
    const NAME: &'static str;
    /// The tables an observation reads.
    const TABLES: &'static [TableId];
    /// The table whose supplying read dates an observation.
    const READ: TableId;

    /// The family's pre-pass entry ([`block::family`] builds it).
    fn family() -> &'static Family;

    /// The fields of [`Self::TABLES`] whose changes make an hour observed.
    fn fields() -> io::Result<Vec<TableFields>>;

    /// Whether the hour's changes reach the observer's inputs; an hour that
    /// follows a known one and does not holds that hour's values.
    fn changed(changes: &FrameChanges) -> bool {
        Self::TABLES
            .iter()
            .any(|table| changes.rows_changed[table.index()])
    }

    /// Binds what an observation needs from the recording's content before a
    /// block is folded or observed (a branch restored from its catalogue binds
    /// again); most observers need nothing. `check` stops a cancelled run.
    fn bind(
        &mut self,
        source: &dashboard_base::Source,
        content: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let _ = (source, content, check);
        Ok(())
    }

    /// The supplying read of the values the last [`Self::observe`] returned,
    /// when the observer itself holds that evidence. A scan resuming straight
    /// after refused hours dates its values by it, but only when it is the read
    /// the scan last held; None (the default) leaves a resumed observation
    /// undated until a fresh read.
    fn supplying_read(&self) -> Option<i32> {
        None
    }

    /// The values `frame` reads. Ok(false) when its inputs are not recorded
    /// at this hour: every name is then unknown, not absent.
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool>;
}

/// The carried state of one keyed family on one branch: its observer, the
/// names numbered so far, the values of the last hour observed and the
/// series accumulator. Its catalogue is this scan, so a branch continues
/// from what its last flush published.
#[derive(Default, Serialize, Deserialize)]
#[serde(bound = "O: Observer")]
pub struct Scan<O> {
    pub observer: O,
    /// Each numbered name, by key from [`FIRST`].
    names: Vec<String>,
    #[serde(skip)]
    index: HashMap<String, u64>,
    current: BTreeMap<u64, i128>,
    last_hour: Option<i32>,
    last_read: Option<i32>,
    /// The read held when consecutive refused hours closed the observation;
    /// a resumption straight after them may date its values by it.
    #[serde(default)]
    refused_read: Option<i32>,
    open: bool,
    batch: Accumulator,
    #[serde(skip)]
    values: Values,
}

impl<O: Observer> Scan<O> {
    /// Every numbered name, in key order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The series key of `name`, if it has been read.
    pub fn key(&self, name: &str) -> Option<u64> {
        self.names
            .iter()
            .position(|known| known == name)
            .map(|at| FIRST + at as u64)
    }

    /// Names and their series keys, in key order.
    pub fn keyed(&self) -> impl Iterator<Item = (&str, u64)> {
        self.names
            .iter()
            .enumerate()
            .map(|(at, name)| (name.as_str(), FIRST + at as u64))
    }

    fn number(&mut self, name: &str) -> u64 {
        if self.index.len() != self.names.len() {
            self.index = self
                .names
                .iter()
                .enumerate()
                .map(|(at, name)| (name.clone(), FIRST + at as u64))
                .collect();
        }
        if let Some(&key) = self.index.get(name) {
            return key;
        }
        let key = FIRST + self.names.len() as u64;
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), key);
        key
    }

    /// Observe one accepted hour with the change facts against the hour
    /// before it.
    pub fn prepare_changed(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let hour = frame.hour;
        self.batch.advance(hour)?;
        let read = changes.slot_read[O::READ.index()];
        if self.open && self.continues_to(hour) && !O::changed(changes) {
            self.hold(hour, read);
            return Ok(());
        }
        self.observe_frame(format, frame, read)
    }

    /// Observe one hour whatever changed (a test's or oracle's own walk).
    pub fn prepare(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        read: bool,
    ) -> io::Result<()> {
        self.batch.advance(frame.hour)?;
        self.observe_frame(format, frame, read)
    }

    fn observe_frame(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        read: bool,
    ) -> io::Result<()> {
        let hour = frame.hour;
        let continuous = self.continues_to(hour);
        let starts = !self.open || !continuous;
        self.last_hour = Some(hour);
        let mut values = std::mem::take(&mut self.values);
        values.clear();
        let known = self.observer.observe(format, frame, &mut values);
        let known = match known {
            Ok(known) => known,
            Err(error) => {
                self.values = values;
                return Err(error);
            }
        };
        if !known {
            if self.open {
                for key in [READ, KNOWN]
                    .into_iter()
                    .chain(self.current.keys().copied())
                    .collect::<Vec<_>>()
                {
                    self.batch.observe(key, hour, None, false);
                }
            }
            self.current.clear();
            // Keep the read only across consecutive refused hours, never a gap.
            self.refused_read = if continuous {
                self.last_read.or(self.refused_read)
            } else {
                None
            };
            self.open = false;
            self.last_read = None;
            self.batch.through(hour);
            self.values = values;
            return Ok(());
        }
        if read {
            self.last_read = Some(hour);
        } else if starts {
            // The values carry an earlier read: the one held before refused hours
            // just ended, when the observer vouches for it; otherwise undated.
            self.last_read = self
                .refused_read
                .filter(|&held| continuous && self.observer.supplying_read() == Some(held));
        }
        self.refused_read = None;
        let mut next = BTreeMap::new();
        for (name, value) in values.iter() {
            let key = self.number(name);
            next.insert(key, value);
        }
        self.values = values;
        for (&key, &value) in &next {
            if starts || self.current.get(&key) != Some(&value) {
                self.batch.observe(key, hour, Some(value), starts);
            }
        }
        for &key in self.current.keys() {
            if !next.contains_key(&key) {
                self.batch.observe(key, hour, None, false);
            }
        }
        if starts {
            self.batch.observe(KNOWN, hour, Some(1), true);
        }
        if read || starts {
            self.batch
                .observe(READ, hour, self.last_read.map(i128::from), starts);
        }
        self.batch.through(hour);
        self.current = next;
        self.open = true;
        Ok(())
    }

    /// Whether `hour` follows the last hour observed.
    pub fn continues_to(&self, hour: i32) -> bool {
        self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour)
    }

    /// An hour after the last one observed at which nothing the observer
    /// reads changed: the held values extend to it, and a supplying read
    /// there dates them again.
    pub fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<()> {
        self.batch.advance(hour)?;
        self.hold(hour, read);
        Ok(())
    }

    fn hold(&mut self, hour: i32, read: bool) {
        self.last_hour = Some(hour);
        if !self.open {
            return;
        }
        if read {
            self.last_read = Some(hour);
            self.batch
                .observe(READ, hour, Some(i128::from(hour)), false);
        }
    }

    /// A block that gave no hour ends the observation: the next hour starts
    /// a new one.
    pub fn unreadable_block(&mut self) {
        self.open = false;
        self.refused_read = None;
    }

    /// What the accumulator holds until its next flush.
    pub fn buffered_bytes(&self) -> usize {
        self.batch.bytes()
    }

    /// Publish the batch and the catalogue (this scan).
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.batch
            .flush(writer, &series_section(O::NAME, branch), 0, cancelled)?;
        dashboard_base::prepared::series::write_catalogue(
            writer,
            O::NAME,
            branch,
            &BTreeMap::new(),
            self,
        )
    }
}

/// The section a keyed family's series pages are published in.
pub fn series_section(name: &str, branch: u64) -> String {
    format!("{name}-series-{branch}")
}
