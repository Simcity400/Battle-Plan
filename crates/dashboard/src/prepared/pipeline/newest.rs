//! Newest block windows first. Before forward preparation, a family whose
//! default view is its branch's newest block window publishes that window
//! from the branch's last block and a look-back proven to hold every fact the
//! window's page carries, under names of its own. Its query answers from it
//! until forward preparation covers the branch, which then publishes the same
//! page bytes for the window. Land battles publish theirs here
//! ([`land_battles::newest`]).

use super::{Accepted, Checkpoint, FamilyTimings};
use crate::{
    Source,
    metadata::check_cancel,
    products::{LoadContext, land_battles},
};
use replay::container::Replay;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// The land battle family's name in the checkpoint's refusals.
const LAND: &str = "land";

/// What the newest windows are prepared from.
pub(super) struct Phase<'a> {
    pub source: &'a Source,
    pub replay: &'a Replay,
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    pub context: &'a LoadContext,
    pub map: &'a dashboard_base::prepared_map::Builder,
    pub last_blocks: &'a BTreeMap<u64, usize>,
    pub cancelled: &'a AtomicBool,
}

/// Publishes, in one commit, the newest land battle window of each branch
/// whose last block forward preparation has neither accepted nor refused, and
/// for whose current last block no earlier preparation published one.
/// Returns the open writer and whether it committed.
pub(super) fn prepare(
    phase: &Phase<'_>,
    mut writer: store::Writer,
    state: &mut Checkpoint,
    saved: &mut Option<store::SourceKey>,
    bytes: &mut u64,
    timings: &mut Option<BTreeMap<&'static str, FamilyTimings>>,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<(store::Writer, bool)> {
    let started = super::start_timing(timings);
    let mut tried = false;
    let mut written = false;
    for (&branch, &last) in phase.last_blocks {
        check_cancel(phase.cancelled)?;
        if state.cursor.next_block > last
            || state
                .refused
                .get(&branch)
                .is_some_and(|refused| !refused.open(LAND))
        {
            continue;
        }
        if saved.as_ref() == Some(phase.key)
            && land_battles::newest::published(phase.store, phase.key, phase.replay, branch)?
        {
            continue;
        }
        tried = true;
        if let Some(newest) =
            land_battles::newest::prepare(phase.source, branch, phase.context, phase.cancelled)?
        {
            newest.write(&mut writer)?;
            written = true;
        }
    }
    if tried {
        super::finish_timing(timings, started, "newest.land", false);
    }
    if !written {
        return Ok((writer, false));
    }
    *bytes = super::write_checkpoint(&mut writer, state, phase.replay, phase.map, *bytes)?;
    check_cancel(phase.cancelled)?;
    writer.commit()?;
    *saved = Some(phase.key.clone());
    let writer = phase.store.extend(phase.key)?;
    accepted(&Accepted {
        sections: super::preparing_sections(
            state,
            state.cursor.next_block,
            phase.replay.index.len(),
        ),
        bytes: *bytes,
        next_block: state.cursor.next_block,
        new_blocks: 0,
        coverage: super::accepted_coverage(&state.map, phase.last_blocks),
    });
    Ok((writer, true))
}
