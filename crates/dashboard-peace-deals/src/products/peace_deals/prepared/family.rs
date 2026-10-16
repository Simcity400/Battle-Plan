//! Peace outcomes prepared on their own, block by block. A pre-pass worker
//! reads a block alone with the peace tables, only at the hours the fields
//! the conference and outcome observer reads change ([`FAMILY`]'s fields):
//! country keys and links, state owners, province states and victory points,
//! war and subject operands, governments, existence, the conference roster's
//! keys, turns and finalization, every finalizer row and the dictionary. Two
//! hours change the observer with no field changing: a subject relation's
//! recorded end, and the next scheduled roster read while the roster holds a
//! conference; the extract names the first as its deadline, and that hour is
//! observed on the last hour's rows. The worker keeps the observed hours'
//! frames (the large hourly tables reduced to the fields the observer reads)
//! and, for each span between them, the hours the roster's slot was read.
//!
//! The family folds each block in order on the carried observer: each kept
//! hour is observed as hourly preparation observes it, and each span takes
//! the observer's step for all its hours at once ([`Prepared::advance_quiet`]):
//! with every operand unchanged, only its frontiers and the snapshot's hour
//! move, and a roster read of the unchanged empty roster marks its read. When
//! a span could change more (a comparison is pending, a conference is held
//! at a roster read, or a relation ends inside it), the rest of the block is
//! observed hour by hour.

use super::Prepared;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{Observed, Reduced, kept_changes, observe_hours},
    prepass::{Extract, Family, Folded},
};
use dashboard_text::metadata::slot_read;
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::{CountryCounterInputRow, CountryRow, ProvinceRow, StateRow, TableId, TableRows, tables},
};
use std::{any::Any, io, ops::Range};

/// The tables the peace observer reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::Provinces,
    TableId::WarRelations,
    TableId::PeaceConferences,
    TableId::PeaceSettlements,
    TableId::Subjects,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::CountryAppearance,
    TableId::CountryCounterInputs,
    TableId::LexicalTokens,
];

pub static FAMILY: Family = Family {
    name: "peace",
    extract: "peace.extract",
    sequential: "peace.sequential",
    fallback: "peace.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the peace tables the observer reads, so that a block is
/// read only at the hours they change. Campaign is checked on every read and
/// read by nothing else here. Row keys and availability are watched with
/// every table.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country tags: the id (the row key), canonical slot and tag. The
        // balances move every hour and none is read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link, which the tags are resolved through and whose view the
        // observer tells apart by allocation.
        TableFields::all(TableId::CountryLinks),
        // Each state's owner (the id is the row key). Controllers, manpower
        // and infrastructure move without a transfer.
        TableFields::named(TableId::States, &["owner"])?,
        // Victory points summed by state. Province owners, controllers,
        // forts, areas and movement gates are not read.
        TableFields::named(TableId::Provinces, &["state", "victory_points"])?,
        // War operands: the pair (the row key), start and end. Casualties
        // move every hour of a war; the cancel byte is not read.
        TableFields::named(TableId::WarRelations, &["start_hour", "end_hour"])?,
        // The roster: each conference (the row key), its turn and whether it
        // finalized. Its ordinal is not read. A read of an unchanged roster
        // is told by the slot's read evidence, which spans carry.
        TableFields::named(TableId::PeaceConferences, &["current_turn", "finalized"])?,
        // Every finalizer row is an event, read whenever the table changes.
        TableFields::all(TableId::PeaceSettlements),
        // Subject relations: the pair (the row key), start, end, cancel byte
        // and level. The autonomy ratio moves every hour.
        TableFields::named(
            TableId::Subjects,
            &["start_hour", "end_hour", "cancel", "state"],
        )?,
        // Each government's ruling party; elections and capitals are not read.
        TableFields::named(TableId::Politics, &["ruling_party"])?,
        // Each party's token (the country and id are the row key).
        // Popularity moves every hour.
        TableFields::named(TableId::PoliticalParties, &["token"])?,
        // Each country's leader ideology; its colours, names and convoys are
        // not read.
        TableFields::named(TableId::CountryAppearance, &["leader_ideology"])?,
        // Whether a country exists: its owned state count. Controlled
        // states and the operand lists are not read.
        TableFields::named(TableId::CountryCounterInputs, &["owned_state_count"])?,
        // The state-taking keyword's spelling, whose view the observer tells
        // apart by allocation, and every finalizer token's.
        TableFields::all(TableId::LexicalTokens),
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        hours: Vec::new(),
        countries: Reduced::new(TableId::Countries),
        states: Reduced::new(TableId::States),
        provinces: Reduced::new(TableId::Provinces),
        counters: Reduced::new(TableId::CountryCounterInputs),
        deadline: None,
        roster_held: false,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// One hour of a block's facts, in hour order.
enum Hour {
    /// An hour the observer observes, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Consecutive recorded hours `first..=last`, after the hour before them,
    /// at which no field the observer reads changed; `reads` are those at
    /// which the roster's slot was read, in order.
    Quiet {
        first: i32,
        last: i32,
        reads: Vec<i32>,
    },
}

struct BlockFacts {
    hours: Vec<Hour>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

struct BlockExtract {
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    states: Reduced<StateRow>,
    provinces: Reduced<ProvinceRow>,
    counters: Reduced<CountryCounterInputRow>,
    /// The first hour after the last observed one that changes the observer
    /// with no field changing.
    deadline: Option<i32>,
    /// Whether the last observed hour's roster was complete and held a
    /// conference, which a roster read updates although its rows are equal.
    roster_held: bool,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

impl BlockExtract {
    /// The frame as the observer reads it: the countries, states, provinces
    /// and country counters reduced to the fields it reads.
    fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        let states = frame.complete::<tables::States>().ok().map(|rows| {
            rows.iter()
                .map(|row| StateRow {
                    id: row.id,
                    owner: row.owner,
                    ..Default::default()
                })
                .collect()
        });
        self.states.apply(&mut kept, states, TableRows::States);
        let provinces = frame.complete::<tables::Provinces>().ok().map(|rows| {
            rows.iter()
                .map(|row| ProvinceRow {
                    id: row.id,
                    state: row.state,
                    victory_points: row.victory_points,
                    ..Default::default()
                })
                .collect()
        });
        self.provinces
            .apply(&mut kept, provinces, TableRows::Provinces);
        let counters = frame
            .complete::<tables::CountryCounterInputs>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryCounterInputRow {
                        country: row.country,
                        owned_state_count: row.owned_state_count,
                        ..Default::default()
                    })
                    .collect()
            });
        self.counters
            .apply(&mut kept, counters, TableRows::CountryCounterInputs);
        kept
    }
}

