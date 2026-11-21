
pub mod block;
pub mod container;
pub mod envelope;
pub mod lane;
pub mod leaf;
pub mod manifest;
pub mod pages;
pub mod restart;
pub mod section;
pub mod state;
pub mod store;
pub mod summary;
pub mod unit;
pub mod walk;
pub mod writer;

use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn invalid_static(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        invalid(what)
    }
}
