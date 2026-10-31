//! The economy ledgers prepared on their own: construction, production and
//! country experience. Each is read daily:
//! its observer acts when a field it reads changes or when one of its tables'
//! daily reads is dated, and an hour between does nothing but advance its
//! accepted hours. So a block is read by its changes ([`prepass::Family`]
//! fields), with each family's daily read hour as the extract's deadline: a
//! pre-pass worker keeps the frames of the hours the family observes (the
//! block's first, each hour a watched field, row key or availability changes,
//! each hour one of its daily reads is due by the clock or a watched table
//! was read) and the spans of quiet hours between them. A dated read inside a
//! quiet span that the clock does not predict (another table of its slot read
//! off its hour) leaves the rest of the block to hourly observation.
//!
//! The family's own thread folds the block in order: each kept hour is
//! observed on the carried state exactly as hourly preparation observes it,
//! and each quiet span advances the state over its hours in constant time
//! (the family's `advance_quiet`, the same effect observing each hour has).
//! When the carried state cannot advance so, the rest of the block is
//! observed hour by hour with the block's own change facts.
//!
//! Each family saves its continuation, which hourly preparation keeps in its
//! branch state, as `<family>-continuation-<branch>`, and publishes the pages
//! hourly preparation publishes, flushing where hourly preparation flushes.

use super::super::compressed_json;
use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{self, Extract, Folded},
    },
    prepared_production::{self, ProductionAccumulator},
    prepared_stability::ExperienceAccumulator,
    products::{
        construction_log::{self, NameSource},
        country_groups::HourSpan,
    },
};
use replay::{container::QuietHours, container::Replay, container::TableFields};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::{DecodedFrame, FrameLog};
use serde::{Serialize, de::DeserializeOwned};
use std::{any::Any, io, ops::Range, sync::atomic::AtomicBool};

/// One hour of a block's facts for a ledger, in hour order.
enum Hour {
    /// An hour the family observes, with its change facts; its frame is the
    /// next the block's kept frames hold.
    Observed { changes: FrameChanges },
    /// Hours `first..=last`, after the hour before, at which nothing the
    /// family reads changed and none of its daily reads is dated.
    Quiet { first: i32, last: i32 },
    /// An hour one of the family's daily reads is dated with no frame read
    /// there: the rest of the block is observed hour by hour from it.
    Hourly(i32),
}

/// A ledger's facts of one block.
struct Facts {
    hours: Vec<Hour>,
    kept: FrameLog,
    /// Frames the extract was given: read or built at its deadlines.
    #[cfg(test)]
    frames: usize,
}

/// A ledger's block extract: the frames of the hours its family observes and
/// the quiet spans between them.
struct DailyExtract {
    format: FormatVersion,
    /// The tables whose daily reads the family's observation dates
    /// (`FrameChanges::slot_read`).
    reads: &'static [TableId],
    hours: Vec<Hour>,
    kept: FrameLog,
    /// The last hour given a frame.
    last: Option<i32>,
    /// After an [`Hour::Hourly`], nothing more is kept.
    stopped: bool,
    #[cfg(test)]
    frames: usize,
}

impl DailyExtract {
    fn start(format: FormatVersion, reads: &'static [TableId]) -> Box<dyn Extract> {
        Box::new(Self {
            format,
            reads,
            hours: Vec::new(),
            kept: FrameLog::new(),
            last: None,
            stopped: false,
            #[cfg(test)]
            frames: 0,
        })
    }

    /// Whether one of the family's daily reads is dated at `position` of
    /// `hours`, as `FrameChanges::slot_read` dates it.
    fn read_due(&self, hours: &QuietHours, position: usize) -> bool {
        let hour = hours.hours()[position];
        self.reads.iter().any(|&table| {
            crate::metadata::slot_read(self.format, &hours.metrics()[position], table, hour)
        })
    }
}

