//! CBuildingDatabase loaded definitions; these layout constants do not prove lifetime or reload stability.

/// Building-database singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_6a58;
/// Template-pointer array appending each parsed definition.
pub const TEMPLATES: usize = 0x40;
/// Template-pointer stride.
#[cfg(test)]
pub const TEMPLATE_STRIDE: usize = 8;
/// Parallel key-record vector; key index i corresponds to template index i plus one.
pub const KEYS: usize = 0x58;
/// Infrastructure template token; its spelling comes from the recorded lexical
/// dictionary. A missing matching template remains absent.
pub const INFRASTRUCTURE_TOKEN: u32 = 0x2fab;
/// Railway template token; its spelling comes from recorded lexical tokens.
/// Province buildings of this type retain railway levels; the matching
/// template may be absent.
pub const RAILWAY_TEMPLATE_TOKEN: u32 = 0x4cc1;
