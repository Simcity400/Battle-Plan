//! The country identity history prepared block by block. A pre-pass worker
//! reads a block alone with the identity tables, only at the hours the fields
//! the history reads change ([`FAMILY`]'s fields), and observes those hours on
//! a history of that block alone, as hourly preparation observes owned
//! states: the block's first hour, every hour whose country keys, links,
//! counter row keys, gain keys or table availability change, and every hour
//! its deadline names. The hours between are quiet: each is stepped without
//! its rows from its read evidence alone (whether all three witness tables
//! were read). The worker keeps the observed hours' frames (countries reduced
//! to the id, slot and tag the history reads, counters to their country and
//! owned-state count) and the quiet spans, with the inputs its history held
//! before each.
//!
//! The coordinator folds the block in order on the carried history: each
//! kept hour is observed as hourly preparation observes it, and each quiet
//! span is stepped as its hours would be observed. Before a span the carried
//! history must hold the inputs the worker's held there; otherwise the rest
//! of the block is observed hour by hour.

use super::{HistoryScan, QuietInputs};
use crate::{
    metadata::CountryKeys,
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{Extract, Family, Folded},
    },
};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::{DecodedFrame, SharedView, TableView},
    rows::{CountryCounterInputRow, CountryRow, TableId, TableRows, tables},
};
use std::{any::Any, io, ops::Range, sync::Arc};

/// The tables the identity history reads: the countries and their links,
/// the counter rows whose country joins count unreadable rows, and the
/// experience gains that witness each country's native instance.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
];

/// The tables whose reads an hour's native witnesses need, all three.
const WITNESSED: [TableId; 3] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperienceGains,
];

pub static FAMILY: Family = Family {
    name: "identities",
    extract: "identities.extract",
    sequential: "identities.sequential",
    fallback: "identities.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the identity tables the history reads, so that a block is
/// read only at the hours they change. Campaign is checked on every read and
/// read by nothing here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country keys (`CountryKeys`, the tag join and the witnesses' names):
        // the id (the row key), canonical slot and tag. The balances change
        // every hour and the history holds none of them.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // The links' slots, and every decode of them: the join and the
        // witnesses' names are kept for one links allocation, so an equal
        // table decoded again is joined again, as hourly preparation does.
        TableFields::all(TableId::CountryLinks),
        // Only the counter rows' countries (the row key): their join counts
        // unresolved and repeated rows. The owned-state counts it publishes
        // to territory move every hour and leave the history unchanged.
        TableFields::keys(TableId::CountryCounterInputs),
        // Only the gains' country and instance (the row key), which name each
        // country's native witness. The amounts move every hour.
        TableFields::keys(TableId::CountryExperienceGains),
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        scan: HistoryScan::new(false, &[]),
        hours: Vec::new(),
        last: None,
        countries: None,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// One hour of a block's facts, in hour order.
enum Hour {
    /// An hour the history observes, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Hours `first..=last`, none changing what the history reads, each
    /// reading all three witness tables (`witnessed`) or not, after a history
    /// holding `inputs`.
    Quiet {
        first: i32,
        last: i32,
        witnessed: bool,
        inputs: QuietInputs,
    },
}

struct BlockFacts {
    hours: Vec<Hour>,
    /// Frames the extract was given.
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

struct BlockExtract {
    scan: HistoryScan,
    hours: Vec<Hour>,
    /// The last hour given, observed or quiet.
    last: Option<i32>,
    /// The countries' keys of the last kept hour and their reduced view,
    /// shared by kept hours with equal keys.
    countries: Option<(CountryKeys, SharedView)>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

impl BlockExtract {
    /// The frame as the history reads it: the countries reduced to their
    /// keys, the counter rows to their country and owned-state count.
    fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        if let Ok(rows) = frame.complete::<tables::Countries>() {
            let held = self
                .countries
                .as_ref()
                .filter(|(keys, _)| keys.same(rows))
                .map(|(_, view)| view.clone());
            let view = held.unwrap_or_else(|| {
                let view: SharedView = TableView::Complete(TableRows::Countries(
                    rows.iter()
                        .map(|row| CountryRow {
                            id: row.id,
                            canonical_slot: row.canonical_slot,
                            tag: row.tag.clone(),
                            ..Default::default()
                        })
                        .collect(),
                ))
                .into();
                self.countries = Some((CountryKeys::read(rows), view.clone()));
                view
            });
            kept.tables[TableId::Countries.index()] = view;
        }
        if let Ok(rows) = frame.complete::<tables::CountryCounterInputs>() {
            kept.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(
                    rows.iter()
                        .map(|row| CountryCounterInputRow {
                            country: row.country,
                            owned_state_count: row.owned_state_count,
                            ..Default::default()
                        })
                        .collect(),
                ))
                .into();
        }
        kept
    }
}

/// The history's observation of one hour, as hourly preparation makes it
/// (the owned-state counts it publishes are territory's, not the history's).
fn observe(scan: &mut HistoryScan, frame: &DecodedFrame, changes: &FrameChanges) {
    scan.observe_owned_states(frame, Some(changes), |_, _, _| {});
}

/// Whether the `position`th of `hours` read all three witness tables.
fn witnessed(hours: &QuietHours, position: usize) -> bool {
    WITNESSED.iter().all(|&table| hours.read(position, table))
}

impl Extract for BlockExtract {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.frames += 1;
        }
        observe(&mut self.scan, frame, changes);
        self.last = Some(frame.hour);
        let kept = self.kept(frame);
        self.hours.push(Hour::Observed {
            frame: kept,
            changes: FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
                updates: None,
            },
        });
        Ok(())
    }

    /// The next hour, while an hour reading every witness table would need
    /// its rows: the held native names are not the last hour's, although no
    /// watched field need change there.
    fn deadline(&self) -> Option<i32> {
        if self.scan.stale_witnesses() {
            self.last?.checked_add(1)
        } else {
            None
        }
    }

    /// Consecutive hours at which nothing the history reads changed: each run
    /// of hours with the same witness reads is stepped at once.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let all = hours.hours();
        let mut position = span.start;
        while position < span.end {
            let read = witnessed(hours, position);
            let mut end = position + 1;
            while end < span.end && witnessed(hours, end) == read {
                end += 1;
            }
            let (first, last) = (all[position], all[end - 1]);
            let inputs = self.scan.quiet_inputs();
            if !self.scan.advance_quiet(first, last, read) {
                return Err(io::Error::other(
                    "Country identity hours were quiet where the history needs their rows",
                ));
            }
            self.hours.push(Hour::Quiet {
                first,
                last,
                witnessed: read,
                inputs,
            });
            self.last = Some(last);
            position = end;
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Country identity block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            #[cfg(any(test, feature = "test-support"))]
            frames: self.frames,
        }))
    }
}

