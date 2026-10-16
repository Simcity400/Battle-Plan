//! Families prepared on their own: before the hourly walk, each prepares every
//! block it has not accepted, from blocks read alone on pre-pass workers and
//! folded in block order, and commits its own pages, its own coverage pages
//! and its progress in the checkpoint. Its queries answer from its own
//! coverage, so it is ready while slower families still prepare.
//!
//! A family prepared on its own is a registry entry ([`Own`]): its pre-pass
//! family, the products it gates, its branch state ([`Branch`]: load, fold or
//! observe a block, flush, save) and the sections the hourly walk keeps for
//! families that read its state in the same hour. Every registered family
//! starts at once on one shared pool of pre-pass workers
//! ([`prepass::Pool`]), sized as the hourly walk's and serving every
//! family's blocks, over the source's one opened recording, so each block is
//! verified once. Each family folds its blocks in order on a thread of its
//! own and commits on its own: commits share one store writer and take turns,
//! and while other families run, a family's writes go through a fork of the
//! writer adopted only once they all succeed, so each commit holds one
//! family's whole batch or nothing of it. A family's refusals, coverage and
//! progress are its own; one that fails or panics leaves the others to
//! finish, and its error is reported once they have.
//!
//! The hourly walk skips a family prepared on its own ([`hourly`]), unless
//! another family reads its state in the same hour: then it keeps its own
//! read of it, the same observation saved under the reader's names
//! ([`kept_as`]) and never published as the family's. Both reads make the
//! same calls on the same frames, so the family's refusals reach its reader
//! at the hours hourly preparation records. A reader prepared on its own
//! keeps a private read of the family instead, as air superiority does of
//! the country groups.
//!
//! A family commits by the hourly batch rules (at each branch's end, after
//! every block budget of its branch ([`super::Batches`]), every block in
//! measured runs, and at the byte budget of what it holds), so its pages are
//! those hourly preparation publishes. Where its batches end follows from the
//! recording and its own bytes alone, never from wall time or from what other
//! families committed meanwhile.

use super::super::Section;
use super::{
    Accepted, BATCH_BYTES, Batches, Checkpoint, Coverage, CoveragePage, FAMILIES, FamilyTimings,
    Refused, SCHEMA, lanes,
};
use crate::{
    Source,
    metadata::check_cancel,
    prepared::prepass::{self, Folded, Outcome, Pool},
    products::{
        LoadContext,
        country_groups::{self, Scan, changes::Prepared as Changes},
        country_history::HistoryScan,
    },
};
use replay::container::{BlockIndex, Replay};
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    io,
    panic::resume_unwind,
    sync::{Mutex, PoisonError, atomic::AtomicBool},
};

type Timings = Option<BTreeMap<&'static str, FamilyTimings>>;

/// A family prepared on its own.
pub(super) trait Own: Sync {
    /// Its pre-pass family: its name (that of its refusals, progress and
    /// coverage pages), tables, read fields and block extract.
    fn family(&self) -> &'static prepass::Family;
    /// The timing row of its whole preparation.
    fn timing(&self) -> &'static str;
    /// The product queries that wait only for its coverage, and refuse from
    /// its refusals.
    fn gates(&self) -> &'static [&'static str];
    /// Sections the hourly walk keeps for families that read this family's
    /// state in the same hour, as (its section, the name the walk keeps it
    /// under). While any is declared, the hourly walk still observes the
    /// family, for those readers alone; otherwise it skips it.
    fn kept(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// The families that read this family's state in the same hour, for
    /// which the hourly walk keeps it ([`Self::kept`]). Once every one is
    /// prepared on its own too, the walk skips this family: a reader prepared
    /// on its own makes its own read of this family's state, on the frames
    /// its own extract reads, under its own sections (as the walk's kept read
    /// does now). None named keeps the family in the walk whatever the
    /// registry holds.
    fn read_by(&self) -> &'static [&'static str] {
        &[]
    }
    /// One branch's state as `load` last committed it, or new without a
    /// commit to read.
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>>;
}

/// What a family's state is loaded from.
pub(super) struct Load<'a> {
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    /// The last committed preparation; None for a family that refused the
    /// branch, whose pages are never read again.
    pub saved: Option<&'a store::SourceKey>,
    pub replay: &'a Replay,
}

impl Load<'_> {
    /// A section of the last committed preparation, if it has one.
    pub(super) fn read(&self, section: &str) -> io::Result<Option<store::Range>> {
        match self.saved {
            None => Ok(None),
            Some(previous) if previous != self.key => {
                self.store.read_prefix(previous, self.key, section, 0)
            }
            Some(_) => self.store.read(self.key, section, 0),
        }
    }
}