impl Extract for DailyExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            if let Hour::Observed { changes } = hour {
                held.changes(changes);
            }
        }
        for view in self.kept.views() {
            held.view(view);
        }
        held.add(self.kept.bytes());
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(test)]
        {
            self.frames += 1;
        }
        if self.stopped {
            return Ok(());
        }
        self.last = Some(frame.hour);
        self.kept.push(frame)?;
        self.hours.push(Hour::Observed {
            changes: FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
                updates: None,
            },
        });
        Ok(())
    }

    /// The next hour at which one of the family's daily reads is due by the
    /// clock, observed although no field changes there: its slot's hour of
    /// the day (every hour for an hourly or sliced table).
    fn deadline(&self) -> Option<i32> {
        if self.stopped {
            return None;
        }
        let last = self.last?;
        (1..=replay_data::cadence::STAGGER_PERIOD)
            .filter_map(|ahead| last.checked_add(ahead))
            .find(|&hour| {
                self.reads.iter().any(|&table| {
                    replay_data::cadence::cadence_in(table, self.format).is_refresh(hour)
                })
            })
    }

    /// Consecutive hours at which nothing the family reads changed. One
    /// dating a daily read off the clock ends what is kept of the block.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        for position in span {
            let hour = hours.hours()[position];
            if self.read_due(hours, position) {
                self.hours.push(Hour::Hourly(hour));
                self.stopped = true;
                return Ok(());
            }
            match self.hours.last_mut() {
                Some(Hour::Quiet { last, .. }) => *last = hour,
                _ => self.hours.push(Hour::Quiet {
                    first: hour,
                    last: hour,
                }),
            }
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Ledger block has no hours"));
        }
        Ok(Box::new(Facts {
            hours: self.hours,
            kept: self.kept,
            #[cfg(test)]
            frames: self.frames,
        }))
    }
}

/// A ledger's branch state, observing as hourly preparation observes it.
trait Ledger {
    /// Before a block's hours: bind what observation reads of the source.
    fn prepare(&mut self, block: &Block<'_>) -> io::Result<()>;
    /// One hour, as hourly preparation observes it.
    fn observe(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()>;
    /// Hours `first..=last` following `frame`, the last observed, at which
    /// nothing the family reads changed and none of its daily reads is
    /// dated: what observing each does. False when the carried state cannot
    /// advance over them without observing them.
    fn quiet(&mut self, first: i32, last: i32, frame: &DecodedFrame) -> io::Result<bool>;
    /// After the block's last hour, `last`, where hourly preparation ends the
    /// block for the family.
    fn finish_block(&mut self, last: i32) -> io::Result<()> {
        let _ = last;
        Ok(())
    }
    /// Before observation goes on from the block read again from its
    /// keyframe, that read's frame of the last hour already observed: a
    /// family that tells tables apart by allocation takes that read's.
    fn rebase(&mut self, frame: &DecodedFrame) {
        let _ = frame;
    }
}

/// Fold one block's facts into the carried state. Returns how it folded and
/// the last hour it reached.
fn fold(
    ledger: &mut impl Ledger,
    family: &'static prepass::Family,
    facts: Box<dyn Any + Send>,
    block: &Block<'_>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<Facts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(test)]
    count(family.name, |counts| counts.frames += facts.frames);
    ledger.prepare(block)?;
    let Facts { hours, kept, .. } = *facts;
    let mut kept = kept.into_frames();
    let mut reached = None;
    let mut last: Option<DecodedFrame> = None;
    for hour in hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed { changes } => {
                // Released first, so the next frame changes it in place.
                drop(last.take());
                let frame = kept.next().ok_or_else(|| {
                    io::Error::other("Ledger facts lack an observed hour's frame")
                })??;
                reached = Some(frame.hour);
                #[cfg(test)]
                count(family.name, |counts| counts.observed += 1);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    ledger.observe(&frame, &changes)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
                last = Some(frame);
            }
            Hour::Quiet { first, last: end } => {
                let advanced = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    match &last {
                        Some(frame) => ledger.quiet(first, end, frame),
                        None => Ok(false),
                    }
                };
                match advanced {
                    Ok(true) => {
                        reached = Some(end);
                        #[cfg(test)]
                        count(family.name, |counts| {
                            counts.quiet += usize::try_from(end - first + 1).unwrap_or(0);
                        });
                    }
                    Ok(false) => {
                        let reached =
                            observe_block(ledger, family, block, Some(first), observed, check)?;
                        return Ok((Folded::Fallback, reached));
                    }
                    // A refusal at the span's first hour, as observing it
                    // would have refused there.
                    Err(error) => {
                        observed(first, Err(error))?;
                        return Ok((Folded::Fast, Some(first)));
                    }
                }
            }
            Hour::Hourly(from) => {
                let reached = observe_block(ledger, family, block, Some(from), observed, check)?;
                return Ok((Folded::Fallback, reached));
            }
        }
    }
    if let Some(end) = reached {
        observed(end, ledger.finish_block(end))?;
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), with
/// the change facts of the block alone, as hourly preparation does, then end
/// the block. Returns the last hour observed.
fn observe_block(
    ledger: &mut impl Ledger,
    family: &'static prepass::Family,
    block: &Block<'_>,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    ledger.prepare(block)?;
    let format = block.replay.format;
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    // The hour before `from` in this read, which the fold already observed.
    let mut before: Option<DecodedFrame> = None;
    let read = block.replay.read_block_recorded_alone(
        block.index,
        family.tables,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let facts = tracker.observe_updates(frame, raw, format);
            if stopped {
                return Ok(());
            }
            if from.is_some_and(|from| frame.hour < from) {
                if frame.hour.checked_add(1) == from {
                    before = Some(frame.clone());
                }
                return Ok(());
            }
            if let Some(before) = before
                .take()
                .filter(|before| before.hour.checked_add(1) == Some(frame.hour))
            {
                ledger.rebase(&before);
            }
            reached = Some(frame.hour);
            #[cfg(test)]
            count(family.name, |counts| counts.observed += 1);
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                ledger.observe(frame, &facts)
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Ledger observation stopped"))
                }
            }
        },
        check,
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    if !stopped && let Some(end) = reached {
        observed(end, ledger.finish_block(end))?;
    }
    Ok(reached)
}

