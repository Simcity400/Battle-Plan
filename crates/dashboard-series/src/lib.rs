#![forbid(unsafe_code)]

pub mod carry;
mod chunked;
pub mod codec;
mod events;
pub mod family;
pub mod hash;
mod format;
mod hierarchy;
mod layout;
pub mod memory;
pub mod pages;
pub mod query;
pub mod rank;
mod read;
mod seal;
pub mod shared;
mod source;
pub mod summary;
pub mod types;
pub mod writer;

pub use pages::{PageRead, PageWrite, StoreRead, StoreWrite};
pub use query::{Cursor, EventQuery, Pinned, Selection, Snapshot, Viewport};
pub use seal::{Pending, Sealed, Work};
pub use types::*;
pub use writer::{Tree, Writer};

use std::io;

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn add(a: u64, b: u64) -> io::Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| invalid("series integer overflow"))
}

/// A push that reports a failed allocation rather than aborting.
pub(crate) trait Grow<T> {
    fn try_push(&mut self, value: T) -> io::Result<()>;
}
impl<T> Grow<T> for Vec<T> {
    fn try_push(&mut self, value: T) -> io::Result<()> {
        engine_allocation::reserve(self, 1)?;
        self.push(value);
        Ok(())
    }
}

#[cfg(test)]
mod family_tests;
#[cfg(test)]
mod tests;