/// The history's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Fold one block's facts into the carried history. Returns how it folded and
/// the last hour it reached.
pub fn fold(
    scan: &mut HistoryScan,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(any(test, feature = "test-support"))]
    FRAMES.with(|count| count.set(count.get() + facts.frames));
    let mut reached = None;
    for hour in facts.hours {
        check()?;
        match hour {
            Hour::Observed { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                OBSERVED.with(|count| count.set(count.get() + 1));
                observe(scan, &frame, &changes);
                if !observed(frame.hour, Ok(()))? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet {
                first,
                last,
                witnessed,
                inputs,
            } => {
                // Equal inputs step as the worker's history stepped, so a
                // failed step leaves the carried history unchanged.
                if scan.quiet_inputs() != inputs || !scan.advance_quiet(first, last, witnessed) {
                    let reached = observe_block(scan, replay, index, Some(first), observed, check)?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(last);
                if !observed(last, Ok(()))? {
                    return Ok((Folded::Fast, reached));
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), with
/// the change facts of the block alone, as hourly preparation does. Returns
/// the last hour observed.
pub fn observe_block(
    scan: &mut HistoryScan,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || from.is_some_and(|from| frame.hour < from) {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(any(test, feature = "test-support"))]
            OBSERVED.with(|count| count.set(count.get() + 1));
            observe(scan, frame, &facts);
            match observed(frame.hour, Ok(())) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Country identity observation stopped"))
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

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Hours the coordinator observed on carried histories, on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn observed_hours() -> usize {
    OBSERVED.with(std::cell::Cell::get)
}

/// Frames the extracts of the blocks folded on this thread were given or
/// built: every hour of a block read whole, only the hours read and those
/// observed between them of a block read by its changes.
#[cfg(any(test, feature = "test-support"))]
pub fn built_frames() -> usize {
    FRAMES.with(std::cell::Cell::get)
}