/// A family's continuation section of one branch.
fn continuation(family: &str, branch: u64) -> String {
    crate::prepared::driver::registry::continuation_section(family, branch)
}

/// Write a continuation, compressed as the hourly branch state holding it is.
fn save<T: Serialize>(writer: &mut store::Writer, section: &str, value: &T) -> io::Result<()> {
    writer.write_at(section, 0, compressed_json(value)?.as_slice())
}

/// A continuation the last commit saved, if it has one.
fn read<T: DeserializeOwned>(load: &Load<'_>, section: &str) -> io::Result<Option<T>> {
    let Some(file) = load.read(section)? else {
        return Ok(None);
    };
    let decoder = zstd::stream::Decoder::new(file)?;
    Ok(Some(serde_json::from_reader(io::BufReader::new(decoder))?))
}

/// The branch's replay blocks, as `Source::block_windows` lists them.
fn block_windows(replay: &Replay, branch: u64) -> Vec<HourSpan> {
    replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| HourSpan {
            first_hour: block.first_hour,
            last_hour: block.last_hour,
        })
        .collect()
}

// --- Construction ----------------------------------------------------------

static CONSTRUCTION_FAMILY: prepass::Family = prepass::Family {
    name: "construction",
    extract: "construction.extract",
    sequential: "construction.sequential",
    fallback: "construction.fallback",
    tables: construction_log::TABLES,
    fields: Some(construction_fields),
    start: construction_start,
};

/// What the construction scan reads, so a block is read only at the hours
/// these change or the queues' daily reads are due.
fn construction_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Owners' tags and provinces' states are read at the queue reads
        // alone, whose frames are kept whole: their keys and availability.
        // The balances and provinces' other fields change every hour.
        TableFields::keys(TableId::Countries),
        TableFields::keys(TableId::Provinces),
        TableFields::all(TableId::CountryLinks),
        // Every hour compares each building's level and healthy level (the
        // key is its location and type); partial health and the status flags
        // move while a building is damaged or repairing, and are not read.
        TableFields::named(TableId::StateBuildings, &["level", "healthy_level"])?,
        TableFields::named(TableId::ProvinceBuildings, &["level", "healthy_level"])?,
        // The daily queues and output, at each read: rows change only there.
        TableFields::all(TableId::BuildingConstruction).when_read(),
        TableFields::all(TableId::RailwayConstruction).when_read(),
        TableFields::all(TableId::ConstructionLineOutput).when_read(),
        // The dictionary is taken by allocation: every rebuild is observed.
        TableFields::all(TableId::LexicalTokens),
    ])
}

