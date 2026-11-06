//! Country groups prepared block by block. A pre-pass worker reads a block
//! alone with the groups' own tables, only at the hours the fields the groups
//! read change ([`FAMILY`]'s fields), and runs a scan of that block alone to
//! find the hours that change the groups: the block's first hour and every
//! hour whose tables, country keys, war operands, continuity or war expiry
//! change, or that reads the dictionary. A war's end needs no field to change:
//! the scan names it as its deadline, and that hour is observed on the last
//! hour's rows. The worker keeps the hours that change the groups, with their
//! frames (countries reduced to the id, slot and tag the groups read), and
//! the spans of unchanged hours between them.
//!
//! The coordinator folds the block in order on the carried groups: each kept
//! hour is observed as hourly preparation observes it, and each unchanged span
//! advances the hourly counts in constant time. Before a span the carried scan
//! must hold the inputs the worker's scan held there; otherwise the rest of
//! the block is observed hour by hour.

// Release reads only TABLES: the rest is the row pre-pass air superiority's extract reads.
#[cfg(any(test, feature = "test-support"))]
use super::{QuietInputs, Scan, changes::Prepared as Changes, projected_countries};
#[cfg(any(test, feature = "test-support"))]
use crate::{
    Source,
    metadata::CountryKeys,
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{Extract, Family, Folded},
    },
    products::LoadContext,
};
#[cfg(any(test, feature = "test-support"))]
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::rows::TableId;
#[cfg(any(test, feature = "test-support"))]
use replay_data::{format::FormatVersion, rows::TableRows, rows::tables};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
#[cfg(any(test, feature = "test-support"))]
use std::{any::Any, io, ops::Range, sync::Arc};

/// The tables the groups read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::HumanControllers,
    TableId::Factions,
    TableId::LexicalTokens,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Capitulation,
];

/// How a columnar walk's world holds each table the groups read, by the fields whose change is
/// an hour the groups observe: countries' slots and tags, wars' dates and instigator, factions'
/// names and templates, capitulations' flags and hours, every field of the others. Every native
/// read of the groups holds them so.
///
/// An hour that changes only fields of factions or capitulations the groups do not read ends as
/// a quiet hour does: their operands read again are equal, every capitulation was named when
/// first read, the names run on, and what the hour seals early a later seal reaches anyway.
pub fn holds() -> std::io::Result<[(TableId, dashboard_world::Hold); 8]> {
    use dashboard_world::Hold;
    let fields = |table, names: &[&str]| -> std::io::Result<Hold> {
        let mut mask = 0_u64;
        for name in names {
            mask |= 1_u64 << replay::columns::field(table, name)?;
        }
        Ok(Hold::Fields(mask))
    };
    Ok([
        (
            TableId::Countries,
            fields(TableId::Countries, &["canonical_slot", "tag"])?,
        ),
        (TableId::CountryLinks, Hold::Whole),
        (TableId::HumanControllers, Hold::Whole),
        (
            TableId::Factions,
            fields(TableId::Factions, &["name", "template_token"])?,
        ),
        (TableId::FactionMembers, Hold::Whole),
        (
            TableId::WarRelations,
            fields(
                TableId::WarRelations,
                &["start_hour", "end_hour", "first_was_instigator"],
            )?,
        ),
        (
            TableId::Capitulation,
            fields(TableId::Capitulation, &["capitulated", "capitulated_hour"])?,
        ),
        (TableId::LexicalTokens, Hold::Whole),
    ])
}

#[cfg(any(test, feature = "test-support"))]
pub static FAMILY: Family = Family {
    name: "groups",
    extract: "groups.extract",
    sequential: "groups.sequential",
    fallback: "groups.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the groups' tables that the scan and the change publisher
/// read, so that a block is read only at the hours they change. Campaign is
/// checked on every read and read by nothing else here.
#[cfg(any(test, feature = "test-support"))]
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country keys (`CountryKeys`): the id (the row key), canonical slot
        // and tag. The balances change every hour and no group reads them.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // War operands: the pair (the row key), start, end and instigator.
        // Casualties change every hour; the cancel byte is not read.
        TableFields::named(
            TableId::WarRelations,
            &["start_hour", "end_hour", "first_was_instigator"],
        )?,
        // Factions' names and templates, and capitulations' flags and hours:
        // a change of their other fields ends as a quiet hour ([`holds`]).
        TableFields::named(TableId::Factions, &["name", "template_token"])?,
        TableFields::named(TableId::Capitulation, &["capitulated", "capitulated_hour"])?,
        // The scan takes any row change of these tables for a change of the
        // groups, and an hour it observes is not a quiet one: the change
        // publisher's refusals counted per hour start again there. So every
        // field is watched, although the groups read only the links' slots,
        // the controllers' counts and the memberships.
        TableFields::all(TableId::CountryLinks),
        TableFields::all(TableId::HumanControllers),
        TableFields::all(TableId::FactionMembers),
        // The dictionary's spellings, and every read of it: a read names the
        // templates again (a new name run) even when its rows are equal.
        TableFields::all(TableId::LexicalTokens).when_read(),
    ])
}

#[cfg(any(test, feature = "test-support"))]
fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        scan: None,
        hours: Vec::new(),
        countries: None,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// One hour of a block's facts, in hour order.
