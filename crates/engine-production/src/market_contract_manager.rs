//! Market contract manager reached through game state.
//! Its two tables use canonical country indices from the country-link map,
//! rather than raw tags.

use crate::{Memory, cgame_state, cpdx_array::Array, read};

/// Owning purchase-contract unique_ptr vector.
pub const OWNED_CONTRACTS: usize = 0;

/// First directional contract-table header.
pub const CONTRACT_TABLE_A: usize = 0x30;
/// Second directional contract-table header.
pub const CONTRACT_TABLE_B: usize = 0x48;
/// Per-country vector stride, indexed by canonical country slot.
pub const COUNTRY_VECTOR_STRIDE: usize = 0x18;

pub(crate) struct Contracts {
    pub pointer: usize,
    pub owned: Array,
    pub sellers: Array,
    pub buyers: Array,
}

impl Contracts {
    pub(crate) fn read(memory: &impl Memory, game_state: usize) -> Result<Self, &'static str> {
        let pointer = game_state
            .checked_add(cgame_state::MARKET_MANAGER)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("market manager unreadable or null")?;
        let owned = pointer
            .checked_add(OWNED_CONTRACTS)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("market owning contract array unreadable or invalid")?;
        let sellers = pointer
            .checked_add(CONTRACT_TABLE_A)
            .and_then(|p| Array::read(memory, p, COUNTRY_VECTOR_STRIDE))
            .ok_or("market seller array unreadable or invalid")?;
        let buyers = pointer
            .checked_add(CONTRACT_TABLE_B)
            .and_then(|p| Array::read(memory, p, COUNTRY_VECTOR_STRIDE))
            .ok_or("market buyer array unreadable or invalid")?;
        Ok(Self {
            pointer,
            owned,
            sellers,
            buyers,
        })
    }

    /// Selects a canonical country's directional vector. The caller checks the
    /// complete vector once for all owned contracts at this endpoint.
    pub(crate) fn country_rows(
        memory: &impl Memory,
        outer: Array,
        slot: u32,
    ) -> Result<Array, &'static str> {
        outer
            .element(slot, COUNTRY_VECTOR_STRIDE)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("market country contract array unreadable or invalid")
    }
}
