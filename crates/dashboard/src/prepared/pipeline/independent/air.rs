//! The air families prepared on their own: the military air ledger, raids
//! and air aces. Each entry loads a branch's state from the sections hourly
//! preparation saves it in, folds or observes a block through its product's
//! own block module, and flushes as hourly preparation flushes it, which
//! writes its pages and its continuation together.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::{air_aces, military_air_ledger, raids},
};
use replay::container::Replay;
use std::{
    any::Any,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The military air ledger: air losses and bombing damage.
pub(in crate::prepared::pipeline) struct AirLedger;

pub(in crate::prepared::pipeline) static AIR_LEDGER: AirLedger = AirLedger;

impl Own for AirLedger {
    fn family(&self) -> &'static prepass::Family {
        &military_air_ledger::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.air_ledger"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["military-air-ledger"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = if load
            .read(&format!("military-air-ledger-{branch}"))?
            .is_some()
        {
            let mut prepared = military_air_ledger::prepared::Prepared::read(
                load.store, load.key, load.saved, branch,
            )?;
            prepared.read_catalogues(load.store, load.key, load.saved, branch)?;
            prepared
        } else {
            Default::default()
        };
        Ok(Box::new(LedgerBranch(Box::new(prepared))))
    }
}

struct LedgerBranch(Box<military_air_ledger::prepared::Prepared>);

impl Branch for LedgerBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        military_air_ledger::prepared::fold(
            &mut self.0,
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
        military_air_ledger::prepared::observe_block(
            &mut self.0,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.0.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.0.flush(writer, branch, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

/// Raid episodes and tallies, with the raid content hourly preparation binds.
pub(in crate::prepared::pipeline) struct Raids;

pub(in crate::prepared::pipeline) static RAIDS: Raids = Raids;

impl Own for Raids {
    fn family(&self) -> &'static prepass::Family {
        &raids::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.raids"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["raids"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = if load.read(&format!("raids-{branch}"))?.is_some() {
            raids::prepared::Prepared::read(load.store, load.key, load.saved, branch)?
        } else {
            Default::default()
        };
        Ok(Box::new(RaidsBranch {
            prepared: Box::new(prepared),
            content: None,
        }))
    }
}

struct RaidsBranch {
    prepared: Box<raids::prepared::Prepared>,
    /// The raid content, read once from the source's cache as hourly
    /// preparation reads it once per run; None when it is unavailable.
    content: Option<Option<Arc<raids::Content>>>,
}

impl RaidsBranch {
    fn content(&mut self, block: &Block<'_>) -> Option<Arc<raids::Content>> {
        self.content
            .get_or_insert_with(|| {
                raids::prepared_content(block.source, block.context)
                    .ok()
                    .and_then(|(content, _)| content)
            })
            .clone()
    }
}

impl Branch for RaidsBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let content = self.content(block);
        raids::prepared::fold(
            &mut self.prepared,
            content.as_deref(),
            facts,
            block.replay,
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
        let content = self.content(block);
        raids::prepared::observe_block(
            &mut self.prepared,
            content.as_deref(),
            block.replay,
            block.index,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.prepared.bytes()
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

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

/// Air aces, their windows and portraits.
pub(in crate::prepared::pipeline) struct Aces;

pub(in crate::prepared::pipeline) static ACES: Aces = Aces;

impl Own for Aces {
    fn family(&self) -> &'static prepass::Family {
        &air_aces::prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.aces"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["air-aces", "air-ace-portraits"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = if load.read(&format!("air-aces-{branch}"))?.is_some() {
            air_aces::prepared::Prepared::resume(
                load.store,
                load.key,
                load.saved,
                load.replay,
                branch,
            )?
        } else {
            Default::default()
        };
        Ok(Box::new(AcesBranch(Box::new(prepared))))
    }
}

struct AcesBranch(Box<air_aces::prepared::Prepared>);

impl Branch for AcesBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        air_aces::prepared::fold(
            &mut self.0,
            facts,
            block.source,
            block.replay,
            block.index,
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
        air_aces::prepared::observe_block(
            &mut self.0,
            block.source,
            block.replay,
            block.index,
            block.context,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    /// The windows of the blocks it went through since the last commit,
    /// which hourly preparation writes at each block's end.
    fn publish(&mut self, writer: &mut store::Writer, _: u64, _: &AtomicBool) -> io::Result<()> {
        self.0.publish_windows(writer)
    }

    fn pending(&self) -> usize {
        self.0.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.0.flush(writer, branch, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
