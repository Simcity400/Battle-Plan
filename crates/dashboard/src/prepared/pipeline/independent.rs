//! Families prepared on their own: beside the hourly walk, each prepares every
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
//! own and commits on its own, through one store writer. While other families
//! run, a family writes its batch into a fork of the open transaction on its
//! own thread and queues it; one family's thread adopts every queued batch,
//! each whole or not at all, and publishes them in one store commit (a group
//! commit, [`commit`]), so families reaching their commits together share one
//! manifest rewrite and one set of flushes. Each family learns its own
//! outcome: its write's and its adoption's errors are its own, and the
//! others' batches still commit. A family's refusals, coverage and progress
//! are its own; one that fails or panics leaves the others to finish, and its
//! error is reported once they have.
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
//! A family whose own thread observes every block hour by hour on the block
//! read alone declares the tables it reads ([`Own::shared_tables`]); those
//! families read each block once, together, ahead of them on reader threads,
//! and each reconstructs its own tables from that read on its own thread
//! ([`shared`]), so each observes exactly the frames of its own read.
//!
//! A family commits by the hourly batch rules (at each branch's end, after
//! every block budget of its branch ([`super::Batches`]), every block in
//! measured runs, and at the byte budget of what it holds), so its pages are
//! those hourly preparation publishes. Where its batches end follows from the
//! recording and its own bytes alone, never from wall time or from what other
//! families committed meanwhile.
//!
//! Work beside the families ([`Beside`], the newest battle windows) runs on
//! threads of its own while they prepare and publishes each batch, as soon as
//! it has one, through the same group commits: a batch with no progress of
//! its own, in sections no family writes, so no family's pages, ranges or
//! progress depend on it.

use super::super::Section;
use super::{
    Accepted, Batches, Checkpoint, Coverage, CoveragePage, FAMILIES, FamilyTimings, Refused,
    SCHEMA, lanes,
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
use dashboard_base::prepared::pages::{Committed, HeldPages, Pages};
use dashboard_base::prepared::priority::{JobPriority, Level};
use replay::{container::BlockIndex, container::Replay};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    io,
    panic::resume_unwind,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, atomic::AtomicBool},
};

pub(super) type Timings = Option<BTreeMap<&'static str, FamilyTimings>>;

/// Work beside the families: its name among the commits' members, and its
/// run, which publishes each batch through [`Publish`] and counts its own
/// time in its timings.
pub(super) struct Beside<'a> {
    pub name: &'static str,
    /// Publish its first view (or finish without one) before the walk competes.
    pub first_view: bool,
    #[allow(clippy::type_complexity)]
    pub run: Box<dyn FnOnce(&mut Publish<'_>, &mut Timings) -> io::Result<()> + Send + 'a>,
}

/// Publishes a batch of work beside the families in their next group commit.
pub(super) struct Publish<'p> {
    #[allow(clippy::type_complexity)]
    commit: &'p mut dyn FnMut(
        Option<&'static str>,
        Publication,
        &mut dyn FnMut(&mut store::Writer) -> io::Result<()>,
    ) -> io::Result<()>,
}

impl Publish<'_> {
    /// Writes the batch (`write`) into a fork of the open transaction and
    /// returns once a group commit published it, or with its error.
    pub(super) fn commit(
        &mut self,
        write: &mut dyn FnMut(&mut store::Writer) -> io::Result<()>,
    ) -> io::Result<()> {
        (self.commit)(None, Publication::Beside, write)
    }
    /// Publish the proven window and its refresh gate in the same commit.
    pub(super) fn window(
        &mut self,
        family: &'static str,
        branch: u64,
        window: Coverage,
        write: &mut dyn FnMut(&mut store::Writer) -> io::Result<()>,
    ) -> io::Result<()> {
        (self.commit)(
            Some(family),
            Publication::Newest {
                branch,
                coverage: window,
            },
            write,
        )
    }
    /// Register newly queued scans in the canonical group metadata before reading them.
    pub(super) fn requested(&mut self, wanted: &[(u64, String)]) -> io::Result<()> {
        (self.commit)(
            None,
            Publication::ProductionRequested(wanted.to_vec()),
            &mut |_| Ok(()),
        )
    }
}

