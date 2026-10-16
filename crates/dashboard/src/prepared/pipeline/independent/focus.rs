//! The national focus order prepared on its own: its blocks read by the
//! fields the focus observation reads, folded on its carried state, its
//! continuation saved under its own name. No other family reads its state in
//! the same hour (the event feed reads its published completions), so the
//! hourly walk skips it. Focus art is evaluated when asked (`focus-art`), so
//! the family reads no content.

use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::prepass::{self, Folded},
    prepared_focus::{FocusIndex, family},
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

/// The focus order, its progress starts and its completions.
pub(in crate::prepared::pipeline) struct Focus;
pub(in crate::prepared::pipeline) static FOCUS: Focus = Focus;

/// The focus continuation, which hourly preparation keeps in its branch
/// state as `index`.
pub(in crate::prepared::pipeline) fn focus_state(branch: u64) -> String {
    format!("focus-state-{branch}")
}

impl Own for Focus {
    fn family(&self) -> &'static prepass::Family {
        &family::FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.focus"
    }

    fn gates(&self) -> &'static [&'static str] {
        &["focus-hour", "focus-unavailable", "focus-order-page"]
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let index = match load.read(&focus_state(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => FocusIndex::default(),
        };
        Ok(Box::new(FocusBranch {
            index: Box::new(index),
        }))
    }
}

/// One branch's focus state and publication.
struct FocusBranch {
    index: Box<FocusIndex>,
}

impl Branch for FocusBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        family::fold(&mut self.index, facts, block.index, observed, check)
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        family::observe_block(&mut self.index, block.replay, block.index, observed, check)
    }

    fn end_block(&mut self, _: &Replay, _: u64) {}

    fn pending(&self) -> usize {
        self.index.buffered_bytes()
    }

    /// The completion events and catalogue, then the focus series and
    /// catalogue, as hourly preparation's flush writes them.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        _: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.index.flush(writer, cancelled)
    }

    fn save(&mut self, _: &mut store::Writer, _: u64) -> io::Result<io::Result<()>> {
        Ok(Ok(()))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &focus_state(branch), &*self.index)
    }
}