/// The first subject relation end after `frame`'s hour, as the observer
/// finds its next one: a relation not cancelled whose recorded end is after
/// its start. None while the Subjects table is not complete.
fn subject_end(frame: &DecodedFrame) -> Option<i32> {
    frame
        .complete::<tables::Subjects>()
        .ok()?
        .iter()
        .filter(|row| row.cancel == 0 && row.end_hour > row.start_hour && row.end_hour > frame.hour)
        .map(|row| row.end_hour)
        .min()
}

/// The first hour after `hour` at which the roster's table is scheduled to be
/// read in a recording of `format`; None for a table never read on schedule.
fn next_roster_read(format: FormatVersion, hour: i32) -> Option<i32> {
    let cadence = replay::cadence::cadence_in(TableId::PeaceConferences, format);
    // Every schedule repeats within a week of hours.
    (1..=24 * 7)
        .filter_map(|step| hour.checked_add(step))
        .find(|&next| cadence.is_refresh(next))
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
                    held.add(reads.capacity() * size_of::<i32>());
                }
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
        self.roster_held = frame
            .complete::<tables::PeaceConferences>()
            .is_ok_and(|rows| !rows.is_empty());
        let roster = self
            .roster_held
            .then(|| next_roster_read(replay.format, frame.hour))
            .flatten();
        self.deadline = [subject_end(frame), roster].into_iter().flatten().min();
        let kept = self.kept(frame);
        self.hours.push(Hour::Observed {
            frame: kept,
            changes: kept_changes(changes),
        });
        Ok(())
    }

    /// The next subject relation end, and the next scheduled roster read
    /// while the roster holds a conference.
    fn deadline(&self) -> Option<i32> {
        self.deadline
    }

    /// Hours at which no field the observer reads changed. A roster read
    /// there is a read of the same rows: kept with the span while the roster
    /// is empty, and refused (the block is then observed hour by hour) while
    /// it holds a conference, which only a scheduled read, a deadline, reads
    /// otherwise.
    fn quiet(&mut self, replay: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, last)) = dashboard_base::prepared::kept::consecutive(hours, span.clone())?
        else {
            return Ok(());
        };
        let reads: Vec<i32> = span
            .filter(|&position| {
                slot_read(
                    replay.format,
                    &hours.metrics()[position],
                    TableId::PeaceConferences,
                    hours.hours()[position],
                )
            })
            .map(|position| hours.hours()[position])
            .collect();
        if self.roster_held && !reads.is_empty() {
            return Err(io::Error::other(
                "A held conference roster was read at an unscheduled hour",
            ));
        }
        if let Some(Hour::Quiet {
            last: end,
            reads: held,
            ..
        }) = self.hours.last_mut()
            && end.checked_add(1) == Some(first)
        {
            *end = last;
            held.extend(reads);
        } else {
            self.hours.push(Hour::Quiet { first, last, reads });
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Peace block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            #[cfg(any(test, feature = "test-support"))]
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried observer. Returns how it folded
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
    #[cfg(any(test, feature = "test-support"))]
    dashboard_base::prepared::kept::note_frames(FAMILY.name, facts.frames);
    let mut reached = None;
    // The last observed hour's frame, whose rows the hours of a span hold.
    let mut last_frame: Option<DecodedFrame> = None;
    for hour in facts.hours {
        check()?;
        match hour {
            Hour::Observed { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                dashboard_base::prepared::kept::note_observed(FAMILY.name);
                let result = prepared.observe_changed(&frame, &changes);
                let live = observed(frame.hour, result)?;
                last_frame = Some(frame);
                if !live {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, last, reads } => {
                let advanced = match &last_frame {
                    Some(frame) => prepared.advance_quiet(frame, first, last, &reads),
                    None => Ok(false),
                };
                let advanced = match advanced {
                    Ok(advanced) => advanced,
                    Err(error) => {
                        reached = Some(first);
                        observed(first, Err(error))?;
                        return Ok((Folded::Fast, reached));
                    }
                };
                if !advanced {
                    let reached =
                        observe_block(prepared, replay, index, Some(first), observed, check)?;
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
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    observe_hours(
        replay,
        index,
        TABLES,
        from,
        observed,
        check,
        |frame, facts| {
            #[cfg(any(test, feature = "test-support"))]
            dashboard_base::prepared::kept::note_observed(FAMILY.name);
            prepared.observe_changed(frame, facts)
        },
    )
}
