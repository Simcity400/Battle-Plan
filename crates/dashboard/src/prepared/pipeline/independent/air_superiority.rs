//! Air superiority prepared on its own, with its own read of the country
//! groups ([`crate::products::air_superiority::block`]). Its branch state is
//! loaded from the sections hourly preparation saves it in (its continuation
//! and catalogue) and from its own read of the groups, which hourly
//! preparation keeps as the groups' published scan and its branch state's
//! change publisher. Registering it ends the hourly walk's read of the groups
//! for it (`Groups::read_by`): with the groups prepared on their own too, the
//! walk no longer observes them.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    products::{
        air_superiority::{block, prepared::Prepared},
        country_groups::{Scan as Groups, changes::Prepared as Changes},
    },
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

/// Air superiority: regional scores, wings, theatres and daily side choices.
pub(in crate::prepared::pipeline) struct AirSuperiority;

pub(in crate::prepared::pipeline) static AIR_SUPERIORITY: AirSuperiority = AirSuperiority;

impl Own for AirSuperiority {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.air_superiority"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["air-superiority"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let prepared = if load.read(&format!("air-superiority-{branch}"))?.is_some() {
            Prepared::read(load.store, load.key, load.saved, branch)?
        } else {
            Prepared::default()
        };
        let groups = match load.read(&block::groups_section(branch))? {
            Some(file) => Groups::read(io::BufReader::new(file))?,
            None => Groups::new(load.replay, branch),
        };
        let changes: Changes = match load.read(&block::state_section(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => Changes::default(),
        };
        Ok(Box::new(SuperiorityBranch(block::OwnBranch::new(
            prepared, groups, changes,
        ))))
    }
}

struct SuperiorityBranch(block::OwnBranch);

/// The outcome of the family's own groups observation at `hour`, with the
/// refusals a test injects for the groups there, as the hourly walk's groups
/// observation finds them.
fn groups_outcome(hour: i32, result: io::Result<()>) -> io::Result<()> {
    #[cfg(test)]
    let result = super::super::lanes::inject("groups", hour, result);
    #[cfg(not(test))]
    let _ = hour;
    result
}

impl Branch for SuperiorityBranch {
    fn begin_block(&mut self) {
        self.0.begin_block();
    }

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
            &mut groups_outcome,
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
            None,
            &mut groups_outcome,
            observed,
            check,
        )
    }

    /// The block's end is folded with the block, where its publication's
    /// refusal is recorded at the block's last hour.
    fn pending(&self) -> usize {
        self.0.pending()
    }

    fn measure(&mut self) {
        self.0.measure();
    }

    fn parts(&mut self) -> Vec<(&'static str, std::time::Duration, u64)> {
        self.0.take_parts()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.0.flush(writer, replay, branch, cancelled)
    }

    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        self.0.save(writer, branch)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        self.0.save_state(writer, branch)
    }
}