/// The block a family folds or observes.
pub(super) struct Block<'a> {
    pub replay: &'a Replay,
    pub index: &'a BlockIndex,
    pub source: &'a Source,
    pub context: &'a LoadContext,
}

/// A family's observation of one hour's outcome: `Ok(true)` while it goes on,
/// `Ok(false)` once it refused the branch.
pub(super) type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// One branch's state of a family prepared on its own.
pub(super) trait Branch {
    /// Before each block it observes.
    fn begin_block(&mut self);
    /// Fold the block's pre-pass facts; how it folded and the last hour it
    /// reached.
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)>;
    /// Observe the block hour by hour, as hourly preparation does; the last
    /// hour observed.
    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>>;
    /// After each block it went on through. Pages hourly preparation writes
    /// at a block's end are held here and written by [`Self::publish`].
    fn end_block(&mut self, replay: &Replay, branch: u64);
    /// Write the pages it holds from the blocks it went through since its
    /// last commit, which hourly preparation writes at each such block's end
    /// (air aces' windows, espionage day pages, production pages). Called at
    /// every commit before [`Self::flush`], also when it refused the branch
    /// later in the batch: hourly preparation had written those blocks' pages
    /// before the refusing one. Its error refuses the branch at the batch's
    /// hour.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _ = (writer, branch, cancelled);
        Ok(())
    }
    /// Bytes held for its next flush, against the batch budget.
    fn pending(&self) -> usize;
    /// Measure the parts of its folds from now on, for a measured run's
    /// timings ([`Self::parts`]).
    fn measure(&mut self) {}
    /// The time each part of its folds took since last asked, by timing
    /// name, with the times it took it; none unless measured. A measured
    /// run counts them as their own timings, within the family's.
    fn parts(&mut self) -> Vec<(&'static str, std::time::Duration, u64)> {
        Vec::new()
    }
    /// Its flush at a batch's end, where hourly preparation flushes it; an
    /// error refuses the branch when hourly preparation's would.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;
    /// Its pages after the flush: the outer error stops its preparation, the
    /// inner one refuses the branch.
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>>;
    /// Its continuation, which hourly preparation keeps in its branch state.
    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()>;
}

/// Country groups. Air superiority reads them in the same hour: while it is
/// in the hourly walk, the walk keeps its own read under air superiority's
/// names; prepared on its own, air superiority makes that read itself.
struct Groups;

static GROUPS: Groups = Groups;

/// Every family prepared on its own, in registry order.
static REGISTRY: [&dyn Own; 19] = [
    &GROUPS,
    &IDENTITIES,
    &WARS,
    &CASUALTIES,
    &quiet_politics::SUBJECTS,
    &quiet_politics::PEACE,
    &quiet_politics::IDEAS,
    &air::AIR_LEDGER,
    &air::RAIDS,
    &air::ACES,
    &economy::CONSTRUCTION,
    &economy::STOCKPILE,
    &economy::PRODUCTION,
    &economy::EXPERIENCE,
    &intelligence::INTELLIGENCE,
    &ledger::LEDGER,
    &focus::FOCUS,
    &air_superiority::AIR_SUPERIORITY,
    &world_tension::TENSION,
];

/// Subjects, peace outcomes and idea history.
pub(super) mod quiet_politics;

/// The military air ledger, raids and air aces.
pub(super) mod air;

/// Construction, equipment stockpiles, production and country experience.
pub(super) mod economy;

/// Decryption, operative placements, agencies, the roster and espionage.
pub(super) mod intelligence;

/// The politics ledger.
pub(super) mod ledger;

/// The national focus order.
pub(super) mod focus;

/// Air superiority, with its own read of the country groups.
pub(super) mod air_superiority;

/// World tension.
pub(super) mod world_tension;

impl Own for Groups {
    fn family(&self) -> &'static prepass::Family {
        &country_groups::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.groups"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["country-groups"]
    }

    fn kept(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("country-groups", "air-superiority-groups"),
            ("country-group-changes", "air-superiority-group-changes"),
        ]
    }

    fn read_by(&self) -> &'static [&'static str] {
        &["air_superiority"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let fresh = || -> Box<dyn Branch> {
            Box::new(GroupsBranch {
                scan: Box::new(Scan::new(load.replay, branch)),
                changes: Box::default(),
            })
        };
        let Some(file) = load.read(&format!("country-groups-{branch}"))? else {
            return Ok(fresh());
        };
        let scan = Scan::read(io::BufReader::new(file))?;
        let file = load
            .read(&groups_state(branch))?
            .ok_or_else(|| super::invalid("Prepared country group changes are missing"))?;
        let changes: Box<Changes> = serde_json::from_reader(io::BufReader::new(file))?;
        Ok(Box::new(GroupsBranch {
            scan: Box::new(scan),
            changes,
        }))
    }
}

