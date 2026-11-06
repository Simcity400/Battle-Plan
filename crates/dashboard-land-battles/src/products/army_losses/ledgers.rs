//! What each retained own-ledger month holds at a reached hour.
//!
//! CombatLogEquipment is read country by country. An ordinary read holds only
//! the last two month entries of each array, the only ones that can still
//! change; a full read holds every retained entry. A full hour (the branch's
//! first, or the hour after a gap) reads every country in full, and a read
//! holding a row of an older entry is full. An older month therefore leaves
//! the table while the game still keeps it.
//!
//! A month's amounts are known at hour H when a read at or before H covered
//! it and every hour from that read to H was read: they are what that read
//! found, its rows or none. One backward walk over the branch's blocks from H
//! ([`fold`]) settles every older month of a reply together. Each block gives
//! its index ([`Provenance`]): for every country's month entries, the latest
//! read that covered them and what it found, and the hours the block did not
//! read. The walk keeps nothing of a block once it is processed, and stops when
//! every month is settled or an unread hour leaves the rest unknown. A month
//! written after its country's latest read, and one no read covered, are
//! unknown too, never zero. A block whose row frames leave these tables to the
//! native capture stops the walk too: what it still asks is not prepared from
//! native recordings yet, never unread.
//!
//! An entry is told apart by its first write, not only its calendar month: a
//! retention below one deletes the current month's entry each day, and a later
//! loss that month writes a new one whose rows its predecessor's never stand for.
use super::month_key;
use dashboard_base::{
    prepared::{
        self,
        query::{native, observations},
    },
    source::Source,
};
use dashboard_text::metadata::check_cancel;
use replay::container::{BlockChange, BlockIndex, TableFields};
use replay_data::{
    rows::{CombatLogEquipmentRow, CombatLogMonthRow, SliceReadRow, TableId, tables},
    sliced::read_hour,
};
use replay_frame::frame::DecodedFrame;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// What is known of one retained month's own-equipment amounts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Witness {
    /// The rows of the latest read that covered the month.
    Rows(Vec<CombatLogEquipmentRow>),
    /// The latest read that covered the month held no amounts of it.
    Empty,
    /// Why the month's amounts are unknown.
    Unknown(&'static str),
}

pub const NOT_READ: &str = "Combat Log equipment of this hour was not read";
const NO_READ_YET: &str = "Combat Log equipment months of a country not read yet are unknown";
const AFTER_READ: &str = "Combat Log equipment months written after their country's latest read are unknown";
const EARLIER_UNREAD: &str = "Combat Log equipment months whose later reads were not all recorded are unknown";
const UNWITNESSED: &str = "Combat Log equipment months no read covered are unknown";
const UNPREPARED: &str = "Older Combat Log equipment months are not prepared from native recordings yet";
const OTHER_ENTRY: &str = "Combat Log equipment rows of another entry of the same month do not witness this one";

/// The tables a read's coverage is known from.
const READ: [TableId; 3] = [TableId::CombatLogEquipment, TableId::CombatLogMonths, TableId::SliceReads];

/// A month entry of one country's log: its group's reference id, its ledger
/// and its first write, before which no read can cover it.
pub type Entry = (u32, u8, i32);

/// One older month a reply asks about: its country's raw id and its entry.
pub type Wanted = (u32, Entry);

/// What is known at the reached hour of one wanted month.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// [`Witness::Rows`] or [`Witness::Empty`].
    Found(Witness),
    /// No read can be accepted; `complete` when every hour back to the
    /// month's first write was read, so no read covered it.
    Unwitnessed { complete: bool },
    /// The walk reached a block whose reads only the native capture holds.
    Unprepared,
}

/// Where earlier reads of the own ledgers are found.
pub trait History {
    /// What is known at the reached hour of each wanted month, in order.
    fn latest(&mut self, wanted: &[Wanted]) -> io::Result<Vec<Lookup>>;
}

/// An hour's CombatLogEquipment rows and the hour each country's were read
/// at, when both were read.
fn sampled(frame: &DecodedFrame) -> Option<(&[CombatLogEquipmentRow], &[SliceReadRow])> {
    Some((
        frame.complete::<tables::CombatLogEquipment>().ok()?,
        frame.complete::<tables::SliceReads>().ok()?,
    ))
}