// Keep observed frames inline to preserve kept-hour allocation and byte accounting.
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::large_enum_variant)]
enum Hour {
    /// An hour the groups observe, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Hours `first..=last`, none changing the groups, after a scan holding
    /// `inputs`.
    Quiet {
        first: i32,
        last: i32,
        inputs: QuietInputs,
    },
}

#[cfg(any(test, feature = "test-support"))]
struct BlockFacts {
    hours: Vec<Hour>,
    /// Frames the extract observed.
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

#[cfg(any(test, feature = "test-support"))]
struct BlockExtract {
    scan: Option<Scan>,
    hours: Vec<Hour>,
    /// The countries' keys of the last kept hour and their reduced view,
    /// shared by kept hours with equal keys.
    countries: Option<(CountryKeys, SharedView)>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl BlockExtract {
    /// The frame as the groups read it: the countries reduced to their keys.
    fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        if let Ok(rows) = frame.complete::<tables::Countries>() {
            let held = self
                .countries
                .as_ref()
                .filter(|(keys, _)| keys.same(rows))
                .map(|(_, view)| view.clone());
            let view = held.unwrap_or_else(|| {
                let keys = CountryKeys::read(rows);
                let view: SharedView =
                    TableView::Complete(TableRows::Countries(projected_countries(&keys))).into();
                self.countries = Some((keys, view.clone()));
                view
            });
            kept.tables[TableId::Countries.index()] = view;
        }
        kept
    }

    /// Hours `first..=last` that do not change the groups, after a scan
    /// holding `inputs`, joined to the unchanged span before them.
    fn unchanged(&mut self, first: i32, last: i32, inputs: QuietInputs) {
        if let Some(Hour::Quiet { last: end, .. }) = self.hours.last_mut() {
            *end = last;
        } else {
            self.hours.push(Hour::Quiet {
                first,
                last,
                inputs,
            });
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Extract for BlockExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = crate::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            if let Hour::Observed { frame, changes } = hour {
                held.frame(frame).changes(changes);
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.frames += 1;
        }
        let scan = self
            .scan
            .get_or_insert_with(|| Scan::new(replay, frame.branch));
        let inputs = scan.quiet_inputs();
        let changed = scan.advance_changed(frame, changes)?.is_some();
        if self.hours.is_empty() || changed {
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
        } else {
            self.unchanged(frame.hour, frame.hour, inputs);
        }
        Ok(())
    }

    /// The scan's next war expiry, which changes the groups with no field
    /// changing.
    fn deadline(&self) -> Option<i32> {
        self.scan.as_ref()?.continuation.next_expiry
    }

    /// Consecutive hours the scan finds unchanged: every field it reads,
    /// the dictionary's reads and its deadline are the pre-pass's to watch,
    /// so it advances over them at once.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let span = &hours.hours()[span];
        let (Some(&first), Some(&last)) = (span.first(), span.last()) else {
            return Ok(());
        };
        let scan = self.scan.as_mut().ok_or_else(|| {
            io::Error::other("Country group hours were quiet before the block's first")
        })?;
        let inputs = scan.quiet_inputs();
        scan.advance_quiet(first, last);
        self.unchanged(first, last, inputs);
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Country group block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            #[cfg(any(test, feature = "test-support"))]
            frames: self.frames,
        }))
    }
}

/// The groups' observation of one hour's outcome: `Ok(true)` while the
/// groups go on, `Ok(false)` once they refused the branch.
#[cfg(any(test, feature = "test-support"))]
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Fold one block's facts into the carried groups, after the block began.
/// Returns how it folded and the last hour it reached.
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub fn fold(
    scan: &mut Scan,
    changes: &mut Changes,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    source: &Source,
    context: &LoadContext,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(any(test, feature = "test-support"))]
    FRAMES.with(|count| count.set(count.get() + facts.frames));
    let mut reached = None;
    for hour in facts.hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed {
                frame,
                changes: facts,
            } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                OBSERVED.with(|count| count.set(count.get() + 1));
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    changes.observe_changed(scan, &frame, &facts, source, context)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet {
                first,
                last,
                inputs,
            } => {
                if scan.quiet_inputs() != inputs {
                    let reached = observe_block(
                        scan,
                        changes,
                        replay,
                        index,
                        source,
                        context,
                        Some(first),
                        observed,
                        check,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(last);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    changes.advance_quiet(scan, first, last)
                };
                if !observed(last, result)? {
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
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub fn observe_block(
    scan: &mut Scan,
    changes: &mut Changes,
    replay: &Replay,
    index: &BlockIndex,
    source: &Source,
    context: &LoadContext,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || from.is_some_and(|from| frame.hour < from) {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(any(test, feature = "test-support"))]
            OBSERVED.with(|count| count.set(count.get() + 1));
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                changes.observe_changed(scan, frame, &facts, source, context)
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Country group observation stopped"))
                }
            }
        },
        check,
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

/// Hours the coordinator observed on carried groups, on this thread.
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

/// This thread's counts, to carry to another ([`add_counts`]).
#[cfg(any(test, feature = "test-support"))]
pub fn counts() -> (usize, usize) {
    (observed_hours(), built_frames())
}

/// Add counts made on another thread to this thread's.
#[cfg(any(test, feature = "test-support"))]
pub fn add_counts((observed, frames): (usize, usize)) {
    OBSERVED.with(|count| count.set(count.get() + observed));
    FRAMES.with(|count| count.set(count.get() + frames));
}
