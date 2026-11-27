
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

pub use answer::{Answer, Decoded, MAGIC, Message, NO_REASON, State, charge, decode};
pub use evaluate::answer_at;
pub use follow::{Counters, Follow, Held, Sink, StepStats};
pub use parse::parse;
pub use query::{Identity, Query, Rows, SeriesMode};

use std::io;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn table_name(table: usize) -> String {
    replay_data::rows::TableId::ALL.get(table).map_or_else(|| format!("Table {table}"), |id| format!("{id:?}"))
}

fn table_words(table: usize) -> String {
    let words = replay_data::rows::TableId::ALL.get(table).map_or_else(|| format!("table {table}"), |id| id.words());
    let mut chars = words.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
