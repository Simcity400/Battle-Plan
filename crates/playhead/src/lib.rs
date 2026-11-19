//! The playhead: the state at the playback head of a recording, kept by steps on the unit's
//! reader primitives. A [`lane::Lane`] is one table's selected cells; a [`Playhead`] steps every
//! selected table's lane ahead of the frame and hands out [`Step`]s of owned change sets, and at a
//! reset each reader's own [`State`], made by its [`Rebuild`] from the sought lane.

mod budget;
pub use budget::{ByteBudget, Charge};
pub mod changes;
mod clock;
pub mod lane;
pub mod pages;
mod playhead;
mod ring;
mod state;
mod step;

pub use clock::{Clock, PageTime};
pub use lane::log::{LiveSet, LogState, Ordinal, Read, Rows, Wanted};
pub use playhead::{Asked, Consumer, Counters, Declared, Hook, Lanes, Lead, LeadHeld, LogReads, Piece, Playhead, Reader, Report, Settings, Timeline};
pub use state::{Rebuild, State};
pub use step::{Built, Fetching, Seeking, Step};

use std::io;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