/// The country-group change publisher's continuation, which hourly
/// preparation keeps in the branch state.
fn groups_state(branch: u64) -> String {
    format!("country-group-state-{branch}")
}

/// One branch's country groups and their change publisher.
struct GroupsBranch {
    scan: Box<Scan>,
    changes: Box<Changes>,
}

impl Branch for GroupsBranch {
    fn begin_block(&mut self) {
        self.scan.begin_block();
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        country_groups::prepared::fold(
            &mut self.scan,
            &mut self.changes,
            facts,
            block.replay,
            block.index,
            block.source,
            block.context,
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
        country_groups::prepared::observe_block(
            &mut self.scan,
            &mut self.changes,
            block.replay,
            block.index,
            block.source,
            block.context,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, replay: &Replay, branch: u64) {
        self.scan.finish(replay, branch);
    }

    fn pending(&self) -> usize {
        self.changes.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.finish(replay, branch);
        self.changes.flush(writer, branch, &self.scan, cancelled)
    }

    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.scan.saved_bytes()?;
        Ok(writer.write_at(&format!("country-groups-{branch}"), 0, bytes.as_slice()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::json(writer, &groups_state(branch), &*self.changes)
    }
}

/// The country identity history. Territory's owned-state observation drives
/// it and the event feed reads it in the same hour, so the hourly walk keeps
/// its own read under territory's name.
pub(super) struct Identities;

pub(super) static IDENTITIES: Identities = Identities;

/// The identity history's published page of one branch, which is also its
/// continuation.
fn identities_section(branch: u64) -> String {
    format!("country-identities-{branch}")
}

impl Own for Identities {
    fn family(&self) -> &'static prepass::Family {
        &crate::products::country_history::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.identities"
    }

    fn gates(&self) -> &'static [&'static str] {
        &[crate::products::country_history::IDENTITIES_PRODUCT]
    }

    fn kept(&self) -> &'static [(&'static str, &'static str)] {
        &[("country-identities", "territory-identities")]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let scan = match load.read(&identities_section(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => HistoryScan::new(false, &[]),
        };
        Ok(Box::new(IdentitiesBranch {
            scan: Box::new(scan),
        }))
    }
}

/// One branch's identity history.
struct IdentitiesBranch {
    scan: Box<HistoryScan>,
}

impl Branch for IdentitiesBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        crate::products::country_history::prepared::fold(
            &mut self.scan,
            facts,
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
        crate::products::country_history::prepared::observe_block(
            &mut self.scan,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        0
    }

    /// Sealed and written whole at each commit, as hourly preparation's
    /// flush writes it.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal();
        writer.write_at(
            &identities_section(branch),
            0,
            serde_json::to_vec(&*self.scan)?.as_slice(),
        )
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

/// The wars' change inputs and relation counters. Their continuation is
/// their published catalogue.
pub(super) struct Wars;

pub(super) static WARS: Wars = Wars;

impl Own for Wars {
    fn family(&self) -> &'static prepass::Family {
        &crate::products::wars::block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.wars"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["wars"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        Ok(Box::new(crate::products::wars::block::OwnBranch::load(
            load.store, load.key, load.saved, branch,
        )?))
    }
}

