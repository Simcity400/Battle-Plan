//! CEquipmentMarketStatus, the country market wrapper.

use crate::{Memory, ccountry, read};

/// CEquipmentMarketStatus stockpile pointer.
pub const STOCKPILE: usize = 0x00;
/// Market-manager pointer, authenticated against `cgame_state::MARKET_MANAGER`.
pub const MANAGER: usize = 0x70;

pub(crate) fn verify_manager(
    memory: &impl Memory,
    country: usize,
    manager: usize,
) -> Result<(), &'static str> {
    let status = country
        .checked_add(ccountry::EQUIPMENT_MARKET_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country equipment market status unreadable or null")?;
    if status
        .checked_add(MANAGER)
        .and_then(|p| read::<u64>(memory, p))
        != Some(manager as u64)
    {
        return Err("country equipment market manager mismatch");
    }
    Ok(())
}
