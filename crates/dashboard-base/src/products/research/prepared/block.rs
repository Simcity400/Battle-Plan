//! Research prepared block by block, on its own. A pre-pass worker reads a
//! block alone and refreshes the research indexes hour by hour from the
//! block's keyframe (the holders by slot, the owners by slot, the read hours
//! of each slot's owners), keeping for each hour after the first what the
//! country step reads: the index values the hour changed, the slots to
//! observe again, whether the holders changed, and each fresh read's rows.
//! It reads every table; the fold reads none.
//!
//! The fold observes the block's first hour on the carried research, from
//! the frame the worker kept, as hourly preparation does. A block starts at
//! a keyframe, whose every view is a new allocation, so the carried indexes
//! and the worker's (made from nothing) both rebuild every index there from
//! the frame alone and select every holder: after that hour they hold the
//! same values, but for the latest future read hour, which the carried
//! indexes keep from earlier hours. When those agree, every later hour's
//! refresh is the worker's, so the fold applies the hour's index changes and
//! takes the country step from the kept rows, carrying each country's state
//! as the walk carries it; at the block's end it takes the worker's indexes.
//!
//! A fresh read joins its assignments to the technology inventory by the
//! template indexes the country's carried state holds (its active and
//! pending assignments), which the worker does not know. At a slot's first
//! fresh read in the block it keeps the owner's whole inventory; at a later
//! one, the rows of every template index the slot's reads in the block
//! assigned. When the carried state of a country of the slot holds another
//! index, or the future read hours disagree, the fold observes the rest of
//! the block hour by hour from the block read alone, its indexes rebuilt from
//! the block's first hour.

use super::Prepared;
use crate::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{self, Extract, Family, Folded},
};
use crate::products::research::{
    FrameReads, FreshRead, Index, IndexChanges, Inventory, Reads, Scan, owned,
};
use replay::{
    container::{BlockIndex, Replay},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::{
        ResearchAppliedBonusRow, ResearchInputRow, ResearchModifierRow, ResearchTechnologyRow,
        TableId,
    },
};
use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The tables research reads: the holders' keys and links, the research
/// slices and their read hours.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ResearchTechnologies,
    TableId::ResearchInputs,
    TableId::ResearchModifiers,
    TableId::ResearchAppliedBonuses,
    TableId::SliceReads,
];

/// [`TABLES`] in table order, as a block is read with them.
fn sorted_tables() -> &'static [TableId] {
    static SORTED: std::sync::LazyLock<Vec<TableId>> = std::sync::LazyLock::new(|| {
        let mut tables = TABLES.to_vec();
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &SORTED
}

/// Research's family: its refusal, progress and coverage name is the hourly
/// walk's. A slice is read every hour, so its blocks are read whole.
pub static FAMILY: Family = Family {
    name: "research",
    extract: "research.extract",
    sequential: "research.sequential",
    fallback: "research.fallback",
    tables: TABLES,
    fields: None,
    start,
};

/// Each hour the fold observes, counted in tests.
pub const OBSERVED: &str = "research.observed";
/// Each of those hours it took from the worker's index changes and rows
/// rather than from a frame, counted in tests.
pub const SHIPPED: &str = "research.shipped";

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        scan: Box::default(),
        first: None,
        hours: Vec::new(),
        assigned: BTreeMap::new(),
        held: 0,
    })
}

/// The block's first hour, which the fold observes on the carried research,
/// and the worker's latest future read hour after it.
struct First {
    frame: DecodedFrame,
    future_read: Option<i32>,
}

/// One owner's inventory as a fresh read joins it.
enum KeptInventory {
    /// The owner's whole part of the inventory.
    Owner(Vec<ResearchTechnologyRow>),
    /// The owner's rows of the template indexes `covered` names.
    Rows {
        owner: u32,
        covered: BTreeSet<u32>,
        rows: BTreeMap<u32, ResearchTechnologyRow>,
    },
}

/// A slot's fresh read at an hour, as the country step reads it.
struct KeptRead {
    inputs: Vec<ResearchInputRow>,
    read: Option<i32>,
    bonuses: Option<Vec<ResearchAppliedBonusRow>>,
    modifiers: Option<Vec<ResearchModifierRow>>,
    inventory: Option<KeptInventory>,
}

/// An hour after the block's first, as the country step reads it.
struct Hour {
    hour: i32,
    /// The slots to observe again and whether the holders changed; None
    /// when a table the indexes need was incomplete.
    refreshed: Option<(BTreeSet<u32>, bool)>,
    changes: IndexChanges,
    /// The fresh reads by slot: every slot one of whose owners was read at
    /// the hour.
    fresh: BTreeMap<u32, KeptRead>,
}

