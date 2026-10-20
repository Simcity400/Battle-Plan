//! One independent diplomacy family; its three input streams never gate each other.
use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::diplomacy_state::{
        block,
        prepared::{self, Prepared},
    },
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};
pub(in crate::prepared::pipeline) struct Diplomacy;
pub(in crate::prepared::pipeline) static DIPLOMACY: Diplomacy = Diplomacy;
impl Own for Diplomacy {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }
    fn timing(&self) -> &'static str {
        "independent.diplomacy_state"
    }
    fn gates(&self) -> &'static [&'static str] {
        &["diplomacy-state"]
    }
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let scan = load
            .read(&prepared::state_section(branch))?
            .map(Prepared::restore)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(State {
            scan: Box::new(scan),
            reached: false,
        }))
    }
}
struct State {
    scan: Box<Prepared>,
    reached: bool,
}
impl Branch for State {
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
        let (folded, reached) = block::fold(&mut self.scan, facts, block.replay, observed, check)?;
        self.reached |= reached.is_some();
        Ok((folded, reached))
    }
    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let reached =
            block::observe_block(&mut self.scan, block.replay, block.index, observed, check)?;
        self.reached |= reached.is_some();
        Ok(reached)
    }
    fn end_block(&mut self, _: &Replay, _: u64) {
        if !self.reached {
            self.scan.break_segment();
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
    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        Ok(self.scan.save(writer, branch))
    }
    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
