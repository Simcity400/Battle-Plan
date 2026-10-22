//! World tension prepared on its own. Its branch state is the tension
//! observer, loaded from the catalogue hourly preparation publishes it in,
//! folded or observed block by block through
//! [`crate::products::world_tension::block`], and flushed as hourly
//! preparation flushes it, which writes its series and its continuation
//! together.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::world_tension::{Scan, block},
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

/// World tension: reads, contributions and threat events.
pub(in crate::prepared::pipeline) struct Tension;

pub(in crate::prepared::pipeline) static TENSION: Tension = Tension;

impl Own for Tension {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.tension"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["world-tension"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        // A branch it refused is never read again; one it has not committed
        // starts new, as hourly preparation's new branch state does.
        let scan = match load.saved {
            Some(_) => crate::prepared::series::read_saved_catalogue::<Scan>(
                load.store,
                load.key,
                load.saved,
                "world-tension",
                branch,
            )?
            .map(|(_, scan)| scan),
            None => None,
        };
        Ok(Box::new(TensionBranch {
            scan: Box::new(scan.unwrap_or_default()),
            reached: false,
        }))
    }
}

struct TensionBranch {
    scan: Box<Scan>,
    /// Whether the block it goes through gave it an hour.
    reached: bool,
}

impl Branch for TensionBranch {
    fn begin_block(&mut self) {
        self.reached = false;
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let (folded, reached) = block::fold(
            &mut self.scan,
            facts,
            block.replay,
            block.index,
            observed,
            check,
        )?;
        self.reached |= reached.is_some();
        Ok((folded, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let reached = block::observe_block(
            &mut self.scan,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )?;
        self.reached |= reached.is_some();
        Ok(reached)
    }

    /// A block that gave no hour ends the observer's segment, as hourly
    /// preparation ends it after a block it read no frame of.
    fn end_block(&mut self, _: &Replay, _: u64) {
        if !self.reached {
            self.scan.unreadable_block();
        }
    }

    fn pending(&self) -> usize {
        self.scan.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.flush(writer, branch, cancelled)
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