impl Branch for crate::products::wars::block::OwnBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.fold_facts(
            facts,
            block.replay,
            block.index,
            block.source,
            block.context,
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
        self.observe_hours(
            block.replay,
            block.index,
            block.source,
            block.context,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.pending_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_batch(writer, branch, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

/// War casualties: country values, pair windows and breakdown inputs. Their
/// series queries route by the family kind `war-casualties`.
pub(super) struct Casualties;

pub(super) static CASUALTIES: Casualties = Casualties;

impl Own for Casualties {
    fn family(&self) -> &'static prepass::Family {
        &crate::products::war_casualties::prepared::block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.casualties"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["war-casualties", "casualty-pairs", "casualty-breakdown"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        use crate::products::war_casualties::prepared::block;
        let saved = load.read(&block::state_section(branch))?;
        Ok(Box::new(block::OwnBranch::load(
            saved,
            load.replay,
            branch,
        )?))
    }
}

impl Branch for crate::products::war_casualties::prepared::block::OwnBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.fold_facts(
            facts,
            block.replay,
            block.index,
            block.source,
            block.context,
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
        self.observe_hours(
            block.replay,
            block.index,
            block.source,
            block.context,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {
        self.finish_block();
    }

    /// The pair windows of the blocks gone through, which hourly preparation
    /// writes at each block's end, also when a later block of the batch
    /// refused the branch.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        self.publish_windows(writer, branch)
    }

    fn pending(&self) -> usize {
        self.pending_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_batch(writer, branch, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        self.write_state(writer, branch)
    }
}

/// The registered families, or none while tests prepare every family hour
/// by hour.
fn registry() -> &'static [&'static dyn Own] {
    #[cfg(test)]
    {
        if TEST_HOURLY.with(std::cell::Cell::get) {
            return &[];
        }
        if let Some(families) = TEST_FAMILIES.with(std::cell::Cell::get) {
            return families;
        }
    }
    &REGISTRY
}

/// The families prepared on their own, in name order (the checkpoint's).
pub(super) fn names() -> Vec<&'static str> {
    let mut names: Vec<_> = registry().iter().map(|own| own.family().name).collect();
    names.sort_unstable();
    names
}

/// Whether the hourly walk observes, flushes, saves and loads `family`:
/// every family not prepared on its own, and one whose state another family
/// reads in the same hour, which the walk keeps under that reader's names
/// ([`kept_as`]).
pub(super) fn hourly(family: &str) -> bool {
    registry()
        .iter()
        .filter(|own| own.family().name == family)
        .all(|own| {
            !own.kept().is_empty()
                && (own.read_by().is_empty()
                    || own.read_by().iter().any(|reader| !registered(reader)))
        })
}

/// Whether `family` is prepared on its own.
pub(super) fn registered(family: &str) -> bool {
    registry().iter().any(|own| own.family().name == family)
}

/// The name the hourly walk keeps `section` under: a reader's own, while the
/// family publishing it is prepared on its own; `section` otherwise.
pub(super) fn kept_as(section: &'static str) -> &'static str {
    registry()
        .iter()
        .flat_map(|own| own.kept())
        .find(|(published, _)| *published == section)
        .map_or(section, |&(_, kept)| kept)
}

/// The family prepared on its own whose coverage `product`'s queries wait
/// for, and whose refusals they answer with.
pub(super) fn gating(product: &str) -> Option<&'static str> {
    registry()
        .iter()
        .find(|own| own.gates().contains(&product))
        .map(|own| own.family().name)
}

/// A family's coverage page for one branch, with its refusals there.
pub(super) fn coverage_section(family: &str, branch: u64) -> String {
    format!("preparation-family-{family}-{branch}")
}

/// A family's accepted progress: the next block it prepares and its refusals
/// by branch, each from its first refusing hour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Progress {
    pub next_block: usize,
    #[serde(default)]
    pub refused: BTreeMap<u64, Refused>,
}

/// What a family's preparation reads.
pub(super) struct Phase<'a> {
    pub source: &'a Source,
    pub replay: &'a Replay,
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    pub context: &'a LoadContext,
    pub map: &'a dashboard_base::prepared_map::Builder,
    pub last_blocks: &'a BTreeMap<u64, usize>,
    /// Where batches end by the recording, as the hourly walk's do.
    pub batches: &'a Batches,
    pub measured: bool,
    pub cancelled: &'a AtomicBool,
}

/// Count a block the family observed hour by hour, or a fold that fell back.
fn note(timings: &mut Timings, name: &'static str) {
    let counted = super::start_timing(timings);
    super::finish_timing(timings, counted, name, false);
    prepass::note(name);
}

/// The store writer and checkpoint every family commits through, in turn.
struct Commits<'s> {
    /// The open transaction; None once a failed commit lost it.
    writer: Option<store::Writer>,
    state: &'s mut Checkpoint,
    saved: &'s mut Option<store::SourceKey>,
    bytes: &'s mut u64,
    committed: bool,
}

