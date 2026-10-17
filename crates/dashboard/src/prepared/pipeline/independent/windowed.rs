//! The windowed logs prepared on their own: the division log, and the naval
//! battles with the ships sunk, naval losses and battle details built on the
//! same scan. Each publishes every block's window where hourly preparation
//! publishes it, at the block's end, and saves its scan in its own section
//! (`division-log-state-<branch>`, `naval-battles-state-<branch>`), the
//! state hourly preparation keeps in its branch state.
//!
//! The division log's blocks are read alone on pre-pass workers and folded
//! from their facts ([`division_log::block`]); its windows are rendered at
//! each block's end and written, with the block's totals, at the next commit.
//!
//! Naval observation carries, from every earlier hour, the battles open and
//! retained, the result and source links, the retained sinkings and their
//! attributions, and reads allocations identity by identity, so a block's
//! observation cannot run apart from the block before: the family's own
//! thread observes each block hour by hour, on the block read alone with the
//! naval tables, the place naming tables and the shared trigger kit that a
//! sunk ship's scripted place name evaluates, with the change facts and
//! recorded updates of the block alone, which are the walk's at every hour.
//! A sunk ship's place name is read only for a ship the sinking scan first
//! finds, at a read of the sunk ships whose rows changed since the read
//! before, the block's first read, or one an entry dated after the read
//! before comes due at. The pre-pass extract reads each block's sunk ships
//! by their changes and finds the last such hour; the family's thread reads
//! the trigger kit through that hour only, and the naval tables after it
//! from a read of its own from the block's first hour, handed the first
//! read's views of the tables not rebuilt since, so their allocations and
//! change facts are those of one read; a ship first found without the kit
//! stops preparation. It publishes each block at the block's end into pages held
//! for its next commit ([`Branch::publish_block`]), which read back what
//! earlier blocks wrote, so it commits where the hourly walk does.
//!
//! Hourly preparation resumes both scans from the frame its stream holds
//! where it loads a branch (the last hour of the recording's block before,
//! when that block is on the same branch); each family does the same where it
//! loads the branch, before binding.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{self, Extract, Folded},
    },
    products::{division_log, naval_battles, ships_sunk},
};
use dashboard_base::prepared::pages::Pages;
use replay::{
    container::{QuietHours, Replay, SeekResult, TableFields},
    format::FormatVersion,
    frame::{DecodedFrame, SharedView, ViewIdentity},
    rows::{TableId, tables},
};
use std::{
    any::Any,
    io,
    ops::Range,
    sync::{LazyLock, atomic::AtomicBool},
};

/// The frame hourly preparation's stream holds when it resumes `block`'s
/// branch at `block`: the last hour of the recording's block before, with
/// every table, when that block is on the same branch; none otherwise.
fn before(block: &Block<'_>) -> io::Result<Option<DecodedFrame>> {
    let index = &block.replay.index;
    let Some(position) = index
        .iter()
        .position(|at| at.branch == block.index.branch && at.first_hour == block.index.first_hour)
    else {
        return Ok(None);
    };
    let Some(previous) = position
        .checked_sub(1)
        .map(|position| &index[position])
        .filter(|previous| previous.branch == block.index.branch)
    else {
        return Ok(None);
    };
    Ok(
        match block.replay.seek(previous.branch, previous.last_hour)? {
            SeekResult::Frame(frame) => Some(*frame),
            SeekResult::Gap { .. } => None,
        },
    )
}

/// `block`'s position among its branch's blocks: its window.
fn ordinal(block: &Block<'_>) -> io::Result<usize> {
    block
        .replay
        .index
        .iter()
        .filter(|at| at.branch == block.index.branch)
        .position(|at| at.first_hour == block.index.first_hour)
        .ok_or_else(|| io::Error::other("A block is not in its branch"))
}

/// A branch's saved scan, compressed JSON, as one section.
fn write_state(writer: &mut store::Writer, section: &str, json: &[u8]) -> io::Result<()> {
    let bytes = zstd::stream::encode_all(json, 1)?;
    writer.write_at(section, 0, bytes.as_slice())
}

/// The division log.
pub(in crate::prepared::pipeline) struct Divisions;

pub(in crate::prepared::pipeline) static DIVISIONS: Divisions = Divisions;

/// The section a branch's division scan is saved in.
pub(in crate::prepared::pipeline) fn divisions_state(branch: u64) -> String {
    format!("division-log-state-{branch}")
}

