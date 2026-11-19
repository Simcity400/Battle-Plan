//! Followed dashboard queries (decision L9): a widget follows an `at`-shaped query; its columns
//! join the playhead's selection; after each step a query the step touched is evaluated against
//! the lanes' cells on the stepper's thread, and the answers that changed go out together, one
//! message a step. Nothing is asked because the hour moved.
//!
//! A query's answer at an hour is one function of a lane's cells ([`answer_at`]): the hook runs it
//! on the playhead's lanes at the head, and anyone may run it on a lane sought to another hour.
//! Panels carrying a product's reply ([`panel`]) are shown on the same playheads and pushed on the same pipe.

mod answer;
pub mod lead;
mod evaluate;
mod follow;
pub mod head;
mod live_rows;
pub mod host;
pub mod live;
pub mod panel;
mod parse;
pub mod push;
mod query;
pub mod registry;

#[cfg(test)]
mod series_tests;
#[cfg(test)]
mod tests;

pub use answer::{Answer, Decoded, MAGIC, Message, NO_REASON, State, decode, merge};
pub use evaluate::answer_at;
pub use follow::{Counters, Follow, Held, Sink, StepStats};
pub use parse::parse;
pub use query::{Identity, Query, Rows};

use std::io;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn table_name(table: usize) -> String {
    replay_data::rows::TableId::ALL.get(table).map_or_else(|| format!("Table {table}"), |id| format!("{id:?}"))
}

/// The table in the plain words a user reads, capitalised to start a sentence.
fn table_words(table: usize) -> String {
    let words = replay_data::rows::TableId::ALL.get(table).map_or_else(|| format!("table {table}"), |id| id.words());
    let mut chars = words.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
