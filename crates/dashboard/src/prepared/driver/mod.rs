//! The preparation driver: the one family registry, the plan selecting the
//! backend of every family ([`plan`]), the publication every family commits
//! through ([`publication`]), and the entry ordinary preparation and its
//! restoration go through. Every family is prepared from the recording by its
//! native backend or refused by name ([`native`]).

pub mod native;
pub(crate) mod plan;
pub(crate) mod publication;
pub mod registry;

use super::{Section, State, pipeline::Accepted};
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
    let _returned = engine_heap::ReturnFreed;
    let options = content.map(|content| &content.options);
    source.resolving_ahead(options, cancelled, || -> io::Result<Accepted> {
        // A recording that cannot be read whole is the campaign map's refusal alone, named in
        // its status, from the first progress the families report on.
        let refused = refusal(source);
        let mut forward = |done: &Accepted| match &refused {
            Some(reason) => {
                let mut done = done.clone();
                map_refused(&mut done.sections, reason);
                accepted(&done);
            }
            None => accepted(done),
        };
        loop {
            let timelines = native::timelines(source, content)?;
            let recording = native::recording(source, timelines.as_deref())?;
            let mut done = native::prepare(source, content, recording, cancelled, &mut forward)?;
            if let Some(reason) = &refused {
                map_refused(&mut done.sections, reason);
            }
            if native::timelines(source, content)? == timelines {
                return Ok(done);
            }
        }
    })
}

/// What a complete preparation of `source` accepted, read from its store,
/// the campaign map's status naming why the recording cannot be read whole
/// while it cannot; None while any planned publisher has work left.
pub fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    let refused = refusal(source);
    let timelines = native::timelines(source, content)?;
    let recording = native::recording(source, timelines.as_deref())?;
    let mut restored = native::restore(source, content, &recording, cancelled)?;
    if let (Some(accepted), Some(reason)) = (&mut restored, refused) {
        map_refused(&mut accepted.sections, &reason);
    }
    Ok(restored)
}

/// Why the campaign map cannot read `source`'s recording in place: its damage.
fn refusal(source: &Source) -> Option<String> {
    let damage = source.replay_ref().recording().damage().first()?;
    Some(format!("The recording cannot be read whole: {damage}"))
}

/// The campaign map's status row in `sections`, failed by its refusal for
/// `reason`; every other row is left as it was.
fn map_refused(sections: &mut [Section], reason: &str) {
    for section in sections.iter_mut().filter(|section| section.id == MAP) {
        section.state = State::Failed;
        section.error = Some(format!("The map cannot read its hours: {reason}"));
    }
}

#[cfg(test)]
mod tests;