fn lock<'m, 's>(commits: &'m Mutex<Commits<'s>>) -> std::sync::MutexGuard<'m, Commits<'s>> {
    commits.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One family's run: its registry entry, its lane in the pool and its
/// progress.
struct Run {
    own: &'static dyn Own,
    lane: usize,
    progress: Progress,
}

/// A family run's result, its timings and (in tests) what it counted.
type Ran = (io::Result<()>, Timings, Timings, Counts);

/// Prepares every family of its own through the recording's last block,
/// committing through `writer`. Returns the open writer and whether anything
/// was committed (with the writer's pending metadata).
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    phase: &Phase<'_>,
    writer: store::Writer,
    state: &mut Checkpoint,
    saved: &mut Option<store::SourceKey>,
    bytes: &mut u64,
    timings: &mut Timings,
    workers: &mut Timings,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<(store::Writer, bool)> {
    let replay = phase.replay;
    let mut runs = Vec::new();
    for &own in registry() {
        let progress = state
            .independent
            .get(own.family().name)
            .cloned()
            .unwrap_or_default();
        if progress.next_block < replay.index.len() {
            runs.push(Run {
                own,
                lane: runs.len(),
                progress,
            });
        }
    }
    if runs.is_empty() {
        return Ok((writer, false));
    }
    // The phase's wall time is the walk's; each family's own rows are worker
    // time, as lane jobs' are, since the families prepare at once.
    let phase_started = super::start_timing(timings);
    #[allow(unused_mut)]
    let mut knobs = prepass::Knobs::current();
    #[cfg(test)]
    {
        knobs.refusals = lanes::test_refusals();
    }
    // Every family's blocks from its own first one, on one set of workers.
    let pool = Pool::start(
        phase.source.replay(),
        runs.iter()
            .map(|run| (run.own.family(), run.progress.next_block))
            .collect(),
        prepass::workers(lanes::parallelism()),
        knobs,
    );
    // With other families committing too, each commit adopts a family's
    // writes only once all of them succeeded.
    let atomic = runs.len() > 1;
    let commits = Mutex::new(Commits {
        writer: Some(writer),
        state,
        saved,
        bytes,
        committed: false,
    });
    let local = || -> (Timings, Timings) {
        (
            timings.as_ref().map(|_| BTreeMap::new()),
            workers.as_ref().map(|_| BTreeMap::new()),
        )
    };
    let ran: Vec<std::thread::Result<Ran>> = if runs.len() == 1 || one_after_another() {
        let mut ran = Vec::with_capacity(runs.len());
        for run in runs {
            let (mut own_timings, mut own_workers) = local();
            let result = prepare_family(
                phase,
                &pool,
                run,
                &commits,
                atomic,
                &mut own_timings,
                &mut own_workers,
                &mut |next| accepted(&next),
            );
            ran.push(Ok((result, own_timings, own_workers, Counts::default())));
        }
        ran
    } else {
        #[cfg(test)]
        let context = TestContext::capture();
        std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::channel::<Accepted>();
            let mut started = Vec::with_capacity(runs.len());
            for run in runs {
                let sender = sender.clone();
                let (pool, commits) = (&pool, &commits);
                let (mut own_timings, mut own_workers) = local();
                #[cfg(test)]
                let context = context.clone();
                let thread = crate::prepared::runner::worker_thread(format!(
                    "preparation of {}",
                    run.own.family().name
                ))
                .spawn_scoped(scope, move || {
                    #[cfg(test)]
                    let _installed = context.install();
                    let result = prepare_family(
                        phase,
                        pool,
                        run,
                        commits,
                        atomic,
                        &mut own_timings,
                        &mut own_workers,
                        &mut |next| {
                            // The caller reports it; a caller that has gone
                            // only misses the report.
                            let _ = sender.send(next);
                        },
                    );
                    (result, own_timings, own_workers, Counts::take())
                });
                // A family whose thread cannot start fails; the others go on.
                started.push(thread);
            }
            drop(sender);
            // Each family's accepted commits, reported in the order they
            // committed, until every family has finished.
            for next in receiver.iter() {
                accepted(&next);
            }
            started
                .into_iter()
                .map(|thread| match thread {
                    Ok(handle) => handle.join(),
                    Err(error) => Ok((Err(error), None, None, Counts::default())),
                })
                .collect()
        })
    };
    drop(pool);
    let mut first_error = None;
    let mut panic = None;
    for ran in ran {
        match ran {
            Ok((result, own_timings, own_workers, counts)) => {
                lanes::merge(workers, own_timings);
                lanes::merge(workers, own_workers);
                counts.add();
                if let Err(error) = result {
                    first_error.get_or_insert(error);
                }
            }
            Err(payload) => {
                panic.get_or_insert(payload);
            }
        }
    }
    super::finish_timing(timings, phase_started, "independent", false);
    if let Some(payload) = panic {
        resume_unwind(payload);
    }
    let Commits {
        writer, committed, ..
    } = commits.into_inner().unwrap_or_else(PoisonError::into_inner);
    if let Some(error) = first_error {
        return Err(error);
    }
    let writer = writer
        .ok_or_else(|| io::Error::other("The prepared writer was lost to a failed commit"))?;
    Ok((writer, committed))
}

