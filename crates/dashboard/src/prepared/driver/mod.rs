//! The preparation driver: the one family registry, the plan selecting the
//! backend of every family ([`plan::current`]), and the entry ordinary
//! preparation and its restoration go through. Every backend is a row
//! backend: the hourly walk, or a family's row implementation of its own.

pub(crate) mod plan;
pub mod registry;

use super::pipeline::{self, Accepted};
use crate::{Source, products::ContentOptions};
use std::{io, sync::atomic::AtomicBool};

/// Prepare every block of `source` its planned publishers have not
/// accepted, reporting each accepted commit to `accepted`.
pub fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    pipeline::prepare_rows(source, content, cancelled, accepted)
}

/// What a complete preparation of `source` accepted, read from its store;
/// None while any planned publisher has work left.
pub fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    pipeline::restore_rows(source, content, cancelled)
}

#[cfg(test)]
mod tests;
