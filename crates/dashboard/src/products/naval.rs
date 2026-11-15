//! Naval battles, ships sunk and naval losses (family 50), read from the recording's own tables:
//! the recorder's NavalBattles, NavalResultObservations and NavalLossTotals beside the game's
//! combats, results and sinking list.

pub mod battles;
pub mod losses;
pub mod sunk;

#[cfg(test)]
mod tests;
