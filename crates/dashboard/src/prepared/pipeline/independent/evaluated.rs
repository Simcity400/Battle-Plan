//! The evaluated families prepared on their own: research, special projects
//! and occupation. Each is a family of its own, with its own registry entry,
//! coverage, refusals and continuation; none reads another's state, and no
//! family of the hourly walk reads theirs in the same hour (the event feed
//! reads the projects' published completions), so the walk skips all three.
//!
//! Research and occupation cannot be read only at the hours their fields
//! change:
//!
//! - Research reads one country slice every hour (`SliceReads`), and a read
//!   moves progress, so no hour is quiet. Each country's carried state (its
//!   open assignments, last read, progress samples and continuity, and the
//!   carry deadline of its last read) decides what each read adds, and
//!   assignments run for months, across any number of blocks, so a block's
//!   countries cannot be observed apart from the block before. Its indexes
//!   (the holders, owners and read hours by slot) follow from the frames
//!   alone: a pre-pass worker refreshes them and keeps each hour's index
//!   changes and fresh reads, and the fold carries every country through
//!   them without reading the block (`research::prepared::block`).
//! - Occupation reads its tables daily, decides law eligibility through the
//!   trigger world at each daily read, and projects owners and controllers
//!   hour by hour by the allocations of their views.
//!
//! Special projects are: a pre-pass worker reads each block only at the
//! hours a field their observer or a scoped name's declared reads read
//! changes, at each new allocation of the dictionary and at each completion
//! date, and the family folds the hours it kept, observing the rest of a
//! block hour by hour where a scoped name cannot be told the same without
//! the whole frame (`special_projects::prepared::block`).
//!
//! Occupation's own thread therefore observes each block hour by hour, as
//! the hourly walk observes it: on the block read alone with the tables its
//! observer reads, with the change facts and recorded updates of the block
//! alone, which are the walk's at every hour (every block starts at a
//! keyframe, whose every table is a new allocation in both reads), as does
//! the projects' where their fold does not continue. Occupation has no
//! pre-pass lane: no block is read on the pool for it. Each
//! binds its content at every block, as the walk does, and flushes and saves
//! where hourly preparation does.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{self, Extract, Folded},
    },
    products::{occupied_states, research, special_projects},
};
use replay::container::Replay;
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{
    any::Any,
    io,
    sync::{LazyLock, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// A block a pre-pass worker read for one of these families.
struct BlockRead;

/// The block extract of a family whose own thread reads its blocks: it reads
/// the block's campaign identity and keeps nothing.
struct ReadExtract;

impl Extract for ReadExtract {
    fn observe(&mut self, _: &Replay, _: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(BlockRead))
    }
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(ReadExtract)
}

/// `tables` with the campaign every read checks, sorted and each once.
fn read_tables(tables: impl IntoIterator<Item = TableId>) -> Vec<TableId> {
    let mut tables: Vec<_> = tables.into_iter().collect();
    tables.push(TableId::Campaign);
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// The facts a worker left: the block was read and nothing kept.
fn read_facts(facts: Box<dyn Any + Send>) -> io::Result<()> {
    facts
        .downcast::<BlockRead>()
        .map(|_| ())
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))
}

/// A measured run's split of a family's own thread: the time it read its
/// blocks (decoding its tables and their change facts) and the time it
/// observed their hours, under the family's two timing rows.
struct Measured {
    rows: (&'static str, &'static str),
    read: Duration,
    observe: Duration,
    blocks: u64,
    hours: u64,
}

impl Measured {
    fn new(rows: (&'static str, &'static str)) -> Self {
        Self {
            rows,
            read: Duration::ZERO,
            observe: Duration::ZERO,
            blocks: 0,
            hours: 0,
        }
    }

    /// The parts since last asked, as [`Branch::parts`] gives them.
    fn take(&mut self) -> Vec<(&'static str, Duration, u64)> {
        let taken = vec![
            (self.rows.0, self.read, self.blocks),
            (self.rows.1, self.observe, self.hours),
        ];
        *self = Self::new(self.rows);
        taken
    }
}

