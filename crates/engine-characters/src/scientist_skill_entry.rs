//! Scientist specialization records reached through the scientist's specialization vector. Its
//! count determines the population and order. Each 0x20-byte record contains a u32 token, signed
//! i64 experience and signed i32 level. Experience and level are serialized only while strictly
//! positive. Reporting token values never gate rows or impose an ordering.

/// Specialization record stride.
pub const STRIDE: usize = 0x20;
/// Full specialization token, u32.
pub const TOKEN: usize = 0x00;
/// Signed i64 experience; serialized only when positive.
pub const EXPERIENCE: usize = 0x10;
/// Signed i32 level; serialized only when positive.
pub const LEVEL: usize = 0x18;
/// Nuclear specialization token low half, for reporting only.
#[cfg(test)]
pub const NUCLEAR_TOKEN_PREFIX: u16 = 0x56c6;
/// Land specialization token low half, for reporting only.
#[cfg(test)]
pub const LAND_TOKEN_PREFIX: u16 = 0x56c9;
