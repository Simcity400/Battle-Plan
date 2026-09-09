//! Static keyword-registration functions used by the engine's trigger grammar.
//! These function addresses are retained without inventing a C++ registry class.

pub const REGISTER_STATIC_KEYWORDS_RVA: usize = 0x0008_2cb0;
/// Keyword registration call target; ability_visibility.rs:75 and archived key-binding disassembly identify 1424AFC10.
pub const REGISTER_KEYWORD_RVA: usize = 0x024a_fc10;
