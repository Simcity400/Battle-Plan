//! Subjects prepared on their own, block by block. A pre-pass worker reads a
//! block alone with the subject tables, only at the hours the fields the
//! subject scan reads change ([`FAMILY`]'s fields): the relations' keys,
//! starts, ends and cancel bytes, the subjects' autonomy states, the
//! countries' keys and every country link. A relation's recorded end changes
//! the relations with no field changing: the extract names the next one as
//! its deadline, and that hour is observed on the last hour's rows. The worker
//! keeps the observed hours' frames (countries reduced to the keys the scan
//! reads) and the spans of hours between them.
//!
//! The family folds each block in order on the carried scan: each kept hour is
//! observed as hourly preparation observes it, and each span takes the scan's
//! unchanged-hour step for all its hours at once ([`Scan::advance_quiet`]),
//! diagnostic rates included. When the carried scan cannot take that step (it
//! does not continue into the span, or a relation it holds ends inside it),
//! the rest of the block is observed hour by hour.
//!
//! The kept hours, reduced views and test counts are
//! [`dashboard_base::prepared::kept`]'s, shared with peace and idea history.

use super::Scan;
pub use dashboard_base::prepared::kept::{
    BlockFacts, Hour, Observed, Reduced, consecutive, kept_changes, observe_hours, push_quiet,
};
#[cfg(any(test, feature = "test-support"))]
pub use dashboard_base::prepared::kept::{counted, note_frames, note_observed};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Folded},
    series::{Accumulator, LateIndex},
};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::{CountryRow, TableId, tables},
};
use std::{any::Any, io, ops::Range};

/// The tables the subject scan reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Subjects,
    TableId::Autonomy,
];

pub static FAMILY: Family = Family {
    name: "subjects",
    extract: "subjects.extract",
    sequential: "subjects.sequential",
    fallback: "subjects.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the subject tables the scan reads, so that a block is read
/// only at the hours they change. Campaign is checked on every read and read
/// by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country keys (`CountryKeys`): the id (the row key), canonical slot
        // and tag. The balances move every hour and the scan reads none.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link: the scan resolves each relation's countries through
        // them, and any link change is a change of its holders.
        TableFields::all(TableId::CountryLinks),
        // A relation's countries (the row key), start, end and cancel byte:
        // what makes it active and dates it. Its autonomy ratio moves every
        // hour, and its `state` is the relation's initial definition, not the
        // level the scan reads.
        TableFields::named(TableId::Subjects, &["start_hour", "end_hour", "cancel"])?,
        // Each subject's level: its autonomy state (the subject is the row
        // key). Scores, thresholds and neighbouring states are not read.
        TableFields::named(TableId::Autonomy, &["state"])?,
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        hours: Vec::new(),
        countries: Reduced::new(TableId::Countries),
        deadline: None,
        frames: 0,
    })
}

struct BlockExtract {
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    /// The first recorded relation end after the last observed hour.
    deadline: Option<i32>,
    frames: usize,
}

/// The first relation end after `frame`'s hour among its relations, as the
/// scan finds its next one: a relation not cancelled whose recorded end is
/// after its start. None while the Subjects table is not complete, when the
/// scan holds no relations.
fn next_end(frame: &DecodedFrame) -> Option<i32> {
    frame
        .complete::<tables::Subjects>()
        .ok()?
        .iter()
        .filter(|row| row.cancel == 0 && row.end_hour > row.start_hour && row.end_hour > frame.hour)
        .map(|row| row.end_hour)
        .min()
}

impl Extract for BlockExtract {
    /// What the block's kept hours hold.
    fn bytes(&self) -> u64 {
        dashboard_base::prepared::kept::hours_bytes(&self.hours)
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        self.deadline = next_end(frame);
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        self.hours.push(Hour::Observed {
            frame: kept,
            changes: kept_changes(changes),
        });
        Ok(())
    }

    /// The next relation end, which changes the relations with no field
    /// changing.
    fn deadline(&self) -> Option<i32> {
        self.deadline
    }

    /// Hours at which no field the scan reads changed and no relation ends:
    /// the fold takes the scan's unchanged step over them.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        if let Some((first, last)) = consecutive(hours, span)? {
            push_quiet(&mut self.hours, first, last);
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Subject block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried scan and its change publisher.
/// Returns how it folded and the last hour it reached.
#[allow(clippy::too_many_arguments)]
pub fn fold(
    scan: &mut Scan,
    events: &mut Accumulator,
    late: &mut LateIndex,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(any(test, feature = "test-support"))]
    note_frames(FAMILY.name, facts.frames);
    let mut reached = None;
    for hour in facts.hours {
        let _cpu = ::replay::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                let result = {
                    let _cpu = ::replay::cpu::scope("fold observe", None);
                    scan.prepare_changes_changed(&frame, &changes, events, late)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, last } => {
                let advanced = {
                    let _cpu = ::replay::cpu::scope("fold observe", None);
                    scan.advance_quiet(first, last, events)
                };
                if !advanced {
                    let reached = observe_block(
                        scan,
                        events,
                        late,
                        replay,
                        index,
                        Some(first),
                        observed,
                        check,
                    )?;
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
#[allow(clippy::too_many_arguments)]
pub fn observe_block(
    scan: &mut Scan,
    events: &mut Accumulator,
    late: &mut LateIndex,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay::cpu::scope("fold driver", None);
    observe_hours(
        replay,
        index,
        TABLES,
        from,
        observed,
        check,
        |frame, facts| {
            #[cfg(any(test, feature = "test-support"))]
            note_observed(FAMILY.name);
            scan.prepare_changes_changed(frame, facts, events, late)
        },
    )
}