struct BlockFacts {
    first: First,
    hours: Vec<Hour>,
    /// The worker's indexes at the block's end.
    index: Index,
}

struct BlockExtract {
    /// The indexes alone, from the block's keyframe.
    scan: Box<Scan>,
    first: Option<First>,
    hours: Vec<Hour>,
    /// Each slot read fresh in the block, with the template indexes its
    /// reads assigned (none for a slot whose slots were all idle).
    assigned: BTreeMap<u32, BTreeSet<u32>>,
    /// The bytes the first frame and the kept hours hold, about.
    held: u64,
}

impl BlockExtract {
    /// The fresh reads of every slot one of whose owners was read at the
    /// frame's hour, each kept with the inventory its join can read.
    fn fresh(&mut self, frame: &DecodedFrame) -> BTreeMap<u32, KeptRead> {
        let Some(reads) = FrameReads::of(frame) else {
            return BTreeMap::new();
        };
        let slots: Vec<u32> = self
            .scan
            .input_dates
            .iter()
            .filter(|(_, dates)| dates.iter().any(|(_, read)| *read == frame.hour))
            .map(|(&slot, _)| slot)
            .collect();
        let mut fresh = BTreeMap::new();
        for slot in slots {
            let owners = self
                .scan
                .owners
                .get(&slot)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let read = reads.fresh_read(owners);
            let assigned: BTreeSet<u32> = read
                .inputs
                .iter()
                .filter(|input| input.technology_present != 0)
                .filter_map(|input| match input.database_index {
                    replay::rows::Recorded::Value(index) => Some(index as u32),
                    _ => None,
                })
                .collect();
            // The owner's whole inventory is kept at the slot's first fresh
            // read in the block only, whatever that read assigned.
            let first_read = !self.assigned.contains_key(&slot);
            let earlier = self.assigned.entry(slot).or_default();
            let inventory = match (&read.inventory, read.inputs.first()) {
                (Some(Inventory::Table(rows)), Some(input)) => {
                    let owner = input.country;
                    Some(if first_read {
                        KeptInventory::Owner(owned(rows, owner, |row| row.country).to_vec())
                    } else {
                        let covered: BTreeSet<u32> = earlier.union(&assigned).copied().collect();
                        let lookup = Inventory::Table(rows);
                        let rows = covered
                            .iter()
                            .filter_map(|&index| {
                                lookup.find(owner, index).map(|row| (index, row.clone()))
                            })
                            .collect();
                        KeptInventory::Rows {
                            owner,
                            covered,
                            rows,
                        }
                    })
                }
                _ => None,
            };
            earlier.extend(assigned);
            fresh.insert(
                slot,
                KeptRead {
                    inputs: read.inputs.iter().map(|row| (*row).clone()).collect(),
                    read: read.read,
                    bonuses: read.bonuses.map(Cow::into_owned),
                    modifiers: read.modifiers.map(Cow::into_owned),
                    inventory,
                },
            );
        }
        fresh
    }
}

impl Extract for BlockExtract {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        count_extracted();
        if self.first.is_none() {
            let refreshed = self.scan.refresh(frame, changes, true, None);
            if refreshed.is_some() {
                // Only the template indexes the first hour's reads assign.
                self.fresh(frame);
            }
            self.held = self.held.saturating_add(frame_bytes(frame));
            self.first = Some(First {
                frame: frame.clone(),
                future_read: self.scan.future_read,
            });
            return Ok(());
        }
        let mut index_changes = IndexChanges::default();
        let refreshed = self
            .scan
            .refresh(frame, changes, true, Some(&mut index_changes));
        let fresh = if refreshed.is_some() {
            self.fresh(frame)
        } else {
            BTreeMap::new()
        };
        let hour = Hour {
            hour: frame.hour,
            refreshed,
            changes: index_changes,
            fresh,
        };
        self.held = self.held.saturating_add(hour_bytes(&hour));
        self.hours.push(hour);
        Ok(())
    }

    /// The first frame, the kept hours and the indexes the block ends with.
    fn bytes(&self) -> u64 {
        self.held.saturating_add(index_bytes(&self.scan))
    }

    fn finish(mut self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let first = self
            .first
            .take()
            .ok_or_else(|| io::Error::other("Research block has no first hour"))?;
        let index = self.scan.take_index();
        Ok(Box::new(BlockFacts {
            first,
            hours: std::mem::take(&mut self.hours),
            index,
        }))
    }
}