fn entry_of(month: &CombatLogMonthRow) -> Entry {
    (month.group, month.kind, month.date)
}

/// The rows of one month entry, in key order; none when its calendar month's
/// rows are another entry's, by their first write.
fn rows_of<'a>(rows: &'a [CombatLogEquipmentRow], month: &CombatLogMonthRow) -> Option<&'a [CombatLogEquipmentRow]> {
    let key = (month.country, month.group, month.kind, month_key(month.date));
    let entry = |row: &CombatLogEquipmentRow| (row.country, row.group, row.kind, row.month);
    let start = rows.partition_point(|row| entry(row) < key);
    let end = start + rows[start..].partition_point(|row| entry(row) == key);
    let held = &rows[start..end];
    held.iter().all(|row| row.date == month.date).then_some(held)
}

/// The rows of `rows`, sorted by country first, that belong to `country`.
fn owned<T>(rows: &[T], country: u32, of: impl Fn(&T) -> u32) -> &[T] {
    let start = rows.partition_point(|row| of(row) < country);
    let end = start + rows[start..].partition_point(|row| of(row) == country);
    &rows[start..end]
}

/// The first-write hour from which each array's entries are among its last
/// two, in `months`, one country's retained months at a read: the entries any
/// read holds.
fn recent_from(months: &[CombatLogMonthRow]) -> BTreeMap<(u32, u8), i32> {
    let mut arrays = BTreeMap::<(u32, u8), [i32; 2]>::new();
    for month in months {
        let last = arrays.entry((month.group, month.kind)).or_insert([i32::MIN; 2]);
        if month.date > last[1] {
            *last = [last[1], month.date];
        } else if month.date > last[0] {
            last[0] = month.date;
        }
    }
    arrays.into_iter().map(|(array, [second, _])| (array, second)).collect()
}

/// Each month's witness, in the order given. `months` are one country's
/// retained own-ledger months at the reached hour, whose `frame` holds that
/// hour's CombatLogEquipment and SliceReads. The months the latest read could
/// not hold are settled together through `history`.
pub fn witnesses(
    frame: &DecodedFrame,
    months: &[&CombatLogMonthRow],
    history: &mut dyn History,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Witness>> {
    let Some((rows, reads)) = sampled(frame) else {
        return Ok(vec![Witness::Unknown(NOT_READ); months.len()]);
    };
    // The first-write hours of each array's entries, oldest first. Entries
    // newer than a retained one are never pruned before it.
    let mut arrays = BTreeMap::<(u32, u32, u8), Vec<i32>>::new();
    for month in months {
        arrays.entry((month.country, month.group, month.kind)).or_default().push(month.date);
    }
    for dates in arrays.values_mut() {
        dates.sort_unstable();
    }
    let mut out = Vec::with_capacity(months.len());
    let mut older = Vec::new();
    for (index, month) in months.iter().enumerate() {
        check_cancel(cancelled)?;
        let Some(read) = read_hour(reads, TableId::CombatLogEquipment, month.country) else {
            out.push(Witness::Unknown(NO_READ_YET));
            continue;
        };
        let held = rows_of(rows, month);
        if let Some(held) = held
            && !held.is_empty()
        {
            out.push(Witness::Rows(held.to_vec()));
            continue;
        }
        if month.date > read {
            out.push(Witness::Unknown(AFTER_READ));
            continue;
        }
        if held.is_none() {
            out.push(Witness::Unknown(OTHER_ENTRY));
            continue;
        }
        // Any read holds its arrays' last two entries: a month among them that
        // the latest read held no rows of has none.
        let dates = &arrays[&(month.country, month.group, month.kind)];
        let newer = dates[dates.partition_point(|&date| date <= month.date)..]
            .iter()
            .take_while(|&&date| date <= read)
            .count();
        out.push(if newer <= 1 { Witness::Empty } else { Witness::Unknown(UNWITNESSED) });
        if newer > 1 {
            older.push((index, (month.country, entry_of(month))));
        }
    }
    let wanted: Vec<Wanted> = older.iter().map(|&(_, wanted)| wanted).collect();
    for ((index, _), lookup) in older.into_iter().zip(history.latest(&wanted)?) {
        out[index] = match lookup {
            Lookup::Found(witness) => witness,
            Lookup::Unwitnessed { complete: true } => Witness::Unknown(UNWITNESSED),
            Lookup::Unwitnessed { complete: false } => Witness::Unknown(EARLIER_UNREAD),
            Lookup::Unprepared => Witness::Unknown(UNPREPARED),
        };
    }
    Ok(out)
}

/// One block's index of every country's own ledgers up to an hour: for each
/// month entry a read covered, the latest such read and what it found (its
/// rows, or none), and the spans of the block's hours not recorded with the
/// tables a read's coverage is known from.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    #[allow(clippy::type_complexity)]
    covered: BTreeMap<(u32, Entry), (i32, Option<Vec<CombatLogEquipmentRow>>)>,
    unread: Vec<(i32, i32)>,
    /// The block's row frames leave those tables to the native capture.
    native: bool,
}

