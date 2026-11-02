//! The `ideas` family on the segment driver: each country's held ideas and appointed advisors
//! projected a segment at a time ([`facts`]), kept and published in order ([`emit`]), and read by
//! the idea history widgets and the event feed ([`query`]).

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(test)]
mod tests;

/// The family's name in the registry and its native binding.
pub const NAME: &str = "ideas";