/// Bytes of `count` values of `T`.
fn sized<T>(count: usize) -> u64 {
    (count as u64).saturating_mul(size_of::<T>() as u64)
}

/// Each research row of a frame, about: what the first frame kept holds.
fn frame_bytes(frame: &DecodedFrame) -> u64 {
    use replay::rows::tables;
    let rows = |bytes: Option<u64>| bytes.unwrap_or(0);
    rows(frame.complete::<tables::Countries>().ok().map(|rows| {
        sized::<replay::rows::CountryRow>(rows.len())
            + rows.iter().map(|row| row.tag.len() as u64).sum::<u64>()
    })) + rows(
        frame
            .complete::<tables::CountryLinks>()
            .ok()
            .map(|rows| sized::<replay::rows::CountryLinkRow>(rows.len())),
    ) + rows(
        frame
            .complete::<tables::ResearchTechnologies>()
            .ok()
            .map(|rows| sized::<ResearchTechnologyRow>(rows.len())),
    ) + rows(frame.complete::<tables::ResearchInputs>().ok().map(|rows| {
        sized::<ResearchInputRow>(rows.len())
            + rows.iter().map(|row| row.token.len() as u64).sum::<u64>()
    })) + rows(
        frame
            .complete::<tables::ResearchModifiers>()
            .ok()
            .map(|rows| sized::<ResearchModifierRow>(rows.len())),
    ) + rows(
        frame
            .complete::<tables::ResearchAppliedBonuses>()
            .ok()
            .map(|rows| sized::<ResearchAppliedBonusRow>(rows.len())),
    ) + rows(
        frame
            .complete::<tables::SliceReads>()
            .ok()
            .map(|rows| sized::<replay::rows::SliceReadRow>(rows.len())),
    )
}

/// What a kept hour holds, about.
fn hour_bytes(hour: &Hour) -> u64 {
    let changes = &hour.changes;
    let holders = changes.countries.as_ref().map_or(0, |(slots, rows)| {
        sized::<(u32, u32)>(slots.len())
            + rows
                .values()
                .map(|holders| {
                    sized::<(usize, u32, Vec<u8>)>(holders.len())
                        + holders.iter().map(|(_, _, tag)| tag.len() as u64).sum::<u64>()
                })
                .sum::<u64>()
    });
    let owners = changes.owners.as_ref().map_or(0, |owners| {
        owners
            .values()
            .map(|owners| sized::<u32>(owners.len()) + 48)
            .sum()
    });
    let dates = |dates: &BTreeMap<u32, Vec<(u32, i32)>>| -> u64 {
        dates
            .values()
            .map(|dates| sized::<(u32, i32)>(dates.len()) + 48)
            .sum()
    };
    let slot_dates = changes
        .slot_dates
        .iter()
        .map(|(_, dates)| sized::<(u32, i32)>(dates.as_ref().map_or(0, Vec::len)) + 48)
        .sum::<u64>();
    let dirty = hour
        .refreshed
        .as_ref()
        .map_or(0, |(dirty, _)| sized::<u32>(dirty.len()) * 4);
    let fresh = hour
        .fresh
        .values()
        .map(|read| {
            sized::<KeptRead>(1)
                + sized::<ResearchInputRow>(read.inputs.len())
                + read
                    .inputs
                    .iter()
                    .map(|row| row.token.len() as u64)
                    .sum::<u64>()
                + sized::<ResearchAppliedBonusRow>(read.bonuses.as_ref().map_or(0, Vec::len))
                + sized::<ResearchModifierRow>(read.modifiers.as_ref().map_or(0, Vec::len))
                + match &read.inventory {
                    Some(KeptInventory::Owner(rows)) => sized::<ResearchTechnologyRow>(rows.len()),
                    Some(KeptInventory::Rows { covered, rows, .. }) => {
                        sized::<u32>(covered.len()) * 4
                            + sized::<(u32, ResearchTechnologyRow)>(rows.len()) * 2
                    }
                    None => 0,
                }
        })
        .sum::<u64>();
    sized::<Hour>(1)
        + holders
        + owners
        + changes.dates.as_ref().map_or(0, dates)
        + slot_dates
        + dirty
        + fresh
}

