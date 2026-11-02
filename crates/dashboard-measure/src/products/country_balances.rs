//! Political and command power (series family `country-measure`): each tag's recorded fixed5
//! balances, projected from the native walk ([`facts`]), kept as runs with their holder and each
//! net drop as an event ([`emit`]), and read back as the prepared series replies and the
//! scoreboard's cells ([`query`]).

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(test)]
mod columnar_tests;

/// The family's name in the registry and its native binding.
pub const NAME: &str = "measure";
