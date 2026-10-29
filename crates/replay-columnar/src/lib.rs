//! The columnar change grammar: a recording's tables held as typed columns at the dense
//! slots the writer assigns, changed in place by the hour's column runs, and the writer
//! that compares an hour's rows with them to make those runs.

pub mod envelope;
pub mod framing;
pub mod leaf;
pub mod manifest;
pub mod restart;
pub mod section;
pub mod state;
pub mod walk;
pub mod writer;

use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}
