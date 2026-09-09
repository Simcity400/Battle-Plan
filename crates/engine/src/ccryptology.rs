//! CCryptology, owned by CIntelligenceAgency.

/// CCryptology primary vtable; old build_profile.rs:478, pinned executable RTTI confirms the class.
pub const VTABLE_RVA: usize = 0x029a_6540;
/// CCryptology inline CCountryDecryptionState array data pointer; old build_profile.rs:480 and read_cryptology_rows.
pub const DECRYPTION_STATES: usize = 0x28;
/// CCryptology decryption-state count, old reader u32; old build_profile.rs:481, signedness not independently re-proved.
pub const DECRYPTION_STATE_COUNT: usize = 0x30;
/// CCryptology decryption-state capacity, old reader u32; old build_profile.rs:482 and read_cryptology_rows bounds check.
pub const DECRYPTION_STATE_CAPACITY: usize = 0x34;
