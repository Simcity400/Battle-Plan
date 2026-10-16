//! The intelligence family prepared on its own: decryption, operative
//! placements, agency comparison, the operatives roster and the espionage
//! log, one observer. No other family reads its state in the same hour, so
//! the hourly walk skips it. A branch's state loads from the sections hourly
//! preparation's flush saves it in; a block is folded or observed through
//! the product's own module, which finishes the block there and stages its
//! espionage day pages; each commit publishes those pages (also once the
//! branch was refused later in the batch), then the flush writes everything
//! hourly preparation's flush writes, its continuation included.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::intelligence_prepared,
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

pub(in crate::prepared::pipeline) struct Intelligence;

pub(in crate::prepared::pipeline) static INTELLIGENCE: Intelligence = Intelligence;

impl Own for Intelligence {
    fn family(&self) -> &'static prepass::Family {
        &intelligence_prepared::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.intelligence"
    }

    fn gates(&self) -> &'static [&'static str] {
        &[
            "decryption",
            "operative-placements",
            "agency-comparison",
            "operatives-roster",
            "espionage-log",
        ]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let state =
            intelligence_prepared::State::load(&mut |section: &str| load.read(section), branch)?;
        Ok(Box::new(IntelligenceBranch(Box::new(state))))
    }
}

struct IntelligenceBranch(Box<intelligence_prepared::State>);

impl Branch for IntelligenceBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.0.fold(
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
        self.0.observe_block(
            block.replay,
            block.index,
            block.source,
            block.context,
            observed,
            check,
        )
    }

    /// The block was finished where it was folded or observed, its
    /// espionage day pages held for [`Branch::publish`].
    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn publish(&mut self, writer: &mut store::Writer, _: u64, _: &AtomicBool) -> io::Result<()> {
        self.0.publish(writer)
    }

    fn pending(&self) -> usize {
        self.0.pending()
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

    /// The flush wrote the family's pages.
    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    /// The flush wrote the family's continuation.
    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
