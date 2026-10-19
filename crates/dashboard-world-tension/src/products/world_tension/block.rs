//! World tension prepared on its own, block by block. A pre-pass worker reads
//! a block alone with the tension tables, only at the hours the fields the
//! tension observer reads change ([`FAMILY`]'s fields): the countries' keys,
//! every country link, and every field of the tension, threat source and
//! country aggregate tables, which the observer takes any row change of as a
//! change of its inputs. The worker keeps the observed hours' frames
//! (countries reduced to the keys the observer reads) and, for each hour
//! between them, whether the tension tables were read there (their slot read,
//! from the hour's read evidence): a read hour whose rows are unchanged starts
//! the held values again and counts its diagnostics, so the hours between are
//! quiet for the rows, not for the reads.
//!
//! The family folds each block in order on the carried observer: each kept
//! hour is observed as hourly preparation observes it, and each hour between
//! takes the observer's unchanged-hour step ([`Scan::advance_quiet`]), which
//! hourly preparation takes there: no table the observer watches whole was
//! decoded again, the country keys are those of the hour before and the hour
//! follows it. When the carried observer does not continue into such an hour,
//! the rest of the block is observed hour by hour.
//!
//! The observer's slot-to-tag and link-to-slot maps, its sources' first and
//! last reads, its countries' first contributions and its warning counts are
//! carried state, read back from the catalogue it publishes, so each block
//! continues the earlier hours' history.

use super::Scan;
use dashboard_base::prepared::kept::{Observed, Reduced, consecutive, kept_changes, observe_hours};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Folded},
};
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{format::FormatVersion, rows::CountryRow, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{any::Any, io, ops::Range};

/// The tables the tension observer reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

pub static FAMILY: Family = Family {
    name: "tension",
    extract: "tension.extract",
    sequential: "tension.sequential",
    fallback: "tension.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the tension tables the observer reads, so that a block is
/// read only at the hours they change. Campaign is checked on every read and
/// read by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country keys (`CountryKeys`): the id (the row key), canonical slot
        // and tag. The balances move every hour; the observer compares the
        // keys of a changed country table and reads nothing else of it.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link: the observer remembers each raw link's slot, and any
        // link change refreshes the slots' tags.
        TableFields::all(TableId::CountryLinks),
        // The observer takes any row change of these three as a change of its
        // inputs (a new read with its sums, sources and diagnostics), so every
        // field is watched and every decoding of them is visited, as hourly
        // preparation's change facts report it. Their reads with equal rows
        // are the quiet hours' read flags.
        TableFields::all(TableId::WorldTension),
        TableFields::all(TableId::ThreatSources),
        TableFields::all(TableId::ThreatCountryAggregates),
    ])
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        format,
        hours: Vec::new(),
        countries: Reduced::new(TableId::Countries),
        frames: 0,
    })
}

/// One hour of a block's facts, in hour order.
#[allow(
    clippy::large_enum_variant,
    reason = "Block facts retain observed frames inline without a per-hour allocation."
)]
enum Hour {
    /// An hour the observer observes, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which no row, key or link the observer reads changed: whether the
    /// tension tables were read at each.
    Quiet { first: i32, reads: Vec<bool> },
}

/// A block's hours as the extract kept them.
struct BlockFacts {
    hours: Vec<Hour>,
    /// Frames the extract was given or built.
    #[cfg_attr(not(test), allow(dead_code))]
    frames: usize,
}

struct BlockExtract {
    format: FormatVersion,
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    frames: usize,
}

impl Extract for BlockExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Observed { frame, changes } => {
                    held.frame(frame).changes(changes);
                }
                Hour::Quiet { reads, .. } => {
                    held.add(reads.capacity());
                }
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        self.hours.push(Hour::Observed {
            frame: kept,
            changes: kept_changes(changes),
        });
        Ok(())
    }

    /// Hours at which nothing the observer reads changed: each hour's slot
    /// read of the tension tables, from its read evidence, as the change
    /// facts of an observed hour carry it.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        let format = self.format;
        let reads = span.map(|position| {
            dashboard_text::metadata::slot_read(
                format,
                &hours.metrics()[position],
                TableId::WorldTension,
                hours.hours()[position],
            )
        });
        match self.hours.last_mut() {
            Some(Hour::Quiet {
                first: held,
                reads: held_reads,
            }) if i64::from(*held) + held_reads.len() as i64 == i64::from(first) => {
                held_reads.extend(reads);
            }
            _ => self.hours.push(Hour::Quiet {
                first,
                reads: reads.collect(),
            }),
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("World tension block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried observer. Returns how it folded
/// and the last hour it reached.
pub fn fold(
    scan: &mut Scan,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
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
    note_frames(FAMILY.name, facts.frames);
    let mut reached = None;
    for hour in facts.hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    scan.prepare_changed(replay.format, &frame, &changes)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, reads } => {
                for (offset, read) in reads.into_iter().enumerate() {
                    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                    // The span's hours are consecutive i32 hours.
                    let hour = first + offset as i32;
                    if !scan.continues_to(hour) {
                        let reached =
                            observe_block(scan, replay, index, Some(hour), observed, check)?;
                        return Ok((Folded::Fallback, reached));
                    }
                    reached = Some(hour);
                    let result = {
                        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                        scan.advance_quiet(hour, read)
                    };
                    if !observed(hour, result)? {
                        return Ok((Folded::Fast, reached));
                    }
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
    scan: &mut Scan,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
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
            scan.prepare_changed(replay.format, frame, facts)
        },
    )
}
