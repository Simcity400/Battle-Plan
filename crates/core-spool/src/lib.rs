mod budget;
mod dictionary;
pub mod paged;
mod reader;
mod types;
mod writer;

pub use budget::{Budget, OWNER_COUNT, Owner, Reservation};
pub use dictionary::{Dictionary, PayloadBlock, PayloadReader};
pub use reader::{Block, BlockLocation, Hour, Hours, Lanes, Next, Reader};
pub use types::*;
pub use writer::{Allowances, Config, Writer};

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
