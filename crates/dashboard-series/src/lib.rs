#![forbid(unsafe_code)]

pub mod budget;
pub mod cache;
pub mod codec;
mod events;
mod format;
mod hierarchy;
mod layout;
pub mod pages;
pub mod query;
pub mod rank;
mod read;
mod seal;
mod source;
pub mod summary;
pub mod types;
pub mod writer;

pub use budget::Budget;
pub use cache::Cache;
pub use pages::{PageRead, PageWrite, StoreRead, StoreWrite};
pub use query::{Cursor, EventQuery, Selection, Snapshot, Viewport};
pub use seal::{Pending, Sealed, Work};
pub use types::*;
pub use writer::Writer;

use std::io;

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn add(a: u64, b: u64) -> io::Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| invalid("series integer overflow"))
}

#[cfg(test)]
mod tests;