/// Retires a family's lane when its preparation ends, however it ends, so the
/// pool stops reading ahead for it.
struct Retire<'p>(&'p Pool, usize);

impl Drop for Retire<'_> {
    fn drop(&mut self) {
        self.0.retire(self.1);
    }
}

/// One family's preparation: every block from its next, folded in order,
/// committed by the hourly batch rules.
#[allow(clippy::too_many_arguments)]
fn prepare_family(
    phase: &Phase<'_>,
    pool: &Pool,
    run: Run,
    commits: &Mutex<Commits<'_>>,
    atomic: bool,
    timings: &mut Timings,
    workers: &mut Timings,
    accepted: &mut dyn FnMut(Accepted),
) -> io::Result<()> {
    let Run {
        own,
        lane,
        mut progress,
    } = run;
    let _retire = Retire(pool, lane);
    let replay = phase.replay;
    let family = own.family();
    let name = family.name;
    let started = super::start_timing(timings);
    let mut held: Option<(u64, Box<dyn Branch>)> = None;
    let mut touched = BTreeSet::new();
    for block in progress.next_block..replay.index.len() {
        check_cancel(phase.cancelled)?;
        let index = &replay.index[block];
        let branch = index.branch;
        if held.as_ref().is_none_or(|(held, _)| *held != branch) {
            let saved = lock(commits).saved.clone();
            let refused = progress.refused.entry(branch).or_default();
            let load = Load {
                store: phase.store,
                key: phase.key,
                saved: saved.as_ref().filter(|_| refused.open(name)),
                replay,
            };
            let mut state = own.load(&load, branch)?;
            if timings.is_some() {
                state.measure();
            }
            held = Some((branch, state));
        }
        let (_, state) = held.as_mut().expect("held family state");
        let (outcome, took) = pool.take(lane, block, phase.cancelled)?;
        if let Some(workers) = workers.as_mut() {
            let entry = workers.entry(family.extract).or_default();
            entry.observe += took;
            entry.calls += 1;
        }
        let refused = progress.refused.entry(branch).or_default();
        let mut reached = None;
        if refused.open(name) {
            state.begin_block();
            let mut observed = |hour: i32, result: io::Result<()>| -> io::Result<bool> {
                refused.check(name, hour, result)?;
                Ok(refused.open(name))
            };
            let mut check = || check_cancel(phase.cancelled);
            let at = Block {
                replay,
                index,
                source: phase.source,
                context: phase.context,
            };
            reached = match outcome {
                Outcome::Facts(facts) => {
                    let (folded, reached) = state.fold(facts, &at, &mut observed, &mut check)?;
                    if folded == Folded::Fallback {
                        note(timings, family.fallback);
                    }
                    reached
                }
                Outcome::Sequential => {
                    note(timings, family.sequential);
                    state.observe_block(&at, &mut observed, &mut check)?
                }
            };
            if refused.open(name) {
                state.end_block(replay, branch);
            }
            if let Some(families) = timings.as_mut() {
                for (part, took, times) in state.parts() {
                    let entry = families.entry(part).or_default();
                    entry.observe += took;
                    entry.calls += times;
                }
            }
        }
        touched.insert(branch);
        let continues = replay
            .index
            .get(block + 1)
            .is_some_and(|next| next.branch == branch);
        // The batch ends where the hourly walk's does by the recording, or
        // at the byte budget of what this family holds: never by wall time,
        // nor by the shared writer, which other families' commits fill.
        if continues
            && !phase.measured
            && !phase.batches.full(block)
            && state.pending() < BATCH_BYTES
        {
            continue;
        }
        // The family's hourly flush, at the hour it would run.
        let commit_started = super::start_timing(timings);
        let hour = reached.unwrap_or(index.last_hour);
        let next = commit(
            commits,
            phase,
            atomic,
            name,
            &mut progress,
            |writer, progress| {
                let refused = progress.refused.entry(branch).or_default();
                // The pages of the blocks it went through, refused since or not.
                let result = state.publish(writer, branch, phase.cancelled);
                refused.check(name, hour, result)?;
                if refused.open(name) {
                    let result = state.flush(writer, replay, branch, phase.cancelled);
                    refused.check(name, hour, result)?;
                }
                if refused.open(name) {
                    let result = state.save(writer, branch)?;
                    refused.check(name, hour, result)?;
                }
                if refused.open(name) {
                    state.save_state(writer, branch)?;
                }
                progress.next_block = block + 1;
                progress.refused.retain(|_, refused| !refused.0.is_empty());
                for &branch in &touched {
                    save_coverage(writer, phase, name, progress, branch)?;
                }
                Ok(())
            },
        )?;
        touched.clear();
        super::finish_timing(timings, commit_started, "independent.commit", true);
        accepted(next);
        if !continues || super::reload_flushed() {
            held = None;
        }
    }
    super::finish_timing(timings, started, own.timing(), false);
    Ok(())
}