impl Own for Divisions {
    fn family(&self) -> &'static prepass::Family {
        &division_log::block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.divisions"
    }

    fn gates(&self) -> &'static [&'static str] {
        &[
            "division-log",
            "division-log-manifest",
            "division-log-totals",
        ]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let saved = load
            .read(&divisions_state(branch))?
            .map(zstd::stream::Decoder::new)
            .transpose()?;
        Ok(Box::new(DivisionsBranch {
            own: division_log::block::OwnBranch::load(saved, load.replay.format)?,
            resume: true,
        }))
    }
}

/// One branch's division log.
struct DivisionsBranch {
    own: division_log::block::OwnBranch,
    /// Whether the state was just loaded: hourly preparation resumes a
    /// branch's scan wherever it loads the branch.
    resume: bool,
}

impl DivisionsBranch {
    fn resume(&mut self, block: &Block<'_>) -> io::Result<()> {
        if std::mem::take(&mut self.resume)
            && let Some(frame) = before(block)?
        {
            self.own.resume(&frame);
        }
        Ok(())
    }
}

impl Branch for DivisionsBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.resume(block)?;
        self.own.fold_facts(
            facts,
            block.source,
            block.context,
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.resume(block)?;
        self.own.observe_hours(
            block.source,
            block.context,
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    /// Each block's window, the manifest and the totals since the last
    /// commit, which hourly preparation writes at each block's end.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own.publish_blocks(writer, branch, cancelled)
    }

    fn pending(&self) -> usize {
        self.own.pending_bytes()
    }

    fn flush(
        &mut self,
        _: &mut store::Writer,
        _: &Replay,
        _: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        Ok(())
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        write_state(writer, &divisions_state(branch), &self.own.state_json()?)
    }
}

/// Naval battles, ships sunk, naval losses and battle details: the family
/// hourly preparation names `naval`.
static NAVAL_FAMILY: prepass::Family = prepass::Family {
    name: "naval",
    extract: "naval.extract",
    sequential: "naval.sequential",
    fallback: "naval.fallback",
    // The extract finds the hours a sinking can first be read at.
    tables: &[TableId::Campaign, TableId::SunkShips],
    fields: Some(naval_fields),
    start: naval_start,
};

/// Each hour the naval family's own thread observes, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const NAVAL_OBSERVED: &str = "naval.observed";

/// Each hour it observes on a frame holding the trigger kit, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const NAVAL_PLACES: &str = "naval.places";

/// The tables naval observation reads: its own
/// ([`naval_battles::PREPARED_TABLES`]) and the shared trigger kit a sunk
/// ship's scripted place name evaluates (`triggers::RECORDED_TABLES`).
fn naval_tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = naval_battles::PREPARED_TABLES.to_vec();
        tables.extend_from_slice(crate::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// The naval extract reads the sunk ships by the changes of every field.
fn naval_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![TableFields::all(TableId::SunkShips)])
}

/// A block a pre-pass worker read for the naval family: the last hour at
/// which the sinking scan can first find a ship, where a sunk ship's place
/// name may be read; None when it can find none in the block.
struct PlaceHours {
    last: Option<i32>,
}

/// The naval block extract. The sinking scan finds ships only at a slot read
/// of the sunk ships, and there only when their rows were rebuilt since the
/// read it last went through, or when an entry dated after that read comes
/// due; equal rows rebuilt find no ship. So it can find one only at the
/// block's first read, a read after a change of the rows, or a read after
/// one whose rows hold an entry dated after it.
struct PlaceExtract {
    format: FormatVersion,
    /// The last read of the sunk ships before the hour observed.
    read: Option<i32>,
    /// Whether their rows changed since that read.
    changed: bool,
    /// The latest date of an entry of the rows, when complete as one row.
    latest: Option<i32>,
    last: Option<i32>,
}

impl PlaceExtract {
    /// A slot read of the sunk ships at `hour`.
    fn read_at(&mut self, hour: i32) {
        let due = self
            .latest
            .zip(self.read)
            .is_some_and(|(latest, read)| latest > read);
        if self.read.is_none() || self.changed || due {
            self.last = Some(hour);
        }
        self.read = Some(hour);
        self.changed = false;
    }
}

