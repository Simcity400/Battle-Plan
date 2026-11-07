//! The preparation driver: the one family registry, the plan selecting the
//! backend of every family ([`plan`]), the publication every family commits
//! through ([`publication`]), and the entry ordinary preparation and its
//! restoration go through. A recording's change-run index ([`super::change_runs`])
//! is written by a lane of the families' union run over the recording it indexes,
//! or else prepared first. Every family is prepared by its native backend or refused
//! by name ([`native`]).

pub mod native;
pub(crate) mod plan;
pub(crate) mod publication;
pub mod registry;

use super::{
    Section, State,
    change_runs::{self, Outcome},
    pipeline::Accepted,
};
use crate::{Source, products::ContentOptions};
use std::{io, sync::atomic::AtomicBool};

/// The status row of the campaign map ([`registry::STATUS`]).
const MAP: &str = "map";

/// Prepare every block of `source` its planned publishers have not
/// accepted, reporting each accepted commit to `accepted`. A timeline a query
/// asked for while it ran is prepared before it ends ([`native::timelines`]).
pub fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    mut accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    let _returned = crate::allocator::ReturnFreed;
    // The families' content resolves beside the index, which reads none of it.
    let options = content.map(|content| &content.options);
    source.resolving_ahead(options, cancelled, || -> io::Result<Accepted> {
        loop {
            // Native recordings refused for the index are the campaign map's refusal alone,
            // named in its status, from the first progress the families report on.
            let index = change_runs::plan(source)?;
            let refused = index.refused().map(str::to_owned);
            let mut forward = |done: &Accepted| match &refused {
                Some(reason) => {
                    let mut done = done.clone();
                    map_refused(&mut done.sections, reason);
                    accepted(&done);
                }
                None => accepted(done),
            };
            let timelines = native::timelines(source, content)?;
            let recording = native::recording(source, timelines.as_deref())?;
            // An index of the recording the families walk is one more lane of their union run:
            // the recording is walked once. Any other is prepared first, in runs of its own.
            let (riding, _) = index.take(Some(&recording), cancelled)?;
            let mut done =
                native::prepare(source, content, recording, riding, cancelled, &mut forward)?;
            if let Some(reason) = refused {
                map_refused(&mut done.sections, &reason);
            }
            if native::timelines(source, content)? == timelines {
                return Ok(done);
            }
        }
    })
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
    let timelines = native::timelines(source, content)?;
    let recording = native::recording(source, timelines.as_deref())?;
    let mut restored = native::restore(source, content, &recording, cancelled)?;
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