/// A family prepared on its own.
pub(super) trait Own: Sync {
    /// Its pre-pass family: its name (that of its refusals, progress and
    /// coverage pages), tables, read fields and block extract.
    fn family(&self) -> &'static prepass::Family;
    /// Whether it chooses its run's block reads from the bound content
    /// ([`Self::bound`]): the pool reads none of its blocks until its own
    /// thread has, before its first block.
    fn binds(&self) -> bool {
        false
    }
    /// Whether the pre-pass pool extracts its blocks. A family whose own
    /// thread observes every block hour by hour on the block read alone,
    /// keeping nothing of an extract, has no lane: the pool reads none of
    /// its blocks for it, and its thread observes each block as its fold of
    /// an extract would.
    fn laned(&self) -> bool {
        true
    }
    /// The tables, fields and extract of this run's block reads, chosen
    /// from the content the phase binds, where that content decides what
    /// the family's observation can read; None reads as [`Self::family`]
    /// declares. Its branches bind the same content, and its facts say
    /// what its extract read.
    fn bound(&self, phase: &Phase<'_>) -> Option<prepass::Bound> {
        let _ = phase;
        None
    }
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
    /// The tables its own thread reads from every block it observes hour by
    /// hour on the block read alone ([`Block::read_alone`]): every selection
    /// it reads is among them. Families that declare them read each block
    /// once, together ([`shared`]); none for a family that does not. The
    /// phase binds the same content as its observer, so a content-dependent
    /// selection may leave out tables that run never reads.
    fn shared_tables(&self, phase: &Phase<'_>) -> &'static [TableId] {
        let _ = phase;
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
    /// The family's share of the shared block reader, when it declares its
    /// tables ([`Own::shared_tables`]).
    pub shared: Option<shared::Lease<'a>>,
    #[cfg(test)]
    read_hook_family: Option<&'static str>,
}

impl Block<'_> {
    /// The block read alone with `tables`, as [`Replay::read_block_alone`]
    /// reads it: each hour handed to `visit` with its recorded updates. A
    /// family that declares its tables reconstructs them from the block's
    /// one shared read, so its frames, their allocations and their recorded
    /// updates of its tables are those of its own read ([`shared`]).
    pub(super) fn read_alone(
        &self,
        tables: &[TableId],
        visit: impl FnMut(Arc<replay_frame::frame::RecordedFrame>, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        #[cfg(test)]
        let mut visit = visit;
        #[cfg(test)]
        let visit = |raw, frame: &DecodedFrame| {
            if let Some(name) = self.read_hook_family {
                let changes =
                    crate::prepared::frame_changes::FrameChanges::alone(frame, self.replay.format);
                if !test_observation(name, frame, &changes)? {
                    return Ok(());
                }
            }
            visit(raw, frame)
        };
        shared::read_alone(
            self.replay,
            self.index,
            self.shared,
            tables,
            None,
            visit,
            check,
        )
    }

    /// [`Self::read_alone`] through the hour `last`. A family that reads
    /// more of its declared tables at some hours reconstructs them through
    /// each such hour from the same shared read, which decodes them once.
    pub(super) fn read_through(
        &self,
        tables: &[TableId],
        last: i32,
        visit: impl FnMut(Arc<replay_frame::frame::RecordedFrame>, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        #[cfg(test)]
        let mut visit = visit;
        #[cfg(test)]
        let visit = |raw, frame: &DecodedFrame| {
            if let Some(name) = self.read_hook_family {
                let changes =
                    crate::prepared::frame_changes::FrameChanges::alone(frame, self.replay.format);
                if !test_observation(name, frame, &changes)? {
                    return Ok(());
                }
            }
            visit(raw, frame)
        };
        shared::read_alone(
            self.replay,
            self.index,
            self.shared,
            tables,
            Some(last),
            visit,
            check,
        )
    }
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
    /// Publish the block it just went through, as hourly preparation does
    /// at the block's end, into pages its next commit writes before
    /// [`Self::publish`]: `pages` reads what earlier blocks of the batch
    /// published there, then what the family committed. Its error refuses
    /// the branch at the block's last hour observed, where hourly
    /// preparation's publication refuses it.
    fn publish_block(&mut self, pages: &mut dyn Pages, block: &Block<'_>) -> io::Result<()> {
        let _ = (pages, block);
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
static REGISTRY: [&dyn Own; 43] = [
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
    &evaluated::RESEARCH,
    &evaluated::PROJECTS,
    &evaluated::OCCUPATION,
    &census::FACTORIES,
    &census::NUKES,
    &event_feed::FEED,
    &windowed::DIVISIONS,
    &windowed::NAVAL,
    &leaders::LEADERS,
    &sampled::AIR,
    &sampled::STABILITY,
    &sampled::NAVY,
    &economy::FUEL,
    &sampled::EQUIPMENT_LOSSES,
    &sampled::SUPPLY,
    &sampled::PARTIES,
    &sampled::FUEL_BRANCHES,
    &sampled::MARKET,
    &sampled::LEND_LEASE_BALANCE,
    &sampled::PRODUCTION_CATALOGUE,
    &sampled::CONVOYS,
    &sampled::POLITICS,
    &sampled::TRADE,
    &sampled::TERRITORY,
];

/// One read of each block for the families that observe every block hour by
/// hour on their own thread.
pub(super) mod shared;

/// Subjects, peace outcomes and idea history.
pub(super) mod quiet_politics;

/// The military air ledger, raids and air aces.
pub(super) mod air;

/// Construction, equipment stockpiles, production, country experience and
/// fuel.
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

/// Research, special projects and occupation.
pub(super) mod evaluated;

/// Country factories and nuclear stockpiles.
pub(super) mod census;

/// The event feed, with its own read of the identity history.
pub(super) mod event_feed;

/// The division log and the naval battles, ships sunk and naval losses.
pub(super) mod windowed;

/// Unit leaders' careers, commands and battles.
pub(super) mod leaders;

/// Air force and navy size, stability and its parts, equipment losses,
/// supply health, the party history, trade flows and territory.
pub(super) mod sampled;

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
/// it and reads it in the same hour: while territory is in the hourly walk,
/// the walk keeps its own read under territory's name; prepared on its own,
/// territory makes that read itself ([`sampled`]). The event feed, which
/// reads it in the same hour too, makes its own read prepared on its own
/// ([`event_feed`]).
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

    /// Territory, prepared on its own, makes its own read of the history
    /// ([`sampled`]).
    fn read_by(&self) -> &'static [&'static str] {
        &["territory"]
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
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
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
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
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

/// Whether any family normally prepared on its own is observed by the walk.
pub(super) fn walks_own() -> bool {
    REGISTRY.iter().any(|own| hourly(own.family().name))
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

/// Products whose replies read the pages of several families up to the
/// hours they answer, with those families. Such a product waits for the
/// coverage every one of them has: its own of a family prepared on its own,
/// the hourly walk's of any other ([`joint`]). Its refusals are those of the
/// families `FAMILIES` lists as read by it.
const JOINT: &[(&str, &[&str])] = &[
    ("operatives-roster", &["intelligence", "identities"]),
    (
        "agency-comparison-identities",
        &["intelligence", "identities"],
    ),
    ("raids-tally", &["raids", "identities"]),
    // Named through the identity history up to their coverage.
    ("country-factories", &["factories", "identities"]),
    ("country-nukes", &["nukes", "identities"]),
    ("supply-health", &["supply", "identities"]),
    ("equipment-losses", &["equipment_losses", "identities"]),
    ("territory", &["territory", "identities"]),
    // Factory faction totals read the faction rosters, the identity history
    // and the factories (dense factory summaries read territory too, and
    // wait for its coverage through their window).
    (
        crate::products::faction_totals::FACTORIES_GATE,
        &["groups", "identities", "factories"],
    ),
    // Casualty lifetimes joined with the wars' app continuity.
    ("casualty-breakdown", &["casualties", "wars"]),
    // Territory faction totals read the faction rosters, the identity
    // history and territory's pages.
    (
        <crate::products::faction_totals::FactionTotals as crate::products::Product>::KEY,
        &["groups", "identities", "territory"],
    ),
    // Civil-war lists with the feed's annexations as outcome evidence.
    ("civil-wars", &["politics_state", "event_feed"]),
    // The feed's own facts and the changes it merges with them.
    (
        "event-feed",
        &[
            "event_feed",
            "groups",
            "subjects",
            "ideas",
            "focus",
            "nukes",
            "projects",
        ],
    ),
];

/// The families whose coverage `product`'s queries wait for together
/// ([`JOINT`]), while any of them is prepared on its own; None otherwise,
/// when the product waits as [`gating`] says.
pub(super) fn joint(product: &str) -> Option<&'static [&'static str]> {
    JOINT
        .iter()
        .find(|(name, _)| *name == product)
        .map(|&(_, families)| families)
        .filter(|families| families.iter().any(|family| registered(family)))
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
    pub ranges: &'a BTreeMap<u64, (i32, i32)>,
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

/// The store writer and checkpoint every family commits through, in groups
/// ([`commit`]).
struct Commits<'s> {
    /// The open transaction: None while a group publishes it, and once a
    /// failed commit lost it (`lost`).
    writer: Option<store::Writer>,
    lost: bool,
    state: &'s mut Checkpoint,
    /// Status and the walk read only the last successful store publication,
    /// even while a group has updated `state` and is committing without the lock.
    snapshot: Checkpoint,
    walk_from: usize,
    saved: &'s mut Option<store::SourceKey>,
    bytes: &'s mut u64,
    committed: bool,
    /// Forks of the open transaction a family is still writing: it joins the
    /// group that publishes the transaction.
    forked: usize,
    /// Family publications waiting to stage their batch get the next writer
    /// before the walk starts another batch.
    waiting: usize,
    /// The open transaction takes no more forks: a group is collecting or
    /// publishing it.
    closed: bool,
    /// A family's thread leads the group commit.
    leading: bool,
    /// Families' batches written, waiting for the group commit.
    queue: Vec<Queued>,
    /// Each queued family's outcome, by its ticket, until it takes it.
    outcomes: BTreeMap<u64, io::Result<Accepted>>,
    tickets: u64,
    /// Families preparing at once that have not finished.
    running: usize,
    grouping: Grouping,
    /// Initial newest-view publications take precedence over the walk.
    startup: BTreeSet<usize>,
    #[cfg(test)]
    newest_first: bool,
    #[cfg(test)]
    on_publish: OnPublish,
    /// Store commits holding a family's batch, and the family commits they
    /// held.
    published: (usize, usize),
}

/// What one commit member publishes beside its staged pages.
enum Publication {
    Family(Progress),
    Walk(Box<Checkpoint>),
    Newest { branch: u64, coverage: Coverage },
    Beside,
    BesideFinished,
    ProductionRequested(Vec<(u64, String)>),
}

/// One family's written batch, or one of work beside the families, queued
/// with its publication metadata for the group commit.
struct Queued {
    ticket: u64,
    lane: usize,
    name: &'static str,
    publication: Publication,
    staged: Vec<store::Staged>,
}

/// The commits, and the condition families wait on for a fork or an outcome.
struct Group<'s> {
    commits: Mutex<Commits<'s>>,
    changed: Condvar,
}

impl<'s> Group<'s> {
    fn wait<'m>(&'m self, guard: MutexGuard<'m, Commits<'s>>) -> MutexGuard<'m, Commits<'s>> {
        self.changed
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn lock<'m, 's>(group: &'m Group<'s>) -> MutexGuard<'m, Commits<'s>> {
    group.commits.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How families' commits join groups. Preparation publishes every batch
/// queued while a group forms; tests also force groupings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grouping {
    /// A group holds the batches queued and being written when its leader
    /// queued its own; families reaching their commits while it publishes
    /// join the next.
    Natural,
    /// Every family commits alone, one after another.
    #[cfg_attr(not(test), allow(dead_code))]
    Alone,
    /// A group waits until every family still preparing has queued.
    #[cfg_attr(not(test), allow(dead_code))]
    Together,
}

impl Commits<'_> {
    /// Whether a family may fork the open transaction now.
    fn admits(&self) -> bool {
        !self.closed
            && self.writer.is_some()
            && (self.grouping != Grouping::Alone
                || (self.forked == 0 && self.queue.is_empty() && !self.leading))
    }

    /// Whether the group the leader collects is complete. Work beside the
    /// families joins whichever group forms; families wait for each other
    /// alone.
    fn collected(&mut self) -> bool {
        match self.grouping {
            Grouping::Together => {
                self.forked == 0
                    && self
                        .queue
                        .iter()
                        .filter(|queued| matches!(&queued.publication, Publication::Family(_)))
                        .count()
                        >= self.running
            }
            Grouping::Natural | Grouping::Alone => {
                self.closed = true;
                self.forked == 0
            }
        }
    }
}

/// A family's fork of the open transaction, counted until its batch is queued
/// or given up, however its write ends.
struct Forked<'m, 's> {
    group: &'m Group<'s>,
    armed: bool,
}

impl<'m, 's> Forked<'m, 's> {
    /// No longer writing; queues `queued` in the same step, under a ticket of
    /// its own, so the group collecting the transaction holds it.
    fn finish(mut self, queued: Option<Queued>) -> (MutexGuard<'m, Commits<'s>>, u64) {
        self.armed = false;
        let mut guard = lock(self.group);
        guard.forked -= 1;
        let ticket = guard.tickets;
        if let Some(mut queued) = queued {
            guard.tickets += 1;
            queued.ticket = ticket;
            guard.queue.push(queued);
        }
        self.group.changed.notify_all();
        (guard, ticket)
    }
}

impl Drop for Forked<'_, '_> {
    fn drop(&mut self) {
        if self.armed {
            lock(self.group).forked -= 1;
            self.group.changed.notify_all();
        }
    }
}

/// Leads a group commit until it hands every member its outcome; one that
/// panics loses the writer and releases the members, which then fail.
struct Leading<'m, 's> {
    group: &'m Group<'s>,
    armed: bool,
}

impl Drop for Leading<'_, '_> {
    fn drop(&mut self) {
        if self.armed {
            let mut guard = lock(self.group);
            guard.lost = true;
            guard.writer = None;
            guard.leading = false;
            guard.closed = false;
            drop(guard);
            self.group.changed.notify_all();
        }
    }
}

/// A newest-view job that finished without a publication no longer holds up
/// the walk. A successful first commit releases it earlier.
struct Startup<'m, 's>(&'m Group<'s>, usize);

impl Drop for Startup<'_, '_> {
    fn drop(&mut self) {
        ready(self.0, self.1);
    }
}

fn ready(group: &Group<'_>, lane: usize) {
    lock(group).startup.remove(&lane);
    group.changed.notify_all();
}

/// The hourly walk stages a batch without holding up family publications.
/// Once ready, its fork and checkpoint join the next group commit.
/// The independent newest views publish before its first batch.
pub(super) struct Walk<'m, 's> {
    group: &'m Group<'s>,
    phase: &'m Phase<'m>,
    lane: usize,
    pool: Option<&'m Pool>,
}

impl Walk<'_, '_> {
    pub(super) fn open(
        &mut self,
        accepted: &mut dyn FnMut(&Accepted),
    ) -> io::Result<store::Writer> {
        let mut guard = lock(self.group);
        loop {
            check_cancel(self.phase.cancelled)?;
            if guard.lost {
                return Err(lost_to(self.phase));
            }
            if guard.startup.is_empty() && guard.admits() {
                return guard.writer.as_mut().ok_or_else(lost)?.stage();
            }
            let next = accepted_now(&guard, self.phase);
            guard = self
                .group
                .changed
                .wait_timeout(guard, std::time::Duration::from_millis(25))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            drop(guard);
            accepted(&next);
            guard = lock(self.group);
        }
    }

    /// Release the walk's registered lanes only after its first-view gate.
    pub(super) fn prepass(&self) -> Option<prepass::PrePass> {
        self.pool.and_then(Pool::walk)
    }

    pub(super) fn folded(&self, took: std::time::Duration) {
        if let Some(pool) = self.pool {
            pool.walk_folded(took);
        }
    }

    pub(super) fn published(&self, took: std::time::Duration) {
        if let Some(pool) = self.pool {
            pool.walk_published(took);
        }
    }

    pub(super) fn commit(
        &mut self,
        writer: store::Writer,
        state: &Checkpoint,
        workers: &mut Timings,
    ) -> io::Result<(Checkpoint, Accepted)> {
        // Writing and flushing the detached batch never closes the family's
        // transaction. Only the ready batch joins the group that adopts it.
        let staged = writer.into_staged()?;
        let forked;
        {
            let mut guard = lock(self.group);
            guard.waiting += 1;
            while !guard.admits() {
                if guard.lost {
                    guard.waiting -= 1;
                    self.group.changed.notify_all();
                    return Err(lost_to(self.phase));
                }
                guard = self.group.wait(guard);
            }
            guard.waiting -= 1;
            guard.forked += 1;
            forked = Forked {
                group: self.group,
                armed: true,
            };
        }
        queued(
            self.group,
            self.phase,
            workers,
            forked,
            Queued {
                ticket: 0,
                lane: self.lane,
                name: "walk",
                publication: Publication::Walk(Box::new(state.clone())),
                staged,
            },
        )?;
        let guard = lock(self.group);
        Ok((guard.snapshot.clone(), accepted_now(&guard, self.phase)))
    }
}

/// A family preparing at once that has finished, however it ended.
struct Finished<'m, 's>(&'m Group<'s>);

impl Drop for Finished<'_, '_> {
    fn drop(&mut self) {
        let mut guard = lock(self.0);
        guard.running = guard.running.saturating_sub(1);
        drop(guard);
        self.0.changed.notify_all();
    }
}

fn lost() -> io::Error {
    io::Error::other("The prepared writer was lost to a failed commit")
}

/// Why a family cannot commit once the writer is lost: cancellation, as a
/// commit after a cancelled one always reported, or the lost writer.
fn lost_to(phase: &Phase<'_>) -> io::Error {
    check_cancel(phase.cancelled).err().unwrap_or_else(lost)
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

/// Prepares every family through the recording's last block beside `walk`,
/// with the work `beside` them on threads of its own. All publish through
/// `writer` in the same group commits. Returns the open writer and
/// whether anything was committed (with the writer's pending metadata).
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    phase: &Phase<'_>,
    writer: store::Writer,
    state: &mut Checkpoint,
    saved: &mut Option<store::SourceKey>,
    bytes: &mut u64,
    timings: &mut Timings,
    workers: &mut Timings,
    beside: Vec<Beside<'_>>,
    accepted: &mut dyn FnMut(&Accepted),
    walk: impl FnOnce(&mut Walk<'_, '_>, &mut dyn FnMut(&Accepted)) -> io::Result<()>,
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
    // Families' own rows are worker time. The phase counts only time outside
    // the walk, whose overlap is timed by its own observations.
    let phase_started = super::start_timing(timings);
    #[allow(unused_mut)]
    let mut knobs = prepass::Knobs::current();
    #[cfg(test)]
    {
        knobs.refusals = lanes::test_refusals();
        if let Some(hook) = TEST_OBSERVATION_HOOK.with(|hook| hook.borrow().clone()) {
            let previous = knobs.hook.take();
            knobs.hook = Some(std::sync::Arc::new(move |name, frame| {
                if let Some(previous) = &previous {
                    match previous(name, frame) {
                        prepass::Fault::Continue => {}
                        fault => return fault,
                    }
                }
                let changes = crate::prepared::frame_changes::FrameChanges::alone(
                    frame,
                    replay_data::format::FormatVersion::CURRENT,
                );
                match hook(name, frame, &changes) {
                    lanes::Fault::Continue => prepass::Fault::Continue,
                    lanes::Fault::Skip => prepass::Fault::Skip,
                    lanes::Fault::Disconnect | lanes::Fault::Error => prepass::Fault::Error,
                    lanes::Fault::Panic => prepass::Fault::Panic,
                }
            }));
            // A hook may target an otherwise quiet hour of its family's read.
            knobs.alone = true;
        }
    }
    let at_once = !one_after_another();
    // The families observing every block hour by hour read each block once,
    // together, each from its own first block: all from the start when they
    // prepare at once, each from its own start otherwise.
    let consumers: Vec<(&[TableId], usize)> = runs
        .iter()
        .map(|run| (run.own.shared_tables(phase), run.progress.next_block))
        .collect();
    // Register every family before the pool can read ahead of its startup.
    let section_consumers: Vec<_> = runs
        .iter()
        .map(|run| replay.section_consumer(run.progress.next_block))
        .collect();
    let shared = if runs.is_empty() {
        None
    } else {
        shared::Shared::start(phase.source.replay(), &consumers, at_once)
    };
    // Every family's blocks from its own first one, on one set of workers,
    // which read the shared reader's blocks with theirs.
    let walk_from = (at_once && prepass::enabled() && state.cursor.next_block < replay.index.len())
        .then_some(state.cursor.next_block);
    let pool = (!runs.is_empty() || walk_from.is_some()).then(|| {
        let lanes = runs
            .iter()
            .map(|run| {
                let from = if run.own.laned() {
                    run.progress.next_block
                } else {
                    replay.index.len()
                };
                (run.own.family(), from, run.own.binds())
            })
            .collect();
        match walk_from {
            Some(from) => Pool::start_with_walk(
                phase.source.replay(),
                lanes,
                prepass::workers(lanes::parallelism()),
                knobs,
                shared.as_ref().map(shared::Shared::depot),
                Some(from),
            ),
            None => Pool::start(
                phase.source.replay(),
                lanes,
                prepass::workers(lanes::parallelism()),
                knobs,
                shared.as_ref().map(shared::Shared::depot),
            ),
        }
    });
    let families = runs.len();
    let walk_lane = families + beside.len();
    let commits = Group {
        commits: Mutex::new(Commits {
            writer: Some(writer),
            lost: false,
            snapshot: state.clone(),
            walk_from: state.cursor.next_block,
            state,
            saved,
            bytes,
            committed: false,
            forked: 0,
            waiting: 0,
            closed: false,
            leading: false,
            queue: Vec::new(),
            outcomes: BTreeMap::new(),
            tickets: 0,
            // One after another, only the family preparing now runs.
            running: if at_once { families } else { families.min(1) },
            grouping: grouping(),
            startup: beside
                .iter()
                .enumerate()
                .filter(|(_, job)| job.first_view)
                .map(|(at, _)| families + at)
                .collect(),
            #[cfg(test)]
            newest_first: TEST_NEWEST_FIRST.with(std::cell::Cell::get),
            #[cfg(test)]
            on_publish: TEST_ON_PUBLISH.with(|hook| hook.borrow().clone()),
            published: (0, 0),
        }),
        changed: Condvar::new(),
    };
    let local = || -> (Timings, Timings) {
        (
            timings.as_ref().map(|_| BTreeMap::new()),
            workers.as_ref().map(|_| BTreeMap::new()),
        )
    };
    #[cfg(test)]
    let context = TestContext::capture();
    let (ran, besides, walked, walk_wall) = std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel::<Accepted>();
        let walked;
        let walk_wall;
        // The work beside the families, each on a thread of its own however
        // the families prepare, its lanes after theirs.
        let mut besides = Vec::with_capacity(beside.len());
        for (at, job) in beside.into_iter().enumerate() {
            let sender = sender.clone();
            let commits = &commits;
            let (mut own_timings, mut own_workers) = local();
            #[cfg(test)]
            let context = context.clone();
            let name = job.name;
            let priority = JobPriority::current();
            let startup = Startup(commits, families + at);
            besides.push(
                crate::prepared::runner::worker_thread(format!("preparation of {name}"))
                    .spawn_scoped(scope, move || {
                        let _startup = startup;
                        priority.enter(Level::Normal);
                        #[cfg(test)]
                        let _installed = context.install();
                        let _cpu = ::replay_frame::cpu::scope("newest", Some(name));
                        let result = {
                            let mut publish = |member: Option<&'static str>,
                                               publication: Publication,
                                               write: &mut dyn FnMut(
                                &mut store::Writer,
                            )
                                -> io::Result<()>| {
                                let _cpu = ::replay_frame::cpu::scope("commit", None);
                                let name = member.unwrap_or(name);
                                let next = commit(
                                    commits,
                                    phase,
                                    families + at,
                                    name,
                                    &mut own_workers,
                                    |writer| {
                                        write(writer)?;
                                        Ok(publication)
                                    },
                                )?;
                                // The caller reports it; a caller that
                                // has gone only misses the report.
                                let _ = sender.send(next);
                                Ok(())
                            };
                            (job.run)(
                                &mut Publish {
                                    commit: &mut publish,
                                },
                                &mut own_timings,
                            )
                        };
                        let result = result.and_then(|()| {
                            let next = commit(
                                commits,
                                phase,
                                families + at,
                                name,
                                &mut own_workers,
                                |_| Ok(Publication::BesideFinished),
                            )?;
                            let _ = sender.send(next);
                            Ok(())
                        });
                        (result, own_timings, own_workers, Counts::take())
                    }),
            );
        }
        let ran: Vec<std::thread::Result<Ran>> = if !at_once {
            let mut ran = Vec::with_capacity(runs.len());
            for (run, section_consumer) in runs.into_iter().zip(section_consumers) {
                let (mut own_timings, mut own_workers) = local();
                let member = shared
                    .as_ref()
                    .and_then(|shared| shared.member(run.lane, true));
                let result = prepare_family(
                    phase,
                    pool.as_ref().expect("the families' pool"),
                    member,
                    section_consumer,
                    run,
                    &commits,
                    &mut own_timings,
                    &mut own_workers,
                    &mut |next| {
                        // What the work beside committed first is reported
                        // first.
                        for earlier in receiver.try_iter() {
                            accepted(&earlier);
                        }
                        accepted(&next);
                    },
                );
                ran.push(Ok((result, own_timings, own_workers, Counts::default())));
            }
            // No group waits for the families once they have finished.
            drop(Finished(&commits));
            let walk_started = std::time::Instant::now();
            walked = walk(
                &mut Walk {
                    group: &commits,
                    phase,
                    lane: walk_lane,
                    pool: pool.as_ref(),
                },
                &mut |_| {
                    for _ in receiver.try_iter() {}
                    let current = accepted_now(&lock(&commits), phase);
                    accepted(&current);
                },
            );
            walk_wall = walk_started.elapsed();
            drop(sender);
            for _ in receiver.iter() {
                let current = accepted_now(&lock(&commits), phase);
                accepted(&current);
            }
            ran
        } else {
            let mut started = Vec::with_capacity(runs.len());
            for (run, section_consumer) in runs.into_iter().zip(section_consumers) {
                let sender = sender.clone();
                let (pool, commits) = (pool.as_ref().expect("the families' pool"), &commits);
                let (mut own_timings, mut own_workers) = local();
                // Moved to the family's thread: dropped with it however it
                // ends, also when it cannot start, so no family waits for it.
                let member = shared
                    .as_ref()
                    .and_then(|shared| shared.member(run.lane, false));
                #[cfg(test)]
                let context = context.clone();
                let priority = JobPriority::current();
                let thread = crate::prepared::runner::worker_thread(format!(
                    "preparation of {}",
                    run.own.family().name
                ))
                .spawn_scoped(scope, move || {
                    priority.enter(Level::Normal);
                    // No group waits for it once it has finished.
                    let _finished = Finished(commits);
                    #[cfg(test)]
                    let _installed = context.install();
                    let result = prepare_family(
                        phase,
                        pool,
                        member,
                        section_consumer,
                        run,
                        commits,
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
                if thread.is_err() {
                    drop(Finished(commits));
                }
                started.push(thread);
            }
            let walk_started = std::time::Instant::now();
            walked = walk(
                &mut Walk {
                    group: &commits,
                    phase,
                    lane: walk_lane,
                    pool: pool.as_ref(),
                },
                &mut |_| {
                    for _ in receiver.try_iter() {}
                    let current = accepted_now(&lock(&commits), phase);
                    accepted(&current);
                },
            );
            walk_wall = walk_started.elapsed();
            drop(sender);
            // Each family's accepted commits, and those of the work beside
            // them, reported in the order they committed, until every one
            // has finished.
            for _ in receiver.iter() {
                let current = accepted_now(&lock(&commits), phase);
                accepted(&current);
            }
            started
                .into_iter()
                .map(|thread| match thread {
                    Ok(handle) => handle.join(),
                    Err(error) => Ok((Err(error), None, None, Counts::default())),
                })
                .collect()
        };
        let besides: Vec<std::thread::Result<Ran>> = besides
            .into_iter()
            .map(|thread| match thread {
                Ok(handle) => handle.join(),
                Err(error) => Ok((Err(error), None, None, Counts::default())),
            })
            .collect();
        (ran, besides, walked, walk_wall)
    });
    // The pool's block reads (decoding and reconstruction, beside every
    // family's extract), counted with the blocks read.
    if let Some(pool) = pool {
        let (spent, reads) = pool.read_time();
        drop(pool);
        if let Some(workers) = workers.as_mut() {
            let entry = workers.entry(prepass::PREPASS_READ).or_default();
            entry.observe += spent;
            entry.calls += reads;
        }
        #[cfg(test)]
        prepass::add_notes(&BTreeMap::from([(
            prepass::PREPASS_READ,
            usize::try_from(reads).unwrap_or(usize::MAX),
        )]));
    }
    if let Some(shared) = shared {
        let (spent, reads) = shared.read_time();
        let deposits = shared.deposits();
        drop(shared);
        if let Some(workers) = workers.as_mut() {
            let entry = workers.entry(shared::SHARED_READ).or_default();
            entry.observe += spent;
            entry.calls += reads;
            workers.entry(shared::SHARED_DEPOSITED).or_default().calls += deposits;
        }
        #[cfg(test)]
        prepass::add_notes(&BTreeMap::from([
            (
                shared::SHARED_READ,
                usize::try_from(reads).unwrap_or(usize::MAX),
            ),
            (
                shared::SHARED_DEPOSITED,
                usize::try_from(deposits).unwrap_or(usize::MAX),
            ),
        ]));
    }
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
    // The work beside the families runs on workers too. Its own rows and
    // commits are worker time; the phase already counts caller time outside
    // the walk. Its errors follow the families'.
    for ran in besides {
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
    if let Err(error) = walked {
        first_error.get_or_insert(error);
    }
    if let Some(started) = phase_started
        && let Some(timings) = timings
    {
        let entry = timings.entry("independent").or_default();
        entry.observe += started.0.elapsed().saturating_sub(walk_wall);
        entry.calls += 1;
    }
    if let Some(payload) = panic {
        resume_unwind(payload);
    }
    let Commits {
        writer,
        committed,
        published: _published,
        ..
    } = commits
        .commits
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    #[cfg(test)]
    TEST_PUBLISHED.with(|counts| {
        let (stores, families) = counts.get();
        counts.set((stores + _published.0, families + _published.1));
    });
    if let Some(error) = first_error {
        return Err(error);
    }
    let writer = writer.ok_or_else(lost)?;
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
    member: Option<shared::Member<'_>>,
    section_consumer: replay::container::SectionConsumer,
    run: Run,
    commits: &Group<'_>,
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
    let family = own.family();
    let replay = phase.replay;
    let name = family.name;
    let _cpu = ::replay_frame::cpu::scope("fold", Some(name));
    let laned = {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        let laned = own.laned();
        if laned && own.binds() {
            pool.bind(lane, own.bound(phase));
        }
        laned
    };
    let started = super::start_timing(timings);
    let mut held: Option<(u64, Box<dyn Branch>)> = None;
    // The pages its blocks published since its last commit.
    let mut pages = HeldPages::default();
    let mut touched = BTreeSet::new();
    for block in progress.next_block..replay.index.len() {
        section_consumer.at(block);
        check_cancel(phase.cancelled)?;
        let index = &replay.index[block];
        let branch = index.branch;
        if let Some(member) = &member {
            // A family that refused the branch reads none of its blocks.
            let reads = progress
                .refused
                .get(&branch)
                .is_none_or(|refused| refused.open(name));
            member.at(block, reads);
        }
        if held.as_ref().is_none_or(|(held, _)| *held != branch) {
            let saved = lock(commits).saved.clone();
            let refused = progress.refused.entry(branch).or_default();
            let load = Load {
                store: phase.store,
                key: phase.key,
                saved: saved.as_ref().filter(|_| refused.open(name)),
                replay,
            };
            let mut state = {
                let _cpu = ::replay_frame::cpu::scope("fold load", None);
                own.load(&load, branch)?
            };
            if timings.is_some() {
                state.measure();
            }
            held = Some((branch, state));
        }
        let (_, state) = held.as_mut().expect("held family state");
        let outcome = if laned {
            let (outcome, took) = pool.take(lane, block, phase.cancelled)?;
            if let Some(workers) = workers.as_mut() {
                let entry = workers.entry(family.extract).or_default();
                entry.observe += took;
                entry.calls += 1;
            }
            Some(outcome)
        } else {
            None
        };
        let folded_started = std::time::Instant::now();
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
                #[cfg(test)]
                read_hook_family: (!laned).then_some(name),
                shared: member
                    .as_ref()
                    .map(|member| shared::Lease { member, block }),
            };
            let cpu_driver = ::replay_frame::cpu::scope("fold driver", None);
            reached = match outcome {
                Some(Outcome::Facts(facts)) => {
                    let cpu_started = ::replay_frame::cpu::thread_cycles();
                    let (folded, reached) = state.fold(facts, &at, &mut observed, &mut check)?;
                    if folded == Folded::Fallback {
                        note(timings, family.fallback);
                        ::replay_frame::cpu::note("fold-fallback", name, cpu_started);
                    } else {
                        ::replay_frame::cpu::note("fold-facts", name, cpu_started);
                    }
                    reached
                }
                Some(Outcome::Sequential) => {
                    note(timings, family.sequential);
                    let cpu_started = ::replay_frame::cpu::thread_cycles();
                    let reached = state.observe_block(&at, &mut observed, &mut check)?;
                    ::replay_frame::cpu::note("fold-sequential", name, cpu_started);
                    reached
                }
                // Its own thread observes the block, as its fold does.
                None => {
                    let cpu_started = ::replay_frame::cpu::thread_cycles();
                    let reached = state.observe_block(&at, &mut observed, &mut check)?;
                    ::replay_frame::cpu::note("fold-own", name, cpu_started);
                    reached
                }
            };
            drop(cpu_driver);
            if refused.open(name) {
                let _cpu = ::replay_frame::cpu::scope("fold block flush", None);
                state.end_block(replay, branch);
                let saved = lock(commits).saved.clone();
                let mut view = pages.view(Committed {
                    store: phase.store,
                    key: phase.key,
                    saved: saved.as_ref(),
                });
                let result = state.publish_block(&mut view, &at);
                refused.check(name, reached.unwrap_or(index.last_hour), result)?;
            }
            if let Some(families) = timings.as_mut() {
                for (part, took, times) in state.parts() {
                    let entry = families.entry(part).or_default();
                    entry.observe += took;
                    entry.calls += times;
                }
            }
        }
        pool.folded(lane, folded_started.elapsed());
        touched.insert(branch);
        let continues = replay
            .index
            .get(block + 1)
            .is_some_and(|next| next.branch == branch);
        // The batch ends where the hourly walk's does by the recording, or
        // at the byte budget of what this family holds: never by wall time,
        // nor by the shared writer, which other families' commits fill.
        if !dashboard_base::prepared::pages::publication_due(
            continues,
            phase.measured,
            phase.batches.full(block),
            || (state.pending(), pages.bytes()),
        ) {
            continue;
        }
        // The family's hourly flush, at the hour it would run.
        let commit_started = super::start_timing(timings);
        let hour = reached.unwrap_or(index.last_hour);
        let cpu_commit = ::replay_frame::cpu::scope("commit", None);
        let next = commit(commits, phase, lane, name, workers, |writer| {
            let write_started = std::time::Instant::now();
            let refused = progress.refused.entry(branch).or_default();
            // The pages of the blocks it went through, refused since or
            // not: those each block published, then those it holds.
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold publish", None);
                pages.write(writer)?;
                state.publish(writer, branch, phase.cancelled)
            };
            refused.check(name, hour, result)?;
            if refused.open(name) {
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold flush", None);
                    state.flush(writer, replay, branch, phase.cancelled)
                };
                refused.check(name, hour, result)?;
            }
            if refused.open(name) {
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold save", None);
                    state.save(writer, branch)?
                };
                refused.check(name, hour, result)?;
            }
            if refused.open(name) {
                let _cpu = ::replay_frame::cpu::scope("fold continuation", None);
                state.save_state(writer, branch)?;
            }
            progress.next_block = block + 1;
            progress.refused.retain(|_, refused| !refused.0.is_empty());
            for &branch in &touched {
                save_coverage(writer, phase, name, &progress, branch)?;
            }
            pool.published(lane, write_started.elapsed());
            Ok(Publication::Family(progress.clone()))
        })?;
        touched.clear();
        drop(cpu_commit);
        super::finish_timing(timings, commit_started, "independent.commit", true);
        accepted(next);
        if !continues || super::reload_flushed() {
            held = None;
        }
    }
    super::finish_timing(timings, started, own.timing(), false);
    Ok(())
}

/// Commits a batch and the publication metadata returned by `write`, then
/// opens the next transaction. Family progress and proven newest-window
/// extents enter the checkpoint with their pages. Returns what was accepted.
///
/// The family forks the open transaction, writes its batch into the fork on
/// its own thread and queues
/// it. The first family to queue while no group forms leads one: it closes
/// the transaction to new forks, waits for the forks still being written,
/// adopts every queued batch (in lane order, each whole or not at all) and
/// publishes them in one store commit. A family reaching its commit while a
/// group publishes forks the next transaction once that is open, and joins
/// the next group. A family's own write and adoption errors are its own: its
/// batch is never listed, and the others' batches commit without it. An
/// error of the joint publication (sources changed, cancellation, the
/// checkpoint or the store's own writes) belongs to no family's batch and
/// would fail any of them alone; it is every member's, and loses the writer
/// as a failed commit always has. A family's pages, ranges and progress are
/// what it writes whoever it commits with: each section is one family's, its
/// ranges numbered by that family's writes alone.
#[allow(clippy::too_many_arguments)]
fn commit(
    group: &Group<'_>,
    phase: &Phase<'_>,
    lane: usize,
    name: &'static str,
    workers: &mut Timings,
    write: impl FnOnce(&mut store::Writer) -> io::Result<Publication>,
) -> io::Result<Accepted> {
    // Declared first, dropped last: the fork, and its duplicate of the
    // writer's lock, close before the fork stops counting.
    let forked;
    let mut fork;
    {
        let mut guard = lock(group);
        guard.waiting += 1;
        while !guard.admits() {
            if guard.lost {
                guard.waiting -= 1;
                group.changed.notify_all();
                return Err(lost_to(phase));
            }
            guard = group.wait(guard);
        }
        let opened = guard.writer.as_mut().expect("an open writer").fork();
        guard.waiting -= 1;
        group.changed.notify_all();
        fork = opened?;
        guard.forked += 1;
        forked = Forked { group, armed: true };
    }
    let written = write(&mut fork)
        .and_then(|publication| fork.into_staged().map(|staged| (publication, staged)));
    let (publication, staged) = match written {
        Ok(written) => written,
        Err(error) => {
            drop(forked.finish(None));
            return Err(error);
        }
    };
    queued(
        group,
        phase,
        workers,
        forked,
        Queued {
            ticket: 0,
            lane,
            name,
            publication,
            staged,
        },
    )
}

fn queued<'m, 's>(
    group: &'m Group<'s>,
    phase: &Phase<'_>,
    workers: &mut Timings,
    forked: Forked<'m, 's>,
    queued: Queued,
) -> io::Result<Accepted> {
    let lane = queued.lane;
    let (mut guard, ticket) = forked.finish(Some(queued));
    if !guard.leading {
        guard.leading = true;
        guard = lead(group, guard, phase, workers);
    }
    loop {
        if let Some(outcome) = guard.outcomes.remove(&ticket) {
            if outcome.is_ok() {
                #[cfg(test)]
                let newest_first = guard.newest_first;
                #[cfg(not(test))]
                let newest_first = false;
                if !newest_first {
                    guard.startup.remove(&lane);
                }
                group.changed.notify_all();
            }
            return outcome;
        }
        if !guard.leading {
            // Its leader panicked before handing it an outcome.
            return Err(lost_to(phase));
        }
        guard = group.wait(guard);
    }
}

/// Collects, adopts and publishes one group, handing each member its
/// outcome. Returns with no group leading.
fn lead<'m, 's>(
    group: &'m Group<'s>,
    guard: MutexGuard<'m, Commits<'s>>,
    phase: &Phase<'_>,
    workers: &mut Timings,
) -> MutexGuard<'m, Commits<'s>> {
    // Declared before the guard, so a panic releases the lock first.
    let mut leading = Leading { group, armed: true };
    let mut guard = guard;
    while !guard.collected() {
        guard = group.wait(guard);
    }
    guard.closed = true;
    let started = super::start_timing(workers);
    let mut queued = std::mem::take(&mut guard.queue);
    // Which families commit together follows timing; the order they are
    // listed in a group does not.
    queued.sort_by_key(|queued| queued.lane);
    let mut members = Vec::with_capacity(queued.len());
    let mut refused = Vec::new();
    match guard.writer.as_mut() {
        Some(writer) => {
            for queued in queued {
                match writer.adopt_all(queued.staged) {
                    Ok(()) => members.push((queued.ticket, (queued.name, queued.publication))),
                    // Pending metadata is not an accepted batch. A refusal
                    // leaves that family's pending or accepted progress intact.
                    Err(error) => refused.push((queued.ticket, Err(error))),
                }
            }
        }
        None => refused.extend(
            queued
                .into_iter()
                .map(|queued| (queued.ticket, Err(lost_to(phase)))),
        ),
    }
    guard.outcomes.extend(refused);
    if !members.is_empty() {
        let (tickets, members): (Vec<u64>, Vec<_>) = members.into_iter().unzip();
        let (published, result) = publish(group, guard, phase, &members);
        guard = published;
        match result {
            Ok(()) => {
                let accepted = accepted_now(&guard, phase);
                guard.outcomes.extend(
                    tickets
                        .into_iter()
                        .map(|ticket| (ticket, Ok(accepted.clone()))),
                );
            }
            Err(error) => {
                let copies: Vec<_> = tickets[1..]
                    .iter()
                    .map(|&ticket| (ticket, Err(io::Error::new(error.kind(), error.to_string()))))
                    .collect();
                guard.outcomes.insert(tickets[0], Err(error));
                guard.outcomes.extend(copies);
            }
        }
        super::finish_timing(workers, started, "independent.publish", true);
    }
    leading.armed = false;
    guard.closed = false;
    guard.leading = false;
    group.changed.notify_all();
    guard
}

/// Writes the checkpoint with `members`' publication metadata into the open
/// transaction and publishes it, then opens the next. The lock is released while the
/// store commits; returns with it held again. A failure loses the writer.
fn publish<'m, 's>(
    group: &'m Group<'s>,
    mut guard: MutexGuard<'m, Commits<'s>>,
    phase: &Phase<'_>,
    members: &[(&'static str, Publication)],
) -> (MutexGuard<'m, Commits<'s>>, io::Result<()>) {
    let commits = &mut *guard;
    let mut finished = Vec::new();
    let mut requested = Vec::new();
    for (name, publication) in members {
        match publication {
            Publication::Family(progress) => {
                commits
                    .state
                    .independent
                    .insert((*name).to_owned(), progress.clone());
            }
            Publication::Walk(state) => {
                let mut state = (**state).clone();
                state.independent = std::mem::take(&mut commits.state.independent);
                state.newest = std::mem::take(&mut commits.state.newest);
                *commits.state = state;
            }
            Publication::Newest { branch, coverage } => {
                commits
                    .state
                    .newest
                    .entry((*name).to_owned())
                    .or_default()
                    .insert(branch.to_string(), coverage.clone());
            }
            Publication::Beside => {}
            Publication::BesideFinished => finished.push(*name),
            Publication::ProductionRequested(scans) => requested.extend(scans.iter().cloned()),
        }
    }
    let Some(mut writer) = commits.writer.take() else {
        commits.lost = true;
        return (guard, Err(lost()));
    };
    let written = super::side::update(&mut writer, &requested, &finished)
        .and_then(|()| {
            super::write_checkpoint_state(&mut writer, commits.state, phase.replay, *commits.bytes)
        })
        .and_then(|bytes| check_cancel(phase.cancelled).map(|()| bytes));
    let bytes = match written {
        Ok(bytes) => bytes,
        Err(error) => {
            commits.lost = true;
            return (guard, Err(error));
        }
    };
    // Families keep folding meanwhile: they read the published store, and
    // those reaching their commits wait for the next transaction.
    drop(guard);
    let next = writer.commit_continue();
    let mut guard = lock(group);
    let commits = &mut *guard;
    let writer = match next {
        Ok(writer) => writer,
        Err(error) => {
            commits.lost = true;
            return (guard, Err(error));
        }
    };
    commits.committed = true;
    *commits.saved = Some(phase.key.clone());
    *commits.bytes = bytes;
    commits.snapshot = commits.state.clone();
    #[cfg(test)]
    if let Some(hook) = &commits.on_publish {
        for (name, publication) in members {
            if let Publication::Family(progress) = publication {
                hook(name, progress.next_block);
            }
        }
    }
    let families = members
        .iter()
        .filter(|(_, publication)| matches!(publication, Publication::Family(_)))
        .count();
    if families > 0 {
        commits.published.0 += 1;
        commits.published.1 += families;
    }
    commits.writer = Some(writer);
    (guard, Ok(()))
}

/// What every family has accepted, as the last commit published it.
fn accepted_now(commits: &Commits<'_>, phase: &Phase<'_>) -> Accepted {
    let state = &commits.snapshot;
    Accepted {
        sections: super::preparing_sections(
            state,
            state.cursor.next_block,
            phase.replay.index.len(),
        ),
        bytes: *commits.bytes,
        next_block: state.cursor.next_block,
        new_blocks: state.cursor.next_block.saturating_sub(commits.walk_from),
        coverage: super::accepted_coverage(&state.map, phase.last_blocks),
        families: super::family_coverages(state, phase.replay, phase.last_blocks),
    }
}

/// A family's accepted extent of `branch` once it has gone through the
/// blocks before `next_block`: the branch's first hour through the last hour
/// of its last block among them, or its last hour when that block is the
/// branch's last. None before the family has gone through any of its blocks.
fn own_coverage(
    replay: &Replay,
    ranges: &BTreeMap<u64, (i32, i32)>,
    last_blocks: &BTreeMap<u64, usize>,
    next_block: usize,
    branch: u64,
) -> Option<Coverage> {
    let &(first, last) = ranges.get(&branch)?;
    let done = replay.index[..next_block]
        .iter()
        .rposition(|index| index.branch == branch)?;
    let complete = last_blocks
        .get(&branch)
        .is_some_and(|&last| last < next_block);
    Some(Coverage {
        schema: SCHEMA,
        first_hour: first,
        last_hour: if complete {
            last
        } else {
            replay.index[done].last_hour
        },
        complete,
    })
}

/// Each family prepared on its own's accepted extent by branch, as its
/// coverage pages publish them ([`save_coverage`]): the preparation status
/// lists them, so a client re-asks a reply waiting on a family when that
/// family's coverage moves.
pub(super) fn coverages(
    independent: &BTreeMap<String, Progress>,
    replay: &Replay,
    ranges: &BTreeMap<u64, (i32, i32)>,
    last_blocks: &BTreeMap<u64, usize>,
) -> BTreeMap<String, BTreeMap<String, Coverage>> {
    independent
        .iter()
        .map(|(name, progress)| {
            let next_block = progress.next_block.min(replay.index.len());
            let branches = ranges
                .keys()
                .filter_map(|&branch| {
                    own_coverage(replay, ranges, last_blocks, next_block, branch)
                        .map(|coverage| (branch.to_string(), coverage))
                })
                .collect();
            (name.clone(), branches)
        })
        .collect()
}

/// The family's accepted extent of one branch, with its refusals there.
fn save_coverage(
    writer: &mut store::Writer,
    phase: &Phase<'_>,
    family: &str,
    progress: &Progress,
    branch: u64,
) -> io::Result<()> {
    let Some(coverage) = own_coverage(
        phase.replay,
        phase.ranges,
        phase.last_blocks,
        progress.next_block,
        branch,
    ) else {
        return Ok(());
    };
    super::json(
        writer,
        &coverage_section(family, branch),
        &CoveragePage {
            coverage,
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
    kept: BTreeMap<&'static str, (usize, usize)>,
    #[cfg(test)]
    notes: BTreeMap<&'static str, usize>,
    #[cfg(test)]
    naval_saves: Vec<usize>,
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
            kept: dashboard_base::prepared::kept::counts(),
            #[cfg(test)]
            notes: prepass::notes(),
            #[cfg(test)]
            naval_saves: windowed::take_naval_saves(),
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
            dashboard_base::prepared::kept::add_counts(&self.kept);
            windowed::add_naval_saves(self.naval_saves);
            country_groups::prepared::add_counts(self.groups);
            crate::products::military_air_ledger::prepared::add_counts(self.air_ledger);
        }
    }
}

/// The caller's test settings a family's thread prepares under.
#[cfg(test)]
#[derive(Clone)]
struct TestContext {
    presentation: Option<(usize, usize)>,
    observation_hook: Option<lanes::Hook>,
    prepass: prepass::TestContext,
    lanes: lanes::TestContext,
    research_fallback: Vec<i32>,
    sampled_fallback: bool,
    reload: bool,
    batch_blocks: Option<usize>,
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
            presentation: crate::products::politics_ledger::prepared::test_presentation(),
            observation_hook: TEST_OBSERVATION_HOOK.with(|hook| hook.borrow().clone()),
            prepass: prepass::TestContext::capture(),
            lanes: lanes::TestContext::capture(),
            research_fallback: crate::products::research::prepared::block::forced_fallback_hours(),
            sampled_fallback: sampled::test_fallback(),
            reload: super::RELOAD_FLUSHED.with(std::cell::Cell::get),
            batch_blocks: super::TEST_BATCH_BLOCKS.with(std::cell::Cell::get),
            registry: test_registry(),
        }
    }

    /// Install the settings on this thread for as long as the guard lives.
    fn install(self) -> impl Drop {
        struct Installed<P, L, R, S, A, O> {
            _prepass: P,
            _presentation: L,
            _research: R,
            _sampled: S,
            _lanes: A,
            _observation: O,
        }
        impl<P, L, R, S, A, O> Drop for Installed<P, L, R, S, A, O> {
            fn drop(&mut self) {}
        }
        let presentation = crate::products::politics_ledger::prepared::install_test_presentation(
            self.presentation,
        );
        let prepass = self.prepass.install();
        let observation = install_observation_hook(self.observation_hook);
        let lanes = self.lanes.install();
        let research = crate::products::research::prepared::block::install_forced_fallback(
            self.research_fallback,
        );
        let sampled = sampled::install_test_fallback(self.sampled_fallback);
        super::RELOAD_FLUSHED.with(|flag| flag.set(self.reload));
        super::TEST_BATCH_BLOCKS.with(|blocks| blocks.set(self.batch_blocks));
        self.registry.install();
        Installed {
            _prepass: prepass,
            _presentation: presentation,
            _research: research,
            _sampled: sampled,
            _lanes: lanes,
            _observation: observation,
        }
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

/// How families' commits join groups: as they reach them, unless a test
/// forces a grouping.
fn grouping() -> Grouping {
    #[cfg(test)]
    {
        TEST_GROUPING.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        Grouping::Natural
    }
}

/// Run `run` with the families' commits grouped by `grouping`.
#[cfg(test)]
pub(super) fn with_test_grouping<T>(grouping: Grouping, run: impl FnOnce() -> T) -> T {
    struct Reset(Grouping);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_GROUPING.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_GROUPING.with(|value| value.replace(grouping)));
    run()
}

/// The store commits families prepared on their own published on this
/// thread's preparations since last taken, and the family commits they held.
#[cfg(test)]
pub(super) fn take_test_publications() -> (usize, usize) {
    TEST_PUBLISHED.with(|counts| counts.replace((0, 0)))
}

#[cfg(test)]
type OnPublish = Option<Arc<dyn Fn(&str, usize) + Send + Sync>>;

#[cfg(test)]
thread_local! {
    static TEST_OBSERVATION_HOOK: std::cell::RefCell<Option<lanes::Hook>> = const { std::cell::RefCell::new(None) };
    static TEST_GROUPING: std::cell::Cell<Grouping> = const { std::cell::Cell::new(Grouping::Natural) };
    static TEST_PUBLISHED: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    static TEST_HOURLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_FAMILIES: std::cell::Cell<Option<&'static [&'static dyn Own]>> =
        const { std::cell::Cell::new(None) };
    static TEST_ONE_AFTER_ANOTHER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_NEWEST_FIRST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_ON_PUBLISH: std::cell::RefCell<OnPublish> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn test_observation(
    name: &'static str,
    frame: &DecodedFrame,
    changes: &crate::prepared::frame_changes::FrameChanges,
) -> io::Result<bool> {
    let hook = TEST_OBSERVATION_HOOK.with(|hook| hook.borrow().clone());
    match hook.map(|hook| hook(name, frame, changes)) {
        None | Some(lanes::Fault::Continue) => Ok(true),
        Some(lanes::Fault::Skip) => Ok(false),
        Some(lanes::Fault::Error | lanes::Fault::Disconnect) => {
            Err(io::Error::other("Injected independent observation error"))
        }
        Some(lanes::Fault::Panic) => panic!("Injected independent observation panic"),
    }
}

#[cfg(test)]
fn install_observation_hook(hook: Option<lanes::Hook>) -> impl Drop {
    struct Reset(Option<lanes::Hook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_OBSERVATION_HOOK.with(|value| *value.borrow_mut() = self.0.take());
        }
    }
    Reset(TEST_OBSERVATION_HOOK.with(|value| value.replace(hook)))
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

/// Complete every newest-view side job before admitting the walk. Fixtures
/// needing all branches' pages and diagnostics choose this order explicitly.
#[cfg(test)]
pub(super) fn with_test_newest_first<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_NEWEST_FIRST.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_NEWEST_FIRST.with(|value| value.replace(true)));
    run()
}

/// Observe actual accepted family publications, on the group leader's thread.
#[cfg(test)]
pub(super) fn with_test_on_publish<T>(
    hook: impl Fn(&str, usize) + Send + Sync + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(OnPublish);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ON_PUBLISH.with(|hook| *hook.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_ON_PUBLISH.with(|value| value.replace(Some(Arc::new(hook)))));
    run()
}
