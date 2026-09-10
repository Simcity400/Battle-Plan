//! NFactions::CFactionTemplate, referenced by CFaction's nullable template pointer.
//! Its bound localisation is not a raw key/string; no new interner walk is used.

pub const VTABLE_RVA: usize = 0x0270_4100;
/// CFactionTemplate native u32 LexerToken; D7B4D0 calls 24A96A0/24A7300 to serialize its spelling.
pub const IDENTIFIER_TOKEN: usize = 0x08;
/// Embedded NPdxLoc::CBoundLocalization; 1475D0 calls its constructor 146AF0 here, and D77360 binds it for display.
pub const BOUND_LOCALISATION: usize = 0xd0;
