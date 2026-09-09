//! CCountryLeader role object, reached through CCharacter's ideology map or a political party vector.

/// CCountryLeader vtable; build_profile.rs:192 and the pinned executable's MSVC RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x0296_a720;
/// CCountryLeader parent CCharacter*; extractor.rs:113 and read_vtable_and_character_ptr validate this link.
pub const CHARACTER: usize = 0x08;
