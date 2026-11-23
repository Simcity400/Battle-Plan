
pub mod boundary;
mod budget;
pub use budget::{ByteBudget, Charge};
pub mod changes;
#[cfg(any(test, feature = "bench"))]
pub mod check;
mod clock;
pub mod lane;
pub mod pages;
mod playhead;
mod ring;
mod state;
mod step;

pub use boundary::Crossed;
pub use clock::{Clock, FRAME_RATES, PageTime, frame_period};
pub use lane::log::{LiveSet, LogState, Ordinal, Read, Rows, Wanted};
pub use playhead::{Asked, Consumer, Counters, Declared, Hook, Lanes, Lead, LeadHeld, LogReads, Piece, Playhead, Reader, Report, Settings, Thread, Timeline};
pub use state::{Rebuild, State};
pub use step::{Built, Fetching, Seeking, Step};

use std::io;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