/// Observe `block` hour by hour on the block read alone with `tables`, with
/// the change facts and recorded updates of the block alone, as the hourly
/// walk observes it; `counted` names each hour observed in tests, and
/// `measured`, in a measured run, splits the time read from the time
/// observed. An hour's observation runs whole: cancellation is checked
/// between hours, so the observation's refusals are the walk's. The last
/// hour observed.
fn observe_alone(
    block: &Block<'_>,
    tables: &[TableId],
    counted: &'static str,
    measured: Option<&mut Measured>,
    mut observe: impl FnMut(&DecodedFrame, &FrameChanges) -> io::Result<()>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let format = block.replay.format;
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let started = measured.is_some().then(Instant::now);
    let mut observing = Duration::ZERO;
    let mut hours = 0;
    let read = block.read_alone(
        tables,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let changes = tracker.observe_updates(frame, raw, format);
            if stopped {
                return Ok(());
            }
            reached = Some(frame.hour);
            prepass::note(counted);
            let at = started.map(|_| Instant::now());
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                observe(frame, &changes)
            };
            if let Some(at) = at {
                observing += at.elapsed();
                hours += 1;
            }
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Family observation stopped"))
                }
            }
        },
        check,
    );
    if let (Some(measured), Some(started)) = (measured, started) {
        measured.read += started.elapsed().saturating_sub(observing);
        measured.observe += observing;
        measured.blocks += 1;
        measured.hours += hours;
    }
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

// Research.

/// Each hour research's fold observes, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const RESEARCH_OBSERVED: &str =
    research::prepared::block::OBSERVED;

/// Each of those hours it took from a worker's index changes and kept rows
/// rather than from a frame, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const RESEARCH_SHIPPED: &str = research::prepared::block::SHIPPED;

/// Research's continuation, which hourly preparation keeps in its branch
/// state as `research`.
pub(in crate::prepared::pipeline) fn research_state(branch: u64) -> String {
    format!("research-state-{branch}")
}

/// Research slot assignments, progress and coverage by country. A pre-pass
/// worker refreshes its indexes and keeps each hour's index changes and
/// fresh reads; the fold carries every country's state through them
/// (`research::prepared::block`).
pub(in crate::prepared::pipeline) struct Research;

pub(in crate::prepared::pipeline) static RESEARCH: Research = Research;

impl Own for Research {
    fn family(&self) -> &'static prepass::Family {
        &research::prepared::block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.research"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = match load.read(&research_state(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => research::prepared::Prepared::default(),
        };
        Ok(Box::new(ResearchBranch {
            prepared: Box::new(prepared),
            measured: None,
        }))
    }
}

/// One branch's research observer, and in a measured run the split of its
/// thread's time between folding blocks (`research.fold`) and reading the
/// blocks it observes hour by hour itself (`research.read`).
struct ResearchBranch {
    prepared: Box<research::prepared::Prepared>,
    measured: Option<Measured>,
}

impl Branch for ResearchBranch {
    fn measure(&mut self) {
        self.measured = Some(Measured::new(("research.fold", "research.read")));
    }

    fn parts(&mut self) -> Vec<(&'static str, Duration, u64)> {
        self.measured
            .as_mut()
            .map(Measured::take)
            .unwrap_or_default()
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let started = self.measured.is_some().then(Instant::now);
        let folded = research::prepared::block::fold(
            &mut self.prepared,
            facts,
            block.replay,
            block.index,
            observed,
            check,
        );
        if let (Some(measured), Some(started)) = (self.measured.as_mut(), started) {
            measured.read += started.elapsed();
            measured.blocks += 1;
        }
        folded
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let started = self.measured.is_some().then(Instant::now);
        let reached = research::prepared::block::observe_block(
            &mut self.prepared,
            block.replay,
            block.index,
            observed,
            check,
        );
        if let (Some(measured), Some(started)) = (self.measured.as_mut(), started) {
            measured.observe += started.elapsed();
            measured.hours += 1;
        }
        reached
    }

