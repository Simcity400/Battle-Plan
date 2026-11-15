//! Economy families served from the recording's own tables with no writer: fuel by branch,
//! resource balance, convoy health, the international market and lend-lease balances.

pub mod fuel;
pub mod fuel_branches;
pub mod lend_lease_balance;
pub mod lend_lease_partners;
pub mod market;
pub mod resources;

#[cfg(test)]
mod tests;
