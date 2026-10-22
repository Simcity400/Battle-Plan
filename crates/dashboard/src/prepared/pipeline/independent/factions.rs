//! One independent publication/continuation for all faction history views.
use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::faction_state::{block, prepared},
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};
pub(in crate::prepared::pipeline) struct Factions;
pub(in crate::prepared::pipeline) static FACTIONS: Factions = Factions;
impl Own for Factions {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }
    fn timing(&self) -> &'static str {
        "independent.faction_state"
    }
    fn gates(&self) -> &'static [&'static str] {
        &["faction-state"]
    }
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let state = match load.read(&prepared::state_section(branch))? {
            Some(file) => {
                serde_json::from_reader(io::BufReader::new(zstd::stream::Decoder::new(file)?))?
            }
            None => prepared::Prepared::default(),
        };
        Ok(Box::new(FactionBranch {
            state: Box::new(state),
        }))
    }
}
struct FactionBranch {
    state: Box<prepared::Prepared>,
}
impl Branch for FactionBranch {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        block::fold(
            &mut self.state,
            facts,
            block.source,
            block.context,
            block.replay,
            block.index,
            observed,
            check,
            block.cancelled,
        )
    }
    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        block::observe_block(
            &mut self.state,
            block.source,
            block.context,
            block.replay,
            block.index,
            None,
            observed,
            check,
            block.cancelled,
        )
    }
    fn pending(&self) -> usize {
        self.state.bytes()
    }
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.state.flush(writer, branch, cancelled)
    }
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        Ok(self.state.save(writer, branch))
    }
    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
