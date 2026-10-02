//! CGeneralProductionLine, the offset-zero base of CRailwayProductionLine in pinned RTTI.

/// General production line created date u32; construction lane reads both building and railway lines at +0x58.
pub const CREATED_DATE: usize = 0x58;
/// The CIC bank draw included in a building or railway line's speed (i64 fixed5).
/// (140F59260, 1419F0B20), reserves it from the country's bank and stores it here;
/// the speed at +0x30 includes it, and each advance withdraws it after spending
/// (140F5572A, 1419EF5DE).
pub const CIC_BANK_DRAW: usize = 0x68;
