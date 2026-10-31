//! The preparation driver: the one family registry, the plan selecting the
//! backend of every family ([`plan`]), the publication every family commits
//! through ([`publication`]), and the entry ordinary preparation and its
//! restoration go through. A recording's change-run index is prepared first
//! ([`super::change_runs`]). A recording with a native columnar recording beside
//! it is then prepared from that, every family by its native backend or refused
//! by name ([`native`]); any other is prepared from its rows, in the hourly walk
//! or by a family's row implementation of its own.

pub mod native;
pub(crate) mod plan;
pub(crate) mod publication;
pub mod registry;

use super::{
    Section, State,
    change_runs::{self, Outcome},
    pipeline::{self, Accepted},
};
use crate::{Source, products::ContentOptions};
use std::{io, sync::atomic::AtomicBool};

/// The status row of the campaign map ([`registry::STATUS`]).
const MAP: &str = "map";

/// Prepare every block of `source` its planned publishers have not
/// accepted, reporting each accepted commit to `accepted`.
pub fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    mut accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    let _returned = crate::allocator::ReturnFreed;
    // The index first: the viewer serves the map from it while the families prepare. Native
    // recordings refused for it are the campaign map's refusal alone, named in its status, from
    // the first progress the families report on.
    let refused = change_runs::prepare(source, cancelled)?;
    let mut forward = |done: &Accepted| match &refused {
        Some(reason) => {
            let mut done = done.clone();
            map_refused(&mut done.sections, reason);
            accepted(&done);
        }
        None => accepted(done),
    };
    let mut done = match native::recording(source)? {
        Some(recording) => native::prepare(source, content, recording, cancelled, &mut forward)?,
        None => {
            native::release(source, content)?;
            pipeline::prepare_rows(source, content, cancelled, forward)?
        }
    };
    if let Some(reason) = refused {
        map_refused(&mut done.sections, &reason);
    }
    Ok(done)
}

/// What a complete preparation of `source` accepted, read from its store,
/// the campaign map's status naming why its native recordings were refused
/// while they are as they were; None while any planned publisher has work
/// left, or while its change-run index is neither the index of its native
/// recordings as they are now nor their refusal.
pub fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    let refused = match change_runs::outcome(source)? {
        Outcome::Stale => return Ok(None),
        Outcome::Indexed => None,
        Outcome::Refused(reason) => Some(reason),
    };
    let mut restored = match native::recording(source)? {
        Some(recording) => native::restore(source, content, &recording, cancelled)?,
        None if native::published(source, content)? => None,
        None => pipeline::restore_rows(source, content, cancelled)?,
    };
    if let (Some(accepted), Some(reason)) = (&mut restored, refused) {
        map_refused(&mut accepted.sections, &reason);
    }
    Ok(restored)
}

/// The campaign map's status row in `sections`, failed by its change-run
/// index's refusal for `reason`; every other row is left as it was.
fn map_refused(sections: &mut [Section], reason: &str) {
    for section in sections.iter_mut().filter(|section| section.id == MAP) {
        section.state = State::Failed;
        section.error = Some(format!(
            "The map's change-run index was not prepared: {reason}"
        ));
    }
}

#[cfg(test)]
mod tests;