fn construction_start(format: FormatVersion) -> Box<dyn Extract> {
    DailyExtract::start(format, &construction_log::READ_TABLES)
}

/// Construction log and spending.
pub(in crate::prepared::pipeline) struct Construction;

pub(in crate::prepared::pipeline) static CONSTRUCTION: Construction = Construction;

impl Own for Construction {
    fn family(&self) -> &'static prepass::Family {
        &CONSTRUCTION_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.construction"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let saved = read(load, &continuation("construction", branch))?;
        Ok(Box::new(ConstructionBranch {
            restore: saved.is_some(),
            prepared: saved.unwrap_or_default(),
            format: load.replay.format,
            windows: block_windows(load.replay, branch),
            names: None,
        }))
    }
}

/// One branch's construction scan and projection.
struct ConstructionBranch {
    prepared: Box<construction_log::prepared::Prepared>,
    format: FormatVersion,
    windows: Vec<HourSpan>,
    /// The source's naming handles, from the first block observed.
    names: Option<NameSource>,
    /// Loaded from a commit: its decoder operands (building tables and
    /// dictionary) are restored from the hour before its next block once,
    /// as hourly preparation restores them from its stream's last frame.
    restore: bool,
}

impl ConstructionBranch {
    /// Hourly preparation restores the scan's decoder operands at each block
    /// from the frame its stream last accepted (the block before in recording
    /// order) when the scan's last hour is that frame's. Held in memory, the
    /// scan already holds them; loaded, it takes them from that frame, read
    /// alone.
    fn restore_previous(&mut self, block: &Block<'_>) -> io::Result<()> {
        let replay = block.replay;
        let Some(position) = replay
            .index
            .iter()
            .position(|index| std::ptr::eq(index, block.index))
        else {
            return Ok(());
        };
        let Some(previous) = position.checked_sub(1).map(|before| &replay.index[before]) else {
            return Ok(());
        };
        if self.prepared.last_hour() != Some(previous.last_hour) {
            return Ok(());
        }
        let last = super::shared::read_last(replay, previous, construction_log::TABLES)?;
        if let Some(frame) = last {
            self.prepared.restore_frame(&frame);
        }
        Ok(())
    }
}

impl Ledger for ConstructionBranch {
    fn prepare(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        if self.names.is_none() {
            self.names = Some(NameSource::of(block.source, block.context));
        }
        if std::mem::take(&mut self.restore) {
            self.restore_previous(block)?;
        }
        Ok(())
    }

    fn observe(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        self.prepared
            .observe_changed_in(frame, self.format, &self.windows, changes)
    }

    fn quiet(&mut self, first: i32, last: i32, frame: &DecodedFrame) -> io::Result<bool> {
        self.prepared.advance_quiet(first, last, frame)
    }

    fn finish_block(&mut self, last: i32) -> io::Result<()> {
        self.prepared.end_block_in(last, &self.windows)
    }

    /// The dictionary is taken by allocation: the fold's held one becomes
    /// the fresh read's, as hourly preparation's stream holds one allocation.
    fn rebase(&mut self, frame: &DecodedFrame) {
        self.prepared.rebase(frame);
    }
}

impl Branch for ConstructionBranch {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        fold(self, &CONSTRUCTION_FAMILY, facts, block, observed, check)
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        observe_block(self, &CONSTRUCTION_FAMILY, block, None, observed, check)
    }

    /// The block's end ran with its last hour (`Ledger::finish_block`).
    fn pending(&self) -> usize {
        self.prepared.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let names = self
            .names
            .as_ref()
            .ok_or_else(|| io::Error::other("Construction flushed before observing a block"))?;
        self.prepared
            .flush_in(writer, replay, &self.windows, names, branch, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        save(
            writer,
            &continuation("construction", branch),
            &*self.prepared,
        )
    }
}

// --- Production ------------------------------------------------------------

/// The output and variant reads production dates.
const PRODUCTION_READS: &[TableId] = &[
    TableId::ProductionLineOutput,
    TableId::RocketProductionOutput,
    TableId::VariantDefinitions,
];

