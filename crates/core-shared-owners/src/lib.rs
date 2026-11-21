
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dated<T> {
    pub since: Hour,
    pub value: T,
}

pub trait Owner {
    fn watermark(&self) -> Option<Hour>;
}

pub fn past_watermark() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "shared owner queried past its watermark",
    )
}

#[cfg(test)]
mod tests;