/// A block's hours observed in order, folded into its [`Provenance`].
struct Builder {
    out: Provenance,
    /// The countries indexed; all when none.
    owners: Option<BTreeSet<u32>>,
    /// The last hour indexed.
    until: i32,
    /// The next hour a continuous block records.
    expected: Option<i32>,
    /// Whether the next observed hour is a full hour.
    full: bool,
    /// Whether the last observed frame had every table read; quiet hours keep it.
    read: bool,
}

impl Builder {
    fn new(first: i32, until: i32, full_start: bool, owners: Option<BTreeSet<u32>>) -> Self {
        Self { out: Provenance::default(), owners, until, expected: Some(first), full: full_start, read: true }
    }

    /// Note a missing span before `hour`, after which the hour is a full one;
    /// say whether `hour` is a full hour.
    fn hour(&mut self, hour: i32) -> bool {
        if let Some(expected) = self.expected
            && expected < hour
        {
            self.out.unread(expected, hour - 1);
            self.full = true;
        }
        self.expected = hour.checked_add(1);
        std::mem::take(&mut self.full)
    }

    /// Index a changed hour; false once past the last hour indexed.
    fn changed(&mut self, frame: &DecodedFrame) -> bool {
        if frame.hour > self.until {
            return false;
        }
        if READ.into_iter().any(|table| native::left_to_native(frame, table)) {
            self.out.native = true;
            return false;
        }
        let full = self.hour(frame.hour);
        let (Some((rows, reads)), Ok(months)) = (sampled(frame), frame.complete::<tables::CombatLogMonths>()) else {
            self.read = false;
            self.out.unread(frame.hour, frame.hour);
            return true;
        };
        self.read = true;
        let table = TableId::CombatLogEquipment.index() as u32;
        let first = reads.partition_point(|read| read.table < table);
        let last = first + reads[first..].partition_point(|read| read.table == table);
        for read in reads[first..last].iter().filter(|read| read.sample_hour == frame.hour) {
            if self.owners.as_ref().is_none_or(|owners| owners.contains(&read.owner)) {
                self.covered(read.owner, frame.hour, full, rows, months);
            }
        }
        true
    }

    /// Index quiet hours; false once past the last hour indexed.
    fn quiet(&mut self, hours: &[i32]) -> bool {
        for &hour in hours {
            if hour > self.until {
                return false;
            }
            self.hour(hour);
            if !self.read {
                self.out.unread(hour, hour);
            }
        }
        true
    }