static PRODUCTION_FAMILY: prepass::Family = prepass::Family {
    name: "production",
    extract: "production.extract",
    sequential: "production.sequential",
    fallback: "production.fallback",
    tables: prepared_production::TABLES,
    fields: Some(production_fields),
    start: production_start,
};

/// What production reads.
fn production_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Holders are the countries' keys, slots and tags.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::all(TableId::CountryLinks),
        // Production ingests again at any allocation change of these tables
        // (`ingest_changed`), so each is watched whole and every rebuild is
        // observed. The lines and output are daily; the variant catalogue's
        // tables change at their reads or with a country's appearance.
        TableFields::all(TableId::CountryAppearance),
        TableFields::all(TableId::LexicalTokens),
        TableFields::all(TableId::VariantDefinitions).when_read(),
        TableFields::all(TableId::VariantModules),
        TableFields::all(TableId::VariantUpgrades),
        TableFields::all(TableId::ProductionLines),
        TableFields::all(TableId::ProductionLineOutput).when_read(),
        TableFields::all(TableId::RocketProductionOutput).when_read(),
    ])
}

fn production_start(format: FormatVersion) -> Box<dyn Extract> {
    DailyExtract::start(format, PRODUCTION_READS)
}

/// Produced equipment and production spending.
pub(in crate::prepared::pipeline) struct Production;

pub(in crate::prepared::pipeline) static PRODUCTION: Production = Production;

impl Own for Production {
    fn family(&self) -> &'static prepass::Family {
        &PRODUCTION_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.production"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let accumulator: Box<ProductionAccumulator> =
            match read(load, &continuation("production", branch))? {
                Some(accumulator) => accumulator,
                None => {
                    // As hourly preparation constructs a branch's state at the
                    // branch's first block.
                    let first = load
                        .replay
                        .index
                        .iter()
                        .find(|block| block.branch == branch)
                        .map_or(0, |block| block.first_hour);
                    Box::new(ProductionAccumulator::new(
                        load.replay.format.version(),
                        branch,
                        first,
                    ))
                }
            };
        if accumulator.schema != prepared_production::SCHEMA {
            return Err(super::super::invalid(
                "Unsupported prepared production state schema",
            ));
        }
        Ok(Box::new(ProductionBranch {
            accumulator,
            format: load.replay.format,
        }))
    }
}

/// One branch's production accumulator, which holds the facts of the
/// endpoints it completed since its last flush.
struct ProductionBranch {
    accumulator: Box<ProductionAccumulator>,
    format: FormatVersion,
}

impl Ledger for ProductionBranch {
    fn prepare(&mut self, _: &Block<'_>) -> io::Result<()> {
        Ok(())
    }

    /// Ingested as hourly preparation ingests it on the coordinator.
    fn observe(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        // Cancellation is checked between hours.
        let cancelled = AtomicBool::new(false);
        self.accumulator
            .ingest_changed(self.format, frame, &cancelled, changes)
    }

    fn quiet(&mut self, first: i32, last: i32, _: &DecodedFrame) -> io::Result<bool> {
        self.accumulator.advance_quiet(first, last)
    }
}

impl Branch for ProductionBranch {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        fold(self, &PRODUCTION_FAMILY, facts, block, observed, check)
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        observe_block(self, &PRODUCTION_FAMILY, block, None, observed, check)
    }

    fn pending(&self) -> usize {
        self.accumulator.buffered_bytes()
    }

    /// The open endpoint, the batch's facts and the spent series, as hourly
    /// preparation flushes them: nothing of the batch is left in memory that
    /// a reloaded continuation would lack.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        _: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.accumulator.flush(writer, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        save(
            writer,
            &continuation("production", branch),
            &*self.accumulator,
        )
    }
}

// --- Experience ------------------------------------------------------------

/// The tables the country-experience scan reads.
const EXPERIENCE_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperience,
    TableId::CountryExperienceGains,
];

/// The banked-experience read the scan dates.
const EXPERIENCE_READS: &[TableId] = &[TableId::CountryExperience];

static EXPERIENCE_FAMILY: prepass::Family = prepass::Family {
    name: "experience",
    extract: "experience.extract",
    sequential: "experience.sequential",
    fallback: "experience.fallback",
    tables: EXPERIENCE_TABLES,
    fields: Some(experience_fields),
    start: experience_start,
};