    fn pending(&self) -> usize {
        self.prepared.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &research_state(branch), &*self.prepared)
    }
}

// Special projects.

/// Each hour the projects' fold observes, counted in tests.
pub(in crate::prepared::pipeline) const PROJECTS_OBSERVED: &str =
    special_projects::prepared::block::OBSERVED;

/// Each block the projects' fold or own read observes with content whose
/// scoped names read the trigger kit, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const PROJECTS_WORLD: &str = "projects.world";

/// The projects' continuation, which hourly preparation keeps in its branch
/// state as `projects`.
pub(in crate::prepared::pipeline) fn projects_state(branch: u64) -> String {
    format!("special-projects-state-{branch}")
}

/// Special projects, their completions and their names. A pre-pass worker
/// reads each block only at the hours a field the observer or a scoped
/// name's declared reads read changes, and the family folds what it kept
/// (`special_projects::prepared::block`).
pub(in crate::prepared::pipeline) struct Projects;

pub(in crate::prepared::pipeline) static PROJECTS: Projects = Projects;

impl Own for Projects {
    fn family(&self) -> &'static prepass::Family {
        &special_projects::prepared::block::FAMILY
    }

    fn binds(&self) -> bool {
        true
    }

    /// Without a scoped project name in the bound content, blocks are read
    /// without the trigger kit, which nothing the observer evaluates then
    /// reads. Content whose bind fails (cancellation) reads as declared.
    fn bound(&self, phase: &super::Phase<'_>) -> Option<prepass::Bound> {
        let mut prepared = special_projects::prepared::Prepared::new(phase.replay, 0);
        prepared.configure(phase.source, phase.context).ok()?;
        special_projects::prepared::block::bound(prepared.reads_trigger_world())
    }

    fn timing(&self) -> &'static str {
        "independent.projects"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = match load.read(&projects_state(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => special_projects::prepared::Prepared::new(load.replay, branch),
        };
        Ok(Box::new(ProjectsBranch {
            prepared: Box::new(prepared),
            measured: None,
        }))
    }
}

/// One branch's projects observer, and in a measured run the split of its
/// thread's time between folding or reading its blocks (`projects.read`,
/// one call a block) and observing their hours (`projects.observe`, one
/// call an hour).
///
/// Hourly preparation also binds the projects' dictionary and completion
/// views to the frame its stream holds where it resumes a branch; the next
/// block starts at a keyframe, whose views are new allocations, so a bound
/// view and none read the same there and the family binds none.
struct ProjectsBranch {
    prepared: Box<special_projects::prepared::Prepared>,
    measured: Option<Measured>,
}

impl ProjectsBranch {
    /// Bind the content, as the walk does at every block, and note a block
    /// whose content reads the trigger kit.
    fn bind(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        self.prepared.configure(block.source, block.context)?;
        #[cfg(test)]
        if self.prepared.reads_trigger_world() {
            prepass::note(PROJECTS_WORLD);
        }
        Ok(())
    }

    /// Account a block's fold or read that took `took` in all, `observing`
    /// of it observing `hours` hours, in a measured run.
    fn account(&mut self, took: Duration, observing: Duration, hours: u64) {
        if let Some(measured) = self.measured.as_mut() {
            measured.read += took.saturating_sub(observing);
            measured.observe += observing;
            measured.blocks += 1;
            measured.hours += hours;
        }
    }
}

impl Branch for ProjectsBranch {
    fn measure(&mut self) {
        self.measured = Some(Measured::new(("projects.read", "projects.observe")));
    }