    /// What `owner`'s read at `hour` covered: its arrays' last two entries,
    /// or every retained entry when the read is full.
    fn covered(&mut self, owner: u32, hour: i32, full_hour: bool, rows: &[CombatLogEquipmentRow], months: &[CombatLogMonthRow]) {
        let rows = owned(rows, owner, |row| row.country);
        let months: Vec<_> = owned(months, owner, |month| month.country)
            .iter()
            .filter(|month| month.date <= hour)
            .cloned()
            .collect();
        let from = recent_from(&months);
        let recent = |month: &CombatLogMonthRow| month.date >= from[&(month.group, month.kind)];
        let full = full_hour
            || months.iter().any(|month| !recent(month) && rows_of(rows, month).is_some_and(|held| !held.is_empty()));
        for month in months.iter().filter(|month| full || recent(month)) {
            if let Some(held) = rows_of(rows, month) {
                self.out.covered.insert((owner, entry_of(month)), (hour, (!held.is_empty()).then(|| held.to_vec())));
            }
        }
    }

    fn finish(mut self, last: i32) -> Provenance {
        let last = last.min(self.until);
        if let Some(expected) = self.expected
            && expected <= last
        {
            self.out.unread(expected, last);
        }
        self.out
    }
}

impl Provenance {
    fn unread(&mut self, first: i32, last: i32) {
        match self.unread.last_mut() {
            Some(span) if span.1.checked_add(1) == Some(first) => span.1 = last,
            _ => self.unread.push((first, last)),
        }
    }

    /// The index of `frames`, a block's recorded hours from `first` to `last`
    /// in order, each observed as changed; `full_start` when the block's first
    /// hour is a full hour.
    pub fn read(frames: &[DecodedFrame], first: i32, last: i32, full_start: bool) -> Self {
        let mut builder = Builder::new(first, last, full_start, None);
        for frame in frames {
            builder.changed(frame);
        }
        builder.finish(last)
    }

    /// The index of a recorded block up to `until`, of `owners` (all when
    /// none), streamed from its changes: only the hours its watched tables
    /// change are reconstructed, and the walk stops past `until`. A block that
    /// cannot be read is unread throughout.
    pub fn stream(
        source: &Source,
        block: &BlockIndex,
        until: i32,
        full_start: bool,
        owners: Option<BTreeSet<u32>>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let fields = READ.map(TableFields::all);
        let mut builder = Builder::new(block.first_hour, until, full_start, owners);
        let mut past = false;
        let read = source.replay_ref().read_block_changes(
            block,
            &fields,
            |change| {
                let more = match change {
                    BlockChange::Changed { frame, .. } => builder.changed(frame),
                    BlockChange::Quiet(quiet) => builder.quiet(quiet.hours()),
                };
                if more {
                    Ok(())
                } else {
                    past = true;
                    Err(io::Error::other("past the hour indexed"))
                }
            },
            || check_cancel(cancelled),
        );
        if read.is_err() && !past {
            check_cancel(cancelled)?;
            let mut out = Self::default();
            out.unread(block.first_hour, block.last_hour.min(until));
            return Ok(out);
        }
        Ok(builder.finish(block.last_hour))
    }

    /// The spans of the block's hours that were not read.
    pub fn unread_hours(&self) -> &[(i32, i32)] {
        &self.unread
    }

    /// The latest read in the block that covered the entry, and what it found.
    pub fn covering(&self, owner: u32, entry: Entry) -> Option<(i32, Witness)> {
        let (hour, rows) = self.covered.get(&(owner, entry))?;
        Some((*hour, rows.clone().map_or(Witness::Empty, Witness::Rows)))
    }
}

