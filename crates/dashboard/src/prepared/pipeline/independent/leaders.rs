//! Unit leaders prepared on their own: the careers, commands and battles the
//! generals leaderboard and general career read. The branch state is the
//! leaders' prepared state, loaded from the continuation and catalogue
//! hourly preparation flushes (`unit-leaders-<branch>`), folded or observed
//! block by block through [`crate::products::unit_leaders::block`] and
//! flushed as hourly preparation flushes it, with the leader content hourly
//! preparation reads (its catalogue, politics content and leader
//! definitions) and the refusals reading it gave. No other family reads the
//! leaders' state in the same hour, so the hourly walk skips them.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::unit_leaders::{
        Content, block,
        prepared::{Metadata, Prepared},
    },
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

/// The leaders' careers, commands and battles.
pub(in crate::prepared::pipeline) struct Leaders;

pub(in crate::prepared::pipeline) static LEADERS: Leaders = Leaders;

impl Own for Leaders {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }

    fn binds(&self) -> bool {
        true
    }

    /// The scan's own tables and the trigger world's core, and the shared
    /// trigger tables leader visibility's declared reads read where the
    /// phase binds a catalogue: without one no leader's visibility is
    /// evaluated. Every hour checks that its evaluations read only what the
    /// block read holds, else the block is observed hour by hour with the
    /// whole trigger kit (`block::bound`).
    fn bound(&self, phase: &super::Phase<'_>) -> Option<prepass::Bound> {
        let content = phase
            .context
            .content()
            .and_then(|options| crate::catalogue::catalogue(phase.source, options))
            .is_ok();
        Some(block::bound(content))
    }

    fn timing(&self) -> &'static str {
        "independent.leaders"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["unit-leaders"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        // A branch it refused is never read again; one it has not committed
        // starts new, as hourly preparation's new branch state does.
        let saved = match load.saved {
            Some(_) => crate::prepared::series::read_saved_catalogue::<Metadata>(
                load.store,
                load.key,
                load.saved,
                "unit-leaders",
                branch,
            )?,
            None => None,
        };
        let prepared = match saved {
            Some((keys, metadata)) => {
                let file = load
                    .read(&format!("unit-leaders-{branch}"))?
                    .ok_or_else(|| {
                        super::super::invalid("Prepared leader continuation is missing")
                    })?;
                Prepared::read(io::BufReader::new(file), keys, metadata)?
            }
            None => Prepared::new(load.replay, branch),
        };
        Ok(Box::new(LeadersBranch {
            prepared: Box::new(prepared),
            content: None,
        }))
    }
}

/// The leader content hourly preparation reads, and why any of it is
/// unavailable.
struct LeaderContent {
    content: Option<Content>,
    errors: Vec<String>,
}

impl LeaderContent {
    fn read(block: &Block<'_>) -> Self {
        let catalogue = block
            .context
            .content()
            .and_then(|options| crate::catalogue::catalogue(block.source, options));
        let mut errors: Vec<_> = catalogue
            .as_ref()
            .err()
            .map(|error| format!("Leader content unavailable: {error}"))
            .into_iter()
            .collect();
        let content = Content::for_catalogue(block.source, block.context, catalogue.ok());
        if let Err(error) = &content {
            errors.push(format!("Leader content unavailable: {error}"));
        }
        Self {
            content: content.ok().flatten(),
            errors,
        }
    }
}

/// One branch's leaders and the content they read.
struct LeadersBranch {
    prepared: Box<Prepared>,
    /// Read at the first block the branch goes through.
    content: Option<LeaderContent>,
}

impl LeadersBranch {
    fn content(&mut self, block: &Block<'_>) -> Option<&Content> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        self.content
            .get_or_insert_with(|| LeaderContent::read(block))
            .content
            .as_ref()
    }
}

impl Branch for LeadersBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.content(block);
        let content = self.content.as_ref().and_then(|read| read.content.as_ref());
        block::fold(
            &mut self.prepared,
            facts,
            block.replay,
            block.index,
            content,
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
        self.content(block);
        let content = self.content.as_ref().and_then(|read| read.content.as_ref());
        block::observe_block(
            &mut self.prepared,
            block.replay,
            block.index,
            None,
            content,
            observed,
            check,
        )
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.prepared.buffered_bytes()
    }

    /// The careers' events, catalogue and continuation, as hourly
    /// preparation's flush writes them.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let read = self
            .content
            .as_ref()
            .ok_or_else(|| io::Error::other("Leader content was not read before the flush"))?;
        self.prepared.flush(
            writer,
            branch,
            read.content.as_ref(),
            &read.errors,
            cancelled,
        )
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
