//! The preparation driver: the one family registry, the plan selecting the
//! backend of every family ([`plan::current`]), the publication every family
//! commits through ([`publication`]), and the entry ordinary preparation and
//! its restoration go through. Every backend is a row backend: the hourly
//! walk, or a family's row implementation of its own. A recording with a native
//! columnar recording also has its change-run index prepared ([`super::change_runs`]).

pub(crate) mod plan;
// The segment driver commits through it; ordinary preparation is still the row walk's.
#[cfg_attr(not(test), allow(dead_code))]
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
    accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    // The index first: the viewer serves the map from it while the families prepare. Native
    // recordings refused for it are the campaign map's refusal alone, named in its status.
    let refused = change_runs::prepare(source, cancelled)?;
    let mut accepted = pipeline::prepare_rows(source, content, cancelled, accepted)?;
    if let Some(reason) = refused {
        map_refused(&mut accepted.sections, &reason);
    }
    Ok(accepted)
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
    let mut restored = pipeline::restore_rows(source, content, cancelled)?;
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