/// The indexes a block ends with, about.
fn index_bytes(scan: &Scan) -> u64 {
    let holders = scan
        .country_rows
        .values()
        .map(|holders| {
            sized::<(usize, u32, Vec<u8>)>(holders.len())
                + holders.iter().map(|(_, _, tag)| tag.len() as u64).sum::<u64>()
        })
        .sum::<u64>();
    let owners = scan
        .owners
        .values()
        .map(|owners| sized::<u32>(owners.len()) + 48)
        .sum::<u64>();
    let dates = scan
        .input_dates
        .values()
        .map(|dates| sized::<(u32, i32)>(dates.len()) + 48)
        .sum::<u64>();
    holders + owners + dates + sized::<u32>(scan.input_slots.len()) * 4
}

/// An hour's rows as the worker kept them, with the read hours of the fold's
/// indexes.
struct KeptReads<'a> {
    dates: &'a BTreeMap<u32, Vec<(u32, i32)>>,
    fresh: &'a BTreeMap<u32, KeptRead>,
}

impl Reads for KeptReads<'_> {
    fn read_hour(&self, slot: u32, owner: u32) -> Option<i32> {
        self.dates
            .get(&slot)?
            .iter()
            .find(|(held, _)| *held == owner)
            .map(|(_, read)| *read)
    }

    fn fresh(&self, slot: u32, _: &[u32]) -> io::Result<FreshRead<'_>> {
        let kept = self
            .fresh
            .get(&slot)
            .ok_or_else(|| io::Error::other("A fresh research read was not kept"))?;
        Ok(FreshRead {
            inputs: kept.inputs.iter().collect(),
            read: kept.read,
            bonuses: kept.bonuses.as_deref().map(Cow::Borrowed),
            modifiers: kept.modifiers.as_deref().map(Cow::Borrowed),
            inventory: kept.inventory.as_ref().map(|inventory| match inventory {
                KeptInventory::Owner(rows) => Inventory::Table(rows),
                KeptInventory::Rows { owner, rows, .. } => Inventory::Rows {
                    owner: *owner,
                    rows,
                },
            }),
        })
    }
}

/// Whether every join an hour's fresh reads can make finds its rows kept:
/// each country of a slot read with a part of its owner's inventory holds
/// only template indexes that part covers.
fn covered(scan: &Scan, hour: &Hour) -> bool {
    hour.fresh.iter().all(|(slot, read)| {
        let Some(KeptInventory::Rows { covered, .. }) = &read.inventory else {
            return true;
        };
        scan.country_rows
            .get(slot)
            .into_iter()
            .flatten()
            .filter_map(|(_, _, tag)| std::str::from_utf8(tag).ok())
            .filter_map(|tag| scan.countries.get(tag))
            .all(|country| {
                country
                    .active
                    .values()
                    .map(|&(index, _, _)| index)
                    .chain(country.pending.iter().map(|pending| pending.2))
                    .all(|index| covered.contains(&index))
            })
    })
}

/// Research's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Fold one block's facts into the carried research. Returns how it folded
/// and the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    let BlockFacts {
        first,
        hours,
        index: end,
    } = *facts;
    check()?;
    let first_hour = first.frame.hour;
    let changes = FrameChanges::alone(&first.frame, replay.format);
    prepass::note(OBSERVED);
    let result = prepared.observe_changed(&first.frame, &changes);
    if !observed(first_hour, result)? {
        return Ok((Folded::Fast, Some(first_hour)));
    }
    drop(first.frame);
    if prepared.scan.future_read != first.future_read || forced(first_hour) {
        let reached = observe_after(prepared, replay, index, Some(first_hour), observed, check)?;
        return Ok((Folded::Fallback, reached.or(Some(first_hour))));
    }
    // The fold's own copy of the read hours, which the country step reads
    // beside its indexes; and the holders' slots.
    let mut dates = prepared.scan.input_dates.clone();
    let mut slots: Option<Arc<HashMap<u32, u32>>> = prepared
        .scan
        .country_reads
        .as_ref()
        .map(|reads| Arc::clone(reads.slots()));
    let mut reached = first_hour;
    for hour in hours {
        check()?;
        if !covered(&prepared.scan, &hour) || forced(hour.hour) {
            let after = observe_after(prepared, replay, index, Some(reached), observed, check)?;
            return Ok((Folded::Fallback, after.or(Some(reached))));
        }
        let Hour {
            hour,
            refreshed,
            changes,
            fresh,
        } = hour;
        prepared.scan.adopt(&changes);
        if let Some((held, _)) = &changes.countries {
            slots = Some(Arc::clone(held));
        }
        if let Some(whole) = &changes.dates {
            dates = whole.clone();
        }
        for (slot, slot_dates) in &changes.slot_dates {
            match slot_dates {
                Some(slot_dates) => {
                    dates.insert(*slot, slot_dates.clone());
                }
                None => {
                    dates.remove(slot);
                }
            }
        }
        prepass::note(OBSERVED);
        prepass::note(SHIPPED);
        let reads = KeptReads {
            dates: &dates,
            fresh: &fresh,
        };
        let result = prepared.observe_with(hour, |scan| {
            scan.observe_refreshed(hour, refreshed, slots.as_deref(), &reads)
        });
        if !observed(hour, result)? {
            return Ok((Folded::Fast, Some(hour)));
        }
        reached = hour;
    }
    prepared.scan.set_index(end);
    Ok((Folded::Fast, Some(reached)))
}