    fn parts(&mut self) -> Vec<(&'static str, Duration, u64)> {
        self.measured
            .as_mut()
            .map(Measured::take)
            .unwrap_or_default()
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.bind(block)?;
        let started = Instant::now();
        let mut hours = 0;
        let folded = special_projects::prepared::block::fold(
            &mut self.prepared,
            facts,
            block.replay,
            block.index,
            observed,
            check,
            &mut || {
                prepass::note(PROJECTS_OBSERVED);
                hours += 1;
            },
        );
        // The fold's observations are not timed apart from its own work.
        self.account(started.elapsed(), Duration::ZERO, hours);
        folded
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.bind(block)?;
        let world = self.prepared.reads_trigger_world();
        let running = AtomicBool::new(false);
        let prepared = &mut self.prepared;
        observe_alone(
            block,
            special_projects::prepared::block::tables(world),
            PROJECTS_OBSERVED,
            self.measured.as_mut(),
            |frame, changes| prepared.observe_changed(frame, changes, &running),
            observed,
            check,
        )
    }

    fn pending(&self) -> usize {
        self.prepared.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &projects_state(branch), &*self.prepared)
    }
}

// Occupation.

/// Occupation's family: its refusal, progress and coverage name is the
/// hourly walk's.
static OCCUPATION_FAMILY: prepass::Family = prepass::Family {
    name: "occupation",
    extract: "occupation.extract",
    sequential: "occupation.sequential",
    fallback: "occupation.fallback",
    // The extract keeps nothing: the observation reads its own tables.
    tables: &[TableId::Campaign],
    fields: None,
    start,
};

/// Each hour occupation's own thread observes, counted in tests.
pub(in crate::prepared::pipeline) const OCCUPATION_OBSERVED: &str = "occupation.observed";

/// The tables occupation's observer reads: states with their owners and
/// controllers, the daily resistance, garrison, law, settings, manpower and
/// collaboration reads, the countries' keys and links and the dictionary;
/// with `world`, also the shared trigger kit a law's visible or available
/// trigger reads at a daily read (`triggers::RECORDED_TABLES`).
fn occupation_tables(world: bool) -> &'static [TableId] {
    const OWN: [TableId; 10] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::StateResistance,
        TableId::StateGarrisons,
        TableId::StateOccupationLaws,
        TableId::OccupationSettings,
        TableId::StateAssignedManpower,
        TableId::CountryCollaboration,
        TableId::LexicalTokens,
    ];
    static OWN_TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| read_tables(OWN));
    static WORLD_TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        read_tables(
            OWN.into_iter()
                .chain(crate::triggers::RECORDED_TABLES.iter().copied()),
        )
    });
    if world {
        WORLD_TABLES.as_slice()
    } else {
        OWN_TABLES.as_slice()
    }
}

/// Occupied states, their windows and the occupiers' trends.
pub(in crate::prepared::pipeline) struct Occupation;

pub(in crate::prepared::pipeline) static OCCUPATION: Occupation = Occupation;

/// Occupation's continuation, which hourly preparation saves after its flush.
fn occupation_state(branch: u64) -> String {
    format!("occupation-{branch}")
}

impl Own for Occupation {
    fn family(&self) -> &'static prepass::Family {
        &OCCUPATION_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.occupation"
    }

    /// Bind the same law content as the observer: a law with neither a
    /// visible nor an available trigger reads no trigger-world tables.
    fn shared_tables(&self, phase: &super::Phase<'_>) -> &'static [TableId] {
        let mut prepared = occupied_states::prepared::Prepared::default();
        prepared.configure(phase.source, phase.context);
        occupation_tables(prepared.reads_trigger_world())
    }

    /// Its own thread observes every block; an extract keeps nothing.
    fn laned(&self) -> bool {
        false
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = match load.read(&occupation_state(branch))? {
            Some(file) => occupied_states::prepared::Prepared::read(io::BufReader::new(file))?,
            None => occupied_states::prepared::Prepared::default(),
        };
        Ok(Box::new(OccupationBranch {
            prepared: Box::new(prepared),
            block: None,
            held: Vec::new(),
            failed: None,
            measured: None,
        }))
    }
}

