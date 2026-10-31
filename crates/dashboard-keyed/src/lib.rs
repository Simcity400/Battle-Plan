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
//! query ([`query`]). On a native recording a family is read a segment at a
//! time instead ([`segment`]), and its emit publishes it on the series engine.
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
pub mod delta;
pub mod query;
pub mod regions;
pub mod segment;
pub mod table_changes;

#[cfg(test)]
mod tests;

use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    prepass::{Family, HeldBytes},
    series::Accumulator,
};
pub use delta::Delta;
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
pub const FIRST: u64 = 2;
/// About what one held value differing from its publication adds to the next catalogue.
const CHANGE_BYTES: usize = 2 * size_of::<u64>() + size_of::<i128>();

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

    pub fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.0.capacity() * size_of::<(String, i128)>());
        for (name, _) in &self.0 {
            bytes.heap(name);
        }
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

    /// Operand-only retention for observers that need no reconstructed frame during the fold.
    fn keeper() -> Option<Box<dyn block::Keeper<Self>>> {
        None
    }

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

    /// The hour as changes from the last known observation; Ok(None), touching nothing published, observes it whole.
    fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        names: &delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        let _ = (format, frame, names, delta);
        Ok(None)
    }

    /// What the observer retains between hours beyond what it publishes.
    fn held(&self, bytes: &mut HeldBytes) {
        let _ = bytes;
    }

    /// About what the observer's part of the catalogue gained since [`Self::published`].
    fn unpublished(&self) -> usize {
        0
    }

    /// The scan's flush published the catalogue.
    fn published(&mut self) {}
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
    #[serde(skip)]
    delta: Delta,
    #[serde(skip)]
    changed: BTreeMap<u64, Option<i128>>,
    /// Bytes of the names numbered since the last flush.
    #[serde(skip)]
    named: usize,
    /// Each held value changed since the last flush, with its value then.
    #[serde(skip)]
    dirty: HashMap<u64, Option<i128>>,
    /// How many of those differ from their value then.
    #[serde(skip)]
    differing: usize,
    #[serde(skip)]
    catalogue_pending: bool,
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

    /// The key of `name` if it has been numbered.
    fn numbered(&mut self, name: &str) -> Option<u64> {
        self.index_names();
        self.index.get(name).copied()
    }

    fn index_names(&mut self) {
        if self.index.len() != self.names.len() {
            self.index = self
                .names
                .iter()
                .enumerate()
                .map(|(at, name)| (name.clone(), FIRST + at as u64))
                .collect();
        }
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
        self.named = self.named.saturating_add(name.len() + size_of::<String>());
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
        self.catalogue_pending = true;
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
        self.catalogue_pending = true;
        self.observe_frame(format, frame, read)
    }

    fn observe_frame(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        read: bool,
    ) -> io::Result<()> {
        self.observe_input(frame.hour, read, &block::Borrowed { format, frame })
    }

    fn prepare_kept(
        &mut self,
        hour: i32,
        changes: &FrameChanges,
        input: &dyn block::Operands<O>,
    ) -> io::Result<()> {
        self.batch.advance(hour)?;
        self.catalogue_pending = true;
        let read = changes.slot_read[O::READ.index()];
        if self.open && self.continues_to(hour) && !O::changed(changes) {
            self.hold(hour, read);
            return Ok(());
        }
        self.observe_input(hour, read, input)
    }

    fn observe_input(
        &mut self,
        hour: i32,
        read: bool,
        input: &dyn block::Operands<O>,
    ) -> io::Result<()> {
        let continuous = self.continues_to(hour);
        let starts = !self.open || !continuous;
        self.last_hour = Some(hour);
        if !starts {
            let mut delta = std::mem::take(&mut self.delta);
            delta.clear();
            self.index_names();
            let observed =
                input.observe_changes(&mut self.observer, &delta::Names(&self.index), &mut delta);
            let done = match observed {
                Ok(Some(true)) => {
                    self.apply(hour, read, &delta);
                    Ok(true)
                }
                Ok(Some(false)) => {
                    self.refused(hour, continuous);
                    Ok(true)
                }
                Ok(None) => Ok(false),
                Err(error) => Err(error),
            };
            delta.clear();
            self.delta = delta;
            if done? {
                return Ok(());
            }
        }
        let mut values = std::mem::take(&mut self.values);
        values.clear();
        let known = input.observe(&mut self.observer, &mut values);
        let known = match known {
            Ok(known) => known,
            Err(error) => {
                self.values = values;
                return Err(error);
            }
        };
        if !known {
            self.refused(hour, continuous);
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
            let before = self.current.get(&key).copied();
            if starts || before != Some(value) {
                self.batch.observe(key, hour, Some(value), starts);
            }
            if before != Some(value) {
                self.touch(key, before, Some(value));
            }
        }
        let ended: Vec<(u64, i128)> = self
            .current
            .iter()
            .filter(|(key, _)| !next.contains_key(key))
            .map(|(&key, &value)| (key, value))
            .collect();
        for (key, value) in ended {
            self.batch.observe(key, hour, None, false);
            self.touch(key, Some(value), None);
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

    /// An hour whose inputs are not recorded: every name becomes unknown.
    fn refused(&mut self, hour: i32, continuous: bool) {
        if self.open {
            for key in [READ, KNOWN]
                .into_iter()
                .chain(self.current.keys().copied())
                .collect::<Vec<_>>()
            {
                self.batch.observe(key, hour, None, false);
            }
        }
        for (key, value) in std::mem::take(&mut self.current) {
            self.touch(key, Some(value), None);
        }
        // Keep the read only across consecutive refused hours, never a gap.
        self.refused_read = if continuous {
            self.last_read.or(self.refused_read)
        } else {
            None
        };
        self.open = false;
        self.last_read = None;
        self.batch.through(hour);
    }

    /// A held value changed: count it once while it differs from its last publication.
    fn touch(&mut self, key: u64, before: Option<i128>, now: Option<i128>) {
        let published = *self.dirty.entry(key).or_insert(before);
        match (before != published, now != published) {
            (false, true) => self.differing += 1,
            (true, false) => self.differing -= 1,
            _ => {}
        }
    }

    /// A known hour given as its changes, published exactly as its whole values.
    fn apply(&mut self, hour: i32, read: bool, delta: &Delta) {
        if read {
            self.last_read = Some(hour);
        }
        self.refused_read = None;
        let mut changed = std::mem::take(&mut self.changed);
        changed.clear();
        for name in &delta.removed {
            if let Some(key) = self.numbered(name) {
                changed.insert(key, None);
            }
        }
        for (name, value) in &delta.set {
            let key = self.number(name);
            changed.insert(key, Some(*value));
        }
        // A whole hour's values set what changed in key order, then end the rest.
        for (&key, value) in &changed {
            let before = self.current.get(&key).copied();
            if value.is_some() && *value != before {
                self.batch.observe(key, hour, *value, false);
                self.touch(key, before, *value);
            }
        }
        for (&key, value) in &changed {
            if let (None, Some(before)) = (value, self.current.get(&key).copied()) {
                self.batch.observe(key, hour, None, false);
                self.touch(key, Some(before), None);
            }
        }
        for (&key, &value) in &changed {
            match value {
                Some(value) => {
                    self.current.insert(key, value);
                }
                None => {
                    self.current.remove(&key);
                }
            }
        }
        changed.clear();
        self.changed = changed;
        if read {
            self.batch
                .observe(READ, hour, self.last_read.map(i128::from), false);
        }
        self.batch.through(hour);
        self.open = true;
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
        self.catalogue_pending = true;
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
        self.catalogue_pending |= self.open || self.refused_read.is_some();
        self.open = false;
        self.refused_read = None;
    }

    /// Pending series records and the catalogue text their next flush can publish.
    pub fn buffered_bytes(&self) -> usize {
        self.batch
            .bytes()
            .saturating_add(self.named)
            .saturating_add(self.differing.saturating_mul(CHANGE_BYTES))
            .saturating_add(self.observer.unpublished())
            .saturating_add(if self.catalogue_pending {
                dashboard_base::prepared::series::catalogue_bytes(&BTreeMap::new(), self)
            } else {
                0
            })
    }

    /// What it retains across flushes, which bounds the catalogue text a flush publishes.
    pub fn held_bytes(&self) -> usize {
        let mut bytes = HeldBytes::default();
        bytes
            .heap(&self.names)
            .table::<String, u64>(self.index.capacity())
            .tree::<u64, i128>(self.current.len())
            .tree::<u64, Option<i128>>(self.changed.len())
            .table::<u64, Option<i128>>(self.dirty.capacity());
        for name in self.index.keys() {
            bytes.heap(name);
        }
        self.values.held(&mut bytes);
        self.delta.held(&mut bytes);
        self.observer.held(&mut bytes);
        usize::try_from(bytes.total()).unwrap_or(usize::MAX)
    }

    /// Publish the batch and the catalogue (this scan).
    pub fn flush<W: dashboard_base::prepared::pages::Pages + ?Sized>(
        &mut self,
        writer: &mut W,
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
        )?;
        self.named = 0;
        self.dirty.clear();
        self.differing = 0;
        self.observer.published();
        self.catalogue_pending = false;
        Ok(())
    }
}

/// The section a keyed family's series pages are published in.
pub fn series_section(name: &str, branch: u64) -> String {
    format!("{name}-series-{branch}")
}