impl Extract for PlaceExtract {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if changes.rows_changed[TableId::SunkShips.index()] {
            self.changed = true;
            self.latest = match frame.complete::<tables::SunkShips>() {
                Ok([row]) => row.entries.iter().map(|entry| entry.sink_hour).max(),
                _ => None,
            };
        }
        if changes.slot_read[TableId::SunkShips.index()] {
            self.read_at(frame.hour);
        }
        Ok(())
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        for position in span {
            let hour = hours.hours()[position];
            if crate::metadata::slot_read(
                self.format,
                &hours.metrics()[position],
                TableId::SunkShips,
                hour,
            ) {
                self.read_at(hour);
            }
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(PlaceHours { last: self.last }))
    }
}

fn naval_start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(PlaceExtract {
        format,
        read: None,
        changed: false,
        latest: None,
        last: None,
    })
}

/// The hours of a block naval observation reads the trigger kit at.
#[derive(Clone, Copy)]
enum Places {
    /// Every hour.
    Every,
    /// Through this hour, and none after it; none at all when None.
    Through(Option<i32>),
}

/// The section a branch's naval scan is saved in.
pub(in crate::prepared::pipeline) fn naval_state(branch: u64) -> String {
    format!("naval-battles-state-{branch}")
}

/// The naval family.
pub(in crate::prepared::pipeline) struct Naval;

pub(in crate::prepared::pipeline) static NAVAL: Naval = Naval;

impl Own for Naval {
    fn family(&self) -> &'static prepass::Family {
        &NAVAL_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.naval"
    }

    fn shared_tables(&self, _: &super::Phase<'_>) -> &'static [TableId] {
        naval_tables()
    }

    fn gates(&self) -> &'static [&'static str] {
        &[
            "naval-battles",
            "naval-battles-manifest",
            "naval-battles-ranking",
            "naval-battle-details",
            "ships-sunk",
            "ships-sunk-manifest",
            "ships-sunk-battle-titles",
            "naval-losses",
        ]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let scan = match load.read(&naval_state(branch))? {
            Some(file) => serde_json::from_reader(zstd::stream::Decoder::new(file)?)?,
            None => naval_battles::Scan::default(),
        };
        Ok(Box::new(NavalBranch {
            scan: Box::new(scan),
            content: None,
            resume: true,
            #[cfg(test)]
            published: None,
        }))
    }
}

/// One branch's naval scan.
struct NavalBranch {
    scan: Box<naval_battles::Scan>,
    /// The sinking content, read once for this state as hourly preparation
    /// reads it once per run.
    content: Option<ships_sunk::PreparedContent>,
    /// Whether the state was just loaded: hourly preparation resumes a
    /// branch's scan wherever it loads the branch.
    resume: bool,
    /// The recording position of the last block it published, in tests.
    #[cfg(test)]
    published: Option<usize>,
}

/// `block`'s position in the recording.
#[cfg(test)]
fn block_position(block: &Block<'_>) -> usize {
    block
        .replay
        .index
        .iter()
        .position(|at| at.branch == block.index.branch && at.first_hour == block.index.first_hour)
        .expect("a recorded block")
}