/// What is known at `until` of each wanted month, from the index of each
/// block of the branch, newest first, with its first and last hours (the
/// newest indexed only to `until`). One backward walk: a month takes the
/// latest read that covered it when every hour from that read to `until` was
/// read, within its block, in each newer block and between them. The walk ends
/// at the first unread hour, which leaves every month still unsettled unknown,
/// at the first block left to the native capture, which leaves them
/// [`Lookup::Unprepared`], and once every month is settled; each block's index
/// is released once processed.
pub fn fold(
    blocks: impl IntoIterator<Item = io::Result<((i32, i32), Arc<Provenance>)>>,
    wanted: &[Wanted],
) -> io::Result<Vec<Lookup>> {
    let mut out: Vec<Option<Lookup>> = vec![None; wanted.len()];
    let mut left = wanted.len();
    let mut newer: Option<i32> = None;
    let mut broken = false;
    let mut blocks = blocks.into_iter();
    // Only an unsettled month pulls, and so reads, an older block.
    while left > 0 {
        let Some(block) = blocks.next() else {
            break;
        };
        let ((first, last), provenance) = block?;
        if newer.is_some_and(|newer| last.checked_add(1) != Some(newer)) {
            broken = true;
            break;
        }
        newer = Some(first);
        // Months first written after this block cannot be covered earlier.
        for (slot, &(_, (_, _, since))) in out.iter_mut().zip(wanted) {
            if slot.is_none() && since > last {
                *slot = Some(Lookup::Unwitnessed { complete: true });
                left -= 1;
            }
        }
        if provenance.native {
            for slot in out.iter_mut().filter(|slot| slot.is_none()) {
                *slot = Some(Lookup::Unprepared);
            }
            break;
        }
        let unread = provenance.unread.iter().map(|&(_, to)| to).max();
        for (slot, &(owner, entry)) in out.iter_mut().zip(wanted) {
            if slot.is_some() {
                continue;
            }
            if let Some((hour, witness)) = provenance.covering(owner, entry) {
                *slot = Some(if unread.is_some_and(|unread| unread > hour) {
                    Lookup::Unwitnessed { complete: false }
                } else {
                    Lookup::Found(witness)
                });
                left -= 1;
            }
        }
        if unread.is_some() {
            broken = true;
            break;
        }
    }
    Ok(out.into_iter().map(|slot| slot.unwrap_or(Lookup::Unwitnessed { complete: !broken })).collect())
}

/// Earlier reads of one branch up to the reached hour.
pub struct Recorded<'a> {
    source: &'a Source,
    branch: u64,
    until: i32,
    cancelled: &'a AtomicBool,
    binding: Option<store::SourceKey>,
}

impl<'a> Recorded<'a> {
    pub fn new(source: &'a Source, branch: u64, until: i32, cancelled: &'a AtomicBool) -> Self {
        Self {
            source,
            branch,
            until,
            cancelled,
            binding: prepared::files(source, None).ok().map(|(_, key)| key),
        }
    }

    /// A block's index up to the reached hour. A block ending by then is
    /// indexed whole for every country and kept in the observation cache under
    /// the recording's binding; the block holding the hour is indexed to it,
    /// for `owners` only. Its first hour is a full hour when no block of the
    /// branch ends just before it.
    fn provenance(&self, block: &BlockIndex, owners: &BTreeSet<u32>) -> io::Result<Arc<Provenance>> {
        let (source, branch, cancelled) = (self.source, self.branch, self.cancelled);
        let full_start = !source
            .replay_ref()
            .index
            .iter()
            .any(|other| other.branch == branch && other.last_hour.checked_add(1) == Some(block.first_hour));
        if block.last_hour > self.until {
            return Provenance::stream(source, block, self.until, full_start, Some(owners.clone()), cancelled).map(Arc::new);
        }
        let build = || Provenance::stream(source, block, block.last_hour, full_start, None, cancelled);
        match &self.binding {
            Some(binding) => observations::scope(source, "army-losses-provenance", Some(binding))
                .get(block, &(), cancelled, build),
            None => build().map(Arc::new),
        }
    }
}

impl History for Recorded<'_> {
    fn latest(&mut self, wanted: &[Wanted]) -> io::Result<Vec<Lookup>> {
        let Some(since) = wanted.iter().map(|&(_, (_, _, since))| since).min() else {
            return Ok(Vec::new());
        };
        let owners: BTreeSet<u32> = wanted.iter().map(|&(owner, _)| owner).collect();
        let (source, until) = (self.source, self.until);
        let mut blocks: Vec<&BlockIndex> = source
            .replay_ref()
            .index
            .iter()
            .filter(|block| block.branch == self.branch && block.first_hour <= until && block.last_hour >= since)
            .collect();
        blocks.sort_by_key(|block| Reverse(block.first_hour));
        fold(
            blocks.into_iter().map(|block| {
                check_cancel(self.cancelled)?;
                Ok(((block.first_hour, block.last_hour.min(until)), self.provenance(block, &owners)?))
            }),
            wanted,
        )
    }
}