/// One branch's occupation observer and the window pages of the blocks it
/// went through since its last commit, and in a measured run the split of
/// its thread's time.
struct OccupationBranch {
    prepared: Box<occupied_states::prepared::Prepared>,
    /// The hours of the block observed last, whose window its end writes.
    block: Option<(i32, i32)>,
    /// The window pages hourly preparation writes at each block's end.
    held: Vec<occupied_states::prepared::HeldWindow>,
    /// A window that could not be encoded, which refuses at the next commit.
    failed: Option<io::Error>,
    measured: Option<Measured>,
}

pub fn occupation_row_reference(
    source: &crate::Source,
    context: &crate::products::LoadContext,
    prototype: &occupied_states::prepared::Prepared,
    writer: &mut store::Writer,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let replay = source.replay_ref();
    let mut states = std::collections::BTreeMap::<u64, OccupationBranch>::new();
    for index in &replay.index {
        let state = states
            .entry(index.branch)
            .or_insert_with(|| OccupationBranch {
                prepared: Box::new(prototype.fresh()),
                block: None,
                held: Vec::new(),
                failed: None,
                measured: None,
            });
        let block = Block {
            replay,
            index,
            source,
            context,
            cancelled,
            shared: None,
            #[cfg(test)]
            read_hook_family: None,
        };
        state.begin_block();
        state.observe_block(
            &block,
            &mut |_, outcome| outcome.map(|()| true),
            &mut || crate::metadata::check_cancel(cancelled),
        )?;
        state.end_block(replay, index.branch);
        state.publish(writer, index.branch, cancelled)?;
        state.flush(writer, replay, index.branch, cancelled)?;
    }
    Ok(())
}

impl OccupationBranch {
    /// Bind the content, as the walk does at every block, then observe the
    /// block hour by hour with the tables its law eligibility reads.
    fn observe_hours(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.prepared.configure(block.source, block.context);
        self.prepared.begin_block();
        self.block = Some((block.index.first_hour, block.index.last_hour));
        let tables = occupation_tables(self.prepared.reads_trigger_world());
        let format = block.replay.format;
        let running = AtomicBool::new(false);
        let prepared = &mut self.prepared;
        observe_alone(
            block,
            tables,
            OCCUPATION_OBSERVED,
            self.measured.as_mut(),
            |frame, changes| prepared.observe(frame, format, &running, changes),
            observed,
            check,
        )
    }
}

impl Branch for OccupationBranch {
    fn measure(&mut self) {
        self.measured = Some(Measured::new(("occupation.read", "occupation.observe")));
    }

    fn parts(&mut self) -> Vec<(&'static str, Duration, u64)> {
        self.measured
            .as_mut()
            .map(Measured::take)
            .unwrap_or_default()
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        read_facts(facts)?;
        Ok((Folded::Fast, self.observe_hours(block, observed, check)?))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.observe_hours(block, observed, check)
    }

    /// The block's window, which hourly preparation writes at its end: held
    /// for the commit.
    fn end_block(&mut self, _: &Replay, branch: u64) {
        let Some((first, last)) = self.block.take() else {
            return;
        };
        if self.failed.is_some() {
            return;
        }
        match self.prepared.hold_block(branch, first, last) {
            Ok(window) => self.held.push(window),
            Err(error) => self.failed = Some(error),
        }
    }

    /// The windows of the blocks it went through, in block order.
    fn publish(&mut self, writer: &mut store::Writer, _: u64, _: &AtomicBool) -> io::Result<()> {
        for window in self.held.drain(..) {
            window.write(writer)?;
        }
        match self.failed.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn pending(&self) -> usize {
        self.prepared.bytes() + self.held.iter().map(|window| window.bytes()).sum::<usize>()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)
    }

    /// Its state, which hourly preparation's flush writes after the pages:
    /// failing to encode it stops preparation, failing to write it refuses.
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.prepared.saved_bytes()?;
        Ok(writer.write_at(&occupation_state(branch), 0, bytes.as_slice()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