/// What the experience scan reads.
fn experience_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Holders are the countries' keys, slots and tags (`CountryKeys`).
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::all(TableId::CountryLinks),
        // The scan compares each country's banked and gain rows whole, so
        // every field is watched: banked experience changes at its daily
        // read, and the hourly gains when an award or its completeness
        // changes; an hour whose gains equal the last one's is quiet.
        TableFields::all(TableId::CountryExperience).when_read(),
        TableFields::all(TableId::CountryExperienceGains),
    ])
}

fn experience_start(format: FormatVersion) -> Box<dyn Extract> {
    DailyExtract::start(format, EXPERIENCE_READS)
}

/// Country experience, apart from the hourly stability family.
pub(in crate::prepared::pipeline) struct Experience;

pub(in crate::prepared::pipeline) static EXPERIENCE: Experience = Experience;

impl Own for Experience {
    fn family(&self) -> &'static prepass::Family {
        &EXPERIENCE_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.experience"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        Ok(Box::new(ExperienceBranch {
            accumulator: read(load, &continuation("experience", branch))?.unwrap_or_default(),
            format: load.replay.format,
        }))
    }
}

/// One branch's experience scan and series.
struct ExperienceBranch {
    accumulator: Box<ExperienceAccumulator>,
    format: FormatVersion,
}

impl Ledger for ExperienceBranch {
    fn prepare(&mut self, _: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        Ok(())
    }

    fn observe(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        self.accumulator
            .observe_changes(frame, self.format, changes)
    }

    fn quiet(&mut self, first: i32, last: i32, _: &DecodedFrame) -> io::Result<bool> {
        self.accumulator.advance_quiet(first, last)
    }
}

impl Branch for ExperienceBranch {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        fold(self, &EXPERIENCE_FAMILY, facts, block, observed, check)
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        observe_block(self, &EXPERIENCE_FAMILY, block, None, observed, check)
    }

    fn pending(&self) -> usize {
        self.accumulator.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.accumulator.flush(writer, branch, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        save(
            writer,
            &continuation("experience", branch),
            &*self.accumulator,
        )
    }
}

// --- Test counts -------------------------------------------------------------

/// What one family's folds did, including work carried from its worker.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::prepared::pipeline) struct Counts {
    /// Frames its extracts were given (read, or built at a deadline).
    pub frames: usize,
    /// Hours it observed on carried state.
    pub observed: usize,
    /// Hours its quiet spans advanced over.
    pub quiet: usize,
}

/// Shared note names let the independent worker's existing count transfer
/// carry all three measurements back to the caller.
#[cfg(test)]
fn count_names(family: &str) -> Option<[&'static str; 3]> {
    Some(match family {
        "construction" => [
            "construction.frames",
            "construction.observed",
            "construction.quiet",
        ],
        "production" => [
            "production.frames",
            "production.observed",
            "production.quiet",
        ],
        "experience" => [
            "experience.frames",
            "experience.observed",
            "experience.quiet",
        ],
        _ => return None,
    })
}

#[cfg(test)]
fn count(family: &'static str, update: impl FnOnce(&mut Counts)) {
    let mut delta = Counts::default();
    update(&mut delta);
    let [frames, observed, quiet] = count_names(family).expect("counted ledger family");
    prepass::add_notes(&std::collections::BTreeMap::from([
        (frames, delta.frames),
        (observed, delta.observed),
        (quiet, delta.quiet),
    ]));
}

/// `family`'s counts, including work carried from preparation workers.
#[cfg(test)]
pub(in crate::prepared::pipeline) fn counts(family: &str) -> Counts {
    let Some([frames, observed, quiet]) = count_names(family) else {
        return Counts::default();
    };
    Counts {
        frames: prepass::noted(frames),
        observed: prepass::noted(observed),
        quiet: prepass::noted(quiet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_compression_preserves_its_bytes() {
        let value: std::collections::BTreeMap<_, _> = (0..5_000)
            .map(|id| (format!("holder {id}"), vec![id; 24]))
            .collect();
        let mut direct = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
        serde_json::to_writer(&mut direct, &value).unwrap();
        assert_eq!(direct.finish().unwrap(), compressed_json(&value).unwrap());
    }
}