/// Commits one family's batch (`write`, with its progress) and its progress
/// in the checkpoint, then opens the next transaction. With other families
/// committing too (`atomic`), the batch is written into a fork of the writer
/// and adopted only when all of it succeeded, so a failed batch leaves
/// nothing behind for another family's commit. Returns what was accepted.
fn commit(
    commits: &Mutex<Commits<'_>>,
    phase: &Phase<'_>,
    atomic: bool,
    name: &str,
    progress: &mut Progress,
    write: impl FnOnce(&mut store::Writer, &mut Progress) -> io::Result<()>,
) -> io::Result<Accepted> {
    let mut guard = lock(commits);
    let commits = &mut *guard;
    let writer = commits
        .writer
        .as_mut()
        .ok_or_else(|| io::Error::other("The prepared writer was lost to a failed commit"))?;
    if atomic {
        let mut fork = writer.fork()?;
        write(&mut fork, progress)?;
        for staged in fork.into_staged()? {
            writer.adopt(staged)?;
        }
    } else {
        write(writer, progress)?;
    }
    commits
        .state
        .independent
        .insert(name.to_owned(), progress.clone());
    *commits.bytes = super::write_checkpoint(
        writer,
        commits.state,
        phase.replay,
        phase.map,
        *commits.bytes,
    )?;
    check_cancel(phase.cancelled)?;
    commits
        .writer
        .take()
        .expect("the open prepared writer")
        .commit()?;
    commits.committed = true;
    *commits.saved = Some(phase.key.clone());
    commits.writer = Some(phase.store.extend(phase.key)?);
    let state = &*commits.state;
    Ok(Accepted {
        sections: super::preparing_sections(
            state,
            state.cursor.next_block,
            phase.replay.index.len(),
        ),
        bytes: *commits.bytes,
        next_block: state.cursor.next_block,
        new_blocks: 0,
        coverage: super::accepted_coverage(&state.map, phase.last_blocks),
    })
}

/// The family's accepted extent of one branch, with its refusals there.
fn save_coverage(
    writer: &mut store::Writer,
    phase: &Phase<'_>,
    family: &str,
    progress: &Progress,
    branch: u64,
) -> io::Result<()> {
    let replay = phase.replay;
    let Some(&(first, last)) = phase.map.progress().ranges.get(&branch) else {
        return Ok(());
    };
    let Some(done) = replay.index[..progress.next_block]
        .iter()
        .rposition(|index| index.branch == branch)
    else {
        return Ok(());
    };
    let complete = phase
        .last_blocks
        .get(&branch)
        .is_some_and(|&last| last < progress.next_block);
    super::json(
        writer,
        &coverage_section(family, branch),
        &CoveragePage {
            coverage: Coverage {
                schema: SCHEMA,
                first_hour: first,
                last_hour: if complete {
                    last
                } else {
                    replay.index[done].last_hour
                },
                complete,
            },
            refused: progress.refused.get(&branch).cloned().unwrap_or_default(),
        },
    )
}

/// While preparing, each family of its own's status rows count its own
/// accepted blocks. Rows turn ready with the preparation that publishes them.
pub(super) fn sections(rows: &mut [Section], state: &Checkpoint, total: usize) {
    for (name, progress) in &state.independent {
        let Some(family) = FAMILIES.iter().find(|family| family.name == name.as_str()) else {
            continue;
        };
        for row in rows
            .iter_mut()
            .filter(|row| family.sections.contains(&row.id.as_str()))
        {
            row.done = progress.next_block.to_string();
            row.total = Some(total.to_string());
        }
    }
}

/// What a family's thread counted for tests, carried back to the caller's.
#[derive(Default)]
struct Counts {
    #[cfg(test)]
    notes: BTreeMap<&'static str, usize>,
    #[cfg(test)]
    groups: (usize, usize),
    #[cfg(test)]
    air_ledger: (usize, usize),
}