/// Observe the block hour by hour on the block read alone, with the change
/// facts and recorded updates of the block alone, as hourly preparation
/// does. With `after`, the hours through it were observed already: the
/// indexes are rebuilt from the block's first hour through it (they read the
/// frames alone), and the hours after it observed. Returns the last hour
/// observed.
pub fn observe_after(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    after: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    if after.is_some() {
        prepared.scan.clear_index();
    }
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        sorted_tables(),
        |raw, frame| {
            let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped {
                return Ok(());
            }
            if after.is_some_and(|after| frame.hour <= after) {
                prepared.scan.refresh(frame, &changes, true, None);
                return Ok(());
            }
            reached = Some(frame.hour);
            prepass::note(OBSERVED);
            let result = prepared.observe_changed(frame, &changes);
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Research observation stopped"))
                }
            }
        },
        || check(),
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

/// Observe the whole block hour by hour, as hourly preparation does: the
/// block whose extract refused or failed. Returns the last hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    observe_after(prepared, replay, index, None, observed, check)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static EXTRACTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
fn count_extracted() {
    EXTRACTED.with(|count| count.set(count.get() + 1));
}

/// Frames the extracts on this thread were given.
#[cfg(any(test, feature = "test-support"))]
pub fn extracted() -> usize {
    EXTRACTED.with(std::cell::Cell::get)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static FORCED: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether a test makes the fold observe the rest of its block hour by hour
/// from `hour`.
fn forced(hour: i32) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        FORCED.with(|forced| forced.borrow().contains(&hour))
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = hour;
        false
    }
}

/// Run `run` with the fold on this thread observing the rest of a block hour
/// by hour from each of `hours` (a block's first hour: after it).
#[cfg(any(test, feature = "test-support"))]
pub fn with_forced_fallback<T>(hours: &[i32], run: impl FnOnce() -> T) -> T {
    struct Reset(Vec<i32>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let previous = std::mem::take(&mut self.0);
            FORCED.with(|forced| *forced.borrow_mut() = previous);
        }
    }
    let _reset = Reset(FORCED.with(|forced| forced.replace(hours.to_vec())));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::research::test_fixtures::sliced_research_frame;
    use crate::test_fixtures::{CAMPAIGN, records};

    /// The owners' whole inventories a block's facts kept, and the inventory
    /// rows they kept in all.
    fn kept_inventory(facts: &BlockFacts) -> (usize, usize) {
        let (mut owners, mut rows) = (0, 0);
        for read in facts.hours.iter().flat_map(|hour| hour.fresh.values()) {
            match &read.inventory {
                Some(KeptInventory::Owner(kept)) => {
                    owners += 1;
                    rows += kept.len();
                }
                Some(KeptInventory::Rows { rows: kept, .. }) => rows += kept.len(),
                None => {}
            }
        }
        (owners, rows)
    }

    /// A country whose research slots are idle at every read, read every
    /// four hours: each block keeps its owner's inventory (four rows) at its
    /// first fresh read alone, and no row at the later ones.
    #[test]
    fn idle_slots_keep_their_owners_inventory_once_per_block() {
        let frames: Vec<_> = (101..=147)
            .map(|hour| sliced_research_frame(hour, hour - hour % 4, &[None, None]))
            .collect();
        let replay =
            replay::container::Replay::from_records(CAMPAIGN.into(), &[records(1, frames)])
                .unwrap();
        let mut reads = 0;
        for index in &replay.index {
            let facts = crate::prepared::prepass::extract_alone(&replay, index, &FAMILY)
                .unwrap()
                .downcast::<BlockFacts>()
                .unwrap();
            reads += facts
                .hours
                .iter()
                .filter(|hour| !hour.fresh.is_empty())
                .count();
            let (owners, rows) = kept_inventory(&facts);
            assert!(owners <= 1, "{owners} whole inventories kept in one block");
            assert_eq!(rows, owners * 4, "later reads of idle slots keep no row");
        }
        assert!(reads >= 10, "only {reads} fresh reads");
    }
}