#[cfg(test)]
thread_local! {
    static NAVAL_SAVES: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The recording position after the last block each naval scan saved on
/// this thread was published through, one per save, and none since.
#[cfg(test)]
pub(in crate::prepared::pipeline) fn take_naval_saves() -> Vec<usize> {
    NAVAL_SAVES.with(|saves| std::mem::take(&mut *saves.borrow_mut()))
}

impl NavalBranch {
    /// Observe the block hour by hour, as hourly preparation does, with the
    /// change facts of the block alone, on frames holding the trigger kit at
    /// the hours of `places`. The last hour observed.
    fn observe_hours(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
        places: Places,
    ) -> io::Result<Option<i32>> {
        if std::mem::take(&mut self.resume)
            && let Some(frame) = before(block)?
        {
            self.scan.resume(&frame);
        }
        let content = self
            .content
            .get_or_insert_with(|| ships_sunk::prepared_content(block.source, block.context));
        self.scan
            .configure(block.source, block.index.branch, content)?;
        let format = block.replay.format;
        // Cancellation is checked between hours; an hour's observation runs
        // whole, so its refusals are the walk's.
        let running = AtomicBool::new(false);
        let scan = &mut self.scan;
        let mut reached = None;
        let mut stopped = false;
        let mut failure = None;
        let mut step = |frame: &DecodedFrame, changes: &FrameChanges, kit: bool| {
            if stopped {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(test)]
            {
                prepass::note(NAVAL_OBSERVED);
                if kit {
                    prepass::note(NAVAL_PLACES);
                }
            }
            scan.observe_prepared(frame, changes);
            let result = scan.observe_details_read(
                block.source,
                block.context,
                frame,
                &running,
                changes,
                kit,
            );
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Naval observation stopped"))
                }
            }
        };
        let through = match places {
            Places::Through(Some(last)) if last >= block.index.last_hour => None,
            Places::Every => None,
            Places::Through(last) => Some(last),
        };
        let read = match through {
            None => {
                let mut tracker = FrameChangeTracker::new();
                block.read_alone(
                    naval_tables(),
                    |raw, frame| {
                        let changes = tracker.observe_updates(frame, raw, format);
                        step(frame, &changes, true)
                    },
                    || check(),
                )
            }
            Some(last) => {
                let tables = naval_battles::PREPARED_TABLES;
                let views = |frame: &DecodedFrame| -> Vec<SharedView> {
                    tables
                        .iter()
                        .map(|table| frame.tables[table.index()].clone())
                        .collect()
                };
                // One tracker over both reads, and the first read's views of
                // the naval tables at its last hour.
                let mut tracker = FrameChangeTracker::new();
                let mut kept: Option<Vec<SharedView>> = None;
                let read = match last {
                    Some(last) => block.read_through(
                        naval_tables(),
                        last,
                        |raw, frame| {
                            let changes = tracker.observe_updates(frame, raw, format);
                            if frame.hour == last {
                                kept = Some(views(frame));
                            }
                            step(frame, &changes, true)
                        },
                        || check(),
                    ),
                    None => Ok(()),
                };
                // The naval tables alone after `last`, from a read of their
                // own from the block's first hour. A table still holding the
                // allocation it held at `last` is handed as the first read's
                // view there, one rebuilt since as its new view: each table's
                // allocations, and so the change facts and every comparison
                // of views, are those of one read.
                read.and_then(|()| {
                    let mut at_last: Option<Vec<ViewIdentity>> = None;
                    block.read_alone(
                        tables,
                        |raw, frame| {
                            if last.is_some_and(|last| frame.hour <= last) {
                                if Some(frame.hour) == last {
                                    at_last = Some(
                                        tables
                                            .iter()
                                            .map(|table| frame.tables[table.index()].identity())
                                            .collect(),
                                    );
                                }
                                return Ok(());
                            }
                            let carried;
                            let frame = match kept.as_ref().zip(at_last.as_ref()) {
                                Some((kept, at_last)) => {
                                    let mut handed = frame.clone();
                                    for (at, table) in tables.iter().enumerate() {
                                        if at_last[at].is(&frame.tables[table.index()]) {
                                            handed.tables[table.index()] = kept[at].clone();
                                        }
                                    }
                                    carried = handed;
                                    &carried
                                }
                                None => frame,
                            };
                            let changes = tracker.observe_updates(frame, raw, format);
                            step(frame, &changes, false)
                        },
                        || check(),
                    )
                })
            }
        };
        if let Some(error) = failure {
            return Err(error);
        }
        read?;
        Ok(reached)
    }
}

impl Branch for NavalBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let facts = facts
            .downcast::<PlaceHours>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
        let reached = self.observe_hours(block, observed, check, Places::Through(facts.last))?;
        Ok((Folded::Fast, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.observe_hours(block, observed, check, Places::Every)
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    /// The block's windows, records, sinking pages and details, which hourly
    /// preparation writes at the block's end, held for the next commit; its
    /// publication reads back pages earlier blocks wrote.
    fn publish_block(&mut self, pages: &mut dyn Pages, block: &Block<'_>) -> io::Result<()> {
        // Cancellation is checked between blocks.
        let running = AtomicBool::new(false);
        let ordinal = ordinal(block)?;
        self.scan
            .publish_configured(block.replay, pages, block.index.branch, ordinal, &running)?;
        #[cfg(test)]
        {
            self.published = Some(block_position(block));
        }
        Ok(())
    }

    fn pending(&self) -> usize {
        0
    }

    fn flush(
        &mut self,
        _: &mut store::Writer,
        _: &Replay,
        _: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        Ok(())
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        #[cfg(test)]
        if let Some(position) = self.published {
            NAVAL_SAVES.with(|saves| saves.borrow_mut().push(position + 1));
        }
        write_state(
            writer,
            &naval_state(branch),
            &serde_json::to_vec(&*self.scan)?,
        )
    }
}
