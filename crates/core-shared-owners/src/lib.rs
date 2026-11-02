//! The shared owners' interfaces (architecture design §2.4): the roster, spellings, variant
//! identity and naming, ownership, state names, character and country naming, and the trigger
//! context, each held once per segment and read by every family. Answers are dated, never zero
//! where unknown, and each owner reports the keys whose answers changed, in hour order. `memory`
//! holds in-memory implementations for tests.

mod characters;
mod history;
pub mod memory;
mod places;
mod roster;
mod spellings;
mod triggers;
mod variants;

pub use characters::*;
pub use core_events::{Hour, Interval, Lookup, Missing, MissingKind, Reason, Truth};
pub use history::{Absence, History};
pub use places::*;
pub use roster::*;
pub use spellings::*;
pub use triggers::*;
pub use variants::*;

use std::io;

/// A value and the first hour of its version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dated<T> {
    pub since: Hour,
    pub value: T,
}

/// What every shared owner has: the hours it has applied. Answers borrow the owner's own
/// storage, never the reader's: a reader's list, opaque or text value an owner keeps is copied
/// on the shared-owner line. An answer lives until the owner next applies an hour; a family
/// keeping it longer copies it on its own ceiling.
pub trait Owner {
    /// The last hour applied. A lookup or change query past it is an error, never a guess.
    fn watermark(&self) -> Option<Hour>;
}

/// The error a query past an owner's watermark returns.
pub fn past_watermark() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "shared owner queried past its watermark",
    )
}

#[cfg(test)]
mod tests;
