//! CGeneralProductionLine, the offset-zero base of railway production lines.

/// General production line created date u32; construction lane reads both building and railway lines at +0x58.
pub const CREATED_DATE: usize = 0x58;
/// CIC-bank draw included in building or railway line speed, signed i64
/// fixed5. Speed refresh reserves and stores it from the country's bank;
/// the cached speed includes it, and each advance withdraws it after spending.
pub const CIC_BANK_DRAW: usize = 0x68;