impl Counts {
    /// This thread's counts.
    fn take() -> Self {
        Self {
            #[cfg(test)]
            notes: prepass::notes(),
            #[cfg(test)]
            groups: country_groups::prepared::counts(),
            #[cfg(test)]
            air_ledger: crate::products::military_air_ledger::prepared::counts(),
        }
    }

    /// Add them to this thread's.
    fn add(self) {
        #[cfg(test)]
        {
            prepass::add_notes(&self.notes);
            country_groups::prepared::add_counts(self.groups);
            crate::products::military_air_ledger::prepared::add_counts(self.air_ledger);
        }
    }
}

/// The caller's test settings a family's thread prepares under.
#[cfg(test)]
#[derive(Clone)]
struct TestContext {
    refusals: Vec<(&'static str, i32)>,
    reload: bool,
    registry: TestRegistry,
}

/// The test's registry settings: which families are prepared on their own,
/// for threads the walk or a family starts (a parallel flush's helpers, lane
/// workers), whose `Refused::live` and `kept_as` must agree with the caller's.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) struct TestRegistry {
    families: Option<&'static [&'static dyn Own]>,
    hourly: bool,
}

/// This thread's registry settings, to install on another thread.
#[cfg(test)]
pub(super) fn test_registry() -> TestRegistry {
    TestRegistry {
        families: TEST_FAMILIES.with(std::cell::Cell::get),
        hourly: TEST_HOURLY.with(std::cell::Cell::get),
    }
}

#[cfg(test)]
impl TestRegistry {
    /// Use these settings on this thread.
    pub(super) fn install(self) {
        TEST_FAMILIES.with(|families| families.set(self.families));
        TEST_HOURLY.with(|hourly| hourly.set(self.hourly));
    }
}

#[cfg(test)]
impl TestContext {
    fn capture() -> Self {
        Self {
            refusals: lanes::test_refusals(),
            reload: super::RELOAD_FLUSHED.with(std::cell::Cell::get),
            registry: test_registry(),
        }
    }

    /// Install the settings on this thread for as long as the guard lives.
    fn install(self) -> impl Drop {
        struct Installed;
        impl Drop for Installed {
            fn drop(&mut self) {
                lanes::set_test_refusals(Vec::new());
            }
        }
        lanes::set_test_refusals(self.refusals);
        super::RELOAD_FLUSHED.with(|flag| flag.set(self.reload));
        self.registry.install();
        Installed
    }
}

/// Whether tests prepare the families one after another rather than at once.
fn one_after_another() -> bool {
    #[cfg(test)]
    {
        TEST_ONE_AFTER_ANOTHER.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(test)]
thread_local! {
    static TEST_HOURLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_FAMILIES: std::cell::Cell<Option<&'static [&'static dyn Own]>> =
        const { std::cell::Cell::new(None) };
    static TEST_ONE_AFTER_ANOTHER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `run` with every family prepared hour by hour, as before.
#[cfg(test)]
pub(super) fn with_test_hourly<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_HOURLY.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_HOURLY.with(|value| value.replace(true)));
    run()
}

/// Run `run` with only the registered families named `names` prepared on
/// their own and every other family in the hourly walk: a family's parity
/// tests compare its own preparation with hourly preparation without other
/// families' own sections, pages and refusals in one mode only.
#[cfg(test)]
pub(super) fn with_test_only<T>(names: &[&str], run: impl FnOnce() -> T) -> T {
    let families: Vec<&'static dyn Own> = REGISTRY
        .iter()
        .copied()
        .filter(|own| names.contains(&own.family().name))
        .collect();
    assert_eq!(families.len(), names.len(), "registered families {names:?}");
    with_test_families(&families, run)
}

/// Run `run` with `families` as the families prepared on their own.
#[cfg(test)]
pub(super) fn with_test_families<T>(families: &[&'static dyn Own], run: impl FnOnce() -> T) -> T {
    struct Reset(Option<&'static [&'static dyn Own]>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_FAMILIES.with(|value| value.set(self.0));
        }
    }
    let families: &'static [&'static dyn Own] = Box::leak(families.to_vec().into_boxed_slice());
    let _reset = Reset(TEST_FAMILIES.with(|value| value.replace(Some(families))));
    run()
}

/// Run `run` with the families prepared on their own one after another, on
/// this thread, rather than at once.
#[cfg(test)]
pub(super) fn with_test_one_after_another<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ONE_AFTER_ANOTHER.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_ONE_AFTER_ANOTHER.with(|value| value.replace(true)));
    run()
}
