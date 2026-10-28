//! The columnar change grammar's reader: a recording's tables held as typed columns at
//! the dense slots the writer assigns, changed in place by the hour's column runs.

pub mod framing;
pub mod index;
pub mod leaf;
pub mod manifest;
pub mod restart;
pub mod section;
pub mod state;
pub mod walk;

use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}
