//! CBuildingTemplate definitions; lexer type id, database ordinal and localized names are distinct.

/// Real-definition allocation size.
pub const SIZE: usize = 0x510;
/// Primary template vtable.
pub const VTABLE_RVA: usize = 0x027c_dd90;
/// Signed naval, military, general and rocket production words, reduced with wrapping i32 arithmetic.
pub const WAR_OVERVIEW_PRODUCTION: [usize; 4] = [0x2f8, 0x2fc, 0x300, 0x30c];
/// Lexer token id, u32.
pub const TYPE_ID: usize = 0x08;
/// Railway template token selecting the database's optional railway definition.
pub const RAIL_WAY_TOKEN: u32 = 0x4cc1;
/// Lookup-enabling byte; nonzero is required and its full meaning is unknown.
pub const LOOKUP_ENABLED_CANDIDATE: usize = 0x20;
/// Localized singular name string.
pub const SINGULAR_NAME: usize = 0x210;
/// Localized plural name string.
pub const PLURAL_NAME: usize = 0x230;
/// Raw script-key string before localization.
pub const SCRIPT_KEY: usize = 0x270;
/// Case-folded script-key hash, u32.
pub const SCRIPT_KEY_HASH: usize = 0x290;
/// Scope field; values below one select province, otherwise state.
pub const SCOPE: usize = 0x2bc;
/// Loader element ordinal, independent of the lexer token.
pub const DATABASE_ORDINAL: usize = 0x2e0;
/// Port byte, written for each parsed occurrence.
pub const IS_PORT: usize = 0x361;
/// Movement fort weight, i32, multiplied by each building's healthy level before the fort test.
pub const MOVEMENT_FORT_WEIGHT: usize = 0x308;
/// Real-definition constructor.
pub const CONSTRUCTOR_RVA: usize = 0x014a_4470;
/// Null-object constructor with empty names.
pub const NULL_CONSTRUCTOR_RVA: usize = 0x0067_4c60;
/// Singular/plural display-name getter.
pub const GET_DISPLAY_NAME_RVA: usize = 0x014a_7570;
/// Port-flag parser.
pub const READ_IS_PORT_RVA: usize = 0x014a_9220;
